//! Targeted tests for the set-based tombstone undo/redo scheme: the property
//! that makes it correct under concurrency is that `deleted_by` is a *set*, so
//! undoing your own delete only removes your own entry - if someone else
//! independently deleted the same node, it correctly stays hidden.

use crdt_core::RgaDoc;
use proptest::prelude::*;

#[test]
fn undo_insert_then_redo() {
    let mut a = RgaDoc::new(1);
    a.local_insert(0, 'h');
    a.local_insert(1, 'i');
    assert_eq!(a.render_text(), "hi");

    let undo_op = a.undo().expect("undo of insert");
    assert_eq!(a.render_text(), "h");

    let redo_op = a.redo().expect("redo of undo");
    assert_eq!(a.render_text(), "hi");
    let _ = (undo_op, redo_op);
}

#[test]
fn delete_undo_redo_undo_returns_to_original() {
    // Regression: redo of a delete must issue a *fresh* delete op and record that
    // new op id on the undo stack. If it re-pushes the original delete's id, the
    // next undo emits Restore{removes: <stale id>}, which doesn't clear the
    // tombstone actually in the set - and undo silently does nothing.
    let mut a = RgaDoc::new(1);
    a.local_insert(0, 'h');
    a.local_insert(1, 'i');
    assert_eq!(a.render_text(), "hi");

    a.local_delete(0).expect("delete 'h'");
    assert_eq!(a.render_text(), "i");

    a.undo().expect("undo the delete");
    assert_eq!(a.render_text(), "hi");

    a.redo().expect("redo the delete");
    assert_eq!(a.render_text(), "i");

    a.undo().expect("undo the redone delete");
    assert_eq!(
        a.render_text(),
        "hi",
        "undo after redo must clear the tombstone the redo actually created"
    );
}

#[test]
fn undo_of_delete_survives_concurrent_independent_delete() {
    // Site 1 types "hi", site 2 (a remote replica) will independently delete the
    // same character that site 1 also deletes-then-undoes. The set-based
    // tombstone must keep the node hidden after site 1's undo, because site 2's
    // delete is still in the set.
    let mut site1 = RgaDoc::new(1);
    let insert_ops = vec![site1.local_insert(0, 'h'), site1.local_insert(1, 'i')];

    // Site 2 starts from the same two inserts.
    let mut site2 = RgaDoc::new(2);
    site2.apply_remote_ops(insert_ops.clone());
    assert_eq!(site2.render_text(), "hi");

    // Site 1 deletes 'h' (position 0), then undoes that delete - locally back to "hi".
    let delete_op_1 = site1.local_delete(0).expect("delete 'h' on site1");
    assert_eq!(site1.render_text(), "i");
    let undo_op_1 = site1.undo().expect("undo the delete");
    assert_eq!(site1.render_text(), "hi");

    // Meanwhile, site 2 *independently* also deletes 'h' (concurrently, before
    // seeing any of site1's ops).
    let delete_op_2 = site2.local_delete(0).expect("delete 'h' on site2");
    assert_eq!(site2.render_text(), "i");

    // Now merge everything into a fresh third replica, in a deliberately
    // "undo arrives before the concurrent independent delete" order.
    let mut merged = RgaDoc::new(3);
    merged.apply_remote_ops(insert_ops);
    merged.apply_remote_ops(vec![delete_op_1, undo_op_1]);
    assert_eq!(merged.render_text(), "hi", "site1's own undo should restore it locally");
    merged.apply_remote_ops(vec![delete_op_2]);

    // Because 'h's deleted_by set still contains site2's independent delete, the
    // node must stay hidden - undoing your own delete does not resurrect content
    // someone else validly deleted too.
    assert_eq!(
        merged.render_text(),
        "i",
        "concurrent independent delete must still hide the node after the other site's undo"
    );
}

