use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::discovery::discover_cdp_url;

pub struct ChromeProcess {
    child: Child,
    pub ws_url: String,
    temp_user_data_dir: Option<PathBuf>,
}

impl ChromeProcess {
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Wait for Chrome to exit on its own (after Browser.close CDP command),
    /// falling back to kill() if it doesn't exit within the timeout.
    /// This allows Chrome to flush cookies and other state to the user-data-dir.
    pub fn wait_or_kill(&mut self, timeout: Duration) {
        let start = std::time::Instant::now();
        let poll_interval = Duration::from_millis(50);

        while start.elapsed() < timeout {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(poll_interval),
                Err(_) => break,
            }
        }

        self.kill();
    }
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        self.kill();
        if let Some(ref dir) = self.temp_user_data_dir {
            for attempt in 0..3 {
                match std::fs::remove_dir_all(dir) {
                    Ok(()) => break,
                    Err(_) if attempt < 2 => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: failed to clean up temp profile {}: {}",
                            dir.display(),
                            e
                        );
                    }
                }
            }
        }
    }
}

pub struct LaunchOptions {
    pub headless: bool,
    pub executable_path: Option<String>,
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
    pub profile: Option<String>,
    pub incognito: bool,
    pub args: Vec<String>,
    pub allow_file_access: bool,
    pub extensions: Option<Vec<String>>,
    pub storage_state: Option<String>,
    pub user_agent: Option<String>,
    pub ignore_https_errors: bool,
    pub color_scheme: Option<String>,
    pub download_path: Option<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            headless: true,
            executable_path: None,
            proxy: None,
            proxy_bypass: None,
            profile: None,
            incognito: false,
            args: Vec::new(),
            allow_file_access: false,
            extensions: None,
            storage_state: None,
            user_agent: None,
            ignore_https_errors: false,
            color_scheme: None,
            download_path: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Virtual display (Xvfb) for headless Linux stealth
// ---------------------------------------------------------------------------

/// Ensure a virtual display is available on headless Linux.
/// Installs Xvfb if needed, starts it, and returns the DISPLAY string.
///
/// The Xvfb process is started as a daemon and persists for the lifetime
/// of the silicon-browser daemon. A lock file prevents multiple instances.
#[cfg(target_os = "linux")]
fn ensure_virtual_display() -> Result<String, String> {
    use std::process::Command;

    // Check if Xvfb is already running (e.g. from a previous session)
    let lock_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".silicon-browser")
        .join("xvfb.display");

    if let Ok(display) = std::fs::read_to_string(&lock_path) {
        let display = display.trim().to_string();
        // Verify the display is still alive
        if Command::new("xdpyinfo")
            .env("DISPLAY", &display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Ok(display);
        }
    }

    // Check if Xvfb is installed
    let has_xvfb = Command::new("which")
        .arg("Xvfb")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !has_xvfb {
        // Try to install Xvfb automatically
        eprintln!("[silicon-browser] No display server found. Installing Xvfb for stealth mode...");

        // Try apt (Debian/Ubuntu)
        let apt_result = Command::new("sh")
            .args(["-c", "command -v apt-get && sudo apt-get update -qq && sudo apt-get install -y -qq xvfb >/dev/null 2>&1"])
            .status();

        if !apt_result.map(|s| s.success()).unwrap_or(false) {
            // Try yum (RHEL/CentOS/Amazon Linux)
            let yum_result = Command::new("sh")
                .args(["-c", "command -v yum && sudo yum install -y -q xorg-x11-server-Xvfb >/dev/null 2>&1"])
                .status();

            if !yum_result.map(|s| s.success()).unwrap_or(false) {
                // Try dnf (Fedora)
                let dnf_result = Command::new("sh")
                    .args(["-c", "command -v dnf && sudo dnf install -y -q xorg-x11-server-Xvfb >/dev/null 2>&1"])
                    .status();

                if !dnf_result.map(|s| s.success()).unwrap_or(false) {
                    // Try apk (Alpine)
                    let apk_result = Command::new("sh")
                        .args(["-c", "command -v apk && sudo apk add --quiet xvfb >/dev/null 2>&1"])
                        .status();

                    if !apk_result.map(|s| s.success()).unwrap_or(false) {
                        return Err("Could not install Xvfb. Install manually: apt install xvfb".to_string());
                    }
                }
            }
        }

        // Verify installation
        if !Command::new("which")
            .arg("Xvfb")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return Err("Xvfb installation failed".to_string());
        }

