#!/bin/bash
# Template: Form Automation Workflow
# Purpose: Fill and submit web forms with validation
# Usage: ./form-automation.sh <form-url>
#
# This template demonstrates the snapshot-interact-verify pattern:
# 1. Navigate to form
# 2. Snapshot to get element refs
# 3. Fill fields using refs
# 4. Submit and verify result
#
# Customize: Update the refs (@e1, @e2, etc.) based on your form's snapshot output

set -euo pipefail

FORM_URL="${1:?Usage: $0 <form-url>}"

echo "Form automation: $FORM_URL"

# Step 1: Navigate to form
silicon-browser open "$FORM_URL"
silicon-browser wait --load networkidle

# Step 2: Snapshot to discover form elements
echo ""
echo "Form structure:"
silicon-browser snapshot -i

# Step 3: Fill form fields (customize these refs based on snapshot output)
#
# Common field types:
#   silicon-browser fill @e1 "John Doe"           # Text input
#   silicon-browser fill @e2 "user@example.com"   # Email input
#   silicon-browser fill @e3 "SecureP@ss123"      # Password input
#   silicon-browser select @e4 "Option Value"     # Dropdown
#   silicon-browser check @e5                     # Checkbox
#   silicon-browser click @e6                     # Radio button
#   silicon-browser fill @e7 "Multi-line text"   # Textarea
#   silicon-browser upload @e8 /path/to/file.pdf # File upload
#
# Uncomment and modify:
# silicon-browser fill @e1 "Test User"
# silicon-browser fill @e2 "test@example.com"
# silicon-browser click @e3  # Submit button

# Step 4: Wait for submission
# silicon-browser wait --load networkidle
# silicon-browser wait --url "**/success"  # Or wait for redirect

# Step 5: Verify result
echo ""
echo "Result:"
silicon-browser get url
silicon-browser snapshot -i

# Optional: Capture evidence
silicon-browser screenshot /tmp/form-result.png
echo "Screenshot saved: /tmp/form-result.png"

# Cleanup
silicon-browser close
echo "Done"
