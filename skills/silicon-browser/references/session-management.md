# Session Management

Multiple isolated browser sessions with state persistence and concurrent browsing.

**Related**: [authentication.md](authentication.md) for login patterns, [SKILL.md](../SKILL.md) for quick start.

## Contents

- [Named Sessions](#named-sessions)
- [Session Isolation Properties](#session-isolation-properties)
- [Session State Persistence](#session-state-persistence)
- [Common Patterns](#common-patterns)
- [Default Session](#default-session)
- [Session Cleanup](#session-cleanup)
- [Best Practices](#best-practices)

## Named Sessions

Use `--session` flag to isolate browser contexts:

```bash
# Session 1: Authentication flow
silicon-browser --session auth open https://app.example.com/login

# Session 2: Public browsing (separate cookies, storage)
silicon-browser --session public open https://example.com

# Commands are isolated by session
silicon-browser --session auth fill @e1 "user@example.com"
silicon-browser --session public get text body
```

## Session Isolation Properties

Each session has independent:
- Cookies
- LocalStorage / SessionStorage
- IndexedDB
- Cache
- Browsing history
- Open tabs

## Session State Persistence

### Save Session State

```bash
# Save cookies, storage, and auth state
silicon-browser state save /path/to/auth-state.json
```

### Load Session State

```bash
# Restore saved state
silicon-browser state load /path/to/auth-state.json

# Continue with authenticated session
silicon-browser open https://app.example.com/dashboard
```

### State File Contents

```json
{
  "cookies": [...],
  "localStorage": {...},
  "sessionStorage": {...},
  "origins": [...]
}
```

## Common Patterns

### Authenticated Session Reuse

```bash
#!/bin/bash
# Save login state once, reuse many times

STATE_FILE="/tmp/auth-state.json"

# Check if we have saved state
if [[ -f "$STATE_FILE" ]]; then
    silicon-browser state load "$STATE_FILE"
    silicon-browser open https://app.example.com/dashboard
else
    # Perform login
    silicon-browser open https://app.example.com/login
    silicon-browser snapshot -i
    silicon-browser fill @e1 "$USERNAME"
    silicon-browser fill @e2 "$PASSWORD"
    silicon-browser click @e3
    silicon-browser wait --load networkidle

    # Save for future use
    silicon-browser state save "$STATE_FILE"
fi
```

### Concurrent Scraping

```bash
#!/bin/bash
# Scrape multiple sites concurrently

# Start all sessions
silicon-browser --session site1 open https://site1.com &
silicon-browser --session site2 open https://site2.com &
silicon-browser --session site3 open https://site3.com &
wait

# Extract from each
silicon-browser --session site1 get text body > site1.txt
silicon-browser --session site2 get text body > site2.txt
silicon-browser --session site3 get text body > site3.txt

# Cleanup
silicon-browser --session site1 close
silicon-browser --session site2 close
silicon-browser --session site3 close
```

### A/B Testing Sessions

```bash
# Test different user experiences
silicon-browser --session variant-a open "https://app.com?variant=a"
silicon-browser --session variant-b open "https://app.com?variant=b"

# Compare
silicon-browser --session variant-a screenshot /tmp/variant-a.png
silicon-browser --session variant-b screenshot /tmp/variant-b.png
```

## Default Session

When `--session` is omitted, commands use the default session:

```bash
# These use the same default session
silicon-browser open https://example.com
silicon-browser snapshot -i
silicon-browser close  # Closes default session
```

## Session Cleanup

```bash
# Close specific session
silicon-browser --session auth close

# List active sessions
silicon-browser session list
```

## Best Practices

### 1. Name Sessions Semantically

```bash
# GOOD: Clear purpose
silicon-browser --session github-auth open https://github.com
silicon-browser --session docs-scrape open https://docs.example.com

# AVOID: Generic names
silicon-browser --session s1 open https://github.com
```

### 2. Always Clean Up

```bash
# Close sessions when done
silicon-browser --session auth close
silicon-browser --session scrape close
```

### 3. Handle State Files Securely

```bash
# Don't commit state files (contain auth tokens!)
echo "*.auth-state.json" >> .gitignore

# Delete after use
rm /tmp/auth-state.json
```

### 4. Timeout Long Sessions

```bash
# Set timeout for automated scripts
timeout 60 silicon-browser --session long-task get text body
```
