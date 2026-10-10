# Silicon Browser npm package

The `silicon-browser` npm package exposes the managed Silicon Browser CLI as `browser`, version 2.0.0. Silicon Apps is the primary installation and update path:

```sh
silicon-apps install browser
browser accounts --json
browser login status --json
silicon-accounts login --app browser
browser login --slt TOKEN
browser setup
```

The package bundles checksum-verified release binaries for macOS and Linux on x64 and arm64, so installation does not run a downloader or postinstall script. Linux requires glibc 2.34 or newer; Windows, Alpine, and other platforms are unsupported.

Carbons sign in on [the product site](https://browser.teamofsilicons.com). The npm package is an alternative distribution; update it with npm. Installing it does not start sign-in.

The CLI remembers its Accounts login across invocations. `browser logout` clears
the saved local login. No organization selection is required.

To prepare an npm release from the native-tested Silicon Apps target artifacts:

```sh
node npm/scripts/prepare-release.mjs /path/to/targets
```

This verifies their version and SHA-256 receipts before copying the four native payloads. Packing and publishing also run the native version and discovery checks, so bundled binaries from the former identity integration cannot be shipped as this release. Preparation does not publish anything.
