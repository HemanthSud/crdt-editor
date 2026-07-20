# Real-Time Collaborative CRDT Text Editor (Portfolio Project)

## Context

This is a from-scratch portfolio project meant to demonstrate genuine algorithmic depth: a real-time collaborative text editor where concurrent edits from multiple users (including edits made fully offline) converge to an identical, uncorrupted document without relying on an existing CRDT library (no Yjs/Automerge). The differentiator the user wants is specifically the hard, easy-to-get-wrong parts: causal delivery of ops, offline reconciliation on reconnect, and undo/redo that stays correct under concurrent remote edits — the milestone most naive implementations break on.

The project directory (`/Users/hemanthsudhaharan/CRDT`) is currently empty — this is a greenfield build. Confirmed decisions: **Rust** backend, **Postgres** persistence, **local-only** demo (no live hosting yet; a recorded multi-client sync + offline-reconciliation demo is the deliverable artifact for the portfolio).

## Architecture

Cargo workspace, three crates, so the CRDT algorithm has exactly one implementation shared by client and server (avoids the classic bug class of client/server CRDT logic silently diverging):

- **`crdt-core`** — pure Rust, no I/O. The RGA algorithm, op types, vector clocks, causal buffering, undo/redo logic. Small, deliberate public API so the WASM boundary stays cheap: `local_insert`, `local_delete`, `apply_remote_ops(Vec<Op>)`, `state_vector() -> VectorClock`, `ops_since(VectorClock) -> Vec<Op>`, `render_text() -> String`, `undo()`, `redo()`.
- **`server`** — `axum` + `tokio-tungstenite` for WebSocket handling, `sqlx` for Postgres, depends on `crdt-core` natively. Owns the canonical replica per document, persists the op log, relays ops between connected clients, serves reconnect sync.
- **`client-wasm`** — thin `wasm-bindgen` wrapper around `crdt-core`, compiled via `wasm-pack --target web`, consumed from the React app (Vite + `vite-plugin-wasm`). Batch calls across the WASM boundary (arrays of ops), never per-keystroke.

**Editor component**: CodeMirror 6, not raw `contenteditable` — it emits clean position-based change events that map directly to RGA insert/delete ops, instead of noisy DOM-mutation diffing.

## CRDT Algorithm: RGA (Replicated Growable Array)

- Each character-run node has a globally unique id `(site_id: u64, counter: u64)`, assigned from a per-site monotonic counter (Lamport-style).
- Insert op: `{id, value, origin_left: OpId}` — "insert this node immediately after node `origin_left`". Tie-breaking for concurrent inserts at the same position uses `site_id` ordering (standard RGA rule).
- Delete op: not a boolean tombstone — each node carries `deleted_by: Set<OpId>`. A node is visible iff the set is empty. This is the mechanism that makes undo-of-a-delete correct under concurrency (see below).
- **Known limitation, documented not built**: RGA has an interleaving anomaly — two sites typing different words concurrently at the same cursor position can interleave character-by-character in the merged result. The state-of-the-art fix (Fugue) requires a subtree-based origin-left/origin-right structure, not an incremental change — that's out of scope for this timeline. Milestone 3 includes a proptest case that *demonstrates* the anomaly, and the README documents Fugue as future work with a link to the paper. This is a deliberate, defensible scoping decision, not an oversight — say so explicitly in the portfolio writeup.

## Causal Delivery & Reconnect Sync

- Each replica (client and server) tracks a **vector clock**: `HashMap<SiteId, u64>` = last-seen counter per site.
- An incoming op whose `origin_left` hasn't been seen yet is buffered in `HashMap<OpId, Vec<PendingOp>>` keyed on the missing dependency, and drained recursively once that dependency arrives. This same buffering path is used for *both* live ops and reconnect replay — no special-cased reconnect logic.
- Reconnect protocol: client sends its vector clock → server computes `ops_since(vector)` per site from the Postgres op log → streams the result back through the normal causal-buffer path.
- Offline support: while disconnected, the client keeps applying local ops to its local `crdt-core` replica (so the UI stays responsive) and queues them (IndexedDB). On reconnect, queued ops are sent to the server exactly like the vector-clock catch-up above — same code path, both directions.

## Undo/Redo

