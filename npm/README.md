# Silicon Browser npm package

This package replaces the previous `silicon-browser` npm CLI with the managed Silicon Browser CLI (`sb`), currently version 0.2.0. Version 1.0.0 is an intentional breaking replacement.

```sh
npx silicon-browser --version
npx sb login
npx sb setup
```

The package bundles verified release binaries for macOS and Linux on x64 and arm64, so installation does not run a downloader or postinstall script. Linux requires glibc 2.34 or newer; Windows, Alpine, and other platforms are unsupported.

See the current product site at https://browser.teamofsilicons.com.
