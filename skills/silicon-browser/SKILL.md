---
name: silicon-browser
description: Browser automation CLI for AI agents. Use when the user needs to interact with websites, including navigating pages, filling forms, clicking buttons, taking screenshots, extracting data, testing web apps, or automating any browser task. Triggers include requests to "open a website", "fill out a form", "click a button", "take a screenshot", "scrape data from a page", "test this web app", "login to a site", "automate browser actions", or any task requiring programmatic web interaction.
allowed-tools: Bash(npx silicon-browser:*), Bash(silicon-browser:*)
---

# Browser Automation with silicon-browser

The CLI uses Chrome/Chromium via CDP directly. Install via `npm i -g silicon-browser`, `brew install silicon-browser`, or `cargo install silicon-browser`. Run `silicon-browser install` to download CloakBrowser plus Chrome for Testing. Silicon Browser prefers CloakBrowser and will automatically recover from a stale implicit default profile if that profile prevents CloakBrowser from launching.

## Core Workflow

Every browser automation follows this pattern:

1. **Navigate**: `silicon-browser open <url>`
2. **Snapshot**: `silicon-browser snapshot -i` (get element refs like `@e1`, `@e2`)
3. **Interact**: Use refs to click, fill, select
4. **Re-snapshot**: After navigation or DOM changes, get fresh refs

```bash
silicon-browser open https://example.com/form
silicon-browser snapshot -i
# Output: @e1 [input type="email"], @e2 [input type="password"], @e3 [button] "Submit"

silicon-browser fill @e1 "user@example.com"
silicon-browser fill @e2 "password123"
silicon-browser click @e3
silicon-browser wait --load networkidle
silicon-browser snapshot -i  # Check result
```

## Command Chaining

Commands can be chained with `&&` in a single shell invocation. The browser persists between commands via a background daemon, so chaining is safe and more efficient than separate calls.

```bash
# Chain open + wait + snapshot in one call
silicon-browser open https://example.com && silicon-browser wait --load networkidle && silicon-browser snapshot -i

# Chain multiple interactions
silicon-browser fill @e1 "user@example.com" && silicon-browser fill @e2 "password123" && silicon-browser click @e3

# Navigate and capture
silicon-browser open https://example.com && silicon-browser wait --load networkidle && silicon-browser screenshot page.png
```

**When to chain:** Use `&&` when you don't need to read the output of an intermediate command before proceeding (e.g., open + wait + screenshot). Run commands separately when you need to parse the output first (e.g., snapshot to discover refs, then interact using those refs).

## Handling Authentication

When automating a site that requires login, choose the approach that fits:

**Option 1: Import auth from the user's browser (fastest for one-off tasks)**

```bash
# Connect to the user's running Chrome (they're already logged in)
silicon-browser --auto-connect state save ./auth.json
# Use that auth state
silicon-browser --state ./auth.json open https://app.example.com/dashboard
```

State files contain session tokens in plaintext -- add to `.gitignore` and delete when no longer needed. Set `SILICON_BROWSER_ENCRYPTION_KEY` for encryption at rest.

**Option 2: Persistent profile (simplest for recurring tasks)**

```bash
# First run: login manually or via automation
silicon-browser --profile ~/.myapp open https://app.example.com/login
# ... fill credentials, submit ...

# All future runs: already authenticated
silicon-browser --profile ~/.myapp open https://app.example.com/dashboard
```

**Option 3: Session name (auto-save/restore cookies + localStorage)**

```bash
silicon-browser --session-name myapp open https://app.example.com/login
# ... login flow ...
silicon-browser close  # State auto-saved

# Next time: state auto-restored
silicon-browser --session-name myapp open https://app.example.com/dashboard
```

**Option 4: Auth vault (credentials stored encrypted, login by name)**

```bash
echo "$PASSWORD" | silicon-browser auth save myapp --url https://app.example.com/login --username user --password-stdin
silicon-browser auth login myapp
```

**Option 5: State file (manual save/load)**

```bash
# After logging in:
silicon-browser state save ./auth.json
# In a future session:
silicon-browser state load ./auth.json
silicon-browser open https://app.example.com/dashboard
```

See [references/authentication.md](references/authentication.md) for OAuth, 2FA, cookie-based auth, and token refresh patterns.

## Essential Commands

