#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

source scripts/gates.sh
run_audit

exec mise exec -- cargo run --package gpui-agent-desktop --locked -- --home "$PWD/.gpui-agent-dev"
