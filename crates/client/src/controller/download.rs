//! Download a link's bytes through the caller's direct CDP connection.
//!
//! Native `downloadPath` addresses Chrome's filesystem, which is remote. This adapter
//! deliberately supports link-target GETs and blob/data URLs, not arbitrary click handlers.
use super::{Error, LocalController, LocalExecution, RunEvent, RunResult, SessionConnection};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const CHUNK_BYTES: usize = 256 * 1024;
const FNV_INITIAL: u32 = 2166136261;

impl LocalController {
    pub(super) fn download_local(
        &self,
        connection: &SessionConnection,
        namespace: &str,
        config: &Path,
        args: &[String],
        started_at: DateTime<Utc>,
        emit: &mut impl FnMut(&RunEvent),
    ) -> Result<LocalExecution, Error> {
        let (selector, destination, options) = download_arguments(args)?;
        let mut output = AtomicDownload::create(Path::new(&destination))?;
        let key = format!("__sb_download_{}", uuid::Uuid::now_v7().simple());
        let invoke = |command: &[String], script: Option<&str>| {
            self.upload_helper(connection, namespace, config, &options, command, script)
        };
        let eval = |script: &str| invoke(&["eval".into(), "--stdin".into()], Some(script));
        let transfer = (|| {
            eval(&capture_script(&key))?;
            // A random synthetic attribute invokes the native resolver without clicking or
            // focusing the page. The temporary prototype hook records the exact target.
            let resolved = invoke(&["get".into(), "attr".into(), selector.clone(), key.clone()], None)?;
            if resolved.get("value").and_then(Value::as_str) != Some(key.as_str()) {
                return Err(Error::Local(
                    "download requires a link in the current page or a same-origin frame; cross-origin frames are unsupported".into(),
                ));
            }
            let begun = eval(&begin_script(&key))?;
            if begun.get("ready") != Some(&Value::Bool(true)) {
                return Err(Error::Local(
                    "download requires an ordinary same-origin HTTP link or blob/data link; button, script, POST and cross-origin downloads are unsupported".into(),
                ));
            }
            let mut digest = Sha256::new();
            let mut offset = 0u64;
            let mut fnv = FNV_INITIAL;
            loop {
                let part = eval(&read_script(&key))?;
                let bytes = decode_chunk(&part, offset)?;
                for byte in &bytes {
                    fnv = (fnv ^ u32::from(*byte)).wrapping_mul(16777619);
                }
                if part.get("fnv").and_then(Value::as_u64) != Some(u64::from(fnv)) {
                    return Err(Error::Protocol("download byte verification failed".into()));
                }
                output
                    .file
                    .write_all(&bytes)
                    .map_err(|_| Error::Local("could not write local download; destination was not replaced".into()))?;
                digest.update(&bytes);
                offset = offset
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| Error::Local("download is too large".into()))?;
                if part.get("done") == Some(&Value::Bool(true)) {
                    break;
                }
            }
            Ok((offset, format!("{:x}", digest.finalize())))
        })();
        // Cancel any remaining reader/fetch and restore the resolver on every path. No
        // network request is retried, and a partial file never replaces the destination.
        let _ = eval(&cleanup_script(&key));
        let (size, sha256) = transfer?;
        let destination = output.commit()?;
        let stdout = if options.iter().any(|option| option == "--json") {
            format!("{}\n", json!({"success":true,"data":{"path":destination,"size_bytes":size,"sha256":sha256}}))
        } else {
            format!("Downloaded {size} bytes to {} (SHA-256 {sha256}).\n", destination.display())
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
}

fn decode_chunk(part: &Value, expected_offset: u64) -> Result<Vec<u8>, Error> {
    let invalid = || Error::Protocol("download returned invalid or out-of-order bytes".into());
    if part.get("offset").and_then(Value::as_u64) != Some(expected_offset)
        || part.get("done").and_then(Value::as_bool).is_none()
    {
        return Err(invalid());
    }
    let encoded = part.get("bytes").and_then(Value::as_str).ok_or_else(invalid)?;
    if encoded.len() > CHUNK_BYTES.div_ceil(3) * 4 {
        return Err(invalid());
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| invalid())?;
    if bytes.len() > CHUNK_BYTES
        || (bytes.is_empty() && part.get("done") != Some(&Value::Bool(true)))
        || (part.get("done") == Some(&Value::Bool(true)) && !bytes.is_empty())
    {
        return Err(invalid());
    }
    Ok(bytes)
}

struct AtomicDownload {
    file: File,
    temporary: PathBuf,
    destination: PathBuf,
    committed: bool,
}
impl AtomicDownload {
    fn create(destination: &Path) -> Result<Self, Error> {
        if destination.as_os_str().is_empty() || destination.file_name().is_none() {
            return Err(Error::Local("download requires a local destination filename".into()));
        }
        let path = if destination.is_absolute() {
            destination.to_owned()
        } else {
            std::env::current_dir()
                .map_err(|_| Error::Local("could not resolve local download path".into()))?
                .join(destination)
        };
        let parent = path.parent().ok_or_else(|| Error::Local("invalid local download path".into()))?;
        std::fs::create_dir_all(parent)
            .map_err(|_| Error::Local("could not create local download directory".into()))?;
        let parent =
            parent.canonicalize().map_err(|_| Error::Local("could not resolve local download directory".into()))?;
        let destination = parent.join(path.file_name().unwrap());
        if destination.is_dir() {
            return Err(Error::Local("download destination is a directory".into()));
        }
        let temporary = parent.join(format!(".sb-download-{}.part", uuid::Uuid::now_v7().simple()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&temporary)
            .map_err(|_| Error::Local("could not create a private local download file".into()))?;
        Ok(Self { file, temporary, destination, committed: false })
    }
    fn commit(mut self) -> Result<PathBuf, Error> {
        self.file
            .sync_all()
            .map_err(|_| Error::Local("could not flush local download; destination was not replaced".into()))?;
        std::fs::rename(&self.temporary, &self.destination)
            .map_err(|_| Error::Local("could not finalize local download; destination was not replaced".into()))?;
        self.committed = true;
        Ok(self.destination.clone())
    }
}
impl Drop for AtomicDownload {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

fn download_arguments(args: &[String]) -> Result<(String, String, Vec<String>), Error> {
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
    let mut options = Vec::new();
    let mut words = Vec::new();
    let mut index = 0;
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
            words.push(value.clone());
            index += 1;
        }
    }
    if words.len() != 3 || words[0] != "download" || words[1].is_empty() || words[2].is_empty() {
        return Err(Error::Local(
            "download requires one link selector and one local destination: download <selector> <path>".into(),
        ));
    }
    if options.iter().any(|option| option == "--download-path" || option.starts_with("--download-path=")) {
        return Err(Error::Local(
            "download takes an explicit local destination; --download-path is not supported for remote browsers".into(),
        ));
    }
    Ok((words[1].clone(), words[2].clone(), options))
}

pub(super) fn capture_script(key: &str) -> String {
    let key = serde_json::to_string(key).unwrap();
    format!(
        r#"(() => {{
 const key={key},patches=[],marker=Symbol.for('silicon-browser.attribute-resolver'),s={{target:null,captures:0,restore:null,controller:null,reader:null,timer:null}};window[key]=s;
 s.restore=()=>{{clearTimeout(s.timer);for(const p of patches){{p.meta.active=false;let current=p.proto.getAttribute;while(current&&current[marker]&&!current[marker].active){{Object.defineProperty(p.proto,'getAttribute',current[marker].descriptor);current=p.proto.getAttribute;}}}}patches.length=0;}};
 const seen=new Set();const visit=w=>{{if(seen.has(w))return;seen.add(w);try{{const proto=w.Element.prototype,descriptor=Object.getOwnPropertyDescriptor(proto,'getAttribute');if(!descriptor||typeof descriptor.value!=='function')return;const original=descriptor.value,meta={{active:true,descriptor}};const capture=function(name){{if(meta.active&&name===key){{s.target=this;s.captures++;return key;}}return original.call(this,name);}};Object.defineProperty(capture,marker,{{value:meta}});Object.defineProperty(proto,'getAttribute',{{...descriptor,value:capture}});patches.push({{proto,meta}});for(let i=0;i<w.frames.length;i++)visit(w.frames[i]);}}catch(_){{}}}};
 visit(window);s.timer=setTimeout(()=>{{s.restore();delete window[key];}},10000);return {{ready:true}};
}})()"#
    )
}

