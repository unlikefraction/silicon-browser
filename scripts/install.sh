#!/bin/sh
# Bootstrap Silicon Apps, then let it verify, install and update Browser.
# Keep the entry point last so a truncated download cannot start installation.
set -eu

fail() { printf 'browser installer: %s\n' "$*" >&2; exit 1; }

main() {
    case "${1:-}" in
        --no-setup) shift ;; # Compatibility: installation no longer starts sign-in or setup.
        --help|-h)
            printf 'Usage: install.sh [--no-setup]\nInstalls browser through Silicon Apps. Sign in and run browser setup afterwards.\n'
            return ;;
    esac
    if [ "$#" -ne 0 ]; then
        printf 'browser installer: unknown option: %s. Run install.sh --help.\n' "$1" >&2
        exit 2
    fi
    : "${HOME:?HOME must be set}"
    PATH="${SILICON_HOME:-$HOME}/.apps/bin:$PATH"
    export PATH
    registry=https://apps.teamofsilicons.com
    if ! command -v silicon-apps >/dev/null 2>&1; then
        for tool in curl sh mktemp rm; do
            command -v "$tool" >/dev/null 2>&1 || fail "Required utility is missing: $tool"
        done
        work=$(mktemp -d "${TMPDIR:-/tmp}/browser-install.XXXXXXXX")
        trap 'rm -rf "$work"' EXIT
        trap 'exit 130' INT
        trap 'exit 143' TERM
        curl --fail --show-error --silent --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
            --retry 3 --connect-timeout 20 --max-time 300 \
            "$registry/install.sh" --output "$work/install-apps.sh" \
            || fail 'Could not download the Silicon Apps installer.'
        sh "$work/install-apps.sh" --server "$registry" \
            || fail 'Silicon Apps bootstrap failed; Browser was not installed.'
    fi
    apps=$(command -v silicon-apps) || fail 'Silicon Apps was not found after installation.'
    "$apps" --server "$registry" install browser \
        || fail 'Silicon Apps could not install Browser; check the published release and target availability.'
    printf '\nBrowser installed through Silicon Apps. Run browser --help.\n'
    printf 'Sign in through Silicon Accounts, then run browser setup to prepare the local controller.\n'
}

main "$@"
