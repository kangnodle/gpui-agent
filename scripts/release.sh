#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
repo_root="$PWD"

case "$(uname -s)" in
    Darwin)
        formats=app,dmg
        export MACOSX_DEPLOYMENT_TARGET=15.0
        ;;
    Linux) formats=deb ;;
    MINGW*|MSYS*|CYGWIN*) formats=nsis ;;
    *) printf 'Unsupported release host: %s\n' "$(uname -s)" >&2; exit 1 ;;
esac

source scripts/gates.sh

run_audit
mise exec -- cargo fmt --all
mise exec -- cargo clippy --workspace --all-targets --release --locked -- -D warnings
mise exec -- cargo build --workspace --all-targets --release --locked
mise exec -- cargo test --workspace --release --locked

mise exec -- cargo install cargo-packager --version 0.11.8 --locked

if [[ "$formats" == deb ]]; then
    mkdir -p target/release
    glibc_version="$(getconf GNU_LIBC_VERSION)"
    # Reject older glibc at install time instead of installing an unusable binary.
    printf 'libc6 (>= %s)\n' "${glibc_version#glibc }" > target/release/deb-depends.txt
    cat packaging/deb-depends.txt >> target/release/deb-depends.txt
fi

mise exec -- cargo packager --release --packages gpui-agent-desktop --formats "$formats" --out-dir "$repo_root/target/release/bundle"

if [[ "$formats" == deb ]]; then
    mise exec -- cargo install cargo-generate-rpm --version 0.21.1 --locked
    mise exec -- cargo generate-rpm --package apps/desktop --output "$repo_root/target/release/bundle"
fi

printf 'Release artifacts: %s\n' "$repo_root/target/release/bundle"
