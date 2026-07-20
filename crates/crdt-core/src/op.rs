use serde::{Deserialize, Serialize};

pub type SiteId = u64;

/// Globally unique identifier for an operation (and, for `Insert`, the node it creates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OpId {
    pub site: SiteId,
    pub counter: u64,
}

impl OpId {
    pub const fn new(site: SiteId, counter: u64) -> Self {
        Self { site, counter }
    }
}

/// A single CRDT operation. `id` is always this operation's own identity, used both
/// for vector-clock bookkeeping and (for `Delete`/`Restore`) for undo targeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Insert `value` immediately after the node `origin_left` (None = start of document).
    Insert {
        id: OpId,
        value: char,
        origin_left: Option<OpId>,
    },
    /// Tombstone the node `target` by adding this op's `id` to its `deleted_by` set.
    Delete { id: OpId, target: OpId },
    /// Undo a prior delete: remove `removes` (a `Delete` op's id) from `target`'s `deleted_by` set.
    Restore {
        id: OpId,
        target: OpId,
        removes: OpId,
    },
}

impl Op {
    pub fn id(&self) -> OpId {
        match self {
            Op::Insert { id, .. } => *id,
            Op::Delete { id, .. } => *id,
            Op::Restore { id, .. } => *id,
        }
    }

    /// The set of node/op ids that must already be present in a replica before this
    /// op can be safely applied. Used by the causal buffer to hold ops that arrive
    /// out of order until their dependencies land.
    pub fn dependencies(&self) -> Vec<OpId> {
        match self {
            Op::Insert { origin_left, .. } => origin_left.iter().copied().collect(),
            Op::Delete { target, .. } => vec![*target],
            Op::Restore { target, removes, .. } => vec![*target, *removes],
        }
    }
}