        eprintln!("[silicon-browser] Xvfb installed successfully.");
    }

    // Find a free display number (try :99, :98, etc.)
    let mut display_num = 99;
    let display = loop {
        let sock_path = format!("/tmp/.X11-unix/X{}", display_num);
        if !std::path::Path::new(&sock_path).exists() {
            break format!(":{}", display_num);
        }
        display_num -= 1;
        if display_num < 90 {
            return Err("No free display number found (tried :90-:99)".to_string());
        }
    };

    // Start Xvfb
    let child = Command::new("Xvfb")
        .args([
            &display,
            "-screen", "0", "1920x1080x24",
            "-ac",           // disable access control (no auth needed)
            "-nolisten", "tcp",
            "+extension", "GLX",  // needed for WebGL
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start Xvfb: {}", e))?;

    // Wait a moment for Xvfb to initialize
    std::thread::sleep(Duration::from_millis(500));

    // Verify it's running
    let alive = Command::new("xdpyinfo")
        .env("DISPLAY", &display)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !alive {
        return Err("Xvfb started but display is not responding".to_string());
    }

    // Save the display number and PID for cleanup / reuse
    let _ = std::fs::create_dir_all(lock_path.parent().unwrap());
    let _ = std::fs::write(&lock_path, &display);

    // Save PID for potential cleanup
    let pid_path = lock_path.with_extension("pid");
    let _ = std::fs::write(&pid_path, child.id().to_string());

    // Detach — Xvfb runs as long as the silicon-browser daemon runs.
    // We intentionally leak the Child handle so it isn't killed on drop.
    std::mem::forget(child);

    eprintln!("[silicon-browser] Virtual display started on {display}");
    Ok(display)
}

#[cfg(not(target_os = "linux"))]
fn ensure_virtual_display() -> Result<String, String> {
    Err("Virtual display is only needed on Linux".to_string())
}

// ---------------------------------------------------------------------------
// Profile management
// ---------------------------------------------------------------------------

/// Returns the base directory for named profiles: ~/.silicon-browser/profiles/
fn get_profiles_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".silicon-browser")
        .join("profiles")
}

/// Resolve a profile name/path to an actual directory path.
///
/// - Absolute or relative paths (containing `/` or `\`) are used as-is
/// - Simple names like "work" resolve to ~/.silicon-browser/profiles/work/
/// - None with incognito=false resolves to the default "silicon" profile
/// - None with incognito=true returns None (temp dir will be used)
pub fn resolve_profile(profile: Option<&str>, incognito: bool) -> Option<PathBuf> {
    if incognito {
        return None;
    }

    let name = profile.unwrap_or("silicon");

    // If it looks like a path, use it directly
    if name.contains('/') || name.contains('\\') || name.starts_with('~') {
        return Some(PathBuf::from(expand_tilde(name)));
    }

    // Named profile → ~/.silicon-browser/profiles/<name>/
    Some(get_profiles_dir().join(name))
}