```bash
# Navigation
silicon-browser open <url>              # Navigate (aliases: goto, navigate)
silicon-browser close                   # Close browser

# Snapshot
silicon-browser snapshot -i             # Interactive elements with refs (recommended)
silicon-browser snapshot -i -C          # Include cursor-interactive elements (divs with onclick, cursor:pointer)
silicon-browser snapshot -s "#selector" # Scope to CSS selector

# Interaction (use @refs from snapshot)
silicon-browser click @e1               # Click element
silicon-browser click @e1 --new-tab     # Click and open in new tab
silicon-browser fill @e2 "text"         # Clear and type text
silicon-browser type @e2 "text"         # Type without clearing
silicon-browser select @e1 "option"     # Select dropdown option
silicon-browser check @e1               # Check checkbox
silicon-browser press Enter             # Press key
silicon-browser keyboard type "text"    # Type at current focus (no selector)
silicon-browser keyboard inserttext "text"  # Insert without key events
silicon-browser scroll down 500         # Scroll page
silicon-browser scroll down 500 --selector "div.content"  # Scroll within a specific container

# Get information
silicon-browser get text @e1            # Get element text
silicon-browser get url                 # Get current URL
silicon-browser get title               # Get page title
silicon-browser get cdp-url             # Get CDP WebSocket URL

# Wait
silicon-browser wait @e1                # Wait for element
silicon-browser wait --load networkidle # Wait for network idle
silicon-browser wait --url "**/page"    # Wait for URL pattern
silicon-browser wait 2000               # Wait milliseconds
silicon-browser wait --text "Welcome"    # Wait for text to appear (substring match)
silicon-browser wait --fn "!document.body.innerText.includes('Loading...')"  # Wait for text to disappear
silicon-browser wait "#spinner" --state hidden  # Wait for element to disappear

# Downloads
silicon-browser download @e1 ./file.pdf          # Click element to trigger download
silicon-browser wait --download ./output.zip     # Wait for any download to complete
silicon-browser --download-path ./downloads open <url>  # Set default download directory

# Viewport & Device Emulation
silicon-browser set viewport 1920 1080          # Set viewport size (default: 1280x720)
silicon-browser set viewport 1920 1080 2        # 2x retina (same CSS size, higher res screenshots)
silicon-browser set device "iPhone 14"          # Emulate device (viewport + user agent)

# Capture
silicon-browser screenshot              # Screenshot to temp dir
silicon-browser screenshot --full       # Full page screenshot
silicon-browser screenshot --annotate   # Annotated screenshot with numbered element labels
silicon-browser screenshot --screenshot-dir ./shots  # Save to custom directory
silicon-browser screenshot --screenshot-format jpeg --screenshot-quality 80
silicon-browser pdf output.pdf          # Save as PDF

# Clipboard
silicon-browser clipboard read                      # Read text from clipboard
silicon-browser clipboard write "Hello, World!"     # Write text to clipboard
silicon-browser clipboard copy                      # Copy current selection
silicon-browser clipboard paste                     # Paste from clipboard

# Diff (compare page states)
silicon-browser diff snapshot                          # Compare current vs last snapshot
silicon-browser diff snapshot --baseline before.txt    # Compare current vs saved file
silicon-browser diff screenshot --baseline before.png  # Visual pixel diff
silicon-browser diff url <url1> <url2>                 # Compare two pages
silicon-browser diff url <url1> <url2> --wait-until networkidle  # Custom wait strategy
silicon-browser diff url <url1> <url2> --selector "#main"  # Scope to element
```

## Stealth & Anti-Detection

silicon-browser is stealth-first. All anti-detection is enabled by default -- no configuration needed.

**What's built in:**
- Chrome 135 user agent, headers, and Client Hints metadata
- Platform-aware WebGL (Apple M1 on macOS, RTX 3060 on Windows)
- Function.prototype.toString protection (all patches look native)
- SpeechSynthesis voice mocking, CSS media query normalization
- Stack trace sanitization (removes CDP injection artifacts)
- Canvas and AudioContext fingerprint noise

**Headless mode:** By default, silicon-browser runs in **offscreen headed mode** (real Chrome window at off-screen coordinates). This passes ALL headless detection because it IS a real headed browser. On headless Linux servers, it auto-installs and starts Xvfb for a virtual display.

