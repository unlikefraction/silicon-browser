#!/usr/bin/env python3
"""Exercise local byte transfers using an installed native controller and Chrome.

No cloud/provider session or real credential is used. Example:
  python3 scripts/test_local_file_transfers.py --controller /path/to/sb-browser-engine
Optional --chrome and --cli select installed Chrome and the sb binary.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.server
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--controller", required=True, type=Path)
    parser.add_argument("--cli", type=Path, default=Path("target/debug/sb"))
    parser.add_argument("--chrome", default=(
        shutil.which("google-chrome") or shutil.which("chromium")
        or "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    ))
    args = parser.parse_args()
    controller, cli = str(args.controller.resolve()), str(args.cli.resolve())
    reports = []
    payload = bytes(range(256)) * 4097
    text = b"LOCAL_UPLOAD_BYTES_not_a_remote_path\n"

    with tempfile.TemporaryDirectory(prefix="sb-local-transfer-") as temporary:
        root = Path(temporary)
        chrome = subprocess.Popen([
            args.chrome, "--headless=new", "--remote-debugging-port=0",
            "--disable-background-networking", "--no-first-run", "--no-default-browser-check",
            "--user-data-dir=" + str(root / "chrome"), "about:blank",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        server = None
        try:
            active_port = root / "chrome" / "DevToolsActivePort"
            deadline = time.monotonic() + 20
            while not active_port.exists():
                if time.monotonic() >= deadline or chrome.poll() is not None:
                    raise RuntimeError("isolated Chrome did not expose its local CDP port")
                time.sleep(0.05)
            cdp = "http://127.0.0.1:" + active_port.read_text().splitlines()[0]

            class Handler(http.server.BaseHTTPRequestHandler):
                def log_message(self, *_args):
                    pass

                def do_GET(self):
                    if self.path == "/fixture":
                        self.respond(b"<!doctype html><title>File transfer fixture</title>", "text/html")
                    elif self.path == "/fixture/file.bin":
                        if "fixture_session=local" not in self.headers.get("Cookie", ""):
                            self.send_error(403)
                        else:
                            self.respond(payload, "application/octet-stream")
                    elif self.path.endswith("/connection"):
                        self.respond_json({
                            "session_id": "local-transfer", "principal_id": "fixture-principal",
                            "cdp_url": cdp, "expires_at": "2099-01-01T00:00:00Z",
                        })
                    else:
                        self.send_error(404)

                def do_POST(self):
                    report = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                    assert self.path.endswith("/commands")
                    reports.append(report)
                    self.respond_json({"command_id": report["command_id"], "sequence": len(reports)})

                def respond_json(self, value):
                    self.respond(json.dumps({"data": value}).encode(), "application/json")

                def respond(self, value, mime):
                    self.send_response(200)
                    self.send_header("Content-Type", mime)
                    self.send_header("Content-Length", str(len(value)))
                    self.end_headers()
                    self.wfile.write(value)

            server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
            threading.Thread(target=server.serve_forever, daemon=True).start()
            backend = f"http://127.0.0.1:{server.server_port}"
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith(("SB_", "IAM_", "BRIEFCASE_", "AGENT_BROWSER_"))}
            env.update({"SB_HOME": str(root / "state"), "SB_BACKEND_URL": backend,
                        "SB_ORG_ID": "fixture", "SB_AUTHTOKEN": "oat_fixture",
                        "SB_CONTROLLER_BIN": controller})

            def run(command, extra=(), expected=0, home=None):
                invocation_env = dict(env)
                if home is not None:
                    invocation_env["SB_HOME"] = str(home)
                result = subprocess.run([cli, "--json", "run", "local-transfer", command, *extra],
                                        env=invocation_env, capture_output=True, text=True, timeout=120)
                assert result.returncode == expected, result.stderr
                return "".join(event.get("chunk", "")
                               for event in map(json.loads, result.stdout.splitlines())
                               if event.get("type") == "stdout")

            def evaluate(script):
                value = json.loads(run("eval " + shlex.quote(script), ["--json"]))
                assert value["success"]
                return value["data"]["result"]

            run("open " + backend + "/fixture")
            evaluate("document.body.innerHTML='<input id=visible type=file multiple aria-label=Upload>"
                     "<input id=hidden type=file style=display:none>';window.observed=[];"
                     "window.originalGetAttribute=Element.prototype.getAttribute;"
                     "window.addEventListener('input',e=>{if(e.target.type==='file')"
                     "window.observed.push(Promise.all(Array.from(e.target.files,f=>f.text())));},true);true")
            (root / "one text.txt").write_bytes(text)
            (root / "two.bin").write_bytes(payload)
            run("upload", ["#hidden", str(root / "one text.txt")])
            assert evaluate("document.querySelector('#hidden').files[0].text()") == text.decode()
            snapshot = json.loads(run("snapshot -i", ["--json"]))["data"]
            ref = next(key for key, value in snapshot["refs"].items() if value.get("name") == "Upload")
            run("upload", ["@" + ref, str(root / "one text.txt"), str(root / "two.bin")])
            actual = evaluate("(async()=>{const f=document.querySelector('#visible').files;"
                              "return {names:Array.from(f,x=>x.name),text:await f[0].text(),"
                              "sha256:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',"
                              "await f[1].arrayBuffer())),b=>b.toString(16).padStart(2,'0')).join(''),"
                              "first:await window.observed[0],last:(await window.observed[1])[0]}})()")
            assert actual["names"] == ["one text.txt", "two.bin"]
            assert actual["text"] == text.decode()
            assert actual["sha256"] == hashlib.sha256(payload).hexdigest()
            assert actual["first"] == [text.decode()] and actual["last"] == text.decode()
            run("upload", ["#hidden", str(root / "missing.txt")], expected=1)
            assert evaluate("document.querySelector('#hidden').files[0].text()") == text.decode()
            evaluate("document.cookie='fixture_session=local; path=/';"
                     "document.body.insertAdjacentHTML('beforeend','<a id=http href=/fixture/file.bin>HTTP download</a>"
                     "<a id=blob>Blob download</a><a id=bad href=https://cross-origin.invalid/file>Unsupported link</a>"
                     "<button id=button onclick=window.didClick=true>Unsupported button</button>');"
                     "document.querySelector('#blob').href=URL.createObjectURL(new Blob(["
                     "new Uint8Array(Array.from({length:1048832},(_,i)=>i%256))]));true")
            http_file = root / "downloads" / "http.bin"
            downloaded = json.loads(run("download", ["#http", str(http_file), "--json"]))
            assert downloaded["success"]
            assert downloaded["data"]["sha256"] == hashlib.sha256(payload).hexdigest()
            assert http_file.read_bytes() == payload
            snapshot = json.loads(run("snapshot -i", ["--json"]))["data"]
            ref = next(key for key, value in snapshot["refs"].items() if value.get("name") == "Blob download")
            blob_file = root / "downloads" / "blob.bin"
            blob_file.write_bytes(b"preexisting-sentinel")
            run("download", ["@" + ref, str(blob_file)])
            assert blob_file.read_bytes() == payload
            # Separate CLI homes can control the same managed browser concurrently.
            concurrent_file = root / "downloads" / "concurrent.bin"
            with ThreadPoolExecutor(max_workers=2) as pool:
                uploaded = pool.submit(run, "upload", ["#hidden", str(root / "two.bin")])
                downloaded = pool.submit(run, "download", ["#http", str(concurrent_file)],
                                         home=root / "state-peer")
                uploaded.result()
                downloaded.result()
            assert concurrent_file.read_bytes() == payload
            assert evaluate("Element.prototype.getAttribute===window.originalGetAttribute") is True
            sentinel = root / "downloads" / "preserve.bin"
            sentinel.write_bytes(b"sentinel")
            for selector in ("#bad", "#button"):
                run("download", [selector, str(sentinel)], expected=1)
                assert sentinel.read_bytes() == b"sentinel"
            assert evaluate("window.didClick===undefined") is True
            run("wait", ["--download", str(sentinel)], expected=1)
            assert sentinel.read_bytes() == b"sentinel"
            assert not list((root / "downloads").glob(".sb-download-*.part"))
            assert all("stdout" not in report and "stderr" not in report for report in reports)
            assert all("LOCAL_UPLOAD_BYTES" not in json.dumps(report) for report in reports)
            assert sum(report["command"] == "upload" for report in reports) == 3
            print(json.dumps({"hidden_input": "passed", "snapshot_ref_multiple_files": "passed",
                              "binary_sha256": "passed", "preexisting_capture_handler": "passed",
                              "missing_file_preserves_selection": "passed",
                              "authenticated_http_download": "passed", "blob_ref_download": "passed",
                              "unsupported_download_preserves_sentinel": "passed",
                              "download_does_not_trigger_button": "passed",
                              "concurrent_transfers_restore_page_methods": "passed",
                              "command_metadata_only": "passed", "binary_bytes": len(payload)}))
        finally:
            # Stop only the daemon created by this isolated fixture namespace.
            namespaces = set()
            for lock in root.rglob("sb-*.lock"):
                namespace = lock.stem
                if namespace in namespaces:
                    continue
                namespaces.add(namespace)
                cleanup_env = {key: value for key, value in os.environ.items()
                               if not key.startswith(("SB_", "IAM_", "BRIEFCASE_", "AGENT_BROWSER_"))}
                cleanup_env.update({"AGENT_BROWSER_CDP": cdp, "AGENT_BROWSER_SESSION": namespace,
                                    "AGENT_BROWSER_CONFIG": str(lock.parent / "controller.json")})
                try:
                    subprocess.run([controller, "close"], env=cleanup_env, timeout=10,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                except (subprocess.TimeoutExpired, OSError):
                    pass
            if server:
                server.shutdown()
            chrome.terminate()
            try:
                chrome.wait(timeout=10)
            except subprocess.TimeoutExpired:
                chrome.kill()
                chrome.wait()


if __name__ == "__main__":
    main()
