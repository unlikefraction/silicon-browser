#!/usr/bin/env python3
"""Isolated IAM CLI regression checks. Synthetic credentials; no live IAM calls."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--expected-version", default="1.2.1",
                    help="Require this installed IAM version (default: 1.2.1).")
args = parser.parse_args()
IAM = shutil.which("iam")
if IAM is None:
    raise SystemExit("iam was not found on PATH")
VERSION = subprocess.check_output([IAM, "--version"], text=True).strip()
if VERSION != f"iam {args.expected_version}":
    raise SystemExit(f"This audit expects iam {args.expected_version}, found {VERSION}")
results = []


def init_home(path):
    path.mkdir(mode=0o700)
    (path / "config.json").write_text(json.dumps({"auto_update": False}))
    return path


def invoke(home, arguments):
    env = {k: v for k, v in os.environ.items() if not k.startswith("SILICON_IAM_")}
    env.update(SILICON_IAM_HOME=str(home), SILICON_IAM_AUTO_UPDATE="false")
    return subprocess.run([IAM, *arguments], env=env, capture_output=True, text=True,
                          stdin=subprocess.DEVNULL, timeout=20)


with tempfile.TemporaryDirectory(prefix=f"iam-{args.expected_version}-offline-") as directory:
    root = Path(directory)
    for iteration in range(3):
        home = init_home(root / f"race-{iteration}")
        fake = {"access_token": "cat_synthetic", "refresh_token": "rft_synthetic",
                "expires_at": "2030-01-01T00:00:00Z", "actor_id": "synthetic"}
        (home / "credentials.json").write_text(json.dumps({
            "sessions": {f"profile-{i}": fake for i in range(3000)}}))
        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            runs = list(pool.map(lambda i: invoke(home, ["--profile", f"profile-{i}",
                                                         "logout", "--local-only"]), range(32)))
        saved = json.loads((home / "credentials.json").read_text())
        remaining = sum(f"profile-{i}" in saved["sessions"] for i in range(32))
        failures = sum(run.returncode != 0 for run in runs)
        results.append({"case": f"concurrent_logout_{iteration + 1}",
                        "failures": failures, "targets_remaining": remaining,
                        "failure_details": [
                            {"profile": f"profile-{i}", "exit": run.returncode,
                             "stderr": run.stderr.replace(str(root), "$ISOLATED_ROOT")[:2000]}
                            for i, run in enumerate(runs) if run.returncode != 0
                        ],
                        "successful_targets_remaining": [
                            f"profile-{i}" for i, run in enumerate(runs)
                            if run.returncode == 0 and f"profile-{i}" in saved["sessions"]
                        ],
                        "pass": failures == 0 and remaining == 0})

    for kind in ("symlink_credentials", "symlink_config", "symlink_lock",
                 "hardlink_credentials", "fifo_credentials", "symlink_home"):
        home = init_home(root / kind)
        victim = root / f"{kind}-victim.json"
        victim.write_text('{"sessions":{},"sentinel":"must remain unchanged"}')
        before = victim.read_bytes()
        name = {"symlink_config": "config.json", "symlink_lock": "credentials.lock"}.get(
            kind, "credentials.json")
        target = home / name
        if kind == "symlink_home":
            alias = root / "linked-home"
            alias.symlink_to(home)
            home = alias
        elif kind == "hardlink_credentials":
            os.link(victim, target)
        elif kind == "fifo_credentials":
            os.mkfifo(target)
        else:
            target.unlink(missing_ok=True)
            target.symlink_to(victim)
        run = invoke(home, ["logout", "--local-only"])
        preserved = victim.read_bytes() == before
        results.append({"case": kind, "exit": run.returncode,
                        "victim_preserved": preserved,
                        "pass": run.returncode != 0 and preserved})

    home = init_home(root / "usage")
    for host in ("[::1]", "127.0.0.1", "127.0.0.2"):
        run = invoke(home, ["--url", f"http://{host}:9", "config", "show"])
        rejected = "base URL must" in run.stderr
        results.append({"case": f"loopback_{host}", "exit": run.returncode,
                        "url_rejected": rejected,
                        "pass": run.returncode == 0 and not rejected})
    for carbon in ("carbon0", "UPPER", "ab", "a" * 31):
        run = invoke(home, ["--url", "http://127.0.0.1:9", "signup", "--email",
                            "synthetic@example.test", "--phone", "+14155550123",
                            "--carbon-id", carbon])
        local_error = "invalid value" in run.stderr and "Carbon" in run.stderr
        results.append({"case": f"invalid_carbon_{carbon}", "exit": run.returncode,
                        "local_validation": local_error,
                        "pass": run.returncode == 2 and local_error})

print(json.dumps({"version": VERSION, "results": results,
                  "all_passed": all(item["pass"] for item in results)}, indent=2))
raise SystemExit(0 if all(item["pass"] for item in results) else 1)