fn begin_script(key: &str) -> String {
    let key = serde_json::to_string(key).unwrap();
    format!(
        r#"(async () => {{
 const key={key},s=window[key];if(!s)throw Error('download resolver expired');s.restore();
 const t=s.target;if(s.captures!==1||!t||!t.isConnected||t.tagName!=='A')return {{ready:false}};
 const w=t.ownerDocument.defaultView;if(!w||!(t instanceof w.HTMLAnchorElement)||!t.hasAttribute('href')||t.hasAttribute('onclick')||typeof t.onclick==='function')return {{ready:false}};
 const u=new w.URL(t.href,t.ownerDocument.baseURI);if(u.username||u.password)return {{ready:false}};
 if(u.protocol==='http:'||u.protocol==='https:'){{if(u.origin!==w.location.origin)return {{ready:false}};}}
 else if(u.protocol==='blob:'){{if(u.origin!==w.location.origin)return {{ready:false}};}}
 else if(u.protocol!=='data:')return {{ready:false}};
 s.controller=new w.AbortController();s.offset=0;s.fnv=2166136261;s.pending=null;s.pendingOffset=0;s.expected=null;
 s.arm=()=>{{clearTimeout(s.timer);s.timer=setTimeout(()=>{{s.controller.abort();if(s.reader)s.reader.cancel().catch(()=>{{}});delete window[key];}},30000);}};s.arm();
 const response=await w.fetch(u.href,{{method:'GET',credentials:'include',mode:(u.protocol==='http:'||u.protocol==='https:')?'same-origin':'cors',redirect:'error',cache:'no-store',signal:s.controller.signal}});
 if(!response.ok||response.type==='opaque')throw Error('download response is unavailable');
 const length=response.headers.get('content-length'),encoding=response.headers.get('content-encoding');if(!encoding&&length&&/^\d+$/.test(length)){{const n=Number(length);if(Number.isSafeInteger(n))s.expected=n;}}
 s.reader=response.body?response.body.getReader():null;s.arm();return {{ready:true}};
}})()"#
    )
}

