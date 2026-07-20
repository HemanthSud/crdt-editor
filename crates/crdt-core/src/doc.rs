use std::collections::{BTreeSet, HashMap};

use crate::op::{Op, OpId, SiteId};

#[derive(Debug, Clone)]
struct Node {
    id: OpId,
    value: char,
    origin_left: Option<OpId>,
    deleted_by: BTreeSet<OpId>,
}

impl Node {
    fn visible(&self) -> bool {
        self.deleted_by.is_empty()
    }
}

/// One entry in a client's local undo/redo stack: the op that was applied, and the
/// op that would reverse it (constructed lazily since it depends on the current
/// document state, e.g. for redoing an insert we need a fresh Restore, not the
/// original Delete).
#[derive(Debug, Clone)]
enum UndoEntry {
    /// This insert created `id`; undoing it deletes that node.
    Insert { id: OpId },
    /// This delete op `delete_id` tombstoned `target`; undoing it restores the node.
    Delete { delete_id: OpId, target: OpId },
}

/// A single-site replica of an RGA (Replicated Growable Array) text document.
///
/// Convergence property: any two replicas that have applied the same set of ops
/// (in any order, causal dependencies permitting) render identical text and
/// identical tombstone state. See the proptest suite in `tests/convergence.rs`.
pub struct RgaDoc {
    site: SiteId,
    counter: u64,
    nodes: Vec<Node>,
    index_by_id: HashMap<OpId, usize>,
    vector_clock: HashMap<SiteId, u64>,
    /// Ops whose dependencies haven't arrived yet, keyed by the missing dependency id.
    pending: HashMap<OpId, Vec<Op>>,
    applied: BTreeSet<OpId>,
    op_log: Vec<Op>,
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<UndoEntry>,
}

