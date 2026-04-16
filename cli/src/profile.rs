// profile.rs - Profile management and sync for silicon-browser.
//
// Profiles can be packed/unpacked locally, or synced between machines
// using a temporary HTTP server + 6-digit OTP (no 3rd party needed).
//
// Commands:
//   silicon-browser profile list              List all profiles
//   silicon-browser profile pack <name>       Export encrypted .silicon file
//   silicon-browser profile unpack <file>     Import encrypted .silicon file
//   silicon-browser push <name>               Serve profile for cloning (HTTP + OTP)
//   silicon-browser clone                     Clone a profile from a push server
//   silicon-browser pull <name>               Re-sync a previously cloned profile

use crate::color;
use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, BufRead, Read as _, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};

/// Magic bytes at the start of a .silicon file.
const MAGIC: &[u8; 8] = b"SILICON\x01";

fn get_profiles_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".silicon-browser")
        .join("profiles")
}

/// Derive a 256-bit key from a password/OTP using SHA-256.
fn derive_key(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"silicon-browser-profile-key:");
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Encrypt bytes with AES-256-GCM.
fn encrypt_bytes(data: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let key = derive_key(password);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("Encryption key error: {}", e))?;

    let mut iv = [0u8; 12];
    getrandom::getrandom(&mut iv).map_err(|e| format!("Failed to generate IV: {}", e))?;

    let encrypted = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&iv), data)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let mut output = Vec::with_capacity(8 + 12 + encrypted.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&iv);
    output.extend_from_slice(&encrypted);
    Ok(output)
}

/// Decrypt bytes with AES-256-GCM.
fn decrypt_bytes(data: &[u8], password: &str) -> Result<Vec<u8>, String> {
    if data.len() < 20 || &data[..8] != MAGIC {
        return Err("Not a valid .silicon profile file".to_string());
    }

    let key = derive_key(password);
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("Decryption key error: {}", e))?;

    let iv = &data[8..20];
    let ciphertext = &data[20..];

    cipher
        .decrypt(aes_gcm::Nonce::from_slice(iv), ciphertext)
        .map_err(|_| "Decryption failed — wrong OTP?".to_string())
}

/// Create a tar.gz archive of a directory.
fn tar_gz_dir(dir: &Path) -> Result<Vec<u8>, String> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let buf = Vec::new();
    let enc = GzEncoder::new(buf, Compression::default());
    let mut tar_builder = tar::Builder::new(enc);

    tar_builder
        .append_dir_all(".", dir)
        .map_err(|e| format!("Failed to archive profile: {}", e))?;

    let enc = tar_builder
        .into_inner()
        .map_err(|e| format!("Failed to finalize archive: {}", e))?;
    let bytes = enc
        .finish()
        .map_err(|e| format!("Failed to compress: {}", e))?;
    Ok(bytes)
}

/// Extract a tar.gz archive to a directory.
fn untar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;

    fs::create_dir_all(dest).map_err(|e| format!("Failed to create directory: {}", e))?;

    let cursor = io::Cursor::new(bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(gz);

    archive
        .unpack(dest)
        .map_err(|e| format!("Failed to extract profile: {}", e))?;
    Ok(())
}

/// Generate a 6-digit OTP.
fn generate_otp() -> String {
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf);
    let num = u32::from_le_bytes(buf) % 900000 + 100000;
    num.to_string()
}