/// Get or create a persistent fingerprint seed for a profile directory.
/// The seed is stored in `<profile_dir>/fingerprint.seed`.
/// Same profile = same fingerprint identity every time.
pub fn get_profile_fingerprint_seed(profile_dir: &Path) -> u64 {
    let seed_path = profile_dir.join("fingerprint.seed");

    // Try to read existing seed
    if let Ok(contents) = std::fs::read_to_string(&seed_path) {
        if let Ok(seed) = contents.trim().parse::<u64>() {
            return seed;
        }
    }

    // Generate new seed and persist it
    let mut buf = [0u8; 8];
    let _ = getrandom::getrandom(&mut buf);
    let seed = 10000 + (u64::from_le_bytes(buf) % 89999);

    // Ensure profile dir exists
    let _ = std::fs::create_dir_all(profile_dir);
    let _ = std::fs::write(&seed_path, seed.to_string());

    seed
}

struct ChromeArgs {
    args: Vec<String>,
    temp_user_data_dir: Option<PathBuf>,
}

fn build_chrome_args(options: &LaunchOptions) -> Result<ChromeArgs, String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-backgrounding-occluded-windows".to_string(),
        "--disable-component-update".to_string(),
        "--disable-default-apps".to_string(),
        "--disable-hang-monitor".to_string(),
        "--disable-popup-blocking".to_string(),
        "--disable-prompt-on-repost".to_string(),
        "--disable-sync".to_string(),
        "--disable-features=Translate".to_string(),
        "--enable-features=NetworkService,NetworkServiceInProcess".to_string(),
        "--metrics-recording-only".to_string(),
        "--password-store=basic".to_string(),
        "--use-mock-keychain".to_string(),
    ];

    // Stealth: add anti-detection Chrome flags
    let stealth_disabled = std::env::var("SILICON_BROWSER_NO_STEALTH").is_ok();
    if !stealth_disabled {
        args.extend(super::super::stealth::get_stealth_chrome_args());
    }

    let has_extensions = options
        .extensions
        .as_ref()
        .is_some_and(|exts| !exts.is_empty());

    // Stealth headless: instead of --headless=new (which has detectable
    // architectural differences), run headed Chrome with the window moved
    // far off-screen. This is indistinguishable from a real human browser
    // because it IS a real headed browser — just not visible.
    //
    // --headless=new is only used as a fallback when SILICON_BROWSER_HEADLESS_REAL=1
    // is set (e.g. in CI environments without a display server).
    let use_real_headless = std::env::var("SILICON_BROWSER_HEADLESS_REAL").is_ok();
    let has_display = std::env::var("DISPLAY").is_ok()
        || std::env::var("WAYLAND_DISPLAY").is_ok()
        || cfg!(target_os = "macos")
        || cfg!(target_os = "windows");

    if options.headless && !has_extensions {
        if use_real_headless {
            // Explicitly requested real headless mode
            args.push("--headless=new".to_string());
        } else if has_display {
            // Display available — use stealth headed mode (offscreen window).
            // Passes ALL headless detection because it IS a real headed browser.
            args.push("--window-position=-32000,-32000".to_string());
        } else if cfg!(target_os = "linux") {
            // Linux with no display — auto-provision Xvfb for full stealth.
            // This gives us a real headed browser on a headless server.
            match ensure_virtual_display() {
                Ok(display) => {
                    std::env::set_var("DISPLAY", &display);
                    args.push("--window-position=-32000,-32000".to_string());
                }
                Err(_) => {
                    // Xvfb unavailable and can't install — fall back to headless
                    args.push("--headless=new".to_string());
                }
            }
        } else {
            args.push("--headless=new".to_string());
        }
    }

    if let Some(ref proxy) = options.proxy {
        args.push(format!("--proxy-server={}", proxy));
    }

    if let Some(ref bypass) = options.proxy_bypass {
        args.push(format!("--proxy-bypass-list={}", bypass));
    }

    // Profile resolution:
    // - --incognito → temp dir (thrown away on close)
    // - --profile <name> → ~/.silicon-browser/profiles/<name>/
    // - --profile /path → use that path directly
    // - (default) → ~/.silicon-browser/profiles/silicon/
    let resolved_profile = resolve_profile(options.profile.as_deref(), options.incognito);

    let temp_user_data_dir = if let Some(ref profile_dir) = resolved_profile {
        std::fs::create_dir_all(profile_dir)
            .map_err(|e| format!("Failed to create profile dir: {}", e))?;
        args.push(format!("--user-data-dir={}", profile_dir.display()));
        None
    } else {
        // Incognito: use a temp dir that gets cleaned up
        let dir =
            std::env::temp_dir().join(format!("silicon-browser-incognito-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create incognito dir: {}", e))?;
        args.push(format!("--user-data-dir={}", dir.display()));
        Some(dir)
    };

    if options.allow_file_access {
        args.push("--allow-file-access-from-files".to_string());
        args.push("--allow-file-access".to_string());
    }

    if let Some(ref exts) = options.extensions {
        if !exts.is_empty() {
            let ext_list = exts.join(",");
            args.push(format!("--load-extension={}", ext_list));
            args.push(format!("--disable-extensions-except={}", ext_list));
        }
    }

    let has_window_size = options
        .args
        .iter()
        .any(|a| a.starts_with("--start-maximized") || a.starts_with("--window-size="));

    if !has_window_size && options.headless && !has_extensions {
        // Stealth: use 1920x1080 (common real resolution) instead of 1280x720
        args.push("--window-size=1920,1080".to_string());
    }

    args.extend(options.args.iter().cloned());

    if should_disable_sandbox(&args) {
        args.push("--no-sandbox".to_string());
    }

    Ok(ChromeArgs {
        args,
        temp_user_data_dir,
    })
}

