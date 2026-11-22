#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

source scripts/gates.sh

run_audit
mise exec -- cargo fmt --all
mise exec -- cargo clippy --workspace --all-targets --locked -- -D warnings
mise exec -- cargo build --workspace --all-targets --locked
mise exec -- cargo test --workspace --locked
args=(--home "$PWD/.gpui-agent-dev" "$@")
for arg in "$@"; do
    case "$arg" in --home|--home=*) args=("$@"); break ;; esac
done
exec mise exec -- cargo run --package gpui-agent-desktop --locked -- "${args[@]}"
