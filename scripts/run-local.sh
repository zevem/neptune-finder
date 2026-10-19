#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

# AppImage-hosted shells can export an argv[0] override that breaks rustup proxies.
unset ARGV0

exec cargo run --release --locked --bin neptune-finder -- "$@"