/// Get the machine's local IP address.
fn get_local_ip() -> String {
    // Try to find a non-loopback IP by connecting to a public DNS
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

// ---------------------------------------------------------------------------
// Push: serve a profile over HTTP with OTP authentication
// ---------------------------------------------------------------------------

/// Start a temporary HTTP server that serves an encrypted profile.
/// Waits for one successful clone, then shuts down.
pub fn run_push(name: &str) {
    let profile_dir = get_profiles_dir().join(name);
    if !profile_dir.exists() {
        eprintln!(
            "{} Profile '{}' not found. Available profiles:",
            color::error_indicator(),
            name
        );
        list_profiles();
        exit(1);
    }

    // Pack and encrypt
    eprint!("Packing profile '{}'...", name);
    let _ = io::stderr().flush();

    let tar_bytes = match tar_gz_dir(&profile_dir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("\n{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let otp = generate_otp();
    let encrypted = match encrypt_bytes(&tar_bytes, &otp) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("\n{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let size_mb = encrypted.len() as f64 / 1_048_576.0;
    eprintln!(" done ({:.1} MB)", size_mb);

    // Bind to a random port
    let listener = match TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{} Failed to start server: {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let port = listener.local_addr().unwrap().port();
    let local_ip = get_local_ip();

    // Start SSH tunnel for public URL (localhost.run — free, no signup)
    let tunnel = start_tunnel(port);

    println!();
    println!("  {} Serving profile '{}'", color::cyan("●"), name);
    println!();
    println!(
        "  Local:  {}",
        color::bold(&format!("http://{}:{}", local_ip, port))
    );
    if let Some(ref url) = tunnel.public_url {
        println!(
            "  Public: {}",
            color::bold(url)
        );
    }
    println!("  OTP:    {}", color::bold(&otp));
    println!();
    if let Some(ref url) = tunnel.public_url {
        println!("  On any machine, run:");
        println!("    silicon-browser clone {}", url);
    } else {
        println!("  On the same network, run:");
        println!(
            "    silicon-browser clone http://{}:{}",
            local_ip, port
        );
    }
    println!();
    println!("  Waiting for clone...");
    println!();

    // Serve until one successful transfer
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if handle_push_connection(stream, &encrypted, name, &otp) {
                    println!(
                        "  {} Profile '{}' sent successfully. Server closed.",
                        color::success_indicator(),
                        name
                    );
                    // Kill the tunnel process
                    drop(tunnel);
                    return;
                }
            }
            Err(e) => {
                eprintln!("  Connection error: {}", e);
            }
        }
    }
}

/// Handle a single HTTP connection for push.
/// Returns true if the profile was successfully sent.
fn handle_push_connection(mut stream: TcpStream, encrypted: &[u8], name: &str, otp: &str) -> bool {
    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the request — extract the path from "GET /path HTTP/1.1"
    let first_line = request.lines().next().unwrap_or("");
    let request_path = first_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/");

    if request_path.starts_with("/info") {
        // Info endpoint: returns profile name and size (no auth needed)
        let body = format!(
            r#"{{"name":"{}","size":{}}}"#,
            name,
            encrypted.len()
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        return false; // Don't shut down for info requests
    }

    if request_path.starts_with("/clone") || request_path.starts_with("/pull") {
        // Check OTP from query string: /clone?otp=123456
        let has_valid_otp = request_path
            .split('?')
            .nth(1)
            .and_then(|query| {
                query.split('&').find_map(|param| {
                    let mut parts = param.splitn(2, '=');
                    if parts.next() == Some("otp") {
                        parts.next()
                    } else {
                        None
                    }
                })
            })
            .map(|provided_otp| provided_otp == otp)
            .unwrap_or(false);

        if !has_valid_otp {
            let body = r#"{"error":"Invalid OTP"}"#;
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            eprintln!("  {} Invalid OTP attempt from {}", color::warning_indicator(),
                stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".into()));
            return false;
        }

        // Send the encrypted profile
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{}.silicon\"\r\nContent-Length: {}\r\nX-Profile-Name: {}\r\nConnection: close\r\n\r\n",
            name,
            encrypted.len(),
            name
        );
        if stream.write_all(response.as_bytes()).is_err() {
            return false;
        }
        if stream.write_all(encrypted).is_err() {
            return false;
        }
        let _ = stream.flush();
        // Give the tunnel time to forward all bytes
        std::thread::sleep(std::time::Duration::from_millis(500));

        eprintln!(
            "  {} Sent to {}",
            color::success_indicator(),
            stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".into())
        );
        return true; // Successful transfer, shut down
    }

    // Unknown request
    let body = "silicon-browser push server. Use silicon-browser clone to download.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    false
}

// ---------------------------------------------------------------------------
// Clone: download a profile from a push server
// ---------------------------------------------------------------------------

/// Clone a profile from a push server.
pub fn run_clone(url: &str) {
    // Get profile info first
    let info_url = format!("{}/info", url.trim_end_matches('/'));
    let (name, size) = match fetch_profile_info(&info_url) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let size_mb = size as f64 / 1_048_576.0;
    println!(
        "Found profile '{}' ({:.1} MB)",
        name, size_mb
    );

    // Prompt for OTP
    eprint!("OTP: ");
    let _ = io::stderr().flush();
    let mut otp = String::new();
    if io::stdin().read_line(&mut otp).is_err() || otp.trim().is_empty() {
        eprintln!("{} OTP required", color::error_indicator());
        exit(1);
    }
    let otp = otp.trim();

    // Download the encrypted profile
    let clone_url = format!("{}/clone?otp={}", url.trim_end_matches('/'), otp);
    println!("Downloading...");

    let encrypted = match fetch_profile_data(&clone_url) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    // Decrypt
    let tar_bytes = match decrypt_bytes(&encrypted, otp) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    // Extract to profile directory
    let profile_dir = get_profiles_dir().join(&name);

    if profile_dir.exists() {
        let backup = get_profiles_dir().join(format!("{}.backup", &name));
        let _ = fs::remove_dir_all(&backup);
        let _ = fs::rename(&profile_dir, &backup);
        println!("  Existing profile backed up to {}.backup/", &name);
    }

    match untar_gz(&tar_bytes, &profile_dir) {
        Ok(()) => {
            // Save the remote URL for future pulls
            let remote_file = profile_dir.join(".remote");
            let _ = fs::write(&remote_file, url);

            println!(
                "{} Cloned profile '{}' successfully",
                color::success_indicator(),
                name
            );
            println!("  Location: {}", profile_dir.display());
            println!();
            println!("  Use it:");
            println!("    silicon-browser --profile {} open https://example.com", name);
        }
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Pull: re-sync from a push server (same as clone but for existing profile)
// ---------------------------------------------------------------------------

/// Pull (re-sync) a profile. The other machine must be running `push`.
pub fn run_pull(name: &str, url: Option<&str>) {
    let profile_dir = get_profiles_dir().join(name);

    // Try to get URL from args, or from saved .remote file
    let url = if let Some(u) = url {
        u.to_string()
    } else {
        let remote_file = profile_dir.join(".remote");
        match fs::read_to_string(&remote_file) {
            Ok(u) if !u.trim().is_empty() => u.trim().to_string(),
            _ => {
                eprintln!(
                    "{} No URL provided and no saved remote for profile '{}'.",
                    color::error_indicator(),
                    name
                );
                eprintln!("  Usage: silicon-browser pull {} <url>", name);
                exit(1);
            }
        }
    };

    // Get info
    let info_url = format!("{}/info", url.trim_end_matches('/'));
    match fetch_profile_info(&info_url) {
        Ok((remote_name, size)) => {
            let size_mb = size as f64 / 1_048_576.0;
            println!("Pulling '{}' from {} ({:.1} MB)", remote_name, url, size_mb);
        }
        Err(e) => {
            eprintln!("{} Cannot reach push server: {}", color::error_indicator(), e);
            eprintln!("  Make sure the other machine is running: silicon-browser push {}", name);
            exit(1);
        }
    }

    // Prompt for OTP
    eprint!("OTP: ");
    let _ = io::stderr().flush();
    let mut otp = String::new();
    if io::stdin().read_line(&mut otp).is_err() || otp.trim().is_empty() {
        eprintln!("{} OTP required", color::error_indicator());
        exit(1);
    }
    let otp = otp.trim();

    // Download
    let clone_url = format!("{}/pull?otp={}", url.trim_end_matches('/'), otp);
    println!("Downloading...");

    let encrypted = match fetch_profile_data(&clone_url) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    // Decrypt
    let tar_bytes = match decrypt_bytes(&encrypted, otp) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    // Backup and extract
    if profile_dir.exists() {
        let backup = get_profiles_dir().join(format!("{}.backup", name));
        let _ = fs::remove_dir_all(&backup);
        let _ = fs::rename(&profile_dir, &backup);
    }

    match untar_gz(&tar_bytes, &profile_dir) {
        Ok(()) => {
            // Save remote for future pulls
            let remote_file = profile_dir.join(".remote");
            let _ = fs::write(&remote_file, &url);

            println!(
                "{} Pulled profile '{}' successfully",
                color::success_indicator(),
                name
            );
        }
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client helpers (minimal, no external deps needed — use std::net)
// ---------------------------------------------------------------------------

/// Fetch profile info from the push server's /info endpoint.
fn fetch_profile_info(url: &str) -> Result<(String, usize), String> {
    let body = http_get(url)?;
    let text = String::from_utf8_lossy(&body);

    // Parse simple JSON: {"name":"...","size":...}
    let name = text
        .split("\"name\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("unknown")
        .to_string();

    let size = text
        .split("\"size\":")
        .nth(1)
        .and_then(|s| s.split([',', '}'].as_ref()).next())
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);

    Ok((name, size))
}

/// Fetch the encrypted profile data from the push server.
fn fetch_profile_data(url: &str) -> Result<Vec<u8>, String> {
    let body = http_get(url)?;
    if body.len() < 20 {
        // Check if it's an error response
        let text = String::from_utf8_lossy(&body);
        if text.contains("Invalid OTP") {
            return Err("Invalid OTP. Check the code and try again.".to_string());
        }
        return Err(format!("Response too small ({} bytes)", body.len()));
    }
    Ok(body)
}

/// HTTP GET supporting both http:// and https:// URLs.
/// Uses raw TCP for http:// (no deps) and reqwest for https://.
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    if url.starts_with("https://") {
        // Use reqwest (already a dependency) for HTTPS
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("Failed to create runtime: {}", e))?;

        rt.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| format!("HTTP client error: {}", e))?;

            let resp = client.get(url)
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;

            let status = resp.status();
            if status.as_u16() == 403 {
                // Try to read error body
                let body = resp.text().await.unwrap_or_default();
                if body.contains("Invalid OTP") {
                    return Err("Invalid OTP. Check the code and try again.".to_string());
                }
                return Err(format!("Forbidden (403): {}", body));
            }
            if !status.is_success() {
                return Err(format!("HTTP error: {}", status));
            }

            // Read all bytes
            let bytes = resp.bytes()
                .await
                .map_err(|e| format!("Failed to read response: {}", e))?;
            Ok(bytes.to_vec())
        })
    } else {
        // Raw TCP for http:// (no TLS needed)
        let stripped = url.strip_prefix("http://").unwrap_or(url);
        let (host_port, path) = stripped.split_once('/').unwrap_or((stripped, ""));
        let path = format!("/{}", path);

        let mut stream = TcpStream::connect(host_port)
            .map_err(|e| format!("Connection failed to {}: {}", host_port, e))?;

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .ok();

        let host = host_port.split(':').next().unwrap_or(host_port);
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            path, host
        );

        stream
            .write_all(request.as_bytes())
            .map_err(|e| format!("Failed to send request: {}", e))?;

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        let header_end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or("Invalid HTTP response")?;

        let headers = String::from_utf8_lossy(&response[..header_end]);
        let status_line = headers.lines().next().unwrap_or("");
        if status_line.contains("403") {
            return Err("Invalid OTP".to_string());
        }
        if !status_line.contains("200") {
            return Err(format!("HTTP error: {}", status_line));
        }

        Ok(response[header_end + 4..].to_vec())
    }
}

// ---------------------------------------------------------------------------
// SSH Tunnel (localhost.run — free, no signup, no API keys)
// ---------------------------------------------------------------------------

struct Tunnel {
    pub public_url: Option<String>,
    child: Option<std::process::Child>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Start an SSH tunnel to localhost.run for a public URL.
/// Falls back gracefully if SSH is unavailable or tunnel fails.
fn start_tunnel(local_port: u16) -> Tunnel {
    // Skip tunnel if explicitly disabled
    if std::env::var("SILICON_BROWSER_NO_TUNNEL").is_ok() {
        return Tunnel {
            public_url: None,
            child: None,
        };
    }

    eprint!("  Setting up public URL...");
    let _ = io::stderr().flush();

    // Try localhost.run: ssh -R 80:localhost:PORT nokey@localhost.run
    // It prints the public URL to stderr like:
    //   ...tunneled with tls termination, https://xxxxx.lhr.life
    let mut child = match Command::new("ssh")
        .args([
            "-o", "StrictHostKeyChecking=no",
            "-o", "ServerAliveInterval=30",
            "-o", "ConnectTimeout=10",
            "-R", &format!("80:localhost:{}", local_port),
            "nokey@localhost.run",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            eprintln!(" skipped (ssh not available)");
            return Tunnel {
                public_url: None,
                child: None,
            };
        }
    };

    // Read stdout/stderr to find the public URL (with timeout)
    let stdout = child.stdout.take();
    let public_url = if let Some(stdout) = stdout {
        let reader = io::BufReader::new(stdout);
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(15);
        let mut found_url: Option<String> = None;

        for line in reader.lines() {
            if start.elapsed() > timeout {
                break;
            }
            if let Ok(line) = line {
                // localhost.run outputs lines like:
                // "abc123.lhr.life tunneled with tls termination, https://abc123.lhr.life"
                // or just a URL on its own line
                if let Some(url) = extract_tunnel_url(&line) {
                    found_url = Some(url);
                    break;
                }
            }
        }

        found_url
    } else {
        None
    };

    if let Some(ref url) = public_url {
        eprintln!(" {}", color::success_indicator());
        // Re-attach stdout so the tunnel process keeps running
        Tunnel {
            public_url: Some(url.clone()),
            child: Some(child),
        }
    } else {
        eprintln!(" skipped (tunnel failed to connect)");
        let _ = child.kill();
        Tunnel {
            public_url: None,
            child: None,
        }
    }
}

/// Extract a public HTTPS URL from a localhost.run output line.
fn extract_tunnel_url(line: &str) -> Option<String> {
    // Match patterns like "https://xxxx.lhr.life" or "https://xxxx.localhost.run"
    for word in line.split_whitespace() {
        let word = word.trim_end_matches(',');
        if word.starts_with("https://") && (word.contains(".lhr.life") || word.contains(".localhost.run")) {
            return Some(word.to_string());
        }
    }
    // Also check for bare domain patterns
    for word in line.split_whitespace() {
        let word = word.trim_end_matches(',');
        if (word.ends_with(".lhr.life") || word.ends_with(".localhost.run")) && !word.starts_with("https://") {
            return Some(format!("https://{}", word));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pack / Unpack (local file-based export/import)
// ---------------------------------------------------------------------------

/// Pack a profile into an encrypted .silicon file.
pub fn pack_profile(name: &str, password: &str, output: Option<&str>) {
    let profile_dir = get_profiles_dir().join(name);
    if !profile_dir.exists() {
        eprintln!(
            "{} Profile '{}' not found",
            color::error_indicator(),
            name
        );
        exit(1);
    }

    let out_path = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("{}.silicon", name)));

    println!("Packing profile '{}'...", name);

    let tar_bytes = match tar_gz_dir(&profile_dir) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let encrypted = match encrypt_bytes(&tar_bytes, password) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    match fs::write(&out_path, &encrypted) {
        Ok(()) => {
            let size_mb = encrypted.len() as f64 / 1_048_576.0;
            println!(
                "{} Packed '{}' -> {} ({:.1} MB, encrypted)",
                color::success_indicator(),
                name,
                out_path.display(),
                size_mb
            );
        }
        Err(e) => {
            eprintln!("{} Failed to write: {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

/// Unpack an encrypted .silicon file into a profile.
pub fn unpack_profile(file: &str, password: &str, name: Option<&str>) {
    let file_path = PathBuf::from(file);
    if !file_path.exists() {
        eprintln!("{} File not found: {}", color::error_indicator(), file);
        exit(1);
    }

    let profile_name = name.unwrap_or_else(|| {
        file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("imported")
    });

    println!("Unpacking '{}'...", file);

    let encrypted = match fs::read(&file_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} Failed to read file: {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let tar_bytes = match decrypt_bytes(&encrypted, password) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let profile_dir = get_profiles_dir().join(profile_name);

    if profile_dir.exists() {
        let backup = get_profiles_dir().join(format!("{}.backup", profile_name));
        let _ = fs::remove_dir_all(&backup);
        let _ = fs::rename(&profile_dir, &backup);
        println!("  Existing profile backed up to {}.backup/", profile_name);
    }

    match untar_gz(&tar_bytes, &profile_dir) {
        Ok(()) => {
            println!(
                "{} Unpacked -> profile '{}'",
                color::success_indicator(),
                profile_name,
            );
        }
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// List profiles
// ---------------------------------------------------------------------------

pub fn list_profiles() {
    let profiles_dir = get_profiles_dir();

    if !profiles_dir.exists() {
        println!("No profiles yet. Run silicon-browser to create the default 'silicon' profile.");
        return;
    }

    let mut profiles: Vec<(String, u64, bool)> = Vec::new();

    if let Ok(entries) = fs::read_dir(&profiles_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".backup") || name.ends_with(".silicon") {
                continue;
            }
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let has_fingerprint = entry.path().join("fingerprint.seed").exists();
                let size = dir_size(&entry.path());
                profiles.push((name, size, has_fingerprint));
            }
        }
    }

    if profiles.is_empty() {
        println!("No profiles found.");
        return;
    }

    profiles.sort_by(|a, b| a.0.cmp(&b.0));

    println!("{}", color::bold("Profiles:"));
    for (name, size, has_fp) in &profiles {
        let size_str = if *size > 1_048_576 {
            format!("{:.1} MB", *size as f64 / 1_048_576.0)
        } else {
            format!("{:.0} KB", *size as f64 / 1024.0)
        };

        let fp_str = if *has_fp {
            let seed_path = get_profiles_dir().join(name).join("fingerprint.seed");
            fs::read_to_string(&seed_path)
                .map(|s| format!(" (fingerprint: {})", s.trim()))
                .unwrap_or_default()
        } else {
            String::new()
        };

        println!("  {} {} [{}]{}", color::cyan("●"), name, size_str, fp_str);
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                } else if ft.is_dir() {
                    total += dir_size(&entry.path());
                }
            }
        }
    }
    total
}

// ---------------------------------------------------------------------------
// CLI entry points
// ---------------------------------------------------------------------------

/// Entry point for `silicon-browser profile <subcommand>`.
pub fn run_profile_command(args: &[String]) {
    let subcommand = args.first().map(|s| s.as_str());

    match subcommand {
        Some("list") | None => list_profiles(),

        Some("pack") => {
            let name = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile pack <name> --password <pw>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            let output = get_flag_value(args, "--out").or_else(|| get_flag_value(args, "-o"));
            pack_profile(name, &password, output.as_deref());
        }

        Some("unpack") => {
            let file = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile unpack <file> --password <pw>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            let name = get_flag_value(args, "--as");
            unpack_profile(file, &password, name.as_deref());
        }

        Some(cmd) => {
            eprintln!("{} Unknown profile command: {}", color::error_indicator(), cmd);
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  silicon-browser profile list");
            eprintln!("  silicon-browser profile pack <name> --password <pw>");
            eprintln!("  silicon-browser profile unpack <file> --password <pw> [--as <name>]");
            eprintln!();
            eprintln!("  silicon-browser push <name>          Serve profile for cloning");
            eprintln!("  silicon-browser clone <url>           Clone from a push server");
            eprintln!("  silicon-browser pull <name> [<url>]   Re-sync a profile");
            exit(1);
        }
    }
}

fn get_password_from_args(args: &[String]) -> String {
    if let Some(p) = get_flag_value(args, "--password") {
        return p;
    }
    if let Ok(p) = std::env::var("SILICON_BROWSER_PROFILE_PASSWORD") {
        return p;
    }
    eprint!("Password: ");
    let _ = io::stderr().flush();
    let mut password = String::new();
    if io::stdin().read_line(&mut password).is_err() || password.trim().is_empty() {
        eprintln!("{} Password required", color::error_indicator());
        exit(1);
    }
    password.trim().to_string()
}

fn get_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
