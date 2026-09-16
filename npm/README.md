# Silicon Browser npm package

The next `silicon-browser` npm release exposes the managed Silicon Browser CLI as `browser`, version 0.2.4. Published npm version 1.0.2 still contains the legacy `sb` 0.2.2 payload; use the current native release or Honeycomb until a new npm version is published.

```sh
browser --version
browser setup
browser login <short-lived-token>
```

The package bundles checksum-verified release binaries for macOS and Linux on x64 and arm64, so installation does not run a downloader or postinstall script. Linux requires glibc 2.34 or newer; Windows, Alpine, and other platforms are unsupported.

See the current product site at https://browser.teamofsilicons.com.

To prepare an npm release from the native-tested Honeycomb target artifacts:

```sh
node npm/scripts/prepare-release.mjs /path/to/targets
```

This verifies their version and SHA-256 receipts before copying the four native payloads. Old `sb-*` files are excluded from the package. Bump the npm package version before publishing; preparation does not publish anything.
