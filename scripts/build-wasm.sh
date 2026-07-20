#!/usr/bin/env bash
# Rebuilds the client-wasm crate and drops the wasm-bindgen output where the
# Vite client imports it from (client/src/wasm). Run this after any change to
# crates/crdt-core or crates/client-wasm.
set -euo pipefail
cd "$(dirname "$0")/.."
wasm-pack build crates/client-wasm --target web --out-dir ../../client/src/wasm
