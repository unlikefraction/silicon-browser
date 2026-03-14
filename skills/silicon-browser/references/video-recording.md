# Video Recording

Capture browser automation as video for debugging, documentation, or verification.

**Related**: [commands.md](commands.md) for full command reference, [SKILL.md](../SKILL.md) for quick start.

## Contents

- [Basic Recording](#basic-recording)
- [Recording Commands](#recording-commands)
- [Use Cases](#use-cases)
- [Best Practices](#best-practices)
- [Output Format](#output-format)
- [Limitations](#limitations)

## Basic Recording

```bash
# Start recording
silicon-browser record start ./demo.webm

# Perform actions
silicon-browser open https://example.com
silicon-browser snapshot -i
silicon-browser click @e1
silicon-browser fill @e2 "test input"

# Stop and save
silicon-browser record stop
```

## Recording Commands

```bash
# Start recording to file
silicon-browser record start ./output.webm

# Stop current recording
silicon-browser record stop

# Restart with new file (stops current + starts new)
silicon-browser record restart ./take2.webm
```

## Use Cases

### Debugging Failed Automation

```bash
#!/bin/bash
# Record automation for debugging

silicon-browser record start ./debug-$(date +%Y%m%d-%H%M%S).webm

# Run your automation
silicon-browser open https://app.example.com
silicon-browser snapshot -i
silicon-browser click @e1 || {
    echo "Click failed - check recording"
    silicon-browser record stop
    exit 1
}

silicon-browser record stop
```

### Documentation Generation

```bash
#!/bin/bash
# Record workflow for documentation

silicon-browser record start ./docs/how-to-login.webm

silicon-browser open https://app.example.com/login
silicon-browser wait 1000  # Pause for visibility

silicon-browser snapshot -i
silicon-browser fill @e1 "demo@example.com"
silicon-browser wait 500

silicon-browser fill @e2 "password"
silicon-browser wait 500

silicon-browser click @e3
silicon-browser wait --load networkidle
silicon-browser wait 1000  # Show result

silicon-browser record stop
```

### CI/CD Test Evidence

```bash
#!/bin/bash
# Record E2E test runs for CI artifacts

TEST_NAME="${1:-e2e-test}"
RECORDING_DIR="./test-recordings"
mkdir -p "$RECORDING_DIR"

silicon-browser record start "$RECORDING_DIR/$TEST_NAME-$(date +%s).webm"

# Run test
if run_e2e_test; then
    echo "Test passed"
else
    echo "Test failed - recording saved"
fi

silicon-browser record stop
```

## Best Practices

### 1. Add Pauses for Clarity

```bash
# Slow down for human viewing
silicon-browser click @e1
silicon-browser wait 500  # Let viewer see result
```

### 2. Use Descriptive Filenames

```bash
# Include context in filename
silicon-browser record start ./recordings/login-flow-2024-01-15.webm
silicon-browser record start ./recordings/checkout-test-run-42.webm
```

### 3. Handle Recording in Error Cases

```bash
#!/bin/bash
set -e

cleanup() {
    silicon-browser record stop 2>/dev/null || true
    silicon-browser close 2>/dev/null || true
}
trap cleanup EXIT

silicon-browser record start ./automation.webm
# ... automation steps ...
```

### 4. Combine with Screenshots

```bash
# Record video AND capture key frames
silicon-browser record start ./flow.webm

silicon-browser open https://example.com
silicon-browser screenshot ./screenshots/step1-homepage.png

silicon-browser click @e1
silicon-browser screenshot ./screenshots/step2-after-click.png

silicon-browser record stop
```

## Output Format

- Default format: WebM (VP8/VP9 codec)
- Compatible with all modern browsers and video players
- Compressed but high quality

## Limitations

- Recording adds slight overhead to automation
- Large recordings can consume significant disk space
- Some headless environments may have codec limitations
