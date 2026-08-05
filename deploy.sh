#!/usr/bin/env bash
set -euo pipefail

readonly PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly BUILT_BINARY="$PROJECT_DIR/target/release/browser-cli"
readonly INSTALLED_BINARY="${HOME}/.cargo/bin/browser-cli"

cd "$PROJECT_DIR"
cargo build --release --locked
install -Dm0755 "$BUILT_BINARY" "$INSTALLED_BINARY"

built_hash="$(sha256sum "$BUILT_BINARY" | cut -d' ' -f1)"
installed_hash="$(sha256sum "$INSTALLED_BINARY" | cut -d' ' -f1)"
[[ "$built_hash" == "$installed_hash" ]] || {
    printf 'browser-cli installed hash mismatch\n' >&2
    exit 1
}
printf 'Installed browser-cli: %s\n' "$installed_hash"
