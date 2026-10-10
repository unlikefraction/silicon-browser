# Silicon UI

Browser uses the actual Button, motion tokens and foundation from https://ui.teamofsilicons.com. Download hashes are recorded in `src/components/silicon-ui/registry-lock.json`; the sole TSX integration addition is the React JSX pragma. `silicon-button.tsx` mounts registry controls within the existing Solid application, forwards reactive disabled/expanded state and click handlers, preserves native form submission, and unmounts the React root with its Solid owner. Solid still owns the existing authentication, context switching and routes.

The upstream MIT license and Team of Silicons / Elia Kuratli attribution are preserved in `src/components/silicon-ui/LICENSE` and included in the published site at `/licenses/silicon-ui.txt`.

Validation: production Vite build and all 45 frontend tests pass. Desktop and mobile browser checks exercise Carbon/Silicon controls, reactive expansion/collapse, registry styling and overflow. No authentication fields were changed.
