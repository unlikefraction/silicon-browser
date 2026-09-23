#!/bin/sh
# Public installer for the pinned Silicon Browser CLI release.
# Keep the entry point last so a truncated download cannot start installation.
set -eu

fail() { printf 'browser installer: %s\n' "$*" >&2; exit 1; }

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
            digest=7bca327c371fc608701a45c82d194cf2503cf856b8edde03edf9d63557a51c0b ;;
        Darwin:x86_64)
            target=x86_64-apple-darwin
            digest=ebac329b3d77b8ae4a621b3edb8ab108024629233c330bad7866d4495c3fa398 ;;
        Linux:aarch64|Linux:arm64)
            target=aarch64-unknown-linux-gnu
            digest=8b2e803e3f3968ae4ca527f0f425d35f0f5d8cfe0b9627fca8fa98023d6fceb2 ;;
        Linux:x86_64)
            target=x86_64-unknown-linux-gnu
            digest=2a207a6bd630b930c02b904f5ff73b79ab77554deace8f17bed24a324a5a30d7 ;;
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
    work=$(mktemp -d "$bin_dir/.browser-install.XXXXXX")
    trap 'rm -rf "$work"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    asset="browser-v0.3.1-$target"
    printf 'Installing browser 0.3.1 for %s…\n' "$target"
    curl --fail --show-error --silent --location --proto '=https' --tlsv1.2 \
        --retry 3 --connect-timeout 20 --max-time 300 \
        "https://github.com/unlikefraction/silicon-browser/releases/download/managed-v0.3.1/$asset.tar.gz" \
        --output "$work/archive.tar.gz"
    if [ "$checksum" = sha256sum ]; then
        actual=$(sha256sum "$work/archive.tar.gz")
    else
        actual=$(shasum -a 256 "$work/archive.tar.gz")
    fi
    [ "${actual%% *}" = "$digest" ] || fail 'Download checksum mismatch; existing installation was not changed.'
    tar -xzf "$work/archive.tar.gz" -C "$work" "$asset/browser"
    chmod 755 "$work/$asset/browser"
    installed_version=$("$work/$asset/browser" --version) || fail 'The release cannot run on this system; existing installation was not changed.'
    [ "$installed_version" = 'browser 0.3.1' ] || fail 'The release has an unexpected command name or version; existing installation was not changed.'
    [ ! -d "$bin_dir/browser" ] || fail "$bin_dir/browser is a directory."
    mv -f "$work/$asset/browser" "$bin_dir/browser"
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
    printf 'Installed %s/browser. New terminals will have browser on PATH.\n' "$bin_dir"
    if [ "$skip_setup" -eq 0 ]; then
        if [ -t 0 ]; then
            "$bin_dir/browser" setup "$@"
        elif ( : < /dev/tty ) 2>/dev/null; then
            "$bin_dir/browser" setup "$@" < /dev/tty
        else
            fail 'CLI installed. Open a terminal and run ~/.local/bin/browser setup to finish authentication and controller setup.'
        fi
    fi
}

main "$@"
