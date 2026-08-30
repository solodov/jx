use super::*;

/// Data needed to build an operator-facing fork sync plan.
pub struct ForkSyncPlanInput {
    pub source: GitHubRepository,
    pub branch: String,
    pub source_branch: String,
    pub upstream_remote: String,
    pub upstream_url: String,
    pub push: bool,
    pub branch_plan: ForkSyncBranchPlan,
}

/// Builds the operator-facing fork sync plan after local jj facts are loaded.
pub fn fork_sync_plan(context: &RepositoryContext, input: ForkSyncPlanInput) -> ForkSyncPlan {
    ForkSyncPlan {
        repository: repository_summary(context),
        source: input.source,
        branch: input.branch,
        source_branch: input.source_branch,
        upstream_remote: input.upstream_remote,
        upstream_url: input.upstream_url,
        push: input.push,
        branch_plan: input.branch_plan,
    }
}

/// Builds the operator-facing fork sync result.
pub fn fork_sync_report(
    plan: ForkSyncPlan,
    upstream: GitRemoteUpdate,
    outcome: ForkSyncBranchOutcome,
    push: Option<PushOutcome>,
) -> ForkSyncReport {
    ForkSyncReport {
        plan,
        upstream,
        outcome,
        push,
    }
}
