#!/usr/bin/env bash
# Rebuilds the client-wasm crate and drops the wasm-bindgen output where the
# Vite client imports it from (client/src/wasm). Run this after any change to
# crates/crdt-core or crates/client-wasm.
#
# The output is committed to git, because the client deploys on Vercel, whose
# build image has no Rust toolchain. To keep committed artifacts from silently
# going stale, this script records a hash of the Rust sources they were built
# from; CI recomputes it and fails on a mismatch (.github/workflows/ci.yml).
set -euo pipefail
cd "$(dirname "$0")/.."

OUT_DIR="client/src/wasm"

# Hash every input the artifacts derive from. CI compares this rather than
# byte-comparing the .wasm, which isn't reproducible across compiler/wasm-opt
# versions and would fail for reasons that have nothing to do with staleness.
source_hash() {
  find crates/crdt-core/src crates/client-wasm/src \
       crates/crdt-core/Cargo.toml crates/client-wasm/Cargo.toml \
       -type f | sort | xargs shasum -a 256 | shasum -a 256 | cut -d' ' -f1
}

if [ "${1:-}" = "--check" ]; then
  expected=$(cat "$OUT_DIR/.source-hash" 2>/dev/null || echo "<missing>")
  actual=$(source_hash)
  if [ "$expected" != "$actual" ]; then
    echo "ERROR: committed WASM artifacts in $OUT_DIR are stale." >&2
    echo "  recorded source hash: $expected" >&2
    echo "  current source hash:  $actual" >&2
    echo "Run ./scripts/build-wasm.sh and commit the result." >&2
    exit 1
  fi
  echo "WASM artifacts are up to date with the Rust sources."
  exit 0
fi

wasm-pack build crates/client-wasm --target web --out-dir ../../client/src/wasm

# wasm-pack drops a `.gitignore` containing `*` into its output directory, which
# would exclude the very artifacts the Vercel build needs committed.
rm -f "$OUT_DIR/.gitignore"
source_hash > "$OUT_DIR/.source-hash"

echo "Built $OUT_DIR (source hash $(cat "$OUT_DIR/.source-hash"))"
