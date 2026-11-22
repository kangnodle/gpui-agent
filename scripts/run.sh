#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

source scripts/gates.sh

run_audit
mise exec -- cargo fmt --all
mise exec -- cargo clippy --workspace --all-targets --locked -- -D warnings
mise exec -- cargo build --workspace --all-targets --locked
mise exec -- cargo test --workspace --locked
exec mise exec -- cargo harness dev --surface desktop -- "$@"
