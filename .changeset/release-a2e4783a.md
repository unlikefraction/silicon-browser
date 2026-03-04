---
"agent-browser": patch
---

Fixed an issue where the --native flag was being passed to child processes even when not explicitly specified on the command line. The flag is now only forwarded when the user explicitly provides it, consistent with how other CLI flags like --allow-file-access and --download-path are handled.
