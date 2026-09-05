# IAM integration findings — 2026-09-05

Historical 1.1.1 findings. See [the 1.2.0 retest](IAM_1_2_0_EXTERNAL_BUGS.md) for current status;
the four local correctness/security bugs below are fixed in the tested 1.2.0 cases.

This is a local, sanitized report. Nothing has been posted upstream. Tests used the installed `iam 1.1.1`; offline reproductions used temporary homes and synthetic credentials. Live observations were confined to the user-authorized testing environment. No real tokens, environment keys, OTPs, or personal contact details appear here.

## Scope and source versions

- Released CLI and client: `silicon-iam-cli 1.1.1`, `silicon-iam-client 1.1.1`, downloaded Cargo source. Client release provenance: commit `b57848ce55f0becf53e9244d2239a2dc714cecb0`.
- Compared GitHub `main` documentation/source checkout: commit `d9fa8745b28a5aff3cd041005fd8855ce10f73ca`.
- Paths below beginning `crates/cli` or `crates/client` identify the corresponding released crate files unless explicitly labeled GitHub main. The registry source directories are `silicon-iam-cli-1.1.1` and `silicon-iam-client-1.1.1`.

## 1. Concurrent CLI invocations lose credential updates and read truncated JSON

**Impact:** Parallel agents can receive credential-store parse errors or report successful local logout while the supposedly removed session remains stored. The same storage path also persists rotated refresh tokens, although a live refresh-token loss was not deliberately induced.

**Reproduced offline:** With 3,000 synthetic sessions in one temporary home, run 32 local logouts against distinct profiles using 16 workers. Observed 2 failures with `EOF while parsing a value at line 1 column 0`; 18 of the 32 targeted sessions remained in the resulting valid JSON. The exact counts depend on scheduling.

**Cause:** `Context::remember` and `Context::forget` in `crates/cli/src/context.rs` load the entire credential document, modify a snapshot, and save it without a shared process lock. `write_json` in `crates/cli/src/store.rs:300` uses `fs::write`, which truncates the destination before rewriting. `Context::renew` also has no lock spanning refresh reservation, network exchange, and commit.

**Expected:** Each successful local logout removes its targeted session; parallel readers always see a complete document. Refresh-token rotation must not race another refresh or overwrite a newer login.

**Suggested fix:** Lock the complete read/modify/write operation across processes, merge updates into the latest state, write a unique owner-only temporary file, sync and atomically rename it. Serialize refresh per session and re-read after acquiring that lock. Preserve the existing pending idempotency key behavior.

Safe offline reproducer (no network calls, fake credentials only):

```python
import concurrent.futures, json, os, pathlib, shutil, subprocess, tempfile

iam = shutil.which("iam")
with tempfile.TemporaryDirectory(prefix="iam-offline-race-") as directory:
    home = pathlib.Path(directory)
    (home / "config.json").write_text(json.dumps({"auto_update": False}))
    fake = {
        "access_token": "cat_synthetic",
        "refresh_token": "rft_synthetic",
        "expires_at": "2030-01-01T00:00:00Z",
        "actor_id": "synthetic",
    }
    (home / "credentials.json").write_text(json.dumps({
        "sessions": {f"profile-{i}": fake for i in range(3000)}
    }))
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("SILICON_IAM_")}
    env.update(SILICON_IAM_HOME=str(home), SILICON_IAM_AUTO_UPDATE="false")

    def logout(i):
        return subprocess.run(
            [iam, "--profile", f"profile-{i}", "logout", "--local-only"],
            env=env, capture_output=True, text=True,
        )

    with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
        results = list(pool.map(logout, range(32)))
    print("failed commands:", sum(result.returncode != 0 for result in results))
    saved = json.loads((home / "credentials.json").read_text())
    print("targeted sessions still stored:",
          sum(f"profile-{i}" in saved["sessions"] for i in range(32)))
```

## 2. Credential writes follow symlinks and overwrite their targets

**Impact:** A symlink at `credentials.json` redirects a normal CLI state write into another file. This is an integrity problem where the home or credential path is misconfigured or writable by another actor; the reproduction does not claim an actor can modify an otherwise protected directory.

**Reproduced offline:** Place a symlink at a temporary `credentials.json` pointing to a temporary victim containing `{"sessions":{},"sentinel":"must remain unchanged"}`. Run `iam logout --local-only` with automatic updates disabled. It exits **0** and changes the victim file, removing the sentinel.

**Cause:** `read_json` uses `fs::read`; `write_json` uses `fs::write` and then `fs::set_permissions`. Each follows symlinks. Permissions are tightened after the write, not at file creation.

**Expected:** Reject symlinked credential and lock files and leave their targets untouched. Create secret-bearing files with restrictive permissions from their first byte.

**Suggested fix:** Open credential/lock files with no-follow semantics, verify regular files and the private home directory, and use an atomic owner-only write. Add a regression that asserts both rejection and byte-for-byte preservation of the symlink target.

