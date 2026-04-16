use crate::color;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};

const LAST_KNOWN_GOOD_URL: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

// ---------------------------------------------------------------------------
// CloakBrowser constants
// ---------------------------------------------------------------------------

/// CloakBrowser version per platform (they differ).
#[cfg(target_os = "macos")]
const CLOAKBROWSER_VERSION: &str = "145.0.7632.109.2";
#[cfg(target_os = "linux")]
const CLOAKBROWSER_VERSION: &str = "145.0.7632.159.7";
#[cfg(target_os = "windows")]
const CLOAKBROWSER_VERSION: &str = "145.0.7632.109.2";
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const CLOAKBROWSER_VERSION: &str = "145.0.7632.109.2";

const CLOAKBROWSER_BASE_URL: &str = "https://cloakbrowser.dev";

pub fn get_browsers_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".silicon-browser")
        .join("browsers")
}

/// Returns the CloakBrowser cache directory (~/.silicon-browser/browsers/cloakbrowser-<version>).
pub fn get_cloakbrowser_dir() -> PathBuf {
    get_browsers_dir().join(format!("cloakbrowser-{}", CLOAKBROWSER_VERSION))
}

// ---------------------------------------------------------------------------
// CloakBrowser binary discovery
// ---------------------------------------------------------------------------

/// Find installed CloakBrowser binary. This is the primary browser for silicon-browser.
pub fn find_installed_cloakbrowser() -> Option<PathBuf> {
    // 1. Check env override
    if let Ok(path) = std::env::var("CLOAKBROWSER_BINARY_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Check standard CloakBrowser cache dir (~/.cloakbrowser/)
    if let Some(home) = dirs::home_dir() {
        let cache_dir = std::env::var("CLOAKBROWSER_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".cloakbrowser"));

        if let Ok(entries) = fs::read_dir(&cache_dir) {
            let mut versions: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("chromium-"))
                })
                .collect();
            versions.sort_by_key(|b| std::cmp::Reverse(b.file_name()));

            for entry in versions {
                if let Some(bin) = cloakbrowser_binary_in_dir(&entry.path()) {
                    if bin.exists() {
                        return Some(bin);
                    }
                }
            }
        }
    }

    // 3. Check our own browsers dir
    let our_dir = get_cloakbrowser_dir();
    if let Some(bin) = cloakbrowser_binary_in_dir(&our_dir) {
        if bin.exists() {
            return Some(bin);
        }
    }

    None
}

fn cloakbrowser_binary_in_dir(dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // CloakBrowser on macOS: Chromium.app/Contents/MacOS/Chromium
        let app = dir.join("Chromium.app/Contents/MacOS/Chromium");
        if app.exists() {
            return Some(app);
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        let bin = dir.join("chrome");
        if bin.exists() {
            return Some(bin);
        }
        None
    }

    #[cfg(target_os = "windows")]
    {
        let bin = dir.join("chrome.exe");
        if bin.exists() {
            return Some(bin);
        }
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Chrome binary discovery (fallback)
// ---------------------------------------------------------------------------

pub fn find_installed_chrome() -> Option<PathBuf> {
    let browsers_dir = get_browsers_dir();
    if !browsers_dir.exists() {
        return None;
    }

    let mut versions: Vec<_> = fs::read_dir(&browsers_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("chrome-"))
        })
        .collect();

    versions.sort_by_key(|b| std::cmp::Reverse(b.file_name()));

    for entry in versions {
        if let Some(bin) = chrome_binary_in_dir(&entry.path()) {
            if bin.exists() {
                return Some(bin);
            }
        }
    }

    None
}

