#!/usr/bin/env python3
"""Opt-in IAM testing smoke: authentication and local data reads, no provider sessions.

Required: SB_TEST_BACKEND (Browser root URL), SB_TEST_APP_SECRET,
SB_TEST_ACTOR (an existing test actor ID), SB_TEST_ORG.
Optional: SB_TEST_CLI, SB_IAM_TEST_KEY, SB_BRIEFCASE_TEST_KEY.
No environments, profiles, sessions, or recording grants are created. Test
actor login creates fresh IAM token families and refresh rotates one family.
Credentials stay in memory and an automatically removed private CLI home.
"""

import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid

from test_live_auth import NoRedirect


class FailedCheck(Exception):
    pass


def main():
    required = ("SB_TEST_BACKEND", "SB_TEST_APP_SECRET", "SB_TEST_ACTOR", "SB_TEST_ORG")
    if missing := [name for name in required if not os.environ.get(name)]:
        print("Missing configuration: " + ", ".join(missing), file=sys.stderr)
        return 1
    backend = os.environ["SB_TEST_BACKEND"].rstrip("/")
    actor, org = os.environ["SB_TEST_ACTOR"], os.environ["SB_TEST_ORG"]
    parsed = urllib.parse.urlsplit(backend)
    try:
        loopback = ipaddress.ip_address(parsed.hostname).is_loopback
    except ValueError:
        loopback = parsed.hostname == "localhost"
    if (parsed.scheme != "https" and not (parsed.scheme == "http" and loopback)) or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("invalid backend")
    if "/testing/" in parsed.path or parsed.path.endswith("/testing"):
        raise ValueError("backend must be a root URL")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+(?::[A-Za-z0-9_.-]+)?", actor) or len(actor) > 256 or actor.startswith(("ask_", "oac_", "oat_", "ort_", "iat_", "irt_", "cat_", "sat_", "crt_", "srt_")):
        raise ValueError("SB_TEST_ACTOR must be an actor ID")
    credentials = {"app_secret": os.environ["SB_TEST_APP_SECRET"]}
    for variable, field in (("SB_IAM_TEST_KEY", "iam_test_key"), ("SB_BRIEFCASE_TEST_KEY", "briefcase_test_environment_key")):
        if variable in os.environ:
            value = os.environ[variable]
            if not re.fullmatch(r"[A-Za-z0-9]{32}", value):
                raise ValueError("invalid optional key")
            credentials[field] = value
    secret = credentials["app_secret"]
    if not secret.startswith("ask_") or not 4 < len(secret) <= 16384 or any(c.isspace() or ord(c) < 32 or ord(c) == 127 for c in secret):
        raise ValueError("invalid app secret")
    opener = urllib.request.build_opener(NoRedirect())
    passed = 0

    def check(label, condition):
        nonlocal passed
        print(f"{'PASS' if condition else 'FAIL'} {label}", flush=True)
        if not condition:
            raise FailedCheck()
        passed += 1

    def request(label, path, *, token=None, body=None, testing=True, expected=200):
        headers = {"Content-Type": "application/json", "X-Org-ID": org}
        if testing:
            headers["x-sb-test-app-secret"] = credentials["app_secret"]
            for field, header in (("iam_test_key", "x-testing-environment-key"), ("briefcase_test_environment_key", "x-sb-test-briefcase-key")):
                if field in credentials:
                    headers[header] = credentials[field]
        if token:
            headers["Authorization"] = "Bearer " + token
        req = urllib.request.Request(backend + path, headers=headers, data=None if body is None else json.dumps(body).encode())
        try:
            response = opener.open(req, timeout=60)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status = response.code
            raw = response.read(2 * 1024 * 1024 + 1)
        check(f"{label} (HTTP {status})", status == expected and len(raw) <= 2 * 1024 * 1024)
        # Response and exception bodies may contain credentials; never print them.
        payload = json.loads(raw)
        if expected == 200:
            check(label + " response envelope", isinstance(payload, dict) and "data" in payload)
            return payload["data"]
        return payload

    context = request("IAM context", "/api/v1/testing/context", body=credentials, testing=False)
    environment = str(uuid.UUID(context["environment_id"]))
    check("IAM context identifies a nonempty test environment", environment != str(uuid.UUID(int=0)) and bool(context["app_id"]))
    prefix = f"/testing/{environment}/api/v1"
    exchange = {"short_lived_token": actor, "org_id": org}
    first = request("first actor login", prefix + "/auth/exchange", body=exchange)
    second = request("second actor login", prefix + "/auth/exchange", body=exchange)
    for session in (first, second):
        check("test login returns bound IAM credentials", session["access_token"].startswith("oat_") and session["refresh_token"].startswith("ort_") and session["org"]["id"] == org)
    check("actor logins create distinct token families", first["access_token"] != second["access_token"] and first["refresh_token"] != second["refresh_token"])
    token = first["access_token"]
    identity = request("identity introspection", prefix + "/me", token=token)
    check("introspection matches exchanged identity", identity["id"] == first["identity"]["id"])
    orgs = request("organization discovery", prefix + "/orgs", token=token)
    check("discovery includes selected organization", any(item["id"] == org for item in orgs))
    for path in ("services", "profiles", "sessions", "recordings", "usage"):
        listings = request(path + " listing", prefix + "/" + path, token=token)
        check(path + " listing shape", isinstance(listings, list))
    rotated = request("refresh test token family", prefix + "/auth/refresh", body={"refresh_token": first["refresh_token"], "org_id": org})
    check("refresh rotates credentials within the same identity", rotated["access_token"] != token and rotated["refresh_token"] != first["refresh_token"] and rotated["identity"]["id"] == identity["id"] and rotated["org"]["id"] == org)
    current = request("refreshed identity introspection", prefix + "/me", token=rotated["access_token"])
    check("refreshed identity is unchanged", current["id"] == identity["id"])
    request("missing test enrollment rejected", prefix + "/me", token=token, testing=False, expected=401)
    other = str(uuid.uuid4())
    while other == environment:
        other = str(uuid.uuid4())
    request("different test environment rejected", f"/testing/{other}/api/v1/me", token=token, expected=403)
    request("test credentials rejected at production route", "/api/v1/me", token=token, expected=400)

    if binary := os.environ.get("SB_TEST_CLI"):
        binary = str(Path(binary).expanduser().resolve())
        with tempfile.TemporaryDirectory(prefix="sb-iam-testing-") as home:
            env = {key: value for key, value in os.environ.items() if not key.startswith("SB_")}
            env["SB_HOME"] = str(Path(home) / "browser")

            def cli(label, arguments, *, selected=True, input=None):
                command = [binary, "--backend", backend, "--json"]
                if selected:
                    command += ["--test", environment]
                result = subprocess.run(command + arguments, input=input, text=True, env=env, capture_output=True, timeout=60)
                check(f"CLI {label} (exit {result.returncode})", result.returncode == 0)
                return json.loads(result.stdout)

            enrolled = cli("enrollment", ["testing", "login", "--credentials-stdin"], selected=False, input=json.dumps(credentials))
            check("CLI enrolled the verified environment", enrolled["configured"] and enrolled["environment"]["environment_id"] == environment)
            enrolled = cli("testing status", ["testing", "status"])
            check("CLI stored configuration verifies", enrolled["configured"] and enrolled["environment"]["environment_id"] == environment)
            authenticated = cli("test actor login", ["--org-id", org, "login", actor])
            check("CLI actor login authenticated", authenticated["authenticated"])
            status = cli("test login status", ["--org-id", org, "login", "status"])
            check("CLI test session remains authenticated", status["authenticated"])
            check("CLI profile listing shape", isinstance(cli("profile listing", ["--org-id", org, "profile", "ls"]), list))
            status = cli("production login status", ["login", "status"], selected=False)
            check("CLI production credentials remain empty", not status["authenticated"])
        check("temporary CLI credentials removed", not Path(home).exists())
    else:
        print("SKIP CLI smoke: SB_TEST_CLI is not set", flush=True)
    print(f"{passed} IAM testing checks passed; no browser/provider sessions created")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except FailedCheck:
        sys.exit(1)
    except (KeyError, TypeError, ValueError, OSError, subprocess.SubprocessError):
        print("FAIL IAM testing smoke: check configuration, backend availability, and the JSON contract; response bodies and credentials are withheld", file=sys.stderr)
        sys.exit(1)