#[test]
fn redo_after_intervening_remote_insert_targets_by_id_not_position() {
    // Undo/redo operate on op ids, not text positions, so an intervening remote
    // edit at an earlier position must not corrupt what gets undone/redone.
    let mut site1 = RgaDoc::new(1);
    site1.local_insert(0, 'a');
    site1.local_insert(1, 'b');
    site1.local_insert(2, 'c'); // "abc"
    let undo_op = site1.undo().unwrap(); // undoes the 'c' insert -> "ab"
    assert_eq!(site1.render_text(), "ab");

    // A remote site inserts at position 0, shifting everything right.
    let mut remote = RgaDoc::new(2);
    let x_op = remote.local_insert(0, 'X');
    site1.apply_remote_ops(vec![x_op]);
    assert_eq!(site1.render_text(), "Xab");

    let redo_op = site1.redo().unwrap(); // should restore 'c' -> "Xabc", not corrupt 'X'/'a'/'b'
    assert_eq!(site1.render_text(), "Xabc");
    let _ = (undo_op, redo_op);
}

// ---------------------------------------------------------------------------
// Property: undo/redo is a lossless, *repeatable* round trip.
//
// Note this is deliberately a single-replica property, and it is NOT covered by
// the convergence harness in tests/convergence.rs. Convergence only asserts that
// every replica agrees; a bug where undo emits a compensating op that targets the
// wrong tombstone still converges perfectly - every replica applies the same op
// stream and lands in the same (semantically wrong) state. That class of bug is
// an *intent* violation, not a convergence violation, so it needs its own
// property. `delete_undo_redo_undo_returns_to_original` above is the concrete
// instance of exactly this bug; this generalizes it.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Edit {
    Insert { pos_frac: f32, ch: char },
    Delete { pos_frac: f32 },
}

fn arb_edit() -> impl Strategy<Value = Edit> {
    prop_oneof![
        2 => (0.0f32..1.0, "[a-z]").prop_map(|(pos_frac, s)| Edit::Insert {
            pos_frac,
            ch: s.chars().next().unwrap_or('x'),
        }),
        1 => (0.0f32..1.0).prop_map(|pos_frac| Edit::Delete { pos_frac }),
    ]
}

fn apply_edits(doc: &mut RgaDoc, edits: &[Edit]) {
    for edit in edits {
        match edit {
            Edit::Insert { pos_frac, ch } => {
                let len = doc.len();
                let pos = ((pos_frac * (len as f32 + 1.0)) as usize).min(len);
                doc.local_insert(pos, *ch);
            }
            Edit::Delete { pos_frac } => {
                let len = doc.len();
                if len > 0 {
                    let pos = ((pos_frac * len as f32) as usize).min(len - 1);
                    doc.local_delete(pos);
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// At *every* depth of the undo stack, redo-then-undo must be an identity.
    ///
    /// The depth matters: an "undo everything, redo everything, undo everything"
    /// formulation looks stronger but is actually masked, because undoing each
    /// insert tombstones its node anyway, so both undo passes bottom out at the
    /// same empty document no matter what the delete entries did. Stepping one
    /// entry at a time and dancing in place is what exposes a redo that recorded
    /// the wrong op id.
    #[test]
    fn redo_then_undo_is_identity_at_every_depth(
        edits in prop::collection::vec(arb_edit(), 1..30),
    ) {
        let mut doc = RgaDoc::new(1);
        apply_edits(&mut doc, &edits);

        let mut before_undo = doc.debug_snapshot();
        loop {
            if doc.undo().is_none() {
                break;
            }
            let after_undo = doc.debug_snapshot();

            doc.redo().expect("redo must be available immediately after an undo");
            prop_assert_eq!(
                doc.debug_snapshot(), before_undo,
                "redo must return exactly to the pre-undo state"
            );

            doc.undo().expect("undo must be available immediately after a redo");
            prop_assert_eq!(
                doc.debug_snapshot(), after_undo.clone(),
                "undoing again after a redo must reach the same state as the first undo"
            );

            before_undo = after_undo;
        }
    }
}