fn chrome_binary_in_dir(dir: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app =
            dir.join("Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
        if app.exists() {
            return Some(app);
        }
        let inner = dir.join("chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
        if inner.exists() {
            return Some(inner);
        }
        let inner_x64 = dir.join(
            "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        );
        if inner_x64.exists() {
            return Some(inner_x64);
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        let bin = dir.join("chrome");
        if bin.exists() {
            return Some(bin);
        }
        let inner = dir.join("chrome-linux64/chrome");
        if inner.exists() {
            return Some(inner);
        }
        None
    }

    #[cfg(target_os = "windows")]
    {
        let bin = dir.join("chrome.exe");
        if bin.exists() {
            return Some(bin);
        }
        let inner = dir.join("chrome-win64/chrome.exe");
        if inner.exists() {
            return Some(inner);
        }
        None
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn platform_key() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "mac-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "mac-x64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        panic!("Unsupported platform for Chrome for Testing download")
    }
}

/// CloakBrowser platform tag for download URL.
fn cloakbrowser_platform_tag() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "darwin-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "darwin-x64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "win-x64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        panic!("Unsupported platform for CloakBrowser download")
    }
}

// ---------------------------------------------------------------------------
// Download helpers
// ---------------------------------------------------------------------------

async fn fetch_download_url() -> Result<(String, String), String> {
    let resp = reqwest::get(LAST_KNOWN_GOOD_URL)
        .await
        .map_err(|e| format!("Failed to fetch version info: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse version info: {}", e))?;

    let channel = body
        .get("channels")
        .and_then(|c| c.get("Stable"))
        .ok_or("No Stable channel found in version info")?;

    let version = channel
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or("No version string found")?
        .to_string();

    let platform = platform_key();

    let url = channel
        .get("downloads")
        .and_then(|d| d.get("chrome"))
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter().find_map(|entry| {
                if entry.get("platform")?.as_str()? == platform {
                    Some(entry.get("url")?.as_str()?.to_string())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| format!("No download URL found for platform: {}", platform))?;

    Ok((version, url))
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = reqwest::get(url)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    let total = resp.content_length();
    let mut bytes = Vec::new();
    let mut stream = resp;
    let mut downloaded: u64 = 0;
    let mut last_pct: u64 = 0;

    loop {
        let chunk = stream
            .chunk()
            .await
            .map_err(|e| format!("Download error: {}", e))?;
        match chunk {
            Some(data) => {
                downloaded += data.len() as u64;
                bytes.extend_from_slice(&data);

                if let Some(total) = total {
                    let pct = (downloaded * 100) / total;
                    if pct >= last_pct + 5 {
                        last_pct = pct;
                        let mb = downloaded as f64 / 1_048_576.0;
                        let total_mb = total as f64 / 1_048_576.0;
                        eprint!("\r  {:.0}/{:.0} MB ({pct}%)", mb, total_mb);
                        let _ = io::stderr().flush();
                    }
                }
            }
            None => break,
        }
    }

    eprintln!();
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

fn extract_zip(bytes: Vec<u8>, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create directory: {}", e))?;

    let cursor = io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to read zip archive: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {}", e))?;

        let enclosed = match file.enclosed_name() {
            Some(name) => name.to_owned(),
            None => continue,
        };
        let raw_name = enclosed.to_string_lossy().to_string();
        let rel_path = raw_name
            .strip_prefix("chrome-")
            .and_then(|s| s.split_once('/'))
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(raw_name.clone());

        if rel_path.is_empty() {
            continue;
        }

        let out_path = dest.join(&rel_path);

        // Defense-in-depth: ensure the resolved path is inside dest
        if !out_path.starts_with(dest) {
            continue;
        }

        if file.is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir {}: {}", out_path.display(), e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create parent dir {}: {}", parent.display(), e)
                })?;
            }
            let mut out_file = fs::File::create(&out_path)
                .map_err(|e| format!("Failed to create file {}: {}", out_path.display(), e))?;
            io::copy(&mut file, &mut out_file)
                .map_err(|e| format!("Failed to write {}: {}", out_path.display(), e))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    Ok(())
}

/// Extract a .tar.gz archive to dest, flattening one level of directory nesting.
fn extract_tar_gz(bytes: Vec<u8>, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    fs::create_dir_all(dest).map_err(|e| format!("Failed to create directory: {}", e))?;

    let cursor = io::Cursor::new(bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = Archive::new(gz);

    for entry_result in archive
        .entries()
        .map_err(|e| format!("Failed to read tar archive: {}", e))?
    {
        let mut entry = entry_result.map_err(|e| format!("Failed to read tar entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to get entry path: {}", e))?
            .to_path_buf();

        let out_path = dest.join(&path);

        // Defense-in-depth: ensure path is inside dest
        if !out_path.starts_with(dest) {
            continue;
        }

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir {}: {}", out_path.display(), e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!("Failed to create parent dir {}: {}", parent.display(), e)
                })?;
            }
            entry
                .unpack(&out_path)
                .map_err(|e| format!("Failed to extract {}: {}", out_path.display(), e))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Install commands