fn read_script(key: &str) -> String {
    let key = serde_json::to_string(key).unwrap();
    format!(
        r#"(async () => {{
 const s=window[{key}];if(!s||!s.arm)throw Error('download transfer expired');s.arm();
 while(!s.pending||s.pendingOffset>=s.pending.length){{const part=s.reader?await s.reader.read():{{done:true}};if(part.done){{if(s.expected!==null&&s.expected!==s.offset)throw Error('download length differs');s.arm();return {{done:true,offset:s.offset,bytes:'',fnv:s.fnv}};}}s.pending=part.value;s.pendingOffset=0;}}
 const b=s.pending.subarray(s.pendingOffset,Math.min(s.pending.length,s.pendingOffset+{chunk})),offset=s.offset;let binary='';
 for(let i=0;i<b.length;i+=16384)binary+=String.fromCharCode(...b.subarray(i,i+16384));for(const value of b)s.fnv=Math.imul(s.fnv^value,16777619)>>>0;
 s.pendingOffset+=b.length;s.offset+=b.length;if(!Number.isSafeInteger(s.offset))throw Error('download is too large');s.arm();return {{done:false,offset,bytes:btoa(binary),fnv:s.fnv}};
}})()"#,
        chunk = CHUNK_BYTES
    )
}

fn cleanup_script(key: &str) -> String {
    let key = serde_json::to_string(key).unwrap();
    format!(
        r#"(() => {{const key={key},s=window[key];if(s){{s.restore();if(s.controller)s.controller.abort();if(s.reader)s.reader.cancel().catch(()=>{{}});delete window[key];}}return true;}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_download_preserves_existing_file_and_removes_temporary() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("existing.txt");
        std::fs::write(&destination, b"sentinel").unwrap();
        let temporary;
        {
            let mut download = AtomicDownload::create(&destination).unwrap();
            temporary = download.temporary.clone();
            download.file.write_all(b"partial replacement").unwrap();
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"sentinel");
        assert!(!temporary.exists());
    }
    #[test]
    fn successful_download_atomically_replaces_with_exact_binary_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        let expected: Vec<u8> = (0..CHUNK_BYTES + 71).map(|index| (index % 251) as u8).collect();
        let mut download = AtomicDownload::create(&destination).unwrap();
        download.file.write_all(&expected).unwrap();
        download.commit().unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), expected);
    }
    #[test]
    fn invalid_chunks_never_count_as_a_successful_download() {
        let valid = json!({"offset":0,"done":false,"bytes":STANDARD.encode([0,255,1])});
        assert_eq!(decode_chunk(&valid, 0).unwrap(), [0, 255, 1]);
        assert!(decode_chunk(&valid, 3).is_err());
        for part in [
            json!({"offset":0,"done":false,"bytes":""}),
            json!({"offset":0,"done":true,"bytes":"YQ=="}),
            json!({"offset":0,"done":false,"bytes":"not base64"}),
        ] {
            assert!(decode_chunk(&part, 0).is_err());
        }
        assert!(
            decode_chunk(&json!({"offset":0,"done":false,"bytes":STANDARD.encode(vec![0;CHUNK_BYTES+1])}), 0).is_err()
        );
    }
    #[test]
    fn download_options_do_not_consume_local_filenames_or_global_values_as_commands() {
        let args =
            ["--user-agent", "download", "--json", "download", "@e2", "./my file.bin", "--pin-tab"].map(String::from);
        let (selector, path, options) = download_arguments(&args).unwrap();
        assert_eq!(selector, "@e2");
        assert_eq!(path, "./my file.bin");
        assert_eq!(options, ["--user-agent", "download", "--json", "--pin-tab"]);
        for words in [
            vec!["download", "@e2"],
            vec!["download", "@e2", "file", "extra"],
            vec!["download", "@e2", "file", "--download-path", "dir"],
        ] {
            assert!(download_arguments(&words.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
        }
    }
}
