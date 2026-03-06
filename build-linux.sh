#!/usr/bin/env bash
# Compila il miner per Linux.
# Uso:
#   ./build-linux.sh                  # release, target host Linux
#   ./build-linux.sh debug            # debug, target host Linux
#   ./build-linux.sh release x86_64-unknown-linux-gnu

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "Errore: questo script è pensato per Linux." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Errore: cargo non trovato. Installa Rust (https://rustup.rs)." >&2
  exit 1
fi

PROFILE="${1:-release}"
TARGET="${2:-$(rustc -vV | sed -n 's/^host: //p')}"

case "$PROFILE" in
  release|debug) ;;
  *)
    echo "Profilo non valido: $PROFILE (usa 'release' o 'debug')." >&2
    exit 1
    ;;
esac

TARGET_INSTALLED=false
while IFS= read -r INSTALLED_TARGET; do
  if [[ "$INSTALLED_TARGET" == "$TARGET" ]]; then
    TARGET_INSTALLED=true
    break
  fi
done < <(rustup target list --installed)

if [[ "$TARGET_INSTALLED" == false ]]; then
  echo "Aggiungo target Rust: $TARGET"
  rustup target add "$TARGET"
fi

BUILD_ARGS=(build --locked --target "$TARGET")
OUT_DIR="target/$TARGET/debug"
if [[ "$PROFILE" == "release" ]]; then
  BUILD_ARGS+=(--release)
  OUT_DIR="target/$TARGET/release"
fi

echo "Compilo ($PROFILE) per target $TARGET..."
cargo "${BUILD_ARGS[@]}"
echo "Build completato: $OUT_DIR/miner"