// ---------------------------------------------------------------------------

pub fn run_install(with_deps: bool) {
    if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        eprintln!(
            "{} Neither CloakBrowser nor Chrome for Testing provide Linux ARM64 builds.",
            color::error_indicator()
        );
        eprintln!("  Install Chromium from your system package manager instead:");
        eprintln!("    sudo apt install chromium-browser   # Debian/Ubuntu");
        eprintln!("    sudo dnf install chromium            # Fedora");
        eprintln!("  Then use: silicon-browser --executable-path /usr/bin/chromium");
        exit(1);
    }

    let is_linux = cfg!(target_os = "linux");

    if is_linux {
        if with_deps {
            install_linux_deps();
        } else {
            println!(
                "{} Linux detected. If browser fails to launch, run:",
                color::warning_indicator()
            );
            println!("  silicon-browser install --with-deps");
            println!();
        }
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            eprintln!(
                "{} Failed to create runtime: {}",
                color::error_indicator(),
                e
            );
            exit(1);
        });

    // Step 1: Install CloakBrowser (primary stealth browser)
    install_cloakbrowser(&rt);

    // Step 2: Install Chrome for Testing (fallback)
    install_chrome(&rt, is_linux, with_deps);
}

fn install_cloakbrowser(rt: &tokio::runtime::Runtime) {
    println!("{}", color::cyan("Installing CloakBrowser (stealth Chromium)..."));

    let dest = get_cloakbrowser_dir();

    if let Some(bin) = cloakbrowser_binary_in_dir(&dest) {
        if bin.exists() {
            println!(
                "{} CloakBrowser {} is already installed",
                color::success_indicator(),
                CLOAKBROWSER_VERSION
            );
            return;
        }
    }

    let tag = cloakbrowser_platform_tag();

    // CloakBrowser uses .tar.gz on macOS/Linux, .zip on Windows
    let ext = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };

    let base_url = std::env::var("CLOAKBROWSER_DOWNLOAD_URL")
        .unwrap_or_else(|_| CLOAKBROWSER_BASE_URL.to_string());

    let url = format!(
        "{}/chromium-v{}/cloakbrowser-{}.{}",
        base_url, CLOAKBROWSER_VERSION, tag, ext
    );

    println!(
        "  Downloading CloakBrowser {} for {}",
        CLOAKBROWSER_VERSION, tag
    );
    println!("  {}", url);

    let bytes = match rt.block_on(download_bytes(&url)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "{} Failed to download CloakBrowser: {}",
                color::warning_indicator(),
                e
            );
            eprintln!("  Will fall back to Chrome for Testing.");
            return;
        }
    };

    let result = if cfg!(target_os = "windows") {
        extract_zip(bytes, &dest)
    } else {
        extract_tar_gz(bytes, &dest)
    };

    match result {
        Ok(()) => {
            println!(
                "{} CloakBrowser {} installed successfully",
                color::success_indicator(),
                CLOAKBROWSER_VERSION
            );
            println!("  Location: {}", dest.display());
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&dest);
            eprintln!(
                "{} Failed to extract CloakBrowser: {}",
                color::warning_indicator(),
                e
            );
            eprintln!("  Will fall back to Chrome for Testing.");
        }
    }
}