impl RgaDoc {
    pub fn new(site: SiteId) -> Self {
        Self {
            site,
            counter: 0,
            nodes: Vec::new(),
            index_by_id: HashMap::new(),
            vector_clock: HashMap::new(),
            pending: HashMap::new(),
            applied: BTreeSet::new(),
            op_log: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    pub fn site(&self) -> SiteId {
        self.site
    }

    fn next_id(&mut self) -> OpId {
        self.counter += 1;
        OpId::new(self.site, self.counter)
    }

    // ---- rendering / queries ----

    pub fn render_text(&self) -> String {
        self.nodes
            .iter()
            .filter(|n| n.visible())
            .map(|n| n.value)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.visible()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn index_of(&self, id: OpId) -> Option<usize> {
        self.index_by_id.get(&id).copied()
    }

    /// Map a visible-text `pos` (0..=len) to the origin_left OpId to insert after.
    fn origin_left_for_position(&self, pos: usize) -> Option<OpId> {
        let mut seen = 0usize;
        let mut last_visible: Option<OpId> = None;
        if pos == 0 {
            return None;
        }
        for n in &self.nodes {
            if n.visible() {
                seen += 1;
                last_visible = Some(n.id);
                if seen == pos {
                    return last_visible;
                }
            }
        }
        last_visible
    }

    /// Map a visible-text `pos` (0..len) to the node id currently occupying it.
    fn visible_node_id_at(&self, pos: usize) -> Option<OpId> {
        self.nodes.iter().filter(|n| n.visible()).nth(pos).map(|n| n.id)
    }

    // ---- local edits (produce an Op, apply it locally, caller broadcasts it) ----

    pub fn local_insert(&mut self, pos: usize, value: char) -> Op {
        let origin_left = self.origin_left_for_position(pos);
        let id = self.next_id();
        let op = Op::Insert { id, value, origin_left };
        self.apply_local(op.clone());
        self.undo_stack.push(UndoEntry::Insert { id });
        self.redo_stack.clear();
        op
    }

    pub fn local_delete(&mut self, pos: usize) -> Option<Op> {
        let target = self.visible_node_id_at(pos)?;
        let id = self.next_id();
        let op = Op::Delete { id, target };
        self.apply_local(op.clone());
        self.undo_stack.push(UndoEntry::Delete { delete_id: id, target });
        self.redo_stack.clear();
        Some(op)
    }

    /// Undo this site's most recent not-yet-undone local edit. Returns the
    /// compensating op (already applied locally) so the caller can broadcast it,
    /// or None if there's nothing left to undo.
    pub fn undo(&mut self) -> Option<Op> {
        let entry = self.undo_stack.pop()?;
        let op = match &entry {
            UndoEntry::Insert { id } => {
                let del_id = self.next_id();
                Op::Delete { id: del_id, target: *id }
            }
            UndoEntry::Delete { delete_id, target } => {
                let restore_id = self.next_id();
                Op::Restore {
                    id: restore_id,
                    target: *target,
                    removes: *delete_id,
                }
            }
        };
        self.apply_local(op.clone());
        self.redo_stack.push(entry);
        Some(op)
    }

    pub fn redo(&mut self) -> Option<Op> {
        let entry = self.redo_stack.pop()?;
        let op = match &entry {
            UndoEntry::Insert { id } => {
                // Redo an insert: node id already exists (it was only tombstoned by
                // undo, never removed) - restore it by removing our own undo-delete.
                // We don't track which delete id undo used per entry here, so instead
                // we re-derive it: the most recent Delete op in the log targeting `id`
                // that came from this site is the undo op to reverse.
                let undo_delete = self
                    .op_log
                    .iter()
                    .rev()
                    .find_map(|op| match op {
                        Op::Delete { id: did, target } if *target == *id && did.site == self.site => {
                            Some(*did)
                        }
                        _ => None,
                    })
                    .expect("redo of insert requires a prior undo-delete in the log");
                let restore_id = self.next_id();
                Op::Restore {
                    id: restore_id,
                    target: *id,
                    removes: undo_delete,
                }
            }
            UndoEntry::Delete { delete_id, target } => {
                let del_id = self.next_id();
                let _ = delete_id;
                Op::Delete { id: del_id, target: *target }
            }
        };
        self.apply_local(op.clone());
        self.undo_stack.push(entry);
        Some(op)
    }

    fn apply_local(&mut self, op: Op) {
        // Local ops are always causally ready (we're the site that created them).
        self.commit(op);
    }

    // ---- remote / causal application ----

    pub fn apply_remote_ops(&mut self, ops: Vec<Op>) {
        for op in ops {
            self.try_apply(op);
        }
    }

    fn try_apply(&mut self, op: Op) {
        if self.applied.contains(&op.id()) {
            return; // idempotent: duplicate delivery is a no-op
        }
        let missing: Vec<OpId> = op
            .dependencies()
            .into_iter()
            .filter(|dep| !self.has(*dep))
            .collect();
        if let Some(&first_missing) = missing.first() {
            self.pending.entry(first_missing).or_default().push(op);
            return;
        }
        self.commit(op);
    }

    /// Whether `id` refers to a node or op already present in this replica
    /// (i.e. a dependency on it is satisfied).
    fn has(&self, id: OpId) -> bool {
        self.applied.contains(&id)
    }

    fn commit(&mut self, op: Op) {
        let id = op.id();
        match &op {
            Op::Insert { id, value, origin_left } => {
                self.insert_node(*id, *value, *origin_left);
            }
            Op::Delete { target, .. } => {
                if let Some(&idx) = self.index_by_id.get(target) {
                    self.nodes[idx].deleted_by.insert(id);
                }
            }
            Op::Restore { target, removes, .. } => {
                if let Some(&idx) = self.index_by_id.get(target) {
                    self.nodes[idx].deleted_by.remove(removes);
                }
            }
        }
        self.applied.insert(id);
        self.observe(id);
        self.op_log.push(op);
        self.drain_pending(id);
    }

    fn drain_pending(&mut self, resolved: OpId) {
        if let Some(waiting) = self.pending.remove(&resolved) {
            for op in waiting {
                self.try_apply(op);
            }
        }
    }

    /// Current index of `origin`, or -1 to represent "start of document" (None).
    fn origin_pos(&self, origin: Option<OpId>) -> isize {
        match origin {
            None => -1,
            Some(oid) => self.index_of(oid).map(|i| i as isize).unwrap_or(-1),
        }
    }

    fn insert_node(&mut self, id: OpId, value: char, origin_left: Option<OpId>) {
        let l_pos = self.origin_pos(origin_left);
        let mut idx = (l_pos + 1) as usize;
        // Classic RGA integrate: skip past any node R whose origin sits at or after
        // our own origin (R is a concurrent sibling or a deeper concurrent
        // insertion anchored past our position); stop once R's origin is strictly
        // before ours, or R is a direct sibling that loses the OpId tie-break.
        // Comparing origin *positions* (not just direct equality) is what makes
        // this correct transitively - a naive "same origin_left only" check is the
        // bug the convergence proptest caught here.
        while idx < self.nodes.len() {
            let r_pos = self.origin_pos(self.nodes[idx].origin_left);
            if r_pos < l_pos {
                break;
            }
            if r_pos == l_pos && self.nodes[idx].id < id {
                break;
            }
            idx += 1;
        }
        self.nodes.insert(
            idx,
            Node {
                id,
                value,
                origin_left,
                deleted_by: BTreeSet::new(),
            },
        );
        self.reindex_from(idx);
    }

    fn reindex_from(&mut self, from: usize) {
        for i in from..self.nodes.len() {
            self.index_by_id.insert(self.nodes[i].id, i);
        }
    }

    fn observe(&mut self, id: OpId) {
        let entry = self.vector_clock.entry(id.site).or_insert(0);
        if id.counter > *entry {
            *entry = id.counter;
        }
    }

    // ---- sync ----

    pub fn state_vector(&self) -> HashMap<SiteId, u64> {
        self.vector_clock.clone()
    }

    /// All applied ops with a counter greater than the given vector clock, per site.
    /// Used both to catch a reconnecting client up and to seed a fresh client.
    pub fn ops_since(&self, since: &HashMap<SiteId, u64>) -> Vec<Op> {
        self.op_log
            .iter()
            .filter(|op| {
                let id = op.id();
                id.counter > *since.get(&id.site).unwrap_or(&0)
            })
            .cloned()
            .collect()
    }

    /// Full structural state (id, value, visible) in document order - used by tests
    /// to assert exact convergence across replicas, not just equal rendered text
    /// (which could mask a bug that happens to still render the same string).
    pub fn debug_snapshot(&self) -> Vec<(OpId, char, bool)> {
        self.nodes.iter().map(|n| (n.id, n.value, n.visible())).collect()
    }
}
