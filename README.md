# CRDT Editor

A real-time collaborative text editor built on a from-scratch CRDT (no Yjs, no
Automerge). The point of this project is the algorithmic core: concurrent
edits from multiple clients - including edits made fully offline - converge to
an identical, uncorrupted document.

Full design rationale (algorithm choice, causal delivery, undo/redo
correctness argument, persistence schema, testing strategy, known
limitations) lives in `PLAN.md`.

## Architecture

A Cargo workspace with three crates, so the CRDT algorithm has exactly **one**
implementation shared by client and server:

- **`crates/crdt-core`** - pure Rust, no I/O. The RGA (Replicated Growable
  Array) algorithm: op types, vector clocks, causal buffering for out-of-order
  delivery, and local-only undo/redo with set-based tombstones. This is the
  crate with the property tests.
- **`crates/server`** - `axum` WebSocket server. Owns a canonical `RgaDoc`
  replica per document, assigns each connecting client a unique site id,
  relays ops, and catches up late joiners with full history.
- **`crates/client-wasm`** - thin `wasm-bindgen` wrapper around `crdt-core`,
  compiled to WASM and consumed by the React client. Client and server run
  the *same compiled algorithm*, not two hand-written ports of it.
- **`client/`** - Vite + React + TypeScript, CodeMirror 6 for the editor
  surface (gives clean position-based change events instead of raw DOM
  mutation diffing).

## Status

| Milestone | Status |
|---|---|
| 1. Single-user editor | Done |
| 2. Naive two-client broadcast | Done |
| 3. Real CRDT (RGA) + causal buffering + WASM wiring | Done |
| 4. Offline queue + reconnect reconciliation + Postgres persistence | Not started |
| 5. Cursor/presence sync | Not started |
| 6. Undo/redo batching (currently per-character) | Partially done - undo/redo works and is tested for concurrency correctness; the `captureTimeout` batching described in `PLAN.md` is not yet implemented |

Verified manually: two browser tabs on the same `?doc=` id converge under
concurrent typing at the same cursor position, and undo propagates live to
other clients.

## Running it

```bash
# 1. Rust toolchain + wasm target (one-time)
rustup target add wasm32-unknown-unknown
cargo install wasm-pack

# 2. Build the WASM client bindings
./scripts/build-wasm.sh

# 3. Server (terminal 1)
cargo run -p server        # listens on ws://127.0.0.1:8787/ws/:doc_id

# 4. Client (terminal 2)
cd client && npm install && npm run dev
```

Open `http://localhost:5173/?doc=demo` in two tabs to see live convergence.

## Tests

```bash
cargo test -p crdt-core
```

Includes a `proptest`-based convergence harness (`tests/convergence.rs`):
random concurrent edits across simulated replicas, delivered out of order and
with duplicates, must converge to byte-identical state on every replica. Also
includes a test that documents RGA's known interleaving anomaly (see
`PLAN.md`), and targeted undo/redo concurrency tests
(`tests/undo_redo.rs`) covering the case that makes the set-based tombstone
design necessary: undoing your own delete must not resurrect a node someone
else independently deleted too.

## Known limitations (by design, for this project's scope)

- **RGA interleaving anomaly**: concurrent inserts at the same position from
  two sites can interleave character-by-character instead of appearing as
  clean runs. The fix (Fugue's subtree-based tie-breaking) is a materially
  different data structure and is out of scope here - see `PLAN.md`.
- **No tombstone GC**: deleted nodes are never purged, so a long-lived
  document grows unbounded. Fine at demo scale; real GC needs causal
  stability tracking.
- **Undo granularity**: currently per-character rather than batched into
  logical edits.
- **Local deployment only**: no hosted demo yet.
