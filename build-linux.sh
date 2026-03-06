#!/usr/bin/env bash
# Compila i progetti miner per Linux.
# Uso:
#   ./build-linux.sh
#   ./build-linux.sh debug
#   ./build-linux.sh release x86_64-unknown-linux-gnu cpuminer
#   ./build-linux.sh release x86_64-unknown-linux-gnu gpuminer
#   ./build-linux.sh release x86_64-unknown-linux-gnu gpuminer cuda

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
PROJECT="${3:-all}"
GPU_BACKEND="${4:-cpu}"

case "$PROFILE" in
  release|debug) ;;
  *)
    echo "Profilo non valido: $PROFILE (usa 'release' o 'debug')." >&2
    exit 1
    ;;
esac

case "$PROJECT" in
  all) ;;
  cpuminer) ;;
  gpuminer) ;;
  *)
    echo "Progetto non valido: $PROJECT (usa 'all', 'cpuminer' o 'gpuminer')." >&2
    exit 1
    ;;
esac

case "$GPU_BACKEND" in
  cpu|cuda) ;;
  *)
    echo "Backend GPU non valido: $GPU_BACKEND (usa 'cpu' o 'cuda')." >&2
    exit 1
    ;;
esac

if [[ "$PROJECT" != "gpuminer" && "$GPU_BACKEND" == "cuda" ]]; then
  echo "Il backend 'cuda' è valido solo con progetto gpuminer." >&2
  exit 1
fi

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
if [[ "$PROJECT" == "all" ]]; then
  BUILD_ARGS+=(--workspace)
elif [[ "$PROJECT" == "cpuminer" ]]; then
  BUILD_ARGS+=(-p cpuminer)
else
  BUILD_ARGS+=(-p gpuminer)
  if [[ "$GPU_BACKEND" == "cuda" ]]; then
    if ! command -v nvcc >/dev/null 2>&1; then
      echo "Errore: nvcc non trovato, impossibile compilare gpuminer con backend CUDA." >&2
      exit 1
    fi
    BUILD_ARGS+=(--features cuda)
  fi
fi

if [[ "$PROFILE" == "release" ]]; then
  BUILD_ARGS+=(--release)
fi

echo "Compilo progetto=$PROJECT backend=$GPU_BACKEND ($PROFILE) per target $TARGET..."
cargo "${BUILD_ARGS[@]}"

if [[ "$PROJECT" == "all" || "$PROJECT" == "cpuminer" ]]; then
  if [[ "$PROFILE" == "release" ]]; then
    echo "Build cpuminer: target/$TARGET/release/cpuminer"
  else
    echo "Build cpuminer: target/$TARGET/debug/cpuminer"
  fi
fi
if [[ "$PROJECT" == "all" || "$PROJECT" == "gpuminer" ]]; then
  if [[ "$PROFILE" == "release" ]]; then
    echo "Build gpuminer: target/$TARGET/release/gpuminer"
  else
    echo "Build gpuminer: target/$TARGET/debug/gpuminer"
  fi
fi