```bash
# Default: stealth headed mode (invisible but real browser)
silicon-browser open https://example.com

# Force traditional headless (for CI where stealth isn't needed)
SILICON_BROWSER_HEADLESS_REAL=1 silicon-browser open https://example.com

# Disable stealth patches entirely
SILICON_BROWSER_NO_STEALTH=1 silicon-browser open https://example.com
```

**Headless Linux servers:** Xvfb is auto-detected, installed (via apt/yum/dnf/apk), and started. Zero configuration needed.

## CAPTCHA Solving

When a page shows a CAPTCHA or bot challenge, use `solve-captcha`:

```bash
silicon-browser open https://example.com
silicon-browser solve-captcha
```

**Supported CAPTCHA types:**
- **Cloudflare Turnstile** -- detects "Just a moment..." pages, clicks the checkbox via CDP
- **reCAPTCHA v2 checkbox** -- human-like mouse movement to click "I'm not a robot"
- **Text CAPTCHAs** -- local OCR engine (no external APIs), reads distorted text from canvas/images
- **hCaptcha** -- checkbox click with behavioral simulation

**How it works:**
1. Detects CAPTCHA type on the current page
2. For checkboxes: generates human-like mouse movement path (based on real recorded data) and clicks
3. For text CAPTCHAs: screenshots the image, runs local OCR, enters the text, and submits
4. All solving is local -- no external APIs, no LLM calls

```bash
# Auto-detect and solve
silicon-browser solve-captcha

# Check result
silicon-browser eval "document.title"
silicon-browser screenshot result.png
```

## Common Patterns

### Form Submission

```bash
silicon-browser open https://example.com/signup
silicon-browser snapshot -i
silicon-browser fill @e1 "Jane Doe"
silicon-browser fill @e2 "jane@example.com"
silicon-browser select @e3 "California"
silicon-browser check @e4
silicon-browser click @e5
silicon-browser wait --load networkidle
```

### Authentication with Auth Vault (Recommended)

```bash
# Save credentials once (encrypted with SILICON_BROWSER_ENCRYPTION_KEY)
# Recommended: pipe password via stdin to avoid shell history exposure
echo "pass" | silicon-browser auth save github --url https://github.com/login --username user --password-stdin

# Login using saved profile (LLM never sees password)
silicon-browser auth login github

# List/show/delete profiles
silicon-browser auth list
silicon-browser auth show github
silicon-browser auth delete github
```

### Authentication with State Persistence

```bash
# Login once and save state
silicon-browser open https://app.example.com/login
silicon-browser snapshot -i
silicon-browser fill @e1 "$USERNAME"
silicon-browser fill @e2 "$PASSWORD"
silicon-browser click @e3
silicon-browser wait --url "**/dashboard"
silicon-browser state save auth.json

# Reuse in future sessions
silicon-browser state load auth.json
silicon-browser open https://app.example.com/dashboard
```

### Session Persistence

```bash
# Auto-save/restore cookies and localStorage across browser restarts
silicon-browser --session-name myapp open https://app.example.com/login
# ... login flow ...
silicon-browser close  # State auto-saved to ~/.silicon-browser/sessions/

# Next time, state is auto-loaded
silicon-browser --session-name myapp open https://app.example.com/dashboard

# Encrypt state at rest
export SILICON_BROWSER_ENCRYPTION_KEY=$(openssl rand -hex 32)
silicon-browser --session-name secure open https://app.example.com

# Manage saved states
silicon-browser state list
silicon-browser state show myapp-default.json
silicon-browser state clear myapp
silicon-browser state clean --older-than 7
```

### Data Extraction

```bash
silicon-browser open https://example.com/products
silicon-browser snapshot -i
silicon-browser get text @e5           # Get specific element text
silicon-browser get text body > page.txt  # Get all page text

# JSON output for parsing
silicon-browser snapshot -i --json
silicon-browser get text @e1 --json
```

### Parallel Sessions

```bash
silicon-browser --session site1 open https://site-a.com
silicon-browser --session site2 open https://site-b.com

silicon-browser --session site1 snapshot -i
silicon-browser --session site2 snapshot -i

silicon-browser session list
```

### Connect to Existing Chrome

```bash
# Auto-discover running Chrome with remote debugging enabled
silicon-browser --auto-connect open https://example.com
silicon-browser --auto-connect snapshot

# Or with explicit CDP port
silicon-browser --cdp 9222 snapshot
```