pub fn launch_chrome(options: &LaunchOptions) -> Result<ChromeProcess, String> {
    let chrome_path = match &options.executable_path {
        Some(p) => PathBuf::from(p),
        None => {
            find_chrome().ok_or("No browser found. Run `silicon-browser install` to download CloakBrowser + Chrome, or use --executable-path.")?
        }
    };

    let ChromeArgs {
        mut args,
        temp_user_data_dir,
    } = build_chrome_args(options)?;

    // CloakBrowser-specific args: --fingerprint for deterministic identity,
    // plus suppress automation-leaking flags that CloakBrowser handles at C++ level
    if is_cloakbrowser(&chrome_path) {
        let stealth_disabled = std::env::var("SILICON_BROWSER_NO_STEALTH").is_ok();
        if !stealth_disabled {
            // Fingerprint seed priority:
            // 1. SILICON_BROWSER_FINGERPRINT env var (explicit override)
            // 2. Profile-pinned seed (persistent per profile)
            // 3. Random seed (incognito mode)
            let resolved_profile = resolve_profile(options.profile.as_deref(), options.incognito);
            let seed = std::env::var("SILICON_BROWSER_FINGERPRINT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| resolved_profile.as_ref().map(|dir| get_profile_fingerprint_seed(dir)))
                .unwrap_or_else(|| {
                    // Incognito: random seed each time
                    let mut buf = [0u8; 8];
                    let _ = getrandom::getrandom(&mut buf);
                    10000 + (u64::from_le_bytes(buf) % 89999)
                });
            args.push(format!("--fingerprint={}", seed));

            // CloakBrowser fingerprint configuration
            args.push("--fingerprint-hardware-concurrency=8".to_string());
            args.push("--fingerprint-device-memory=8".to_string());
        }

        // Remove flags that CloakBrowser handles natively / that conflict.
        // --disable-blink-features=AutomationControlled causes a yellow warning
        // bar in headed mode ("unsupported command-line flag") which is a giveaway.
        args.retain(|a| {
            !a.starts_with("--enable-automation")
                && !a.starts_with("--enable-unsafe-swiftshader")
                && !a.starts_with("--disable-blink-features")
                && !a.starts_with("--disable-infobars")
                && !a.starts_with("--disable-ipc-flooding-protection")
                && !a.starts_with("--enable-gpu-rasterization")
                && !a.starts_with("--enable-zero-copy")
                && !a.starts_with("--enforce-webrtc-ip-permission-check")
                && !a.starts_with("--webrtc-ip-handling-policy")
                && !a.contains("AutomationControlled")
        });
    }

    let cleanup_temp_dir = |dir: &Option<PathBuf>| {
        if let Some(ref d) = dir {
            let _ = std::fs::remove_dir_all(d);
        }
    };

    let mut child = Command::new(&chrome_path)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            cleanup_temp_dir(&temp_user_data_dir);
            format!("Failed to launch Chrome at {:?}: {}", chrome_path, e)
        })?;

    let stderr = child.stderr.take().ok_or_else(|| {
        let _ = child.kill();
        cleanup_temp_dir(&temp_user_data_dir);
        "Failed to capture Chrome stderr".to_string()
    })?;
    let reader = BufReader::new(stderr);

    let ws_url = match wait_for_ws_url(reader) {
        Ok(url) => url,
        Err(e) => {
            let _ = child.kill();
            cleanup_temp_dir(&temp_user_data_dir);
            return Err(e);
        }
    };

    Ok(ChromeProcess {
        child,
        ws_url,
        temp_user_data_dir,
    })
}

