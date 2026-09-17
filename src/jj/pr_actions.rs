use super::*;

/// Exact local object identity for a PR head; a change ID is offered only when unambiguous.
pub struct PrActionRevision {
    pub commit_id: String,
    pub change_id: Option<String>,
}

impl JjWorkspace {
    /// Looks up an exact GitHub head without fetching, snapshotting, or falling back to the worktree.
    /// Rewritten/divergent change IDs are omitted rather than pointing actions at another commit.
    pub fn pr_action_revision(&self, head_oid: &str) -> Option<PrActionRevision> {
        if head_oid.len() != 40 || !head_oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let id = CommitId::try_from_hex(head_oid)?;
        let commit = self.load_commit(&id).ok()?;
        let change_id = commit.change_id().reverse_hex();
        let change_id = self
            .resolve_single_revision(&change_id, "jx PR action")
            .ok()
            .filter(|resolved| resolved.id() == commit.id())
            .map(|_| change_id);
        Some(PrActionRevision {
            commit_id: commit.id().hex(),
            change_id,
        })
    }
}