### Color Scheme (Dark Mode)

```bash
# Persistent dark mode via flag (applies to all pages and new tabs)
silicon-browser --color-scheme dark open https://example.com

# Or via environment variable
SILICON_BROWSER_COLOR_SCHEME=dark silicon-browser open https://example.com

# Or set during session (persists for subsequent commands)
silicon-browser set media dark
```

### Viewport & Responsive Testing

```bash
# Set a custom viewport size (default is 1280x720)
silicon-browser set viewport 1920 1080
silicon-browser screenshot desktop.png

# Test mobile-width layout
silicon-browser set viewport 375 812
silicon-browser screenshot mobile.png

# Retina/HiDPI: same CSS layout at 2x pixel density
# Screenshots stay at logical viewport size, but content renders at higher DPI
silicon-browser set viewport 1920 1080 2
silicon-browser screenshot retina.png

# Device emulation (sets viewport + user agent in one step)
silicon-browser set device "iPhone 14"
silicon-browser screenshot device.png
```

The `scale` parameter (3rd argument) sets `window.devicePixelRatio` without changing CSS layout. Use it when testing retina rendering or capturing higher-resolution screenshots.

### Visual Browser (Debugging)

```bash
silicon-browser --headed open https://example.com
silicon-browser highlight @e1          # Highlight element
silicon-browser inspect                # Open Chrome DevTools for the active page
silicon-browser record start demo.webm # Record session
silicon-browser profiler start         # Start Chrome DevTools profiling
silicon-browser profiler stop trace.json # Stop and save profile (path optional)
```

Use `SILICON_BROWSER_HEADED=1` to enable headed mode via environment variable. Browser extensions work in both headed and headless mode.

### Local Files (PDFs, HTML)

```bash
# Open local files with file:// URLs
silicon-browser --allow-file-access open file:///path/to/document.pdf
silicon-browser --allow-file-access open file:///path/to/page.html
silicon-browser screenshot output.png
```

### iOS Simulator (Mobile Safari)

```bash
# List available iOS simulators
silicon-browser device list

# Launch Safari on a specific device
silicon-browser -p ios --device "iPhone 16 Pro" open https://example.com

# Same workflow as desktop - snapshot, interact, re-snapshot
silicon-browser -p ios snapshot -i
silicon-browser -p ios tap @e1          # Tap (alias for click)
silicon-browser -p ios fill @e2 "text"
silicon-browser -p ios swipe up         # Mobile-specific gesture

# Take screenshot
silicon-browser -p ios screenshot mobile.png

# Close session (shuts down simulator)
silicon-browser -p ios close
```

**Requirements:** macOS with Xcode, Appium (`npm install -g appium && appium driver install xcuitest`)

**Real devices:** Works with physical iOS devices if pre-configured. Use `--device "<UDID>"` where UDID is from `xcrun xctrace list devices`.

## Security

All security features are opt-in. By default, silicon-browser imposes no restrictions on navigation, actions, or output.

### Content Boundaries (Recommended for AI Agents)

Enable `--content-boundaries` to wrap page-sourced output in markers that help LLMs distinguish tool output from untrusted page content:

```bash
export SILICON_BROWSER_CONTENT_BOUNDARIES=1
silicon-browser snapshot
# Output:
# --- SILICON_BROWSER_PAGE_CONTENT nonce=<hex> origin=https://example.com ---
# [accessibility tree]
# --- END_SILICON_BROWSER_PAGE_CONTENT nonce=<hex> ---
```

### Domain Allowlist

Restrict navigation to trusted domains. Wildcards like `*.example.com` also match the bare domain `example.com`. Sub-resource requests, WebSocket, and EventSource connections to non-allowed domains are also blocked. Include CDN domains your target pages depend on:

```bash
export SILICON_BROWSER_ALLOWED_DOMAINS="example.com,*.example.com"
silicon-browser open https://example.com        # OK
silicon-browser open https://malicious.com       # Blocked
```

### Action Policy

Use a policy file to gate destructive actions:

```bash
export SILICON_BROWSER_ACTION_POLICY=./policy.json
```

Example `policy.json`:

```json
{ "default": "deny", "allow": ["navigate", "snapshot", "click", "scroll", "wait", "get"] }
```

Auth vault operations (`auth login`, etc.) bypass action policy but domain allowlist still applies.

### Output Limits

