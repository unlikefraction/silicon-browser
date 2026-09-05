//! Local bytes must cross CDP: Chromium's file-path command addresses the remote filesystem.
use super::{Error, LocalController, LocalExecution, RunEvent, RunResult, SessionConnection};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};
const CHUNK_BYTES: usize = 1024 * 1024;

struct UploadFile {
    file: File,
    name: String,
    mime: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
}
impl LocalController {
    pub(super) fn upload_local(
        &self,
        connection: &SessionConnection,
        namespace: &str,
        config: &Path,
        args: &[String],
        started_at: DateTime<Utc>,
        emit: &mut impl FnMut(&RunEvent),
    ) -> Result<LocalExecution, Error> {
        let (selector, paths, options) = upload_arguments(args)?;
        let mut files = Vec::new();
        for path in &paths {
            let file =
                File::open(path).map_err(|_| Error::Local(format!("could not read local upload file: {path}")))?;
            let metadata = file.metadata().map_err(|_| Error::Local("could not inspect local upload file".into()))?;
            if !metadata.is_file() {
                return Err(Error::Local("upload requires regular local files; directories are unsupported".into()));
            }
            let name = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| Error::Local("upload filename must be valid UTF-8".into()))?
                .to_owned();
            files.push(UploadFile {
                file,
                name,
                mime: mime_guess::from_path(path).first_or_octet_stream().to_string(),
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
        let key = format!("__sb_upload_{}", uuid::Uuid::new_v4().simple());
        let invoke = |command: &[String], script: Option<&str>| {
            self.upload_helper(connection, namespace, config, &options, command, script)
        };
        let eval = |script: &str| invoke(&["eval".into(), "--stdin".into()], Some(script));
        let key_json = serde_json::to_string(&key).unwrap();
        let result = (|| {
            eval(&super::download::capture_script(&key))?;
            let captured = invoke(&["get".into(), "attr".into(), selector.clone(), key.clone()], None)?;
            if captured.get("value").and_then(Value::as_str) != Some(key.as_str()) {
                return Err(Error::Local(
                    "file input could not be safely resolved; cross-origin frames are unsupported".into(),
                ));
            }
            let resolved = eval(&format!(
                r#"(() => {{ const s=window[{key_json}]; if(!s) throw Error('upload resolver expired'); s.restore(); const t=s.target; if(s.captures!==1 || !t || !(t instanceof t.ownerDocument.defaultView.HTMLInputElement) || t.type!=='file' || !t.isConnected) throw Error('file input could not be safely resolved; cross-origin frames are unsupported'); if({count}>1 && !t.multiple) throw Error('file input does not accept multiple files'); s.chunks=[];s.files=[];return {{ready:true}}; }})()"#,
                count = files.len()
            ))?;
            if resolved.get("ready") != Some(&Value::Bool(true)) {
                return Err(Error::Local("file input could not be resolved safely".into()));
            }
            let mut expected = Vec::new();
            let mut buffer = vec![0u8; CHUNK_BYTES];
            for file in &mut files {
                let mut size = 0u64;
                let mut digest = Sha256::new();
                let mut fnv = 2166136261u32;
                loop {
                    let count = file
                        .file
                        .read(&mut buffer)
                        .map_err(|_| Error::Local("could not read local upload bytes".into()))?;
                    if count == 0 {
                        break;
                    }
                    size += count as u64;
                    digest.update(&buffer[..count]);
                    for byte in &buffer[..count] {
                        fnv = (fnv ^ u32::from(*byte)).wrapping_mul(16777619);
                    }
                    let encoded = STANDARD.encode(&buffer[..count]);
                    let received = eval(&format!(
                        r#"(() => {{const s=window[{key_json}];if(!s || !s.target.isConnected) throw Error('upload target changed');const b=Uint8Array.from(atob('{encoded}'),c=>c.charCodeAt(0));s.chunks.push(b);return b.length;}})()"#
                    ))?;
                    if received.as_u64() != Some(count as u64) {
                        return Err(Error::Protocol("local upload chunk was not acknowledged".into()));
                    }
                }
                let current =
                    file.file.metadata().map_err(|_| Error::Local("could not recheck local upload file".into()))?;
                if size != file.size || current.len() != file.size || current.modified().ok() != file.modified {
                    return Err(Error::Local("local file changed during upload; retry with a stable file".into()));
                }
                let modified = file
                    .modified
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |value| value.as_millis().min(u64::MAX as u128) as u64);
                let metadata = json!({"name":file.name,"type":file.mime,"lastModified":modified});
                let built = eval(&format!(
                    r#"(() => {{const s=window[{key_json}],m={metadata};if(!s || !s.target.isConnected) throw Error('upload target changed');const w=s.target.ownerDocument.defaultView;const f=new w.File(s.chunks,m.name,{{type:m.type,lastModified:m.lastModified}});s.files.push(f);s.chunks=[];return {{size:f.size,name:f.name}};}})()"#
                ))?;
                if built.get("size").and_then(Value::as_u64) != Some(size)
                    || built.get("name").and_then(Value::as_str) != Some(file.name.as_str())
                {
                    return Err(Error::Protocol("local upload file construction failed".into()));
                }
                expected
                    .push(json!({"name":file.name,"size":size,"sha256":format!("{:x}",digest.finalize()),"fnv":fnv}));
            }
            // All bytes are verified before assignment and before any site input/change handler.
            let verified = eval(&commit_script(&key, &expected))?;
            if verified.get("uploaded").and_then(Value::as_u64) != Some(files.len() as u64) {
                return Err(Error::Protocol("local upload was not confirmed".into()));
            }
            Ok(())
        })();
        // Cleanup restores the temporary resolver even when native resolution/evaluation fails.
        let _ = eval(&format!(
            r#"(() => {{const s=window[{key_json}];if(s){{s.restore();delete window[{key_json}];}}return true;}})()"#
        ));
        result?;
        let stdout = if options.iter().any(|option| option == "--json") {
            format!("{}\n", json!({"success":true,"data":{"uploaded":files.len()}}))
        } else {
            format!("Uploaded {} local file(s).\n", files.len())
        };
        emit(&RunEvent::Stdout { chunk: stdout.clone() });
        let result = RunResult {
            session_id: connection.session_id.clone(),
            exit_code: 0,
            started_at,
            finished_at: Utc::now(),
            stdout,
            stderr: String::new(),
        };
        emit(&RunEvent::Finished { result: result.clone() });
        Ok(LocalExecution { result, truncated: false })
    }

    pub(super) fn upload_helper(
        &self,
        connection: &SessionConnection,
        namespace: &str,
        config: &Path,
        options: &[String],
        command: &[String],
        script: Option<&str>,
    ) -> Result<Value, Error> {
        if connection.expires_at <= Utc::now() {
            return Err(Error::Local("session expired during local file transfer".into()));
        }
        let mut process = self.process(connection, namespace, config);
        process.args(options).arg("--json").args(command).stdout(Stdio::piped()).stderr(Stdio::null());
        process.stdin(if script.is_some() { Stdio::piped() } else { Stdio::null() });
        let mut child =
            process.spawn().map_err(|_| Error::Local("could not start local file transfer; run `sb setup`".into()))?;
        let mut stdin = child.stdin.take();
        let owned_script = script.map(str::to_owned);
        let writer = std::thread::spawn(move || {
            if let (Some(mut stdin), Some(script)) = (stdin.take(), owned_script) {
                stdin.write_all(script.as_bytes())
            } else {
                Ok(())
            }
        });
        let mut pipe = child.stdout.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.by_ref().take(2 * 1024 * 1024).read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(35);
        let status = loop {
            if Instant::now() >= deadline || connection.expires_at <= Utc::now() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Local("local file transfer timed out".into()));
            }
            if let Some(status) =
                child.try_wait().map_err(|_| Error::Local("could not observe local file transfer".into()))?
            {
                break status;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let bytes = reader
            .join()
            .map_err(|_| Error::Local("local file transfer output failed".into()))?
            .map_err(|_| Error::Local("local file transfer output failed".into()))?;
        let _ = writer.join();
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Local("local file transfer did not return a valid result".into()))?;
        if !status.success() || value.get("success") != Some(&Value::Bool(true)) {
            let message = value.get("error").and_then(Value::as_str).unwrap_or("");
            let detail = if message.contains("cross-origin frames are unsupported") {
                "file input could not be safely resolved; cross-origin frames are unsupported"
            } else if message.contains("does not accept multiple files") {
                "this file input does not accept multiple files"
            } else {
                "local file transfer failed; no successful transfer was confirmed"
            };
            return Err(Error::Local(detail.into()));
        }
        let data = value.get("data").cloned().unwrap_or(Value::Null);
        Ok(data.get("result").cloned().unwrap_or(data))
    }
}

