# agent-browser 0.36.0: remote download gaps

Verified against upstream tag `v0.36.0`, commit `eb05921bad874cd2a1b4fa5d1149f1ed26576cae`, on 6 September 2026. These are source-confirmed behavior gaps; this audit did not run a paid cloud download or assert that a returned filename proves byte delivery.

## `download <selector> <path>` does not transport remote bytes

The native handler resolves and creates the destination directory on the CLI machine, passes that directory to Chrome's `Browser.setDownloadBehavior`, clicks the selector, and waits for a download event. It then looks for the resulting GUID-named file in the CLI's local directory and renames it locally. There is no CDP response-body read, stream read, provider download retrieval, or other transfer from the remote browser filesystem.

When Chrome runs remotely, `downloadPath` names a directory on that remote Chrome host. The local CLI directory is a different filesystem. With a fresh local destination, a completed remote download therefore cannot satisfy the handler's local file lookup. If the destination already exists locally, the fallback can return success without replacing or verifying its bytes.

Sources: [download handler, lines 6722–6876](https://github.com/vercel-labs/agent-browser/blob/v0.36.0/cli/src/native/actions.rs#L6722), [CDP download configuration, line 2176](https://github.com/vercel-labs/agent-browser/blob/v0.36.0/cli/src/native/browser.rs#L2176), [Chrome's download behavior contract](https://chromedevtools.github.io/devtools-protocol/tot/Browser/#method-setDownloadBehavior).

## `wait --download [path]` returns a label, not a downloaded local file

The handler subscribes to future download-progress events and returns the supplied `path` string when it sees `completed`. It does not create, copy, rename, open, or verify that path. It also does not configure download behavior or enable events itself; a remote CDP connection starts with `download_path: None`.

This command can report success while no file exists at the requested local path. Calling it after a quickly completed download can instead miss the event entirely and time out.

Sources: [wait handler, lines 9888–9925](https://github.com/vercel-labs/agent-browser/blob/v0.36.0/cli/src/native/actions.rs#L9888), [remote CDP connection initialization](https://github.com/vercel-labs/agent-browser/blob/v0.36.0/cli/src/native/browser.rs#L611).

## Event correlation is insufficient for shared browsers

Both handlers accept browser-wide download-progress events without correlating completion to the requested download's GUID and originating frame. The download handler overwrites its remembered GUID whenever it sees another `downloadWillBegin`. Another tab's download can complete the wait. `Browser.setDownloadBehavior` changes behavior for the browser context, so simultaneous controllers using different destination paths can also interfere with each other.

These are additional reasons not to claim local download success based only on a completed browser event or an existing local path.

## Minimum safe correction

Until a verified byte-transfer path exists, the Browser CLI should reject remote `download` and `wait --download` clearly before triggering a download. The rejection should explain that the operation is unsupported; it must not print a local destination as if it had been written. Browser metadata APIs and the Browser backend should not proxy file bytes.

A real implementation needs a client-side download broker over the existing direct CDP connection. For HTTP responses, Chrome exposes response-stage interception with `Fetch.takeResponseBodyAsStream` and sequential `IO.read`, allowing bounded chunks to be written to a temporary local file and atomically finalized. This is an implementation direction, not a claim of universal download coverage: the intercepted response must be correlated, unrelated paused requests continued, cleanup performed, and blob/data URLs handled separately. Taking a response stream prevents continuing that same response unchanged; the client must complete or cancel it deliberately. Re-fetching an observed URL alone is not equivalent to the original authenticated request and can repeat POST side effects or miss blob content.

Sources: [Fetch response streaming semantics](https://chromedevtools.github.io/devtools-protocol/tot/Fetch/#method-takeResponseBodyAsStream), [IO read](https://chromedevtools.github.io/devtools-protocol/tot/IO/#method-read).

Acceptance must compare the actual local bytes and digest for a fresh destination, confirm that a pre-existing sentinel cannot cause false success, exercise a blob download and an authenticated HTTP response, and verify that another tab's download does not satisfy the wrong request. Screenshot, PDF, upload and download each require their own byte-level checks.

## Silicon Browser adapter

Silicon Browser now bypasses the native download handler for supported link targets. The local controller resolves the exact link using a temporary, uniquely marked attribute lookup, restores the page method, and fetches ordinary same-origin HTTP targets with browser credentials, or blob/data targets. It reads bounded byte chunks over the direct CDP connection, verifies offsets and a rolling checksum, calculates local SHA-256, and atomically finalizes a private local file. It does not wait for browser-wide download events. Unsupported button, script, POST and cross-origin cases fail explicitly; `wait --download` and `--download-path` are rejected.

The companion upload adapter transfers local bytes into browser `File` objects, verifies them before assigning the file input or dispatching input/change events, and supports hidden inputs and snapshot references. Both adapters share a resolver whose cleanup preserves other active resolver instances.

The opt-in [local browser regression](../scripts/test_local_file_transfers.py) passed against Chrome and the pinned native 0.36.0 controller: full 1,048,832-byte SHA-256 upload, immediate page event reads, authenticated HTTP and blob downloads, snapshot references, preserved existing destinations on failure, removed partial files, concurrent transfers from separate CLI homes, and metadata-only API reports. Run it with `python3 scripts/test_local_file_transfers.py --controller /path/to/sb-browser-engine`; an installed Chrome and a built `target/debug/sb` are test prerequisites only.