Prevent context flooding from large pages:

```bash
export SILICON_BROWSER_MAX_OUTPUT=50000
```

## Diffing (Verifying Changes)

Use `diff snapshot` after performing an action to verify it had the intended effect. This compares the current accessibility tree against the last snapshot taken in the session.

```bash
# Typical workflow: snapshot -> action -> diff
silicon-browser snapshot -i          # Take baseline snapshot
silicon-browser click @e2            # Perform action
silicon-browser diff snapshot        # See what changed (auto-compares to last snapshot)
```

For visual regression testing or monitoring:

```bash
# Save a baseline screenshot, then compare later
silicon-browser screenshot baseline.png
# ... time passes or changes are made ...
silicon-browser diff screenshot --baseline baseline.png

# Compare staging vs production
silicon-browser diff url https://staging.example.com https://prod.example.com --screenshot
```

`diff snapshot` output uses `+` for additions and `-` for removals, similar to git diff. `diff screenshot` produces a diff image with changed pixels highlighted in red, plus a mismatch percentage.

## Timeouts and Slow Pages

The default timeout is 25 seconds. This can be overridden with the `SILICON_BROWSER_DEFAULT_TIMEOUT` environment variable (value in milliseconds). For slow websites or large pages, use explicit waits instead of relying on the default timeout:

```bash
# Wait for network activity to settle (best for slow pages)
silicon-browser wait --load networkidle

# Wait for a specific element to appear
silicon-browser wait "#content"
silicon-browser wait @e1

# Wait for a specific URL pattern (useful after redirects)
silicon-browser wait --url "**/dashboard"

# Wait for a JavaScript condition
silicon-browser wait --fn "document.readyState === 'complete'"

# Wait a fixed duration (milliseconds) as a last resort
silicon-browser wait 5000
```

When dealing with consistently slow websites, use `wait --load networkidle` after `open` to ensure the page is fully loaded before taking a snapshot. If a specific element is slow to render, wait for it directly with `wait <selector>` or `wait @ref`.

## Session Management and Cleanup

When running multiple agents or automations concurrently, always use named sessions to avoid conflicts:

```bash
# Each agent gets its own isolated session
silicon-browser --session agent1 open site-a.com
silicon-browser --session agent2 open site-b.com

# Check active sessions
silicon-browser session list
```

Always close your browser session when done to avoid leaked processes:

```bash
silicon-browser close                    # Close default session
silicon-browser --session agent1 close   # Close specific session
```

If a previous session was not closed properly, the daemon may still be running. Use `silicon-browser close` to clean it up before starting new work.

To auto-shutdown the daemon after a period of inactivity (useful for ephemeral/CI environments):

```bash
SILICON_BROWSER_IDLE_TIMEOUT_MS=60000 silicon-browser open example.com
```

## Ref Lifecycle (Important)

Refs (`@e1`, `@e2`, etc.) are invalidated when the page changes. Always re-snapshot after:

- Clicking links or buttons that navigate
- Form submissions
- Dynamic content loading (dropdowns, modals)

```bash
silicon-browser click @e5              # Navigates to new page
silicon-browser snapshot -i            # MUST re-snapshot
silicon-browser click @e1              # Use new refs
```

## Annotated Screenshots (Vision Mode)

Use `--annotate` to take a screenshot with numbered labels overlaid on interactive elements. Each label `[N]` maps to ref `@eN`. This also caches refs, so you can interact with elements immediately without a separate snapshot.

```bash
silicon-browser screenshot --annotate
# Output includes the image path and a legend:
#   [1] @e1 button "Submit"
#   [2] @e2 link "Home"
#   [3] @e3 textbox "Email"
silicon-browser click @e2              # Click using ref from annotated screenshot
```

Use annotated screenshots when:

- The page has unlabeled icon buttons or visual-only elements
- You need to verify visual layout or styling
- Canvas or chart elements are present (invisible to text snapshots)
- You need spatial reasoning about element positions

## Semantic Locators (Alternative to Refs)

When refs are unavailable or unreliable, use semantic locators:

```bash
silicon-browser find text "Sign In" click
silicon-browser find label "Email" fill "user@test.com"
silicon-browser find role button click --name "Submit"
silicon-browser find placeholder "Search" type "query"
silicon-browser find testid "submit-btn" click
```

## JavaScript Evaluation (eval)