fn wait_for_ws_url(reader: BufReader<std::process::ChildStderr>) -> Result<String, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let prefix = "DevTools listening on ";
    let mut stderr_lines: Vec<String> = Vec::new();

    for line in reader.lines() {
        if std::time::Instant::now() > deadline {
            return Err(chrome_launch_error(
                "Timeout waiting for Chrome DevTools URL",
                &stderr_lines,
            ));
        }
        let line = line.map_err(|e| format!("Failed to read Chrome stderr: {}", e))?;
        if let Some(url) = line.strip_prefix(prefix) {
            return Ok(url.trim().to_string());
        }
        stderr_lines.push(line);
    }

    Err(chrome_launch_error(
        "Chrome exited before providing DevTools URL",
        &stderr_lines,
    ))
}

fn chrome_launch_error(message: &str, stderr_lines: &[String]) -> String {
    let relevant: Vec<&String> = stderr_lines
        .iter()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("error")
                || lower.contains("fatal")
                || lower.contains("sandbox")
                || lower.contains("namespace")
                || lower.contains("permission")
                || lower.contains("cannot")
                || lower.contains("failed")
                || lower.contains("abort")
        })
        .collect();

    if relevant.is_empty() {
        if stderr_lines.is_empty() {
            return format!("{} (no stderr output from Chrome)", message);
        }
        let last_lines: Vec<&String> = stderr_lines.iter().rev().take(5).collect();
        return format!(
            "{}\nChrome stderr (last {} lines):\n  {}",
            message,
            last_lines.len(),
            last_lines
                .into_iter()
                .rev()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }

    let hint = if relevant.iter().any(|l| {
        let lower = l.to_lowercase();
        lower.contains("sandbox") || lower.contains("namespace")
    }) {
        "\nHint: try --args \"--no-sandbox\" (required in containers, VMs, and some Linux setups)"
    } else {
        ""
    };

    format!(
        "{}\nChrome stderr:\n  {}{}",
        message,
        relevant
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n  "),
        hint
    )
}

