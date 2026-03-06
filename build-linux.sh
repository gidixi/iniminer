#!/bin/bash
# Compila il miner per Linux (x86_64).
# Esegui su Linux o in WSL: ./build-linux.sh

set -e
cd "$(dirname "$0")"
cargo build --release
echo "Build completato: target/release/miner"
