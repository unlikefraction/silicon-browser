# Silicon Browser npm package

The `silicon-browser` npm package exposes the managed Silicon Browser CLI as `browser`, version 0.3.1.

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

This verifies their version and SHA-256 receipts before copying the four native payloads. Preparation does not publish anything.
