# silicon-browser

**The most reliable browser for your AI agent.**

Silicon Browser is a stealth-first, terminal-native headless browser CLI built for AI agents. It uses [CloakBrowser](https://cloakbrowser.dev/)'s patched Chromium binary (33 C++ source-level patches) as its default browser engine, combined with the ref-based interaction model of [agent-browser](https://github.com/vercel-labs/agent-browser) and 18 additional JavaScript-level stealth evasions for defense-in-depth.

**Two layers of stealth. Zero configuration.**

## Why Silicon Browser?

Regular headless browsers get flagged by Cloudflare, DataDome, PerimeterX, and every other anti-bot system within milliseconds. Silicon Browser fixes this at two levels:

### Layer 1: CloakBrowser (C++ binary-level patches)
- **33 Chromium source patches** compiled into the binary -- not JS injection
- TLS fingerprints (JA3/JA4) naturally match real Chrome
- Canvas, WebGL, Audio fingerprints spoofed at the engine level
- CDP input signals eliminated -- programmatic input looks like hardware input
- `navigator.webdriver` removed at the C++ level
- Random fingerprint seed per session (`--fingerprint=<seed>` for persistence)

### Layer 2: JavaScript stealth evasions (defense-in-depth)
On top of CloakBrowser's binary patches, 18 additional JS evasions are injected:

- `navigator.webdriver` -> false (prototype-level patching)
- Realistic plugin array (5 Chrome plugins)
- WebGL vendor/renderer masking (no more SwiftShader detection)
- Canvas & AudioContext fingerprint noise
- Proper `window.chrome` object with runtime, loadTimes, csi
- CDP artifact removal (`cdc_*`, `$cdc_*` properties)
- WebRTC IP leak prevention
- Realistic HTTP headers (Sec-Ch-Ua, Sec-Fetch-*, Accept)
- Default Chrome 131 user-agent string
- Screen/window dimensions matching real monitors (1920x1080)
- navigator.connection, hardwareConcurrency, deviceMemory
- Permissions API, Notification.permission patching
- ...and more

All stealth features are **on by default**. No configuration needed.

## Quick Start

```bash
# Install
npm install -g silicon-browser

# Download Chrome
silicon-browser install

# Browse the web
silicon-browser open https://example.com
silicon-browser snapshot -i          # get interactive elements
silicon-browser click @e3            # click by ref
silicon-browser fill @e5 "query"     # fill input
silicon-browser screenshot           # capture screenshot
```

## How It Works

Silicon Browser uses a **ref-based interaction model** -- instead of fragile CSS selectors, every interactive element gets a deterministic ref like `@e1`, `@e2`, etc:

```bash
$ silicon-browser snapshot -i
@e1  link "Home"
@e2  button "Sign In"
@e3  input "Search..."
@e4  link "About"

$ silicon-browser click @e2
```

This reduces context usage by up to 93% compared to screenshot-based approaches.

## Stealth Architecture

Silicon Browser applies stealth at four layers:

### 1. CloakBrowser Binary (primary)
A patched Chromium with 33 C++ source-level modifications. Since it IS a real Chromium binary, TLS fingerprints, HTTP/2 settings, and browser internals all match real Chrome. `silicon-browser install` downloads it automatically.

### 2. Chrome Launch Flags
Anti-automation flags are injected at Chrome startup:
- `--disable-blink-features=AutomationControlled`
- `--fingerprint=<random>` (CloakBrowser deterministic fingerprint seed)
- `--webrtc-ip-handling-policy=disable_non_proxied_udp`
- `--enable-gpu-rasterization`
- `--disable-infobars`

### 3. JavaScript Injection
18 stealth evasions run via `Page.addScriptToEvaluateOnNewDocument` **before any page code executes** -- defense-in-depth on top of CloakBrowser's binary patches.

### 4. Network Headers
Realistic HTTP headers are set via CDP's `Network.setExtraHTTPHeaders`, including Client Hints (`Sec-Ch-Ua`), Fetch metadata, and proper Accept headers.

## Configuration

### Stealth (on by default)
```bash
# Disable stealth (for debugging)
SILICON_BROWSER_NO_STEALTH=1 silicon-browser open https://example.com

# Pin fingerprint seed (same identity across sessions)
SILICON_BROWSER_FINGERPRINT=42069 silicon-browser open https://example.com

# Custom user agent
silicon-browser --user-agent "Mozilla/5.0 ..." open https://example.com

# With proxy
silicon-browser --proxy "http://user:pass@proxy:8080" open https://example.com
```

### Sessions
```bash
# Named sessions (persistent state)
silicon-browser --session work open https://example.com
silicon-browser --session personal open https://other.com
silicon-browser session list
```

### Profiles (persistent cookies/storage)
```bash
silicon-browser --profile ~/.silicon-browser/profiles/main open https://example.com
```

### Cloud Providers
```bash
silicon-browser -p browserbase open https://example.com
silicon-browser -p browserless open https://example.com
```

## All Commands

| Command | Description |
|---------|-------------|
| `open <url>` | Navigate to URL |
| `snapshot` | Get accessibility tree (add `-i` for interactive only) |
| `click @ref` | Click element by ref |
| `fill @ref "text"` | Fill input field |
| `type "text"` | Type text (keystroke-level) |
| `screenshot` | Capture screenshot |
| `get text @ref` | Extract text from element |
| `get html @ref` | Get HTML of element |
| `get value @ref` | Get input value |
| `evaluate "js"` | Execute JavaScript |
| `scroll down` | Scroll the page |
| `select @ref "value"` | Select dropdown option |
| `hover @ref` | Hover over element |
| `tabs` | List open tabs |
| `tab new <url>` | Open new tab |
| `tab select <n>` | Switch to tab |
| `close` | Close browser |
| `install` | Download Chrome |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `SILICON_BROWSER_NO_STEALTH` | Set to `1` to disable stealth features |
| `SILICON_BROWSER_FINGERPRINT` | Pin fingerprint seed for CloakBrowser (consistent identity) |
| `SILICON_BROWSER_USER_AGENT` | Default user agent string |
| `CLOAKBROWSER_BINARY_PATH` | Use a custom CloakBrowser binary path |
| `CLOAKBROWSER_CACHE_DIR` | Custom CloakBrowser cache directory |
| `CLOAKBROWSER_DOWNLOAD_URL` | Custom CloakBrowser download server |
| `SILICON_BROWSER_PROXY` | Default proxy URL |
| `SILICON_BROWSER_SESSION` | Default session name |
| `SILICON_BROWSER_IDLE_TIMEOUT_MS` | Auto-shutdown after inactivity |
| `SILICON_BROWSER_SOCKET_DIR` | Custom socket directory |

## Building from Source

```bash
# Prerequisites: Rust toolchain
git clone https://github.com/unlikefraction/silicon-browser.git
cd silicon-browser
cargo build --release --manifest-path cli/Cargo.toml

# The binary is at cli/target/release/silicon-browser
```

## Credits

Silicon Browser builds on the excellent work of:
- [agent-browser](https://github.com/vercel-labs/agent-browser) by Vercel -- the ref-based browser automation CLI
- [CloakBrowser](https://cloakbrowser.dev/) -- anti-detection browser techniques
- [puppeteer-extra-plugin-stealth](https://github.com/berstend/puppeteer-extra) -- JS-level evasion patterns

## License

Apache-2.0
