use super::*;

pub(in crate::commands) fn fork_sync_confirmation_prompt(plan: &ForkSyncPlan) -> String {
    let push = if plan.push { " and push" } else { "" };
    match &plan.branch_plan.operation {
        ForkSyncBranchOperation::AlreadySynced if plan.branch_plan.push_needed && plan.push => {
            format!("Push fork branch `{}` to origin?", plan.branch)
        }
        ForkSyncBranchOperation::AlreadySynced => {
            format!("Verify fork branch `{}`?", plan.branch)
        }
        ForkSyncBranchOperation::FastForward => format!(
            "Fast-forward fork branch `{}` to `{}`{push}?",
            plan.branch,
            source_branch_ref(plan)
        ),
        ForkSyncBranchOperation::Rebase { .. } => format!(
            "Rebase fork branch `{}` onto `{}`{push}?",
            plan.branch,
            source_branch_ref(plan)
        ),
    }
}

pub(in crate::commands) fn render_fork_sync(report: &ForkSyncReport) -> String {
    render_plain_output(|formatter| write_fork_sync(formatter, report))
}

fn write_fork_sync(formatter: &mut dyn Formatter, report: &ForkSyncReport) -> io::Result<()> {
    writeln!(
        formatter,
        "Fork sync: {} <- {}",
        fork_branch_label(&report.plan),
        source_branch_label(&report.plan)
    )?;
    write_upstream_remote(formatter, &report.upstream)?;
    write_fork_sync_outcome(formatter, &report.outcome)?;
    write_fork_sync_push(formatter, report)?;
    Ok(())
}

fn write_upstream_remote(
    formatter: &mut dyn Formatter,
    upstream: &GitRemoteUpdate,
) -> io::Result<()> {
    match &upstream.action {
        GitRemoteUpdateAction::AlreadyConfigured => {
            writeln!(
                formatter,
                "Upstream remote: {} -> {}",
                upstream.remote, upstream.url
            )
        }
        GitRemoteUpdateAction::Added => writeln!(
            formatter,
            "Upstream remote: added {} -> {}",
            upstream.remote, upstream.url
        ),
        GitRemoteUpdateAction::Updated { old_url } => writeln!(
            formatter,
            "Upstream remote: updated {} from {} to {}",
            upstream.remote, old_url, upstream.url
        ),
    }
}

fn write_fork_sync_outcome(
    formatter: &mut dyn Formatter,
    outcome: &ForkSyncBranchOutcome,
) -> io::Result<()> {
    match &outcome.operation {
        ForkSyncBranchOutcomeKind::AlreadySynced => writeln!(
            formatter,
            "Local branch: {} already matches {}",
            outcome.branch,
            upstream_branch_ref(outcome)
        )?,
        ForkSyncBranchOutcomeKind::FastForward => writeln!(
            formatter,
            "Fast-forwarded {}: {} -> {}",
            outcome.branch, outcome.old_short_commit_id, outcome.new_short_commit_id
        )?,
        ForkSyncBranchOutcomeKind::Rebased {
            root_short_change_id,
            commit_count,
        } => writeln!(
            formatter,
            "Rebased {} from {}: {} onto {}",
            commit_count_i64(*commit_count as i64),
            root_short_change_id,
            outcome.branch,
            upstream_branch_ref(outcome)
        )?,
    }

    if outcome.current_updated {
        writeln!(formatter, "Updated current workspace")?;
    }
    write_rebased_commit_details(formatter, outcome)
}

fn write_rebased_commit_details(
    formatter: &mut dyn Formatter,
    outcome: &ForkSyncBranchOutcome,
) -> io::Result<()> {
    let commits = outcome
        .rebased_commits
        .iter()
        .filter(|commit| !is_uninformative_rebased_commit(commit))
        .collect::<Vec<_>>();
    if commits.is_empty() {
        return Ok(());
    }

    writeln!(formatter)?;
    writeln!(formatter, "Rebased locally:")?;
    for commit in commits {
        let suffix = if commit.has_conflict {
            " [conflict]"
        } else {
            ""
        };
        writeln!(
            formatter,
            "  {:<8}  {}{}",
            commit.short_change_id,
            first_description_line(&commit.description),
            suffix
        )?;
    }
    Ok(())
}

fn write_fork_sync_push(formatter: &mut dyn Formatter, report: &ForkSyncReport) -> io::Result<()> {
    if !report.plan.push {
        return writeln!(formatter, "Push disabled (--no-push)");
    }
    if fork_sync_outcome_has_conflicts(&report.outcome) {
        return writeln!(formatter, "Push skipped: rebased commits have conflicts");
    }

    match &report.push {
        Some(push) if push.pushed_refs > 0 => {
            writeln!(formatter, "Pushed {} to origin", push.branch)
        }
        Some(push) => writeln!(formatter, "Origin already had {}", push.branch),
        None => Ok(()),
    }
}

pub(in crate::commands) fn fork_sync_plan_needs_confirmation(plan: &ForkSyncPlan) -> bool {
    !matches!(
        plan.branch_plan.operation,
        ForkSyncBranchOperation::AlreadySynced
    ) || (plan.push && plan.branch_plan.push_needed)
}

pub(in crate::commands) fn fork_sync_outcome_has_conflicts(
    outcome: &ForkSyncBranchOutcome,
) -> bool {
    outcome
        .rebased_commits
        .iter()
        .any(|commit| commit.has_conflict)
}

fn fork_branch_label(plan: &ForkSyncPlan) -> String {
    format!("{}/{}", plan.repository.github_slug, plan.branch)
}

fn source_branch_label(plan: &ForkSyncPlan) -> String {
    format!("{}/{}", plan.source.slug(), plan.source_branch)
}

fn source_branch_ref(plan: &ForkSyncPlan) -> String {
    format!("{}@{}", plan.source_branch, plan.upstream_remote)
}

fn upstream_branch_ref(outcome: &ForkSyncBranchOutcome) -> String {
    format!("{}@{}", outcome.upstream_branch, outcome.upstream_remote)
}

fn first_description_line(description: &str) -> &str {
    description.lines().next().unwrap_or("(no description)")
}