- **Local-only undo** (each client can only undo its own past ops, not global undo) — matches Yjs's `UndoManager` scoping. Global undo in CRDTs is an open hard problem; scoping to local-only is the correct, defensible choice here.
- Undo of an insert = tombstone that node (add a marker `OpId` to its `deleted_by` set).
- Undo of a delete = remove your own `OpId` from that node's `deleted_by` set. Because the tombstone is a *set*, this composes correctly: if someone else has also (independently) deleted the same node, their entry remains and the node correctly stays hidden. This set-vs-boolean distinction is the core correctness argument for the whole scheme — call it out explicitly when demoing.
- Undo/redo operate on op IDs, not positions/indices, so there's no OT-style "stack invalidation" when intervening remote edits occur — this is a designed property.
- **Batching**: coalesce local ops into one undo-stack entry using a timeout window (Yjs's `captureTimeout` pattern, ~500ms or until a non-adjacent edit) — otherwise undo is unusably fine-grained (single character).
- **GC interaction (documented limitation)**: real tombstone garbage collection is not implemented for this project — purging a tombstone that a live undo stack references would break undo. Real GC (causal-stability based purge) is noted as future work; for a demo-scale document this is fine to skip entirely.

## Persistence (Postgres)

- `ops(doc_id, site_id, counter, op_type, parent_op_id, payload jsonb, created_at)`, `UNIQUE(doc_id, site_id, counter)` for idempotent replay/dedup.
- `snapshots(doc_id, vector_clock jsonb, state bytea, created_at)`, taken every N ops. Document reload = latest snapshot + replay the op-log tail after its vector clock.

## Testing Strategy (build this in Milestone 3, not at the end)

Property-based convergence harness using `proptest`:
- Spin up N `SimReplica`s wrapping `crdt-core`.
- Generate random per-site op sequences (insert/delete, later undo/redo).
- Shuffle delivery order per replica, including duplicate delivery and pre-dependency delivery, to exercise the causal buffer.
- Assert `render_text()` and full tombstone state are byte-identical across all replicas after convergence.
- Reuse and extend this exact harness in Milestone 6 for undo/redo — don't write a separate suite.

## Milestones (mapped to the user's original 6, with realistic effort)

1. **Single-user editor** (~2-3 days) — CodeMirror 6 in React, local-only state, no networking yet. Establishes the editor↔op-emission boundary.
2. **Naive two-client broadcast** (~2-3 days) — WS relay with no conflict resolution (last-write-wins), to make the corruption problem visible before solving it. Basic axum WS server, no persistence yet.
3. **Real CRDT (RGA) + causal buffering + proptest harness** (~1.5-2 weeks, highest risk alongside M6) — `crdt-core` crate, vector clocks, causal buffer, convergence test suite. Everything later depends on this being solid; bugs here cascade silently.
4. **Offline queue + reconnect reconciliation + Postgres persistence** (~1 week) — IndexedDB local queue, vector-clock-based reconnect sync, op log + snapshot schema.
5. **Cursor/presence sync** (~3-4 days) — ephemeral (non-persisted) WS messages; anchor cursors to RGA node IDs, not raw text indices, so they don't drift under concurrent edits.
6. **Undo/redo across concurrent remote edits** (~1.5-2 weeks, highest risk alongside M3) — set-based tombstone undo/redo, local-only scoping, batching, extended proptest coverage.

Realistic total: **6-8 weeks part-time**. M3 and M6 are the two milestones to budget the most slack for.

## Verification

- Each milestone gets both a proptest convergence check (extended incrementally) and a manual multi-client test: open the React client in 2+ browser windows/tabs, edit concurrently, kill network on one tab (DevTools offline mode) mid-edit, reconnect, confirm convergence.
- Final deliverable: a recorded demo (GIF/video) showing concurrent multi-client editing, an offline edit reconciling on reconnect, and undo/redo surviving a concurrent remote edit — this is the artifact that goes in the portfolio/README.

## Known Limitations (documented, not built — explicitly scope-cut for this timeline)

- Fugue-style interleaving avoidance (RGA's anomaly is demonstrated by a test, not fixed).
- Real tombstone GC (causal-stability purge).
- Live deployment (local demo only for now; client on Vercel + WS server on Fly.io/Railway is a natural follow-up if the user wants to host it later).
