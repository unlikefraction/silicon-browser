# Command Reference

Complete reference for all silicon-browser commands. For quick start and common patterns, see SKILL.md.

## Navigation

```bash
silicon-browser open <url>      # Navigate to URL (aliases: goto, navigate)
                              # Supports: https://, http://, file://, about:, data://
                              # Auto-prepends https:// if no protocol given
silicon-browser back            # Go back
silicon-browser forward         # Go forward
silicon-browser reload          # Reload page
silicon-browser close           # Close browser (aliases: quit, exit)
silicon-browser connect 9222    # Connect to browser via CDP port
```

## Snapshot (page analysis)

```bash
silicon-browser snapshot            # Full accessibility tree
silicon-browser snapshot -i         # Interactive elements only (recommended)
silicon-browser snapshot -c         # Compact output
silicon-browser snapshot -d 3       # Limit depth to 3
silicon-browser snapshot -s "#main" # Scope to CSS selector
```

## Interactions (use @refs from snapshot)

```bash
silicon-browser click @e1           # Click
silicon-browser click @e1 --new-tab # Click and open in new tab
silicon-browser dblclick @e1        # Double-click
silicon-browser focus @e1           # Focus element
silicon-browser fill @e2 "text"     # Clear and type
silicon-browser type @e2 "text"     # Type without clearing
silicon-browser press Enter         # Press key (alias: key)
silicon-browser press Control+a     # Key combination
silicon-browser keydown Shift       # Hold key down
silicon-browser keyup Shift         # Release key
silicon-browser hover @e1           # Hover
silicon-browser check @e1           # Check checkbox
silicon-browser uncheck @e1         # Uncheck checkbox
silicon-browser select @e1 "value"  # Select dropdown option
silicon-browser select @e1 "a" "b"  # Select multiple options
silicon-browser scroll down 500     # Scroll page (default: down 300px)
silicon-browser scrollintoview @e1  # Scroll element into view (alias: scrollinto)
silicon-browser drag @e1 @e2        # Drag and drop
silicon-browser upload @e1 file.pdf # Upload files
```

## Get Information

```bash
silicon-browser get text @e1        # Get element text
silicon-browser get html @e1        # Get innerHTML
silicon-browser get value @e1       # Get input value
silicon-browser get attr @e1 href   # Get attribute
silicon-browser get title           # Get page title
silicon-browser get url             # Get current URL
silicon-browser get cdp-url         # Get CDP WebSocket URL
silicon-browser get count ".item"   # Count matching elements
silicon-browser get box @e1         # Get bounding box
silicon-browser get styles @e1      # Get computed styles (font, color, bg, etc.)
```

## Check State

```bash
silicon-browser is visible @e1      # Check if visible
silicon-browser is enabled @e1      # Check if enabled
silicon-browser is checked @e1      # Check if checked
```

## Screenshots and PDF

```bash
silicon-browser screenshot          # Save to temporary directory
silicon-browser screenshot path.png # Save to specific path
silicon-browser screenshot --full   # Full page
silicon-browser pdf output.pdf      # Save as PDF
```

## Video Recording

```bash
silicon-browser record start ./demo.webm    # Start recording
silicon-browser click @e1                   # Perform actions
silicon-browser record stop                 # Stop and save video
silicon-browser record restart ./take2.webm # Stop current + start new
```

## Wait

```bash
silicon-browser wait @e1                     # Wait for element
silicon-browser wait 2000                    # Wait milliseconds
silicon-browser wait --text "Success"        # Wait for text (or -t)
silicon-browser wait --url "**/dashboard"    # Wait for URL pattern (or -u)
silicon-browser wait --load networkidle      # Wait for network idle (or -l)
silicon-browser wait --fn "window.ready"     # Wait for JS condition (or -f)
```

## Mouse Control

```bash
silicon-browser mouse move 100 200      # Move mouse
silicon-browser mouse down left         # Press button
silicon-browser mouse up left           # Release button
silicon-browser mouse wheel 100         # Scroll wheel
```

## Semantic Locators (alternative to refs)

```bash
silicon-browser find role button click --name "Submit"
silicon-browser find text "Sign In" click
silicon-browser find text "Sign In" click --exact      # Exact match only
silicon-browser find label "Email" fill "user@test.com"
silicon-browser find placeholder "Search" type "query"
silicon-browser find alt "Logo" click
silicon-browser find title "Close" click
silicon-browser find testid "submit-btn" click
silicon-browser find first ".item" click
silicon-browser find last ".item" click
silicon-browser find nth 2 "a" hover
```

## Browser Settings

