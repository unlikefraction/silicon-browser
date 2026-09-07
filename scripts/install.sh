#!/bin/sh
# Public installer for the pinned Silicon Browser CLI release.
# Keep the entry point last so a truncated download cannot start installation.
set -eu

fail() { printf 'sb installer: %s\n' "$*" >&2; exit 1; }

add_path() {
    profile=$1
    line='export PATH="$HOME/.local/bin:$PATH"'
    if ! grep -Fqx "$line" "$profile" 2>/dev/null; then
        printf '\n# Silicon Browser CLI\n%s\n' "$line" >> "$profile"
    fi
}

main() {
    skip_setup=0
    if [ "${1:-}" = --no-setup ]; then skip_setup=1; shift; fi
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64|Darwin:aarch64)
            target=aarch64-apple-darwin
            digest=33f75d6fac760ad69301020e006f0c5b056914fe7ff9c087d0552e19cc5aa484 ;;
        Darwin:x86_64)
            target=x86_64-apple-darwin
            digest=a8008dc67c6c244ecd8d13a4c2d67c60ab89727b336fd0754d3e4a767aaae94d ;;
        Linux:aarch64|Linux:arm64)
            target=aarch64-unknown-linux-gnu
            digest=9525427706f3726fabb1f730d2ad8123353134a636453f6861a362b2f1e6da52 ;;
        Linux:x86_64)
            target=x86_64-unknown-linux-gnu
            digest=dae0218324eb8266e74fd05d454f69bdcb28ad4a3f09ae0618d71146831144f3 ;;
        *) fail 'Supported platforms: macOS and Linux on x86-64 or ARM64.' ;;
    esac
    case "$target" in
        *linux*)
            libc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)
            case "$libc" in
                'glibc '*)
                    version=${libc#glibc }
                    major=${version%%.*}
                    minor=${version#*.}; minor=${minor%%.*}
                    if ! { [ "$major" -gt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -ge 34 ]; }; }; then
                        fail 'Linux requires glibc 2.34 or later. Upgrade your distribution before installing.'
                    fi ;;
                *) fail 'Linux requires glibc 2.34 or later; musl/Alpine is not supported by this release.' ;;
            esac ;;
    esac
    for tool in curl tar mktemp grep chmod mv mkdir rm; do
        command -v "$tool" >/dev/null 2>&1 || fail "Required utility is missing: $tool"
    done
    if command -v sha256sum >/dev/null 2>&1; then
        checksum=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum=shasum
    else
        fail 'Install sha256sum or shasum to verify the download.'
    fi
    : "${HOME:?HOME must be set}"
    bin_dir="$HOME/.local/bin"
    mkdir -p "$bin_dir"
    work=$(mktemp -d "$bin_dir/.sb-install.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    asset="sb-v0.1.1-$target"
    printf 'Installing sb 0.1.1 for %s…\n' "$target"
    curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 \
        --retry 3 --connect-timeout 20 --max-time 300 \
        "https://github.com/unlikefraction/silicon-browser/releases/download/managed-v0.1.1/$asset.tar.gz" \
        --output "$work/archive.tar.gz"
    if [ "$checksum" = sha256sum ]; then
        actual=$(sha256sum "$work/archive.tar.gz")
    else
        actual=$(shasum -a 256 "$work/archive.tar.gz")
    fi
    [ "${actual%% *}" = "$digest" ] || fail 'Download checksum mismatch; existing installation was not changed.'
    tar -xzf "$work/archive.tar.gz" -C "$work" "$asset/sb"
    chmod 755 "$work/$asset/sb"
    "$work/$asset/sb" --version || fail 'The release cannot run on this system; existing installation was not changed.'
    [ ! -d "$bin_dir/sb" ] || fail "$bin_dir/sb is a directory."
    mv -f "$work/$asset/sb" "$bin_dir/sb"
    add_path "$HOME/.profile"
    add_path "$HOME/.bashrc"
    if [ -f "$HOME/.bash_profile" ]; then
        add_path "$HOME/.bash_profile"
    elif [ -f "$HOME/.bash_login" ]; then
        add_path "$HOME/.bash_login"
    fi
    mkdir -p "${ZDOTDIR:-$HOME}"
    add_path "${ZDOTDIR:-$HOME}/.zshrc"
    case "${SHELL:-}" in
        */fish)
            fish_dir="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d"
            mkdir -p "$fish_dir"
            printf 'fish_add_path "$HOME/.local/bin"\n' > "$fish_dir/silicon-browser.fish" ;;
    esac
    PATH="$bin_dir:$PATH"
    export PATH
    printf 'Installed %s/sb. New terminals will have sb on PATH.\n' "$bin_dir"
    if [ "$skip_setup" -eq 0 ]; then
        if [ -t 0 ]; then
            "$bin_dir/sb" setup "$@"
        elif ( : < /dev/tty ) 2>/dev/null; then
            "$bin_dir/sb" setup "$@" < /dev/tty
        else
            fail 'CLI installed. Open a terminal and run ~/.local/bin/sb setup to finish authentication and controller setup.'
        fi
    fi
}

main "$@"