pub fn find_chrome() -> Option<PathBuf> {
    // 1. CloakBrowser (preferred — stealth Chromium with C++ level patches)
    if let Some(p) = crate::install::find_installed_cloakbrowser() {
        return Some(p);
    }

    // 2. Check Chrome downloaded by `silicon-browser install`
    if let Some(p) = crate::install::find_installed_chrome() {
        return Some(p);
    }

    // 3. Check system-installed Chrome
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for c in &candidates {
            let p = PathBuf::from(c);
            if p.exists() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "google-chrome",
            "google-chrome-stable",
            "chromium-browser",
            "chromium",
        ];
        for name in &candidates {
            if let Ok(output) = Command::new("which").arg(name).output() {
                if output.status.success() {
                    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ];
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(&local).join(r"Google\Chrome\Application\chrome.exe");
            if p.exists() {
                return Some(p);
            }
        }
        for c in &candidates {
            let p = PathBuf::from(c);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 4. Fallback: check Playwright's browser cache (for existing installs)
    if let Some(p) = find_playwright_chromium() {
        return Some(p);
    }

    None
}

/// Check if a path points to a CloakBrowser binary (not stock Chrome).
pub fn is_cloakbrowser(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    path_str.contains("cloakbrowser") || path_str.contains(".cloakbrowser")
}

pub fn read_devtools_active_port(user_data_dir: &Path) -> Option<(u16, String)> {
    let path = user_data_dir.join("DevToolsActivePort");
    let content = std::fs::read_to_string(&path).ok()?;
    let mut lines = content.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let ws_path = lines
        .next()
        .unwrap_or("/devtools/browser")
        .trim()
        .to_string();
    Some((port, ws_path))
}

pub async fn auto_connect_cdp() -> Result<String, String> {
    let user_data_dirs = get_chrome_user_data_dirs();

    for dir in &user_data_dirs {
        if let Some((port, ws_path)) = read_devtools_active_port(dir) {
            // Try HTTP endpoint first (pre-M144)
            if let Ok(ws_url) = discover_cdp_url(port).await {
                return Ok(ws_url);
            }
            // M144+: direct WebSocket
            let ws_url = format!("ws://127.0.0.1:{}{}", port, ws_path);
            return Ok(ws_url);
        }
    }

    // Fallback: probe common ports
    for port in [9222u16, 9229] {
        if let Ok(ws_url) = discover_cdp_url(port).await {
            return Ok(ws_url);
        }
    }

    Err("No running Chrome instance found. Launch Chrome with --remote-debugging-port or use --cdp.".to_string())
}

fn get_chrome_user_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let base = home.join("Library/Application Support");
            for name in ["Google/Chrome", "Google/Chrome Canary", "Chromium"] {
                dirs.push(base.join(name));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = dirs::home_dir() {
            let config = home.join(".config");
            for name in ["google-chrome", "google-chrome-unstable", "chromium"] {
                dirs.push(config.join(name));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let base = PathBuf::from(local);
            for name in [
                r"Google\Chrome\User Data",
                r"Google\Chrome SxS\User Data",
                r"Chromium\User Data",
            ] {
                dirs.push(base.join(name));
            }
        }
    }

    dirs
}

/// Returns true if Chrome's sandbox should be disabled because the environment
/// doesn't support it (containers, VMs, CI runners, running as root).
fn should_disable_sandbox(existing_args: &[String]) -> bool {
    if existing_args.iter().any(|a| a == "--no-sandbox") {
        return false; // already set by user
    }

    // CI environments (GitHub Actions, GitLab CI, etc.) often lack user namespace
    // support due to AppArmor or kernel restrictions.
    if std::env::var("CI").is_ok() {
        return true;
    }

    #[cfg(unix)]
    {
        // Root user -- standard container default, Chrome sandbox requires non-root
        if unsafe { libc::geteuid() } == 0 {
            return true;
        }

        // Docker container
        if Path::new("/.dockerenv").exists() {
            return true;
        }

        // Podman container
        if Path::new("/run/.containerenv").exists() {
            return true;
        }

        // Generic container detection: cgroup contains docker/kubepods/lxc
        if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
            if cgroup.contains("docker") || cgroup.contains("kubepods") || cgroup.contains("lxc") {
                return true;
            }
        }
    }

    false
}

/// Search Playwright's browser cache for a Chromium binary.
/// Legacy fallback for users who previously installed Chromium via Playwright.
fn find_playwright_chromium() -> Option<PathBuf> {
    let mut search_dirs = Vec::new();

    if let Ok(custom) = std::env::var("PLAYWRIGHT_BROWSERS_PATH") {
        search_dirs.push(PathBuf::from(custom));
    }

    if let Some(home) = dirs::home_dir() {
        search_dirs.push(home.join(".cache/ms-playwright"));
    }

    for dir in &search_dirs {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut matches: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|n| n.starts_with("chromium-"))
                        .unwrap_or(false)
                })
                .filter_map(|e| {
                    let candidate = build_playwright_binary_path(&e.path());
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                })
                .collect();
            // Sort descending so the newest version wins
            matches.sort();
            matches.reverse();
            if let Some(p) = matches.into_iter().next() {
                return Some(p);
            }
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn build_playwright_binary_path(chromium_dir: &Path) -> PathBuf {
    chromium_dir.join("chrome-linux64/chrome")
}