Safe offline reproducer:

```python
import json, os, pathlib, shutil, subprocess, tempfile

iam = shutil.which("iam")
with tempfile.TemporaryDirectory(prefix="iam-offline-symlink-") as directory:
    root = pathlib.Path(directory)
    home = root / "iam"
    home.mkdir()
    (home / "config.json").write_text(json.dumps({"auto_update": False}))
    victim = root / "victim.json"
    victim.write_text('{"sessions":{},"sentinel":"must remain unchanged"}')
    before = victim.read_bytes()
    (home / "credentials.json").symlink_to(victim)
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("SILICON_IAM_")}
    env.update(SILICON_IAM_HOME=str(home), SILICON_IAM_AUTO_UPDATE="false")
    result = subprocess.run([iam, "logout", "--local-only"],
                            env=env, capture_output=True, text=True)
    print("exit:", result.returncode, "victim changed:", victim.read_bytes() != before)
```

## 3. Documented IPv6 loopback URL is rejected locally

**Reproduced offline:** In an empty temporary IAM home with updates disabled:

```sh
iam --url 'http://[::1]:9' system health
```

Actual: exit **2**, `the base URL must use HTTPS; HTTP is limited to localhost, 127.0.0.1, or ::1`. No connection is attempted. Expected: accept the documented loopback URL, then attempt the request (port 9 need not host a server).

**Cause:** `ClientBuilder::new` in `crates/client/src/client.rs` compares `Url::host_str()` to `"::1"`. The URL crate's IPv6 host string includes brackets: `"[::1]"`. `iam --help` explicitly advertises `::1` as supported.

**Suggested fix:** Match `Url::host()` as `Host::Ipv6(address)` and test `address.is_loopback()`; likewise use typed IPv4 handling. Add client URL-construction tests covering IPv4 and IPv6 loopback.

## 4. Signup rejects an invalid Carbon ID only after both OTP ceremonies

**Observed live by the integration test:** A Carbon ID containing digit `0` was rejected only after email and phone verification.

**Source confirmation:** `signup` in `crates/cli/src/commands/auth.rs:443` starts signup, dispatches/verifies the email OTP, dispatches/verifies the phone OTP, and only then sends `args.carbon_id` to `complete`. The CLI parses the ID as an unconstrained string.

**Documentation precision:** GitHub main `docs/openapi.yaml:3793` does specify `^[a-z1-9_-]{3,30}$`, which excludes `0`. The issue is late CLI validation and missing guidance in the signup flag help, not absence from the entire API contract.

**Suggested fix:** Validate the documented ID syntax before creating a signup session; include the format in `iam signup --help`. Optionally check availability before sending OTPs while preserving authoritative final validation for races. A regression should assert that an ID containing `0` produces a useful local error and sends zero HTTP requests.

## 5. Documented loopback Application base URLs are blocked at the public edge

**Observed live by the integration test:** Application creation through the authorized IAM testing plane using `base_url: "http://127.0.0.1:18085"` returned **403**, `Content-Type: text/html`, `Server: awselb/2.0`, no `x-request-id`, and an ELB Forbidden HTML body. CLI reporting was `Forbidden (unrecognized_error)` / exit **4**. A raw API request reproduced the rejection. The equivalent create request with a distinct handle and `base_url: "https://browser.example.test"` returned **201**.

**Documentation:** GitHub main `docs/cli/README.md:156`, `:225`, and `:231` demonstrate Application creation with loopback HTTP base URLs. Those examples cannot currently complete through the tested public endpoint.

**Classification:** Confirmed integration failure with evidence of edge-generated rejection. This does **not** establish an IAM authorization bug or identify the specific WAF rule.

**Suggested fix:** Reconcile the edge policy with the documented testing workflow. Either permit the intended test-only loopback Application configuration or document the requirement for a public HTTPS base URL. Keep structured error bodies and correlation IDs available where possible so the CLI can distinguish an edge rejection from an IAM permission denial.

## 6. GitHub main documentation trails the published release

The inspected GitHub main checkout declares CLI/client **1.1.0**, and `docs/client/README.md` recommends `silicon-iam-client = "1.1.0"`; installed/published crates are **1.1.1** with a different recorded source commit. A caret dependency may still resolve the newer patch, but source audits against main do not necessarily inspect the code running in the released binary.

**Suggested fix:** Publish matching release tags and link docs to the release source. State the tested package version in integration instructions. For this Browser audit, downloaded 1.1.1 crate code and actual responses were treated as authoritative.

## Browser-side status

Browser's own CLI already uses atomic, locked, owner-only state and serialized refresh; its targeted tests passed. This report does not alter installed IAM source or claim the upstream findings are repaired. Serialize concurrent IAM operations against a shared home until upstream storage is fixed, or use independently managed IAM profiles/homes where appropriate. Existing real credentials were never used in the offline tests.
