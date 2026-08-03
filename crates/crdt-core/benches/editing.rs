//! Where does the time actually go?
//!
//! The headline result is that the CRDT algorithm is *not* the expensive part.
//! Applying a remote op stays near-constant (~1.7us even at 50k nodes) because
//! the op names its own `origin_left`, so integration jumps straight to the right
//! place. Everything slow is bookkeeping around it:
//!
//! - `origin_left_for_position` / `visible_node_id_at` walk the node list to turn
//!   an editor cursor position back into a node id - O(n) per local edit.
//! - `reindex_from` rebuilds the `index_by_id` map for every node *after* the
//!   insertion point. Appending touches one entry; inserting at position 0 in a
//!   50k-node document rebuilds all 50k, which is why editing at the top of a
//!   large document is ~25x slower than editing at the bottom.
//!
//! Both are fixed by the same thing - an order-statistic index (rope / balanced
//! tree) mapping position <-> node - which is why that's the logical next
//! optimization rather than anything to do with RGA itself.
//!
//! Run with `cargo bench -p crdt-core`. Numbers in README.md are from an M1 Pro.

use std::time::Duration;

use crdt_core::{Op, RgaDoc};
use criterion::{
    criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput,
};
use std::hint::black_box;

const SIZES: [usize; 3] = [1_000, 10_000, 50_000];

/// Ops that build an `n`-character document by appending, and a replica holding it.
/// Built via the remote path, which is the linear one - constructing a 50k-char
/// document by simulated typing would take ~1s on its own.
fn document(n: usize) -> (RgaDoc, Vec<Op>) {
    let mut src = RgaDoc::new(1);
    let ops: Vec<Op> = (0..n).map(|i| src.local_insert(i, 'a')).collect();
    let mut doc = RgaDoc::new(2);
    doc.apply_remote_ops(ops.clone());
    (doc, ops)
}

/// Clone `doc` and force its internal `Vec<Node>` to grow *before* measurement.
///
/// `Vec::clone` allocates exact capacity, so without this the first insert in
/// every measured iteration pays a realloc plus a memcpy of the entire node
/// list. At 50k nodes that costs more than the operation under test, and would
/// be reported as if it were the cost of editing. Real documents grow amortized
/// and pay it only logarithmically often.
fn warmed(doc: &RgaDoc, n: usize) -> RgaDoc {
    let mut d = doc.clone();
    d.local_insert(n, 'w'); // triggers the one-time realloc, untimed
    d
}

/// Latency of one keystroke in a document that already holds `n` characters.
/// This is the number that decides whether the editor feels responsive: it needs
/// to stay far below a 16ms frame.
fn keystroke_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("keystroke_latency");
    group.sample_size(30).measurement_time(Duration::from_secs(6));

    for n in SIZES {
        let (doc, _) = document(n);

        // Appending at the end is the common case, and the worst one for the
        // position scan: it walks the entire node list to find the last node.
        group.bench_with_input(BenchmarkId::new("insert_at_end", n), &n, |b, &n| {
            b.iter_batched_ref(
                || warmed(&doc, n),
                |d| black_box(d.local_insert(n, 'x')),
                BatchSize::LargeInput,
            )
        });

        // Inserting at position 0 returns immediately from the position scan, so
        // this isolates the remaining costs: the Vec::insert memmove and the
        // reindex of every node after the insertion point.
        group.bench_with_input(BenchmarkId::new("insert_at_start", n), &n, |b, &n| {
            b.iter_batched_ref(
                || warmed(&doc, n),
                |d| black_box(d.local_insert(0, 'x')),
                BatchSize::LargeInput,
            )
        });

        group.bench_with_input(BenchmarkId::new("delete_at_middle", n), &n, |b, &n| {
            b.iter_batched_ref(
                || warmed(&doc, n),
                |d| black_box(d.local_delete(n / 2)),
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

/// Applying an op that arrived over the network. No position translation is
/// needed - the op names its own `origin_left` - so this is the fast path.
fn remote_op(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_remote_op");
    group.sample_size(30).measurement_time(Duration::from_secs(6));

    for n in SIZES {
        let (doc, _) = document(n);
        // One more append, produced by a different site, to apply on top.
        let mut peer = RgaDoc::new(3);
        peer.apply_remote_ops({
            let (_, ops) = document(n);
            ops
        });
        let incoming = peer.local_insert(n, 'z');

        group.bench_with_input(BenchmarkId::new("append", n), &n, |b, &n| {
            b.iter_batched_ref(
                || warmed(&doc, n),
                |d| d.apply_remote_ops(vec![incoming.clone()]),
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

/// Cold-loading a document from its op log - what the server does on startup and
/// what every client does on connect (the `welcome` message).
fn load_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("load_document");
    group.sample_size(20).measurement_time(Duration::from_secs(6));

    for n in SIZES {
        let (_, ops) = document(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter_batched(
                || ops.clone(),
                |ops| {
                    let mut d = RgaDoc::new(9);
                    d.apply_remote_ops(ops);
                    black_box(d.len())
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

/// The client calls this after *every* op to refresh the editor, so its cost is
/// paid on each keystroke on top of the edit itself.
fn render(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_text");
    group.sample_size(50);

    for n in SIZES {
        let (doc, _) = document(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| black_box(doc.render_text()))
        });
    }
    group.finish();
}

criterion_group!(benches, keystroke_latency, remote_op, load_document, render);
criterion_main!(benches);