```bash
silicon-browser set viewport 1920 1080          # Set viewport size
silicon-browser set viewport 1920 1080 2        # 2x retina (same CSS size, higher res screenshots)
silicon-browser set device "iPhone 14"          # Emulate device
silicon-browser set geo 37.7749 -122.4194       # Set geolocation (alias: geolocation)
silicon-browser set offline on                  # Toggle offline mode
silicon-browser set headers '{"X-Key":"v"}'     # Extra HTTP headers
silicon-browser set credentials user pass       # HTTP basic auth (alias: auth)
silicon-browser set media dark                  # Emulate color scheme
silicon-browser set media light reduced-motion  # Light mode + reduced motion
```

## Cookies and Storage

```bash
silicon-browser cookies                     # Get all cookies
silicon-browser cookies set name value      # Set cookie
silicon-browser cookies clear               # Clear cookies
silicon-browser storage local               # Get all localStorage
silicon-browser storage local key           # Get specific key
silicon-browser storage local set k v       # Set value
silicon-browser storage local clear         # Clear all
```

## Network

```bash
silicon-browser network route <url>              # Intercept requests
silicon-browser network route <url> --abort      # Block requests
silicon-browser network route <url> --body '{}'  # Mock response
silicon-browser network unroute [url]            # Remove routes
silicon-browser network requests                 # View tracked requests
silicon-browser network requests --filter api    # Filter requests
```

## Tabs and Windows

```bash
silicon-browser tab                 # List tabs
silicon-browser tab new [url]       # New tab
silicon-browser tab 2               # Switch to tab by index
silicon-browser tab close           # Close current tab
silicon-browser tab close 2         # Close tab by index
silicon-browser window new          # New window
```

## Frames

```bash
silicon-browser frame "#iframe"     # Switch to iframe
silicon-browser frame main          # Back to main frame
```

## Dialogs

```bash
silicon-browser dialog accept [text]  # Accept dialog
silicon-browser dialog dismiss        # Dismiss dialog
```

## JavaScript

```bash
silicon-browser eval "document.title"          # Simple expressions only
silicon-browser eval -b "<base64>"             # Any JavaScript (base64 encoded)
silicon-browser eval --stdin                   # Read script from stdin
```

Use `-b`/`--base64` or `--stdin` for reliable execution. Shell escaping with nested quotes and special characters is error-prone.

```bash
# Base64 encode your script, then:
silicon-browser eval -b "ZG9jdW1lbnQucXVlcnlTZWxlY3RvcignW3NyYyo9Il9uZXh0Il0nKQ=="

# Or use stdin with heredoc for multiline scripts:
cat <<'EOF' | silicon-browser eval --stdin
const links = document.querySelectorAll('a');
Array.from(links).map(a => a.href);
EOF
```

## State Management

```bash
silicon-browser state save auth.json    # Save cookies, storage, auth state
silicon-browser state load auth.json    # Restore saved state
```

## Global Options

```bash
silicon-browser --session <name> ...    # Isolated browser session
silicon-browser --json ...              # JSON output for parsing
silicon-browser --headed ...            # Show browser window (not headless)
silicon-browser --full ...              # Full page screenshot (-f)
silicon-browser --cdp <port> ...        # Connect via Chrome DevTools Protocol
silicon-browser -p <provider> ...       # Cloud browser provider (--provider)
silicon-browser --proxy <url> ...       # Use proxy server
silicon-browser --proxy-bypass <hosts>  # Hosts to bypass proxy
silicon-browser --headers <json> ...    # HTTP headers scoped to URL's origin
silicon-browser --executable-path <p>   # Custom browser executable
silicon-browser --extension <path> ...  # Load browser extension (repeatable)
silicon-browser --ignore-https-errors   # Ignore SSL certificate errors
silicon-browser --help                  # Show help (-h)
silicon-browser --version               # Show version (-V)
silicon-browser <command> --help        # Show detailed help for a command
```

## Debugging

```bash
silicon-browser --headed open example.com   # Show browser window
silicon-browser --cdp 9222 snapshot         # Connect via CDP port
silicon-browser connect 9222                # Alternative: connect command
silicon-browser console                     # View console messages
silicon-browser console --clear             # Clear console
silicon-browser errors                      # View page errors
silicon-browser errors --clear              # Clear errors
silicon-browser highlight @e1               # Highlight element
silicon-browser inspect                     # Open Chrome DevTools for this session
silicon-browser trace start                 # Start recording trace
silicon-browser trace stop trace.zip        # Stop and save trace
silicon-browser profiler start              # Start Chrome DevTools profiling
silicon-browser profiler stop trace.json    # Stop and save profile
```

## Environment Variables

```bash
SILICON_BROWSER_SESSION="mysession"            # Default session name
SILICON_BROWSER_EXECUTABLE_PATH="/path/chrome" # Custom browser path
SILICON_BROWSER_EXTENSIONS="/ext1,/ext2"       # Comma-separated extension paths
SILICON_BROWSER_PROVIDER="browserbase"         # Cloud browser provider
SILICON_BROWSER_STREAM_PORT="9223"             # WebSocket streaming port
SILICON_BROWSER_HOME="/path/to/silicon-browser"  # Custom install location
```
