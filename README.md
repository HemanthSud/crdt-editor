# CRDT Editor

A real-time collaborative text editor built on a CRDT written from scratch — no Yjs,
no Automerge. Multiple people edit the same document at once, their concurrent edits
converge to an identical result, and each person can undo their own work without
stepping on anyone else's.

The editor is the demo. The interesting part is the ~450 lines of Rust underneath it,
and the tests that keep it honest.

<!-- Record the demo (see "Demo script"), save to docs/demo.gif, then uncomment:
![Two clients editing concurrently](docs/demo.gif)
**[Live demo →](https://your-app.vercel.app/?doc=demo)**
-->

---

## Contents

- [What it does](#what-it-does)
- [Why it's built this way](#why-its-built-this-way)
- [How the CRDT works](#how-the-crdt-works)
- [Correctness: two properties, not one](#correctness-two-properties-not-one)
- [Performance](#performance)
- [Getting started](#getting-started)
- [Project layout](#project-layout)
- [Testing and benchmarks](#testing-and-benchmarks)
- [Deployment](#deployment)
- [Troubleshooting](#troubleshooting)
- [Known limitations](#known-limitations)
- [Roadmap](#roadmap)

---

## What it does

Open the same document in two browser tabs and type in both at once. There is no
locking, no "last write wins", and no server arbitration deciding whose edit
survives — both edits survive, and every client independently computes the same
final document.

Three behaviours are worth calling out, because they're the ones that are hard:

1. **Concurrent edits converge.** Two people typing at the same cursor position at
   the same moment end up with byte-identical documents.
2. **Out-of-order delivery is safe.** An operation that depends on an operation you
   haven't received yet gets buffered and applied later, automatically. This is the
   normal code path, not an error case.
3. **Undo survives concurrency.** If you delete a character and undo it, but someone
   else *also* deleted that character independently, it stays deleted. Your undo
   reverses your action — not theirs.

---

## Why it's built this way

### One algorithm, two runtimes

The CRDT lives in `crates/crdt-core` — pure Rust, no I/O, no networking. The server
depends on it natively. The browser runs **the same crate** compiled to WebAssembly.

```
crates/crdt-core   ── RGA, vector clocks, causal buffer, undo/redo   (no I/O)
      │
      ├── crates/server       axum WebSocket relay, canonical replica per document
      └── crates/client-wasm  wasm-bindgen shim  →  client/  (React + CodeMirror 6)
```

The usual approach is to write the CRDT once in the backend language and again in
JavaScript for the browser. Those two implementations then drift, and the resulting
corruption bugs are miserable to reproduce — they only appear under specific
interleavings, on someone else's machine. Compiling one implementation to both
targets makes that entire bug class impossible rather than merely unlikely.

### CodeMirror instead of `contenteditable`

CodeMirror 6 emits clean position-based change events (`from`, `to`, `inserted`),
which map directly onto CRDT insert/delete operations. Raw `contenteditable` would
mean diffing DOM mutations to guess what the user did.

---

## How the CRDT works

The algorithm is **RGA** (Replicated Growable Array).

**Every character is a node with a globally unique id.** The id is
`(site_id, counter)` — which replica created it, and its local sequence number.
Ids are never reused and never renumbered, so they're stable references that
survive any amount of concurrent editing.

**Inserts name their left neighbour, not a position.** An insert says "put this
character immediately after node X", not "put it at index 7". Index 7 means
different things on different replicas at different times; node X does not. When
two replicas concurrently insert after the same node, both inserts are kept and
ordered deterministically by comparing their ids — so every replica independently
arrives at the same order without talking to anyone.

**Deletes are tombstones, and tombstones are a set.** Deleting doesn't remove the
node; it adds the deleting operation's id to that node's `deleted_by` set. A node
is visible only when that set is empty. This is the detail the whole undo design
rests on:

> If you delete a character and someone else independently deletes the same
> character, the set holds *both* ids. When you undo, only your id is removed —
> the other one remains, so the character correctly stays hidden. A boolean
> `is_deleted` flag would resurrect content that someone else had validly deleted.

**Out-of-order operations are buffered, not dropped.** Each operation declares its
dependencies (the node it attaches to, the delete it reverses). If any dependency
hasn't arrived, the operation waits in a pending map keyed on what it's missing,
and is applied automatically when that arrives — recursively, so a chain of
waiting operations unblocks in one pass. Re-applying an operation is a no-op, so
duplicate delivery is harmless. The same path serves live edits and
catch-up-on-connect, which means there's no separate reconnect logic to get subtly
wrong.

**Undo targets operation ids, never positions.** Because undo references the node
it affects rather than an index, remote edits arriving mid-session can't invalidate
the undo stack — the failure mode that makes undo in OT-based editors so fragile.
Undo is scoped to your own edits, matching how Yjs's `UndoManager` behaves; global
multi-user undo is an open research problem.

---

## Correctness: two properties, not one

`cargo test --workspace` runs 7 tests. The centrepiece is a `proptest` harness that
generates random concurrent edit sequences across 4 simulated replicas, delivers
every operation to every replica in an independently shuffled and ~30% duplicated
order, and asserts all replicas end byte-identical — comparing full structural
state including tombstones, not just rendered text, since matching text could hide
a structural bug.

That harness caught a real transitivity bug in RGA integration: comparing only
direct `origin_left` equality instead of origin *positions* converges fine in
simple cases and diverges under deeper concurrency.

**But convergence testing has a blind spot, and it's worth understanding:**

> A convergence test cannot catch an undo bug. Every replica applies the same
> operation stream, so if undo emits a compensating operation that targets the
> wrong tombstone, all replicas still agree perfectly — on the wrong document.
> It converges. It just doesn't do what the user asked for.

This codebase had exactly that bug: `delete → undo → redo → undo` left the
character deleted, because redo issued a *new* delete operation but recorded the
*old* one's id for the next undo to reverse. It passed 200 cases of the convergence
harness without complaint.

Catching it needs a separate **intent** property — that redo-then-undo is an
identity at every depth of the undo stack
([`redo_then_undo_is_identity_at_every_depth`](crates/crdt-core/tests/undo_redo.rs)).
Note that the obvious formulation of that property ("undo everything, redo
everything, undo everything") is *also* blind to the bug, because undoing each
insert tombstones its node anyway and both passes bottom out on an empty document.
The property has to step one entry at a time.

Two lessons, both cheap to state and expensive to learn: consistency is not
correctness, and a passing test is not evidence until you've watched it fail.

---

## Performance

`cargo bench -p crdt-core`. Medians from a criterion run on an Apple M1 Pro.

| Operation | 1,000 chars | 10,000 chars | 50,000 chars |
|---|---:|---:|---:|
| Apply a remote operation | 304 ns | 394 ns | 1.7 µs |
| Type at end of document | 2.0 µs | 8.6 µs | 250 µs |
| **Type at start of document** | **58 µs** | **622 µs** | **6.2 ms** |
| Delete mid-document | 0.6 µs | 4.6 µs | 133 µs |
| `render_text()` (whole document) | 1.2 µs | 10.5 µs | 48 µs |
| Load document from operation log | 291 µs | 4.2 ms | 17.8 ms |

**Applying a remote operation is effectively constant time**, even at 50k nodes.
Remote operations carry their own `origin_left` id, so integration jumps straight
to the insertion point with no search. The RGA algorithm itself is cheap.

**Every slow path is bookkeeping, not CRDT logic.** Local edits are O(n) for one
reason: the editor supplies a *cursor position*, and translating that back into a
node id means walking the node list. That's a data-structure problem, not a
consistency problem.

**Typing at the top of a document is ~25× slower than typing at the bottom**
(6.2 ms vs 250 µs at 50k chars). `reindex_from` rebuilds the position index for
every node after the insertion point — appending touches one entry, inserting at
position 0 rebuilds all 50,000. At 6.2 ms that's over a third of a 16 ms frame
budget, so it's the first thing a user would actually feel.

Both slow paths collapse into a single fix: an order-statistic index (rope or
balanced tree) maintaining position ↔ node. That's the next optimization worth
doing, and it has nothing to do with RGA. Until then the honest bound is:
comfortable to roughly 10k characters, visibly laggy when editing near the top of
something much larger.

*Measurement note:* `Vec::clone` allocates exact capacity, so a naive benchmark
charges each measured insert for a realloc plus a full memcpy of the node list —
which at 50k nodes costs more than the operation under test. The setup helper
forces that growth before the timer starts. The first version of this benchmark
reported remote-operation application as linear for exactly that reason; correcting
it moved the 50k number from 140 µs to 1.7 µs, an 80× difference between the
artifact and the truth.

---

## Getting started

### Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| [Rust](https://rustup.rs) | stable (1.75+) | server and CRDT core |
| `wasm32-unknown-unknown` target | — | compiling the CRDT for the browser |
| [wasm-pack](https://rustwasm.github.io/wasm-pack/) | 0.12+ | generating JS bindings for the WASM module |
| [Node.js](https://nodejs.org) | 20+ | the Vite/React client |

```bash
# Rust, if you don't have it
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# One-time project setup
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

### Build and run

Three steps. The first is only needed again when you change Rust code.

```bash
# 1. Compile the CRDT to WebAssembly for the browser.
#    Re-run after any change to crates/crdt-core or crates/client-wasm.
./scripts/build-wasm.sh

# 2. Start the WebSocket server (terminal 1) — listens on 127.0.0.1:8787
cargo run -p server

# 3. Start the client (terminal 2) — serves on localhost:5173
cd client
npm install
npm run dev
```

Then open **<http://localhost:5173/?doc=demo>** in two browser tabs.

The `?doc=` query parameter names the document. Any two clients using the same
value share a document; change it to work on a different one. It defaults to
`demo` if omitted.

### Demo script

Three things worth trying, in increasing order of how hard they are to get right:

1. **Convergence** — type at the same cursor position in both tabs simultaneously.
   Both land on identical text, and neither cursor jumps around.
2. **Undo under concurrency** — press `Cmd+Z` / `Ctrl+Z` in one tab while the other
   keeps typing. Your last edit is undone and the undo propagates live.
3. **The set-based tombstone** — delete the *same character* in both tabs, then
   undo in one of them. The character stays deleted, because the other tab's delete
   is still in the set. Naive implementations resurrect it here.

Also try pasting an emoji and editing around it in both tabs — editor positions are
UTF-16 code units while the CRDT indexes by Unicode scalar, and the conversion
between them is a genuinely easy thing to get wrong.

---

## Project layout

```
.
├── crates/
│   ├── crdt-core/            The algorithm. Pure Rust, no I/O.
│   │   ├── src/op.rs         Operation types (Insert/Delete/Restore), OpId, dependencies
│   │   ├── src/doc.rs        RgaDoc: integration, causal buffer, undo/redo, sync
│   │   ├── tests/            Convergence proptest + undo/redo correctness
│   │   └── benches/          Criterion benchmarks
│   ├── server/               axum WebSocket relay; owns a canonical replica per doc
│   └── client-wasm/          wasm-bindgen wrapper exposing crdt-core to JavaScript
├── client/
│   └── src/
│       ├── Editor.tsx        CodeMirror integration, minimal-diff remote updates
│       ├── useCrdtClient.ts  WASM lifecycle, WebSocket, position conversion
│       └── wasm/             Generated bindings (committed — see Deployment)
├── scripts/build-wasm.sh     Rebuilds WASM + records a source hash for CI
├── Dockerfile, fly.toml      Server deployment
└── PLAN.md                   Original design document and milestone plan
```

### How a keystroke flows through the system

1. You type. CodeMirror reports a position-based change.
2. `useCrdtClient` converts UTF-16 positions to code-point indices and calls into
   the WASM module.
3. `crdt-core` creates an operation, applies it locally, and returns it as JSON.
4. The operation goes over the WebSocket to the server, which applies it to its own
   canonical replica and broadcasts it to everyone connected.
5. Each other client applies it — buffering first if its dependencies haven't
   arrived — re-renders the text, and CodeMirror receives a *minimal* diff so
   nobody's cursor moves.

---

## Testing and benchmarks

```bash
cargo test --workspace          # 7 tests: convergence proptest + undo/redo properties
cargo bench -p crdt-core        # criterion benchmarks (takes a few minutes)
cargo clippy --workspace --all-targets
cd client && npm run build      # typecheck + production build
```

CI (`.github/workflows/ci.yml`) runs the tests, verifies the benchmarks still
compile, checks the committed WASM artifacts aren't stale, and builds the client.

---

## Deployment

The client is a static site; the server is a long-lived WebSocket process. They
deploy separately.

### Server (Fly.io)

```bash
fly launch --no-deploy    # once, to create the app
fly deploy
```

Document state lives **in memory** (`AppState.rooms`), so the server must not be
auto-stopped or scaled beyond one machine — `fly.toml` sets
`auto_stop_machines = false` and `min_machines_running = 1` for that reason. With
auto-stop enabled, a document would silently vanish whenever the app idled out
between visitors. Persistence is the next milestone.

### Client (Vercel)

Set the project's **root directory** to `client`, and add an environment variable:

```
VITE_WS_URL=wss://<your-app>.fly.dev
```

It must be `wss://` — an HTTPS page is not permitted to open a plaintext `ws://`
connection, and the browser will silently block it as mixed content.

### Why the WASM output is committed

Vercel's build image has no Rust toolchain, so the generated bindings in
`client/src/wasm/` are checked into git. To stop them going stale,
`scripts/build-wasm.sh` records a hash of the Rust sources they were built from,
and CI fails if the two drift apart:

```bash
./scripts/build-wasm.sh --check    # what CI runs
```

If that fails, run `./scripts/build-wasm.sh` and commit the result.

---

## Troubleshooting

**Status pill shows "disconnected"** — the server isn't running, or is on a
different port. Check `cargo run -p server` is up and that `/health` responds:
`curl http://127.0.0.1:8787/health` should return `ok`.

**Blank page, console error about WASM** — the bindings haven't been generated.
Run `./scripts/build-wasm.sh`, then restart the Vite dev server.

**Edits appear in one tab but not the other** — confirm both tabs use the same
`?doc=` value. Different values are different documents, which is working as
intended.

**Changed Rust code and nothing happened** — the browser loads the compiled WASM,
not your source. Re-run `./scripts/build-wasm.sh`.

**`wasm-pack: command not found`** — `cargo install wasm-pack`, and ensure
`~/.cargo/bin` is on your `PATH`.

**Mixed-content error in production** — `VITE_WS_URL` is `ws://` where it needs to
be `wss://`.

---

## Known limitations

Scope decisions rather than oversights — each is a real piece of work with a known
shape.

- **RGA interleaving anomaly.** Two people typing *different words* at the same
  position concurrently can interleave character-by-character instead of staying as
  clean runs. Every replica agrees on the same interleaving, so this is a usability
  flaw, not a correctness one. The fix is [Fugue](https://arxiv.org/abs/2305.00583)'s
  subtree-based tie-breaking, which is a materially different data structure. The
  test `documents_rga_interleaving_anomaly` pins the current behaviour.
- **No tombstone garbage collection.** Deleted nodes are never purged, so a
  long-lived document grows without bound. Real GC needs causal-stability tracking,
  and purging a tombstone that a live undo stack still references would break undo.
- **One node per code point.** A grapheme cluster assembled from several code points
  (`👨‍👩‍👧`) can be split by concurrent edits. Yjs has the same property at this level.
- **Undo granularity is per-character**, not batched into logical edits. Yjs's
  `captureTimeout` approach is the standard fix.
- **No persistence.** Restarting the server discards every document.
- **Local edits are O(n)** — measured above. Comfortable to ~10k characters.

---

## Roadmap

| Milestone | Status |
|---|---|
| 1. Single-user editor | Done |
| 2. Naive two-client broadcast | Done |
| 3. RGA + causal buffering + WASM wiring | Done |
| 4. Offline queue + reconnect reconciliation + Postgres persistence | Next |
| 5. Cursor / presence sync | Planned |
| 6. Undo/redo across concurrent edits | Done and property-tested; batching still open |

Design rationale, the original milestone breakdown, and the reasoning behind each
scoping decision live in [`PLAN.md`](PLAN.md).