#[cfg(target_os = "macos")]
fn build_playwright_binary_path(chromium_dir: &Path) -> PathBuf {
    chromium_dir.join("chrome-mac/Chromium.app/Contents/MacOS/Chromium")
}

#[cfg(target_os = "windows")]
fn build_playwright_binary_path(chromium_dir: &Path) -> PathBuf {
    chromium_dir.join("chrome-win/chrome.exe")
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            return home
                .join(rest.strip_prefix('/').unwrap_or(rest))
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::EnvGuard;

    #[cfg(unix)]
    fn spawn_noop_child() -> Child {
        Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[cfg(windows)]
    fn spawn_noop_child() -> Child {
        Command::new("cmd.exe")
            .args(["/C", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn test_find_chrome_returns_some_on_host() {
        // This test only makes sense on systems with Chrome installed
        if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
            let result = find_chrome();
            // Don't assert Some -- CI may not have Chrome
            if let Some(path) = result {
                assert!(path.exists());
            }
        }
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/test/path");
        assert!(!expanded.starts_with('~'));
        assert!(expanded.ends_with("test/path"));
    }

    #[test]
    fn test_expand_tilde_no_tilde() {
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
    }

    #[test]
    fn test_read_devtools_active_port_missing() {
        let result = read_devtools_active_port(Path::new("/nonexistent"));
        assert!(result.is_none());
    }

    #[test]
    fn test_should_disable_sandbox_skips_if_already_set() {
        let args = vec!["--headless=new".to_string(), "--no-sandbox".to_string()];
        assert!(!should_disable_sandbox(&args));
    }

    #[test]
    fn test_chrome_launch_error_no_stderr() {
        let msg = chrome_launch_error("Chrome exited", &[]);
        assert!(msg.contains("no stderr output"));
    }

    #[test]
    fn test_chrome_launch_error_with_sandbox_hint() {
        let lines = vec![
            "some log line".to_string(),
            "Failed to move to new namespace: sandbox error".to_string(),
        ];
        let msg = chrome_launch_error("Chrome exited", &lines);
        assert!(msg.contains("sandbox error"));
        assert!(msg.contains("Hint:"));
        assert!(msg.contains("--no-sandbox"));
    }

    #[test]
    fn test_chrome_launch_error_generic() {
        let lines = vec!["info line".to_string(), "another info line".to_string()];
        let msg = chrome_launch_error("Chrome exited", &lines);
        assert!(msg.contains("last 2 lines"));
    }

    #[test]
    fn test_find_playwright_chromium_nonexistent() {
        let _guard = EnvGuard::new(&["PLAYWRIGHT_BROWSERS_PATH"]);
        _guard.set("PLAYWRIGHT_BROWSERS_PATH", "/nonexistent/path");
        let result = find_playwright_chromium();
        assert!(result.is_none());
    }

    #[test]
    fn test_build_args_headless_includes_headless_flag() {
        let opts = LaunchOptions {
            headless: true,
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(result.args.iter().any(|a| a == "--headless=new"));
        assert!(result.args.iter().any(|a| a == "--window-size=1280,720"));
        // Temp dir created when no profile
        assert!(result.temp_user_data_dir.is_some());
        let dir = result.temp_user_data_dir.unwrap();
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_args_headed_no_headless_flag() {
        let opts = LaunchOptions {
            headless: false,
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(!result.args.iter().any(|a| a.contains("--headless")));
        assert!(!result.args.iter().any(|a| a.starts_with("--window-size=")));
        // Temp dir created when no profile
        assert!(result.temp_user_data_dir.is_some());
        let dir = result.temp_user_data_dir.unwrap();
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_args_temp_user_data_dir_created() {
        let opts = LaunchOptions::default();
        let result = build_chrome_args(&opts).unwrap();
        let dir = result.temp_user_data_dir.as_ref().unwrap();
        assert!(dir.exists());
        assert!(result
            .args
            .iter()
            .any(|a| a.starts_with("--user-data-dir=")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_build_args_profile_no_temp_dir() {
        let opts = LaunchOptions {
            profile: Some("/tmp/my-profile".to_string()),
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(result.temp_user_data_dir.is_none());
        assert!(result
            .args
            .iter()
            .any(|a| a == "--user-data-dir=/tmp/my-profile"));
    }

    #[test]
    fn test_build_args_custom_window_size_not_overridden() {
        let opts = LaunchOptions {
            headless: true,
            args: vec!["--window-size=1920,1080".to_string()],
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(!result.args.iter().any(|a| a == "--window-size=1280,720"));
        assert!(result.args.iter().any(|a| a == "--window-size=1920,1080"));
        if let Some(ref dir) = result.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_build_args_start_maximized_suppresses_default_window_size() {
        let opts = LaunchOptions {
            headless: true,
            args: vec!["--start-maximized".to_string()],
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(!result.args.iter().any(|a| a == "--window-size=1280,720"));
        assert!(result.args.iter().any(|a| a == "--start-maximized"));
        if let Some(ref dir) = result.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_build_args_disables_translate() {
        let opts = LaunchOptions::default();
        let result = build_chrome_args(&opts).unwrap();
        assert!(result
            .args
            .iter()
            .any(|a| a.contains("--disable-features") && a.contains("Translate")));
        if let Some(ref dir) = result.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_build_args_headless_with_extensions_skips_headless_flag() {
        let opts = LaunchOptions {
            headless: true,
            extensions: Some(vec!["/tmp/my-ext".to_string()]),
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(
            !result.args.iter().any(|a| a.contains("--headless")),
            "headless flag should be omitted when extensions are present"
        );
        assert!(
            !result.args.iter().any(|a| a.contains("--window-size")),
            "window-size should be omitted when extensions force headed mode"
        );
        assert!(result
            .args
            .iter()
            .any(|a| a.starts_with("--load-extension=")));
        if let Some(ref dir) = result.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_build_args_headed_with_extensions_no_headless_flag() {
        let opts = LaunchOptions {
            headless: false,
            extensions: Some(vec!["/tmp/my-ext".to_string()]),
            ..Default::default()
        };
        let result = build_chrome_args(&opts).unwrap();
        assert!(
            !result.args.iter().any(|a| a.contains("--headless")),
            "headless flag should not be present in headed mode"
        );
        assert!(result
            .args
            .iter()
            .any(|a| a.starts_with("--load-extension=")));
        if let Some(ref dir) = result.temp_user_data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn test_chrome_process_drop_cleans_temp_dir() {
        let dir = std::env::temp_dir().join(format!(
            "silicon-browser-chrome-drop-test-{}",
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::create_dir_all(&dir);
        assert!(dir.exists());

        {
            // Simulate a ChromeProcess with a temp dir but a dummy child.
            // We can't actually spawn Chrome here, but we can verify the Drop
            // logic by creating a small helper process.
            let child = spawn_noop_child();
            let _process = ChromeProcess {
                child,
                ws_url: String::new(),
                temp_user_data_dir: Some(dir.clone()),
            };
            // _process dropped here
        }

        assert!(!dir.exists(), "Temp dir should be cleaned up on drop");
    }
}
