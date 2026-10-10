# Silicon UI integration

Browser uses the official [Silicon UI foundation](https://ui.teamofsilicons.com/foundation) and adapts its [Button](https://ui.teamofsilicons.com/button), [Input](https://ui.teamofsilicons.com/input), [Textarea](https://ui.teamofsilicons.com/textarea), and [Badge](https://ui.teamofsilicons.com/badge) CSS for native Solid controls. The React 19 motion wrappers are not included. Forms, account switching, keyboard behavior, and the existing application layout keep their current runtime.

`foundation.css` is copied unchanged from `registry/foundation.css` in the [official registry](https://ui.teamofsilicons.com/r/silicon-foundation.json), retrieved 10 October 2026. Its SHA-256 is `28e3780a68f2caa28f7b961ba4c7825f681db45bb2f421828b9a4983388451d0`. `controls.css` adapts the corresponding registry CSS selectors to Browser's native elements and preserves reduced-motion support. Application surfaces, spacing, radii, focus, and colors read the same foundation tokens.

The upstream MIT license and attribution are preserved in [LICENSE](./LICENSE).