Use `eval` to run JavaScript in the browser context. **Shell quoting can corrupt complex expressions** -- use `--stdin` or `-b` to avoid issues.

```bash
# Simple expressions work with regular quoting
silicon-browser eval 'document.title'
silicon-browser eval 'document.querySelectorAll("img").length'

# Complex JS: use --stdin with heredoc (RECOMMENDED)
silicon-browser eval --stdin <<'EVALEOF'
JSON.stringify(
  Array.from(document.querySelectorAll("img"))
    .filter(i => !i.alt)
    .map(i => ({ src: i.src.split("/").pop(), width: i.width }))
)
EVALEOF

# Alternative: base64 encoding (avoids all shell escaping issues)
silicon-browser eval -b "$(echo -n 'Array.from(document.querySelectorAll("a")).map(a => a.href)' | base64)"
```

**Why this matters:** When the shell processes your command, inner double quotes, `!` characters (history expansion), backticks, and `$()` can all corrupt the JavaScript before it reaches silicon-browser. The `--stdin` and `-b` flags bypass shell interpretation entirely.

**Rules of thumb:**

- Single-line, no nested quotes -> regular `eval 'expression'` with single quotes is fine
- Nested quotes, arrow functions, template literals, or multiline -> use `eval --stdin <<'EVALEOF'`
- Programmatic/generated scripts -> use `eval -b` with base64

## Configuration File

Create `silicon-browser.json` in the project root for persistent settings:

```json
{
  "headed": true,
  "proxy": "http://localhost:8080",
  "profile": "./browser-data"
}
```

Priority (lowest to highest): `~/.silicon-browser/config.json` < `./silicon-browser.json` < env vars < CLI flags. Use `--config <path>` or `SILICON_BROWSER_CONFIG` env var for a custom config file (exits with error if missing/invalid). All CLI options map to camelCase keys (e.g., `--executable-path` -> `"executablePath"`). Boolean flags accept `true`/`false` values (e.g., `--headed false` overrides config). Extensions from user and project configs are merged, not replaced.

## Deep-Dive Documentation

| Reference                                                            | When to Use                                               |
| -------------------------------------------------------------------- | --------------------------------------------------------- |
| [references/commands.md](references/commands.md)                     | Full command reference with all options                   |
| [references/snapshot-refs.md](references/snapshot-refs.md)           | Ref lifecycle, invalidation rules, troubleshooting        |
| [references/session-management.md](references/session-management.md) | Parallel sessions, state persistence, concurrent scraping |
| [references/authentication.md](references/authentication.md)         | Login flows, OAuth, 2FA handling, state reuse             |
| [references/video-recording.md](references/video-recording.md)       | Recording workflows for debugging and documentation       |
| [references/profiling.md](references/profiling.md)                   | Chrome DevTools profiling for performance analysis        |
| [references/proxy-support.md](references/proxy-support.md)           | Proxy configuration, geo-testing, rotating proxies        |

## Browser Engine Selection

Use `--engine` to choose a local browser engine. The default is `chrome`.

```bash
# Use Lightpanda (fast headless browser, requires separate install)
silicon-browser --engine lightpanda open example.com

# Via environment variable
export SILICON_BROWSER_ENGINE=lightpanda
silicon-browser open example.com

# With custom binary path
silicon-browser --engine lightpanda --executable-path /path/to/lightpanda open example.com
```

Supported engines:
- `chrome` (default) -- Chrome/Chromium via CDP
- `lightpanda` -- Lightpanda headless browser via CDP (10x faster, 10x less memory than Chrome)

Lightpanda does not support `--extension`, `--profile`, `--state`, or `--allow-file-access`. Install Lightpanda from https://lightpanda.io/docs/open-source/installation.

## Ready-to-Use Templates

| Template                                                                 | Description                         |
| ------------------------------------------------------------------------ | ----------------------------------- |
| [templates/form-automation.sh](templates/form-automation.sh)             | Form filling with validation        |
| [templates/authenticated-session.sh](templates/authenticated-session.sh) | Login once, reuse state             |
| [templates/capture-workflow.sh](templates/capture-workflow.sh)           | Content extraction with screenshots |

```bash
./templates/form-automation.sh https://example.com/form
./templates/authenticated-session.sh https://app.example.com/login
./templates/capture-workflow.sh https://example.com ./output
```
