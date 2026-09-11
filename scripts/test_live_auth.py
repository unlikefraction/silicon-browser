#!/usr/bin/env python3
"""Opt-in live auth contract checks; no browser/provider sessions are created.

Run against a backend configured with IAM_TEST_ENVIRONMENT_KEY. Supply a fresh
IAM SLT authorized for the test organization in SB_TEST_SLT, its org in
SB_TEST_ORG, and the backend URL in
SB_TEST_BACKEND. Optional SB_TEST_CLI points to an sb binary. Credentials remain
in memory; the CLI uses a temporary home and an invocation-only bearer override.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def main():
    required = ("SB_TEST_BACKEND", "SB_TEST_ORG", "SB_TEST_SLT")
    if missing := [name for name in required if not os.environ.get(name)]:
        print("Missing configuration: " + ", ".join(missing), file=sys.stderr)
        return 1
    backend = os.environ["SB_TEST_BACKEND"].rstrip("/")
    org = os.environ["SB_TEST_ORG"]
    slt = os.environ["SB_TEST_SLT"]
    url = urllib.parse.urlsplit(backend)
    if (url.scheme != "https" and not (
        url.scheme == "http" and url.hostname in {"localhost", "127.0.0.1", "::1"}
    )) or not url.hostname or url.username or url.password or url.query or url.fragment:
        raise ValueError("SB_TEST_BACKEND must be HTTPS or loopback HTTP without credentials")
    opener = urllib.request.build_opener(NoRedirect())
    results = []

    def check(label, condition):
        results.append(bool(condition))
        print(f"{'PASS' if condition else 'FAIL'} {label}", flush=True)

    def request(label, path, *, token=None, scope=org, body=None, expected=200):
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if scope is not None:
            headers["X-Org-ID"] = scope
        req = urllib.request.Request(
            backend + path, headers=headers,
            data=None if body is None else json.dumps(body).encode(),
        )
        start = time.monotonic()
        try:
            response = opener.open(req, timeout=40)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            status = response.code
            raw = response.read(2 * 1024 * 1024)
        check(f"{label} (HTTP {status}, {time.monotonic()-start:.3f}s)", status == expected)
        # Never print response bodies: successful authentication contains secrets.
        data = json.loads(raw)
        return data.get("data", data)

    request("health", "/healthz")
    session = request("SLT exchange", "/api/v1/auth/exchange", body={
        "short_lived_token": slt, "org_id": org,
    })
    if "access_token" not in session:
        return 1
    retry = request("SLT recovery", "/api/v1/auth/exchange", body={
        "short_lived_token": slt, "org_id": org,
    })
    check("same exchange returns same credentials", session == retry)
    token = session["access_token"]
    identity = request("identity", "/api/v1/me", token=token)
    check("identity matches exchanged actor", identity == session["identity"])
    orgs = request("organization discovery", "/api/v1/orgs", token=token, scope=None)
    check("discovery stays in bound organization", len(orgs) == 1 and orgs[0]["id"] == org)
    for path in ("services", "profiles", "sessions", "recordings", "usage"):
        request(path + " listing", "/api/v1/" + path, token=token)
    request("missing bearer rejected", "/api/v1/me", expected=401)
    request("missing organization rejected", "/api/v1/me", token=token, scope=None, expected=400)
    # IAM returns inactive for a token queried under the wrong organization.
    request("wrong organization rejected", "/api/v1/me", token=token, scope="wrong-audit-org", expected=401)
    request("fabricated bearer rejected", "/api/v1/me", token="oat_" + "x" * 43, expected=401)
    request("invalid refresh rejected", "/api/v1/auth/refresh", body={
        "refresh_token": "ort_" + "x" * 43, "org_id": org,
    }, expected=401)
    rotated = request("refresh", "/api/v1/auth/refresh", body={
        "refresh_token": session["refresh_token"], "org_id": org,
    })
    recovered = request("refresh recovery", "/api/v1/auth/refresh", body={
        "refresh_token": session["refresh_token"], "org_id": org,
    })
    check("refresh recovery returns same credentials", rotated == recovered)
    if "access_token" in rotated:
        request("refreshed identity", "/api/v1/me", token=rotated["access_token"])
        if binary := os.environ.get("SB_TEST_CLI"):
            with tempfile.TemporaryDirectory(prefix="sb-live-auth-") as home:
                env = {k: v for k, v in os.environ.items() if not k.startswith("SB_")}
                env.update(SB_HOME=home, SB_AUTHTOKEN=rotated["access_token"])
                for command in ([], ["profile", "ls"], ["session", "ls"], ["recording", "ls"]):
                    result = subprocess.run(
                        [binary, "--backend", backend, "--json", *command],
                        env=env, capture_output=True, timeout=40,
                    )
                    check("CLI " + (" ".join(command) or "identity/org discovery"), result.returncode == 0)
                check("CLI bearer override never persisted", all(
                    rotated["access_token"].encode() not in p.read_bytes()
                    for p in Path(home).rglob("*") if p.is_file()
                ))
    print(f"{sum(results)}/{len(results)} live auth checks passed")
    return 0 if all(results) else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (KeyError, ValueError, OSError, subprocess.SubprocessError):
        # Exceptions can contain credential-bearing URLs or process arguments.
        print("FAIL live auth runner: check configuration, backend availability, and JSON contract", file=sys.stderr)
        sys.exit(1)