fn upload_arguments(args: &[String]) -> Result<(String, Vec<String>, Vec<String>), Error> {
    let at = super::first_controller_command_index(args)
        .filter(|index| args[*index] == "upload")
        .ok_or_else(|| Error::Local("upload command is missing".into()))?;
    let selector = args
        .get(at + 1)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Local("upload requires an input selector and local file paths".into()))?
        .clone();
    let mut options = args[..at].to_vec();
    let mut files = Vec::new();
    let mut index = at + 2;
    const VALUES: &[&str] = &[
        "--headers",
        "--extension",
        "--init-script",
        "--enable",
        "--state",
        "--proxy-bypass",
        "--args",
        "--user-agent",
        "--device",
        "--color-scheme",
        "--download-path",
        "--max-output",
        "--allowed-domains",
        "--action-policy",
        "--confirm-actions",
        "--screenshot-dir",
        "--screenshot-quality",
        "--screenshot-format",
        "--idle-timeout",
        "--ca-cert",
        "--model",
        "--restore-save",
        "--restore-check-url",
        "--restore-check-text",
        "--restore-check-fn",
    ];
    while let Some(value) = args.get(index) {
        if VALUES.contains(&value.as_str()) {
            let next = args.get(index + 1).ok_or_else(|| Error::Local("browser option requires a value".into()))?;
            options.extend([value.clone(), next.clone()]);
            index += 2;
        } else if value.starts_with('-') {
            options.push(value.clone());
            index += 1;
            if args.get(index).is_some_and(|value| matches!(value.as_str(), "true" | "false")) {
                options.push(args[index].clone());
                index += 1;
            }
        } else {
            files.push(value.clone());
            index += 1;
        }
    }
    Ok((selector, files, options))
}

