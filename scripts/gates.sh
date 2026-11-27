#!/usr/bin/env bash
# Shared monorepo gates, sourced by dev.sh, run.sh and release.sh.
# Each gate runs through mise and the Cargo harness so scripts and CI share one definition:
#   audit = cargo-deny + cargo-machete + cargo-geiger
# Set SKIP_GATES=1 to bypass them (e.g. offline, where cargo-deny cannot fetch advisories).

require_tools() {
    local missing=() tool
    for tool in "$@"; do
        mise exec -- cargo "$tool" --version >/dev/null 2>&1 || missing+=("cargo-$tool")
    done
    if ((${#missing[@]})); then
        printf 'Missing tools: %s\n' "${missing[*]}" >&2
        printf 'Install: mise install, then mise run audit:install\n' >&2
        return 1
    fi
}

run_audit() {
    [[ "${SKIP_GATES:-}" == 1 ]] && return 0
    require_tools deny machete geiger
    mise run audit
}
