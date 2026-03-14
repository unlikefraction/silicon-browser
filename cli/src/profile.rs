// profile.rs - Profile pack/unpack/push/pull for silicon-browser.
//
// Enables syncing browser profiles (cookies, fingerprint, login state) between
// machines. Profiles are encrypted with AES-256-GCM before transfer.
//
// Usage:
//   silicon-browser profile list
//   silicon-browser profile pack <name> -p <password>
//   silicon-browser profile unpack <file> -p <password>
//   silicon-browser profile push <name> <user@host> [-p <password>]
//   silicon-browser profile pull <name> <user@host> [-p <password>]

use crate::color;
use aes_gcm::{aead::Aead, aead::KeyInit, Aes256Gcm};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};

/// Magic bytes at the start of a .silicon file to identify it.
const MAGIC: &[u8; 8] = b"SILICON\x01";

fn get_profiles_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".silicon-browser")
        .join("profiles")
}

/// Derive a 256-bit key from a password using SHA-256.
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

    // Format: MAGIC (8) + IV (12) + encrypted data (includes 16-byte auth tag)
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
        .map_err(|_| "Decryption failed — wrong password?".to_string())
}

/// Create a tar.gz archive of a directory, returning the bytes.
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

/// Pack a profile into an encrypted .silicon file.
pub fn pack_profile(name: &str, password: &str, output: Option<&str>) {
    let profile_dir = get_profiles_dir().join(name);
    if !profile_dir.exists() {
        eprintln!(
            "{} Profile '{}' not found at {}",
            color::error_indicator(),
            name,
            profile_dir.display()
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
        eprintln!(
            "{} File not found: {}",
            color::error_indicator(),
            file
        );
        exit(1);
    }

    // Derive profile name from filename if not specified
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

    // Back up existing profile if present
    if profile_dir.exists() {
        let backup = get_profiles_dir().join(format!("{}.backup", profile_name));
        let _ = fs::remove_dir_all(&backup);
        let _ = fs::rename(&profile_dir, &backup);
        println!(
            "  Existing profile backed up to {}.backup/",
            profile_name
        );
    }

    match untar_gz(&tar_bytes, &profile_dir) {
        Ok(()) => {
            println!(
                "{} Unpacked -> profile '{}' at {}",
                color::success_indicator(),
                profile_name,
                profile_dir.display()
            );
        }
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

/// Push a profile to a remote server over SSH.
pub fn push_profile(name: &str, remote: &str, password: &str) {
    let profile_dir = get_profiles_dir().join(name);
    if !profile_dir.exists() {
        eprintln!(
            "{} Profile '{}' not found",
            color::error_indicator(),
            name
        );
        exit(1);
    }

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

    // Write to temp file
    let tmp_file = std::env::temp_dir().join(format!("{}.silicon", name));
    if let Err(e) = fs::write(&tmp_file, &encrypted) {
        eprintln!("{} Failed to write temp file: {}", color::error_indicator(), e);
        exit(1);
    }

    let size_mb = encrypted.len() as f64 / 1_048_576.0;
    println!(
        "Pushing '{}' to {} ({:.1} MB, encrypted)...",
        name, remote, size_mb
    );

    // Parse remote: user@host or user@host:/path
    let (ssh_dest, remote_path) = if remote.contains(':') {
        let parts: Vec<&str> = remote.splitn(2, ':').collect();
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (
            remote.to_string(),
            format!("~/.silicon-browser/profiles/{}.silicon", name),
        )
    };

    // Ensure remote directory exists
    let mkdir_cmd = format!(
        "mkdir -p $(dirname {})",
        shell_escape(&remote_path)
    );
    let _ = Command::new("ssh")
        .args([&ssh_dest, &mkdir_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // SCP the file
    let scp_dest = format!("{}:{}", ssh_dest, remote_path);
    let status = Command::new("scp")
        .args(["-q", &tmp_file.to_string_lossy(), &scp_dest])
        .status();

    let _ = fs::remove_file(&tmp_file);

    match status {
        Ok(s) if s.success() => {
            println!(
                "{} Pushed '{}' to {}",
                color::success_indicator(),
                name,
                scp_dest
            );
            println!();
            println!("  On the server, run:");
            println!(
                "    silicon-browser profile unpack ~/.silicon-browser/profiles/{}.silicon -p <password>",
                name
            );
        }
        Ok(_) => {
            eprintln!("{} SCP failed. Check SSH access to {}", color::error_indicator(), ssh_dest);
            exit(1);
        }
        Err(e) => {
            eprintln!("{} Failed to run scp: {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

/// Pull a profile from a remote server over SSH.
pub fn pull_profile(name: &str, remote: &str, password: &str) {
    // Parse remote
    let (ssh_dest, remote_path) = if remote.contains(':') {
        let parts: Vec<&str> = remote.splitn(2, ':').collect();
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (
            remote.to_string(),
            format!("~/.silicon-browser/profiles/{}.silicon", name),
        )
    };

    let scp_src = format!("{}:{}", ssh_dest, remote_path);
    let tmp_file = std::env::temp_dir().join(format!("{}.silicon", name));

    println!("Pulling '{}' from {}...", name, scp_src);

    let status = Command::new("scp")
        .args(["-q", &scp_src, &tmp_file.to_string_lossy()])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            eprintln!(
                "{} SCP failed. Check SSH access and that the profile exists on {}",
                color::error_indicator(),
                ssh_dest
            );
            exit(1);
        }
        Err(e) => {
            eprintln!("{} Failed to run scp: {}", color::error_indicator(), e);
            exit(1);
        }
    }

    // Unpack it
    unpack_profile(&tmp_file.to_string_lossy(), password, Some(name));
    let _ = fs::remove_file(&tmp_file);
}

/// List all profiles.
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
            let ft = entry.file_type().unwrap_or_else(|_| {
                // Fallback: treat as file
                fs::metadata(entry.path())
                    .map(|m| m.file_type())
                    .unwrap_or_else(|_| entry.file_type().unwrap())
            });
            if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            } else if ft.is_dir() {
                total += dir_size(&entry.path());
            }
        }
    }
    total
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Entry point: parse `silicon-browser profile <subcommand>` args.
pub fn run_profile_command(args: &[String]) {
    let subcommand = args.first().map(|s| s.as_str());

    match subcommand {
        Some("list") | None => list_profiles(),

        Some("pack") => {
            let name = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile pack <name> -p <password>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            let output = get_flag_value(args, "--out").or_else(|| get_flag_value(args, "-o"));
            pack_profile(name, &password, output.as_deref());
        }

        Some("unpack") => {
            let file = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile unpack <file.silicon> -p <password>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            let name = get_flag_value(args, "--as");
            unpack_profile(file, &password, name.as_deref());
        }

        Some("push") => {
            let name = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile push <name> <user@host> -p <password>", color::error_indicator());
                exit(1);
            });
            let remote = args.get(2).unwrap_or_else(|| {
                eprintln!("{} Missing remote. Usage: silicon-browser profile push <name> <user@host>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            push_profile(name, remote, &password);
        }

        Some("pull") => {
            let name = args.get(1).unwrap_or_else(|| {
                eprintln!("{} Usage: silicon-browser profile pull <name> <user@host> -p <password>", color::error_indicator());
                exit(1);
            });
            let remote = args.get(2).unwrap_or_else(|| {
                eprintln!("{} Missing remote. Usage: silicon-browser profile pull <name> <user@host>", color::error_indicator());
                exit(1);
            });
            let password = get_password_from_args(args);
            pull_profile(name, remote, &password);
        }

        Some(cmd) => {
            eprintln!("{} Unknown profile command: {}", color::error_indicator(), cmd);
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  silicon-browser profile list");
            eprintln!("  silicon-browser profile pack <name> -p <password>");
            eprintln!("  silicon-browser profile unpack <file> -p <password> [--as <name>]");
            eprintln!("  silicon-browser profile push <name> <user@host> -p <password>");
            eprintln!("  silicon-browser profile pull <name> <user@host> -p <password>");
            exit(1);
        }
    }
}

fn get_password_from_args(args: &[String]) -> String {
    // Check -p or --password flag
    if let Some(p) = get_flag_value(args, "-p") {
        return p;
    }
    if let Some(p) = get_flag_value(args, "--password") {
        return p;
    }

    // Check env var
    if let Ok(p) = std::env::var("SILICON_BROWSER_PROFILE_PASSWORD") {
        return p;
    }

    // Prompt
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
        .map(|s| s.clone())
}