fn commit_script(key: &str, expected: &[Value]) -> String {
    let key = serde_json::to_string(key).unwrap();
    let expected = serde_json::to_string(expected).unwrap();
    format!(
        r#"(async () => {{
 const key={key},s=window[key],expected={expected};if(!s||!s.target.isConnected)throw Error('upload target changed');
 const t=s.target,w=t.ownerDocument.defaultView;if(s.files.length!==expected.length)throw Error('upload file count differs');
 for(let i=0;i<s.files.length;i++){{const f=s.files[i],e=expected[i];if(f.name!==e.name||f.size!==e.size)throw Error('upload metadata differs');
 if(w.crypto&&w.crypto.subtle){{const hash=await w.crypto.subtle.digest('SHA-256',await f.arrayBuffer());const hex=Array.from(new Uint8Array(hash),b=>b.toString(16).padStart(2,'0')).join('');if(hex!==e.sha256)throw Error('upload bytes differ');}}
 else{{let hash=2166136261;const r=f.stream().getReader();while(true){{const part=await r.read();if(part.done)break;for(const b of part.value)hash=Math.imul(hash^b,16777619)>>>0;}}if(hash!==e.fnv)throw Error('upload bytes differ');}}}}
 const transfer=new w.DataTransfer();for(const f of s.files)transfer.items.add(f);t.files=transfer.files;
 if(t.files.length!==expected.length)throw Error('upload assignment failed');for(let i=0;i<t.files.length;i++){{if(t.files[i].name!==expected[i].name||t.files[i].size!==expected[i].size)throw Error('upload assignment differs');await t.files[i].slice(0,1).arrayBuffer();}}
 delete window[key];t.dispatchEvent(new w.Event('input',{{bubbles:true,composed:true}}));t.dispatchEvent(new w.Event('change',{{bubbles:true,composed:true}}));return {{uploaded:expected.length}};
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upload_paths_and_global_options_are_separated_without_rewriting_names() {
        let args = vec![
            "--user-agent",
            "upload",
            "--json",
            "upload",
            "@e2",
            "./my file.txt",
            "./second.csv",
            "--pin-tab",
            "--download-path",
            "./downloads",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
        let (selector, files, options) = upload_arguments(&args).unwrap();
        assert_eq!(selector, "@e2");
        assert_eq!(files, ["./my file.txt", "./second.csv"]);
        assert_eq!(options, ["--user-agent", "upload", "--json", "--pin-tab", "--download-path", "./downloads"]);
    }
}