fn install_chrome(rt: &tokio::runtime::Runtime, is_linux: bool, with_deps: bool) {
    println!();
    println!(
        "{}",
        color::cyan("Installing Chrome for Testing (fallback)...")
    );

    let (version, url) = match rt.block_on(fetch_download_url()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    let dest = get_browsers_dir().join(format!("chrome-{}", version));

    if let Some(bin) = chrome_binary_in_dir(&dest) {
        if bin.exists() {
            println!(
                "{} Chrome {} is already installed",
                color::success_indicator(),
                version
            );
            return;
        }
    }

    println!("  Downloading Chrome {} for {}", version, platform_key());
    println!("  {}", url);

    let bytes = match rt.block_on(download_bytes(&url)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    };

    match extract_zip(bytes, &dest) {
        Ok(()) => {
            println!(
                "{} Chrome {} installed successfully",
                color::success_indicator(),
                version
            );
            println!("  Location: {}", dest.display());

            if is_linux && !with_deps {
                println!();
                println!(
                    "{} If you see \"shared library\" errors when running, use:",
                    color::yellow("Note:")
                );
                println!("  silicon-browser install --with-deps");
            }
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&dest);
            eprintln!("{} {}", color::error_indicator(), e);
            exit(1);
        }
    }
}

fn install_linux_deps() {
    println!("{}", color::cyan("Installing system dependencies..."));

    let (pkg_mgr, deps) = if which_exists("apt-get") {
        let libasound = if package_exists_apt("libasound2t64") {
            "libasound2t64"
        } else {
            "libasound2"
        };

        (
            "apt-get",
            vec![
                "libxcb-shm0",
                "libx11-xcb1",
                "libx11-6",
                "libxcb1",
                "libxext6",
                "libxrandr2",
                "libxcomposite1",
                "libxcursor1",
                "libxdamage1",
                "libxfixes3",
                "libxi6",
                "libgtk-3-0",
                "libpangocairo-1.0-0",
                "libpango-1.0-0",
                "libatk1.0-0",
                "libcairo-gobject2",
                "libcairo2",
                "libgdk-pixbuf-2.0-0",
                "libxrender1",
                libasound,
                "libfreetype6",
                "libfontconfig1",
                "libdbus-1-3",
                "libnss3",
                "libnspr4",
                "libatk-bridge2.0-0",
                "libdrm2",
                "libxkbcommon0",
                "libatspi2.0-0",
                "libcups2",
                "libxshmfence1",
                "libgbm1",
            ],
        )
    } else if which_exists("dnf") {
        (
            "dnf",
            vec![
                "nss",
                "nspr",
                "atk",
                "at-spi2-atk",
                "cups-libs",
                "libdrm",
                "libXcomposite",
                "libXdamage",
                "libXrandr",
                "mesa-libgbm",
                "pango",
                "alsa-lib",
                "libxkbcommon",
                "libxcb",
                "libX11-xcb",
                "libX11",
                "libXext",
                "libXcursor",
                "libXfixes",
                "libXi",
                "gtk3",
                "cairo-gobject",
            ],
        )
    } else if which_exists("yum") {
        (
            "yum",
            vec![
                "nss",
                "nspr",
                "atk",
                "at-spi2-atk",
                "cups-libs",
                "libdrm",
                "libXcomposite",
                "libXdamage",
                "libXrandr",
                "mesa-libgbm",
                "pango",
                "alsa-lib",
                "libxkbcommon",
            ],
        )
    } else {
        eprintln!(
            "{} No supported package manager found (apt-get, dnf, or yum)",
            color::error_indicator()
        );
        exit(1);
    };

    let install_cmd = match pkg_mgr {
        "apt-get" => {
            format!(
                "sudo apt-get update && sudo apt-get install -y {}",
                deps.join(" ")
            )
        }
        _ => format!("sudo {} install -y {}", pkg_mgr, deps.join(" ")),
    };

    println!("Running: {}", install_cmd);
    let status = Command::new("sh").arg("-c").arg(&install_cmd).status();

    match status {
        Ok(s) if s.success() => {
            println!(
                "{} System dependencies installed",
                color::success_indicator()
            )
        }
        Ok(_) => eprintln!(
            "{} Failed to install some dependencies. You may need to run manually with sudo.",
            color::warning_indicator()
        ),
        Err(e) => eprintln!(
            "{} Could not run install command: {}",
            color::warning_indicator(),
            e
        ),
    }
}

fn which_exists(cmd: &str) -> bool {
    #[cfg(unix)]
    {
        Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn package_exists_apt(pkg: &str) -> bool {
    Command::new("apt-cache")
        .arg("show")
        .arg(pkg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
