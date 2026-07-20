//! Convergence property tests: the whole point of this project is that replicas
//! never corrupt each other's state. These tests simulate N sites editing
//! concurrently (including out-of-order and duplicate delivery, which a real
//! network / offline-reconnect path will actually produce) and assert every
//! replica ends up in an identical state.

use crdt_core::{Op, RgaDoc};
use proptest::prelude::*;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::Rng;
use rand::SeedableRng;

const N_SITES: u64 = 3;

#[derive(Debug, Clone)]
enum Action {
    Insert { site: u64, pos_frac: f32, ch: char },
    Delete { site: u64, pos_frac: f32 },
}

fn arb_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        (0..N_SITES, 0.0f32..1.0, "[a-zA-Z]").prop_map(|(site, pos_frac, s)| Action::Insert {
            site,
            pos_frac,
            ch: s.chars().next().unwrap_or('x'),
        }),
        (0..N_SITES, 0.0f32..1.0).prop_map(|(site, pos_frac)| Action::Delete { site, pos_frac }),
    ]
}

fn frac_to_pos(frac: f32, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        ((frac * (len as f32 + 1.0)) as usize).min(len)
    }
}

/// Run `actions` against `N_SITES` local replicas (each action's local position is
/// resolved against *that site's own* current view, simulating concurrent editors
/// who each see their own document), collect every op produced, then deliver the
/// full op set to every replica in a shuffled (and duplicated) order and assert
/// they all converge to the same state.
fn run_and_check_convergence(actions: &[Action], seed: u64) {
    let mut replicas: Vec<RgaDoc> = (0..N_SITES).map(RgaDoc::new).collect();
    let mut all_ops: Vec<Op> = Vec::new();

    for action in actions {
        match action {
            Action::Insert { site, pos_frac, ch } => {
                let r = &mut replicas[*site as usize];
                let pos = frac_to_pos(*pos_frac, r.len());
                all_ops.push(r.local_insert(pos, *ch));
            }
            Action::Delete { site, pos_frac } => {
                let r = &mut replicas[*site as usize];
                if r.len() > 0 {
                    let pos = frac_to_pos(*pos_frac, r.len()).min(r.len() - 1);
                    if let Some(op) = r.local_delete(pos) {
                        all_ops.push(op);
                    }
                }
            }
        }
    }

    let mut rng = StdRng::seed_from_u64(seed);

    // Deliver the full op log to every replica, in an independently shuffled,
    // partially-duplicated order per replica - this exercises the causal buffer
    // (an Insert can easily arrive after a Delete that targets it) and idempotent
    // re-application.
    let mut final_states = Vec::new();
    for site in 0..N_SITES {
        let mut delivery = all_ops.clone();
        delivery.shuffle(&mut rng);
        // Duplicate roughly a third of the ops to simulate at-least-once delivery.
        let dupes: Vec<Op> = delivery
            .iter()
            .filter(|_| rng.gen_bool(0.3))
            .cloned()
            .collect();
        delivery.extend(dupes);
        delivery.shuffle(&mut rng);

        // Fresh replica per site, applying only via the remote-op / causal-buffer
        // path so this test genuinely exercises reordering + idempotency rather
        // than reusing the replica that produced the ops locally (which already
        // has them applied in causal order).
        let mut r = RgaDoc::new(site);
        r.apply_remote_ops(delivery);
        final_states.push((r.render_text(), r.debug_snapshot()));
    }

    let (first_text, first_snapshot) = &final_states[0];
    for (text, snapshot) in &final_states[1..] {
        prop_assert_eq_states(first_text, text, first_snapshot, snapshot);
    }
}

fn prop_assert_eq_states(
    text_a: &str,
    text_b: &str,
    snap_a: &[(crdt_core::OpId, char, bool)],
    snap_b: &[(crdt_core::OpId, char, bool)],
) {
    assert_eq!(text_a, text_b, "rendered text diverged between replicas");
    assert_eq!(snap_a, snap_b, "structural state (including tombstones) diverged between replicas");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn replicas_converge_under_random_concurrent_edits(
        actions in prop::collection::vec(arb_action(), 1..40),
        seed in any::<u64>(),
    ) {
        run_and_check_convergence(&actions, seed);
    }
}

/// Demonstrates RGA's known interleaving anomaly (documented limitation, not a
/// bug): two sites concurrently typing different words at the same position can
/// have their characters interleave in the merged result instead of appearing as
/// clean runs. This is expected of plain RGA - the fix (Fugue's subtree-based
/// tie-breaking) is out of scope for this project and noted in the README.
/// The important property this test *does* assert is convergence: however the
/// characters interleave, every replica agrees on the same interleaving.
#[test]
fn documents_rga_interleaving_anomaly() {
    let mut a = RgaDoc::new(1);
    let mut b = RgaDoc::new(2);

    // Both sites start from the same empty document and concurrently insert at
    // position 0, one character at a time, without seeing each other's ops yet.
    let word_a = "AAA";
    let word_b = "BBB";
    let mut ops = Vec::new();
    for ch in word_a.chars() {
        ops.push(a.local_insert(0, ch));
    }
    for ch in word_b.chars() {
        ops.push(b.local_insert(0, ch));
    }

    let mut replica1 = RgaDoc::new(99);
    let mut replica2 = RgaDoc::new(100);
    replica1.apply_remote_ops(ops.clone());
    let mut reversed = ops;
    reversed.reverse();
    replica2.apply_remote_ops(reversed);

    // Convergence still holds regardless of delivery order...
    assert_eq!(replica1.render_text(), replica2.render_text());
    // ...but the result is not simply "AAABBB" or "BBBAAA" - characters from the
    // two concurrent inserts interleave under tie-breaking by OpId. This is the
    // anomaly: a human would expect two clean words, not an interleaved mix.
    let text = replica1.render_text();
    assert_eq!(text.len(), 6);
    assert!(text.contains('A') && text.contains('B'));
}
