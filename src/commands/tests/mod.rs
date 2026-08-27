use super::*;
use crate::{
    domain::{
        BookmarkAction, BookmarkPlan, CheckWorkspaceSummary, ForkStatusComparison, GitHubReadiness,
        PullRequestAction, RepositorySummary, StatusComparison, StatusState,
    },
    github::{
        AuthenticatedUser, GitHubError, LabelApplyResult, PullRequestAutoMergeStatus,
        PullRequestCheck, PullRequestCheckStatus, PullRequestHead, PullRequestLabel,
        PullRequestMergeStatus, PullRequestRecord, PullRequestReviewActivity,
        PullRequestReviewRequest, PullRequestReviewRequests, PullRequestReviewStatus,
        PullRequestReviewerResponse, PullRequestStatusRecord, PullRequestTimelineEvent,
        PullRequestTimelineEventKind, ReviewerSelection, ReviewerSyncResult,
    },
    jj::{
        ChangeSummary, PushedBookmarkSummary, PushedCommitSummary, RebasedCommitSummary,
        StackPublishMetrics, StatusRemoteFacts, StatusWorkspaceFacts, TrackedPushOutcome,
        TrunkSummary, WorkspaceAddOptions, WorkspaceEntry, WorkspaceRemoveOptions, WorkspaceStatus,
        WorkspaceVisibility,
    },
    repository::{StackMetadataNode, StackMetadataWorkItemHandlerRun},
};
use jj_lib::{
    config::StackedConfig,
    git,
    ref_name::RemoteName,
    repo::StoreFactories,
    settings::UserSettings,
    workspace::{default_working_copy_factories, Workspace},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

mod basic;
mod checks;
mod clone;
mod diff;
mod fetch;
mod open;
mod pull_request;
mod push;
mod remote_status;
mod render;
mod review;
mod shell;
mod stack;
mod sync;
mod work;

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

fn visible_in(workspaces: &[&str], includes_current: bool) -> WorkspaceVisibility {
    WorkspaceVisibility {
        names: workspaces
            .iter()
            .map(|workspace| (*workspace).to_owned())
            .collect(),
        includes_current,
    }
}

fn current_workspace_visibility() -> WorkspaceVisibility {
    visible_in(&["default"], true)
}

fn test_prompt_handlers() -> PromptHandlers<'static> {
    PromptHandlers {
        pull_request_previewer: &NoPullRequestPreview,
        pull_request_selector: &SelectFirstPullRequest,
        reviewer_selector: &SelectAllReviewers,
        pull_request_confirmer: &AlwaysConfirmPullRequest,
        push_confirmer: &AlwaysConfirmPush,
        repository_initialization_confirmer: &AlwaysConfirmRepositoryInitialization,
        repository_creation_confirmer: &AlwaysConfirmRepositoryCreation,
        workspace_remove_confirmer: &AlwaysConfirmWorkspaceRemove,
    }
}

fn example_bookmark_link(bookmark: &str) -> String {
    linked_bookmark_text("https://github.com/example-owner/example-repo", bookmark)
}

fn example_pull_request_link(number: u64) -> String {
    osc8_link(
        &format!("https://github.com/example-owner/example-repo/pull/{number}"),
        &format!("#{number}"),
    )
}

fn fork_status(
    state: ForkStatusState,
    source_ahead_by: i64,
    fork_ahead_by: i64,
) -> ForkStatusReport {
    ForkStatusReport {
        fork: GitHubRepository {
            owner: "example-owner".to_owned(),
            name: "example-repo".to_owned(),
        },
        fork_branch: "main".to_owned(),
        source: GitHubRepository {
            owner: "source-owner".to_owned(),
            name: "example-repo".to_owned(),
        },
        source_branch: "main".to_owned(),
        comparison: ForkStatusComparison {
            state,
            source_ahead_by,
            fork_ahead_by,
        },
    }
}

#[derive(Default)]
struct RecordingProgress {
    messages: std::cell::RefCell<Vec<String>>,
    finished: std::cell::Cell<bool>,
}

impl RecordingProgress {
    fn messages(&self) -> Vec<String> {
        self.messages.borrow().clone()
    }
}

impl ProgressSink for RecordingProgress {
    fn status(&self, message: &str) {
        self.messages.borrow_mut().push(message.to_owned());
    }

    fn finish(&self) {
        self.finished.set(true);
    }
}

struct RecordingPullRequestSelector {
    selected: usize,
    labels: std::cell::RefCell<Vec<Vec<String>>>,
}

impl RecordingPullRequestSelector {
    fn new(selected: usize) -> Self {
        Self {
            selected,
            labels: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl PullRequestSelector for RecordingPullRequestSelector {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        self.labels
            .borrow_mut()
            .push(choices.iter().map(|choice| choice.label.clone()).collect());
        choices
            .get(self.selected)
            .map(|choice| choice.pull_request.clone())
            .ok_or(PullRequestSelectionError::NoPullRequests)
    }
}

struct CancellingPullRequestSelector;

impl PullRequestSelector for CancellingPullRequestSelector {
    fn select_pull_request(
        &self,
        _choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        Err(PullRequestSelectionError::Cancelled)
    }
}

fn pull_request_choice_record(
    number: u64,
    title: &str,
    head_branch: &str,
    base_branch: &str,
    draft: bool,
) -> PullRequestRecord {
    PullRequestRecord {
        number,
        title: title.to_owned(),
        body: None,
        head_branch: head_branch.to_owned(),
        base_branch: base_branch.to_owned(),
        html_url: None,
        draft,
        merged: false,
        reviewers: ReviewerSelection::default(),
    }
}

struct RecordingPullRequestPreviewer {
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}

impl PullRequestPreviewer for RecordingPullRequestPreviewer {
    fn show_preview(
        &self,
        _plan: &PullRequestPlan,
        _status: &WorkspaceStatus,
        _prepare_effects: &[PullRequestEventEffect],
    ) {
        self.events.borrow_mut().push("preview");
    }
}

struct RecordingReviewerSelector {
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    selected: ReviewerSelection,
}

impl ReviewerSelector for RecordingReviewerSelector {
    fn select_reviewers(
        &self,
        _candidates: &[ReviewerCandidate],
        _preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        self.events.borrow_mut().push("reviewers");
        Ok(self.selected.clone())
    }
}

struct FixedReviewerSelector {
    selected: ReviewerSelection,
}

struct CancellingReviewerSelector;

impl ReviewerSelector for CancellingReviewerSelector {
    fn select_reviewers(
        &self,
        _candidates: &[ReviewerCandidate],
        _preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        Err(ReviewerSelectionError::Cancelled)
    }
}

impl ReviewerSelector for FixedReviewerSelector {
    fn select_reviewers(
        &self,
        _candidates: &[ReviewerCandidate],
        _preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        Ok(self.selected.clone())
    }
}

struct CheckedReviewerSelector;

impl ReviewerSelector for CheckedReviewerSelector {
    fn select_reviewers(
        &self,
        candidates: &[ReviewerCandidate],
        preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        let choices = reviewer_choices(candidates, preselected);
        Ok(selection_from_choices(
            choices.iter().filter(|choice| choice.checked),
        ))
    }
}

struct FixedPullRequestConfirmer {
    confirmed: bool,
}

impl PullRequestConfirmer for FixedPullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        _plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        Ok(self.confirmed)
    }
}

struct SequencePullRequestConfirmer {
    confirmations: std::cell::RefCell<Vec<bool>>,
    titles: std::cell::RefCell<Vec<String>>,
}

impl SequencePullRequestConfirmer {
    fn new(confirmations: impl IntoIterator<Item = bool>) -> Self {
        Self {
            confirmations: std::cell::RefCell::new(confirmations.into_iter().collect()),
            titles: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl PullRequestConfirmer for SequencePullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        self.titles.borrow_mut().push(plan.title.clone());
        Ok(self.confirmations.borrow_mut().remove(0))
    }
}

struct FixedPushConfirmer {
    confirmed: bool,
}

impl PushConfirmer for FixedPushConfirmer {
    fn confirm_push(&self, _plan: &PushPlan) -> Result<bool, PushConfirmationError> {
        Ok(self.confirmed)
    }
}

struct FixedWorkspaceRemoveConfirmer {
    confirmed: bool,
}

impl WorkspaceRemoveConfirmer for FixedWorkspaceRemoveConfirmer {
    fn confirm_workspace_remove(
        &self,
        _workspace: &WorkspaceEntry,
        _display_root: &str,
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        Ok(self.confirmed)
    }
}

fn existing_pull_request(draft: bool) -> PullRequestRecord {
    PullRequestRecord {
        number: 7,
        title: "existing PR".to_owned(),
        body: Some("existing body".to_owned()),
        head_branch: "example-user/02-a1b2c3d4".to_owned(),
        base_branch: "main".to_owned(),
        html_url: Some("https://github.com/example-owner/example-repo/pull/7".to_owned()),
        draft,
        merged: false,
        reviewers: ReviewerSelection::default(),
    }
}

fn preview_plan() -> PullRequestPlan {
    PullRequestPlan {
        repository: RepositorySummary {
            origin_name: "origin",
            origin_url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github_slug: "example-owner/example-repo".to_owned(),
            github_url: "https://github.com/example-owner/example-repo".to_owned(),
            token_source: "GH_TOKEN environment variable".to_owned(),
            config: "defaults",
            default_reviewers: "none".to_owned(),
        },
        task_id: None,
        bookmark: BookmarkPlan {
            branch: "example-user/02-zzzzzzzz".to_owned(),
            action: BookmarkAction::Create,
        },
        target_commit_id: "a1b2c3d4e5f6".to_owned(),
        title: "example change".to_owned(),
        body: String::new(),
        changed_files: vec!["src/main.rs".to_owned()],
        change_lines: vec!["M src/main.rs".to_owned()],
        base: "main".to_owned(),
        base_pull_request: None,
        head: PullRequestHead::same_repository("example-owner", "example-user/02-zzzzzzzz"),
        labels: Vec::new(),
        draft: false,
        existing_pull_request: None,
        reviewer_candidates: Vec::new(),
        reviewers: ReviewerSelection::default(),
    }
}

struct FakeServices {
    workspace_log: String,
    workspace_log_annotations: std::cell::RefCell<Vec<Vec<LogBookmarkAnnotation>>>,
    previous_commit_log: String,
    next_commit_log: String,
    workspace_status: WorkspaceStatus,
    workspace_status_requests: std::cell::RefCell<Vec<Option<String>>>,
    workspace: WorkspaceFacts,
    description_rewrites: std::cell::RefCell<Vec<(String, String)>>,
    check_snapshots: std::cell::RefCell<Vec<WorkingCopySnapshot>>,
    check_command_outputs: std::cell::RefCell<Vec<CheckCommandOutput>>,
    check_command_calls: std::cell::RefCell<Vec<RepoCheckConfig>>,
    hook_command_outputs: std::cell::RefCell<Vec<HookCommandOutput>>,
    hook_command_calls: std::cell::RefCell<Vec<(PathBuf, RepoHook)>>,
    workspace_delete_events: std::cell::RefCell<Vec<String>>,
    status_workspace: StatusWorkspaceFacts,
    status_workspace_error: Option<String>,
    check: CheckReport,
    status: StatusReport,
    stack_trunk_branch_head_sha: String,
    status_uses_context_remotes: bool,
    clean_status_repos: Vec<String>,
    github_login: String,
    pull_request_bookmarks: Vec<String>,
    pull_request_bookmark_calls: std::cell::Cell<usize>,
    open_pull_request_candidates: Vec<String>,
    open_pull_request_selectors: std::cell::RefCell<Vec<Option<String>>>,
    authored_open_pull_requests_by_head: BTreeMap<String, PullRequestRecord>,
    authored_open_pull_request_head_calls: std::cell::RefCell<Vec<(String, String)>>,
    authored_open_pull_requests: Vec<PullRequestRecord>,
    authored_open_pull_request_calls: std::cell::RefCell<Vec<String>>,
    pull_requests_by_head: BTreeMap<String, PullRequestRecord>,
    open_pull_request_head_calls: std::cell::RefCell<Vec<String>>,
    pull_request_head_calls: std::cell::RefCell<Vec<String>>,
    pull_requests_by_number: BTreeMap<u64, PullRequestRecord>,
    pull_request_number_calls: std::cell::RefCell<Vec<u64>>,
    pull_request_statuses: BTreeMap<u64, PullRequestStatusRecord>,
    pull_requests_with_history: BTreeMap<u64, PullRequestWithHistory>,
    pull_request_status_calls: std::cell::RefCell<Vec<Vec<u64>>>,
    review_requests: Vec<PullRequestReviewRequest>,
    review_requests_saml_enforced: bool,
    github_user_display_names: BTreeMap<String, String>,
    opened_urls: std::cell::RefCell<Vec<String>>,
    global_fetch_ready_roots: Option<BTreeSet<PathBuf>>,
    origin_push_access_roots: Option<BTreeSet<PathBuf>>,
    up_to_date_sync_roots: BTreeSet<PathBuf>,
    fetch_origin_roots: std::cell::RefCell<Vec<PathBuf>>,
    fetch_options: std::cell::RefCell<Vec<FetchOptions>>,
    fetch_origin_failures_before_success: std::cell::Cell<usize>,
    push_tracked_roots: std::cell::RefCell<Vec<PathBuf>>,
    push_syncable_revision_requests: std::cell::RefCell<Vec<Option<String>>>,
    push_rejections_before_success: std::cell::Cell<usize>,
    tracked_changed_files: Vec<String>,
    bookmark_changed_files: Vec<String>,
    changed_file_bookmark_requests: std::cell::RefCell<Vec<Vec<String>>>,
    fetch: FetchOutcome,
    stack_move: StackMoveOutcome,
    stack_move_requests: std::cell::RefCell<Vec<(Vec<String>, StackMoveTarget)>>,
    local_stack_branches: std::cell::RefCell<Vec<Vec<LocalStackBranch>>>,
    stack_publish_facts: Option<StackPublishFacts>,
    stack_publish_facts_by_revision: BTreeMap<String, StackPublishFacts>,
    stack_publish_selections: std::cell::RefCell<Vec<StackPublishSelection>>,
    stack_plan_facts: Option<StackPlanFacts>,
    stack_plan_selections: std::cell::RefCell<Vec<StackPlanSelection>>,
    bookmark_update: BookmarkUpdate,
    push: PushOutcome,
    push_bookmark_calls: std::cell::RefCell<Vec<String>>,
    advance_trunk: AdvanceTrunkOutcome,
    advance_trunk_calls: std::cell::Cell<usize>,
    tracked_push: TrackedPushOutcome,
    sync_conflicted_bookmarks: Vec<crate::jj::SkippedPushBookmarkSummary>,
    sync_pull_requests: Vec<PullRequestRecord>,
    sync_pull_request_pushes: std::cell::RefCell<Vec<TrackedPushOutcome>>,
    sync_pull_request_metadata: std::cell::RefCell<Vec<StackMetadata>>,
    pull_request_action: PullRequestAction,
    published_pull_request_count: std::cell::Cell<u64>,
    metadata_only_pull_request_count: std::cell::Cell<u64>,
    published_plans: std::cell::RefCell<Vec<PullRequestPlan>>,
    metadata_only_plans: std::cell::RefCell<Vec<PullRequestPlan>>,
    pull_request_plan_task_ids: std::cell::RefCell<Vec<Option<String>>>,
    pull_request_url: Option<String>,
    pull_request_event_effects: Vec<domain::PullRequestEventEffect>,
    existing_pull_request: Option<PullRequestRecord>,
    reviewer_candidates: Vec<ReviewerCandidate>,
    expected_reviewers: Option<ReviewerSelection>,
    expected_task_id: Option<Option<String>>,
    expected_labels: Vec<String>,
    expected_labels_by_publish: Option<Vec<Vec<String>>>,
    expected_draft: Option<bool>,
    expected_drafts: Option<Vec<bool>>,
    expected_clone: Option<(String, PathBuf)>,
    expected_init_repository: Option<PathBuf>,
    init_repository_calls: std::cell::Cell<usize>,
    expected_workspace_add_current_dir: Option<PathBuf>,
    expected_workspace_add: Option<WorkspaceAddOptions>,
    workspace_add_error: Option<String>,
    workspace_add_metadata_blocker: Option<PathBuf>,
    workspace_add_existing_shared_path: Option<PathBuf>,
    workspaces: Vec<WorkspaceEntry>,
    workspace_remove_current_dirs: std::cell::RefCell<Vec<PathBuf>>,
    workspace_removes: std::cell::RefCell<Vec<WorkspaceRemoveOptions>>,
    initial_publish_target: InitialPublishTarget,
    prepared_initial_publish_target: Option<InitialPublishTarget>,
    prepare_initial_publish_calls: std::cell::Cell<usize>,
    created_repository: RepositoryCreation,
    create_repository_calls: std::cell::Cell<usize>,
    expected_bootstrap: Option<(String, InitialPublishTarget)>,
    bootstrap_push: BootstrapPushOutcome,
}

impl Default for FakeServices {
    fn default() -> Self {
        let repository = RepositorySummary {
            origin_name: "origin",
            origin_url: "https://github.com/example-owner/example-repo.git".to_owned(),
            github_slug: "example-owner/example-repo".to_owned(),
            github_url: "https://github.com/example-owner/example-repo".to_owned(),
            token_source: "GH_TOKEN environment variable".to_owned(),
            config: "defaults",
            default_reviewers: "none".to_owned(),
        };

        Self {
            workspace_log: "workspace log\n".to_owned(),
            workspace_log_annotations: std::cell::RefCell::new(Vec::new()),
            previous_commit_log: "previous commit graph\n".to_owned(),
            next_commit_log: "next commit graph\n".to_owned(),
            workspace_status: workspace_status(),
            workspace_status_requests: std::cell::RefCell::new(Vec::new()),
            workspace: workspace_facts(),
            description_rewrites: std::cell::RefCell::new(Vec::new()),
            check_snapshots: std::cell::RefCell::new(Vec::new()),
            check_command_outputs: std::cell::RefCell::new(Vec::new()),
            check_command_calls: std::cell::RefCell::new(Vec::new()),
            hook_command_outputs: std::cell::RefCell::new(Vec::new()),
            hook_command_calls: std::cell::RefCell::new(Vec::new()),
            workspace_delete_events: std::cell::RefCell::new(Vec::new()),
            status_workspace: status_workspace_facts(),
            status_workspace_error: None,
            check: CheckReport {
                repository: repository.clone(),
                workspace: CheckWorkspaceSummary {
                    trunk_branch: "main".to_owned(),
                    trunk_short_commit_id: "11112222".to_owned(),
                    current_short_commit_id: "a1b2c3d4".to_owned(),
                    current_is_empty: false,
                    stack_index: 2,
                },
                github: GitHubReadiness {
                    login: "example-user".to_owned(),
                    default_branch: Some("main".to_owned()),
                    can_push: true,
                },
                bookmark: BookmarkPlan {
                    branch: "example-user/02-zzzzzzzz".to_owned(),
                    action: BookmarkAction::Create,
                },
            },
            status: StatusReport {
                remotes: vec![domain::RemoteStatusReport {
                    name: "origin".to_owned(),
                    url: "https://github.com/example-owner/example-repo.git".to_owned(),
                    github_url: "https://github.com/example-owner/example-repo".to_owned(),
                    branch: "main".to_owned(),
                    local_trunk_sha: "1111222233334444".to_owned(),
                    local_trunk_short_sha: "11112222".to_owned(),
                    local_ahead_by: 0,
                    comparison: StatusComparison {
                        state: StatusState::GithubAhead,
                        github_ahead_by: 3,
                        github_behind_by: 0,
                        counts_exact: true,
                    },
                }],
                fork: None,
            },
            stack_trunk_branch_head_sha: "aaaabbbbccccdddd".to_owned(),
            status_uses_context_remotes: false,
            clean_status_repos: Vec::new(),
            github_login: "example-user".to_owned(),
            pull_request_bookmarks: Vec::new(),
            pull_request_bookmark_calls: std::cell::Cell::new(0),
            open_pull_request_candidates: Vec::new(),
            open_pull_request_selectors: std::cell::RefCell::new(Vec::new()),
            authored_open_pull_requests_by_head: BTreeMap::new(),
            authored_open_pull_request_head_calls: std::cell::RefCell::new(Vec::new()),
            authored_open_pull_requests: Vec::new(),
            authored_open_pull_request_calls: std::cell::RefCell::new(Vec::new()),
            pull_requests_by_head: BTreeMap::new(),
            open_pull_request_head_calls: std::cell::RefCell::new(Vec::new()),
            pull_request_head_calls: std::cell::RefCell::new(Vec::new()),
            pull_requests_by_number: BTreeMap::new(),
            pull_request_number_calls: std::cell::RefCell::new(Vec::new()),
            pull_request_statuses: BTreeMap::new(),
            pull_requests_with_history: BTreeMap::new(),
            pull_request_status_calls: std::cell::RefCell::new(Vec::new()),
            review_requests: Vec::new(),
            review_requests_saml_enforced: false,
            github_user_display_names: BTreeMap::new(),
            opened_urls: std::cell::RefCell::new(Vec::new()),
            global_fetch_ready_roots: None,
            origin_push_access_roots: None,
            up_to_date_sync_roots: BTreeSet::new(),
            fetch_origin_roots: std::cell::RefCell::new(Vec::new()),
            fetch_options: std::cell::RefCell::new(Vec::new()),
            fetch_origin_failures_before_success: std::cell::Cell::new(0),
            push_tracked_roots: std::cell::RefCell::new(Vec::new()),
            push_syncable_revision_requests: std::cell::RefCell::new(Vec::new()),
            push_rejections_before_success: std::cell::Cell::new(0),
            tracked_changed_files: vec!["src/main.rs".to_owned()],
            bookmark_changed_files: vec!["src/main.rs".to_owned()],
            changed_file_bookmark_requests: std::cell::RefCell::new(Vec::new()),
            fetch: FetchOutcome {
                branch: "main".to_owned(),
                trunk: None,
                changed_remote_bookmarks: 1,
                changed_remote_tags: 0,
                abandoned_commits: 0,
                rebased_trunk_children: 1,
                rebased_descendants: 2,
                skipped_trunk_children: 0,
                current_repaired: true,
                rebased_commits: vec![
                    RebasedCommitSummary {
                        short_change_id: "changeaa".to_owned(),
                        old_short_commit_id: "aaaabbbb".to_owned(),
                        new_short_commit_id: "ccccdddd".to_owned(),
                        description: "example change".to_owned(),
                        has_conflict: false,
                        is_empty: false,
                        workspace_visibility: current_workspace_visibility(),
                    },
                    RebasedCommitSummary {
                        short_change_id: "changebb".to_owned(),
                        old_short_commit_id: "eeeeffff".to_owned(),
                        new_short_commit_id: "12345678".to_owned(),
                        description: "follow-up change".to_owned(),
                        has_conflict: false,
                        is_empty: false,
                        workspace_visibility: current_workspace_visibility(),
                    },
                    RebasedCommitSummary {
                        short_change_id: "changezz".to_owned(),
                        old_short_commit_id: "9999aaaa".to_owned(),
                        new_short_commit_id: "bbbbcccc".to_owned(),
                        description: "(no description)".to_owned(),
                        has_conflict: false,
                        is_empty: true,
                        workspace_visibility: current_workspace_visibility(),
                    },
                ],
            },
            stack_move: StackMoveOutcome {
                source_short_commit_ids: vec!["a1b2c3d4".to_owned()],
                target_short_commit_id: "11112222".to_owned(),
                rebased_commits: 1,
                skipped_commits: 0,
                current_updated: true,
            },
            stack_move_requests: std::cell::RefCell::new(Vec::new()),
            local_stack_branches: std::cell::RefCell::new(Vec::new()),
            stack_publish_facts: None,
            stack_publish_facts_by_revision: BTreeMap::new(),
            stack_publish_selections: std::cell::RefCell::new(Vec::new()),
            stack_plan_facts: None,
            stack_plan_selections: std::cell::RefCell::new(Vec::new()),
            bookmark_update: BookmarkUpdate {
                branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
                created: true,
            },
            push: PushOutcome {
                branch: "example-user/abc-123-02-zzzzzzzz".to_owned(),
                pushed_refs: 1,
                pushed_commits: vec![PushedCommitSummary {
                    short_commit_id: "a1b2c3d4".to_owned(),
                    description: "example change".to_owned(),
                }],
            },
            push_bookmark_calls: std::cell::RefCell::new(Vec::new()),
            advance_trunk: AdvanceTrunkOutcome {
                branch: "main".to_owned(),
                old_short_commit_id: "11112222".to_owned(),
                new_short_commit_id: "a1b2c3d4".to_owned(),
                trunk: None,
                current_updated: true,
            },
            advance_trunk_calls: std::cell::Cell::new(0),
            tracked_push: TrackedPushOutcome {
                pushed_refs: 2,
                bookmarks: vec![
                    PushedBookmarkSummary {
                        branch: "example-user/current".to_owned(),
                        old_short_commit_id: Some("11112222".to_owned()),
                        new_short_commit_id: Some("a1b2c3d4".to_owned()),
                        old_short_change_id: Some("changeoo".to_owned()),
                        new_short_change_id: Some("changecc".to_owned()),
                        old_description: Some("previous example change".to_owned()),
                        new_description: Some("example change".to_owned()),
                        pull_request_description: Some("example change".to_owned()),
                        pull_request_base: Some("main".to_owned()),
                        new_workspace_visibility: current_workspace_visibility(),
                    },
                    PushedBookmarkSummary {
                        branch: "example-user/old".to_owned(),
                        old_short_commit_id: Some("99990000".to_owned()),
                        new_short_commit_id: None,
                        old_short_change_id: Some("changedd".to_owned()),
                        new_short_change_id: None,
                        old_description: Some("obsolete example change".to_owned()),
                        new_description: None,
                        pull_request_description: None,
                        pull_request_base: None,
                        new_workspace_visibility: WorkspaceVisibility::default(),
                    },
                ],
                pushed_commits: vec![PushedCommitSummary {
                    short_commit_id: "a1b2c3d4".to_owned(),
                    description: "example change".to_owned(),
                }],
            },
            sync_conflicted_bookmarks: Vec::new(),
            sync_pull_requests: Vec::new(),
            sync_pull_request_pushes: std::cell::RefCell::new(Vec::new()),
            sync_pull_request_metadata: std::cell::RefCell::new(Vec::new()),
            pull_request_action: PullRequestAction::Created,
            published_pull_request_count: std::cell::Cell::new(0),
            metadata_only_pull_request_count: std::cell::Cell::new(0),
            published_plans: std::cell::RefCell::new(Vec::new()),
            metadata_only_plans: std::cell::RefCell::new(Vec::new()),
            pull_request_plan_task_ids: std::cell::RefCell::new(Vec::new()),
            pull_request_url: Some(
                "https://github.com/example-owner/example-repo/pull/42".to_owned(),
            ),
            pull_request_event_effects: Vec::new(),
            existing_pull_request: None,
            reviewer_candidates: Vec::new(),
            expected_reviewers: None,
            expected_task_id: None,
            expected_labels: Vec::new(),
            expected_labels_by_publish: None,
            expected_draft: None,
            expected_drafts: None,
            expected_clone: None,
            expected_init_repository: None,
            init_repository_calls: std::cell::Cell::new(0),
            expected_workspace_add_current_dir: None,
            expected_workspace_add: None,
            workspace_add_error: None,
            workspace_add_metadata_blocker: None,
            workspace_add_existing_shared_path: None,
            workspaces: vec![WorkspaceEntry {
                name: "default".to_owned(),
                root: PathBuf::from("/workspace"),
                is_current: true,
            }],
            workspace_remove_current_dirs: std::cell::RefCell::new(Vec::new()),
            workspace_removes: std::cell::RefCell::new(Vec::new()),
            initial_publish_target: InitialPublishTarget {
                commit_id: "a1b2c3d4e5f6".to_owned(),
                short_commit_id: "a1b2c3d4".to_owned(),
                description: "example change".to_owned(),
            },
            prepared_initial_publish_target: None,
            prepare_initial_publish_calls: std::cell::Cell::new(0),
            created_repository: RepositoryCreation {
                repository: GitHubRepository {
                    owner: "example-owner".to_owned(),
                    name: "example-repo".to_owned(),
                },
                html_url: "https://github.com/example-owner/example-repo".to_owned(),
                private: true,
            },
            create_repository_calls: std::cell::Cell::new(0),
            expected_bootstrap: None,
            bootstrap_push: BootstrapPushOutcome {
                branch: "main".to_owned(),
                short_commit_id: "a1b2c3d4".to_owned(),
                description: "example change".to_owned(),
                working_copy_short_commit_id: Some("bf4799d5".to_owned()),
            },
        }
    }
}

impl FakeServices {
    fn assert_publish_plan(&self, plan: &PullRequestPlan, publish_index: usize) {
        if let Some(expected) = &self.expected_reviewers {
            assert_eq!(&plan.reviewers, expected);
        }
        if let Some(expected) = &self.expected_labels_by_publish {
            assert_eq!(plan.labels, expected[publish_index]);
        } else {
            assert_eq!(plan.labels, self.expected_labels);
        }
        if let Some(expected) = &self.expected_drafts {
            assert_eq!(plan.draft, expected[publish_index]);
        } else if let Some(expected) = self.expected_draft {
            assert_eq!(plan.draft, expected);
        }
    }

    fn pull_request_record_for_plan(
        &self,
        plan: &PullRequestPlan,
        number: u64,
    ) -> PullRequestRecord {
        let html_url = self.pull_request_url.clone().map(|url| {
            url.strip_suffix("/42")
                .map_or(url.clone(), |prefix| format!("{prefix}/{number}"))
        });

        PullRequestRecord {
            number,
            title: plan.title.clone(),
            body: (!plan.body.is_empty()).then_some(plan.body.clone()),
            head_branch: plan.head.branch.clone(),
            base_branch: plan.base.clone(),
            html_url,
            draft: plan.draft,
            merged: false,
            reviewers: ReviewerSelection::default(),
        }
    }

    fn fake_workspace_facts(&self, revision: Option<&str>) -> Result<WorkspaceFacts, JjError> {
        let mut workspace = self.workspace.clone();
        if revision.is_some() {
            workspace.target_change.commit_id = "deadbeefcafebabe".to_owned();
            workspace.target_change.short_commit_id = "deadbeef".to_owned();
            workspace.stack_index = 3;
        }
        if revision == Some("feedfacecafebeef") {
            workspace.target_change.commit_id = "feedfacecafebeef".to_owned();
            workspace.target_change.short_commit_id = "feedface".to_owned();
            if let Some((_, description)) = self.description_rewrites.borrow().last() {
                workspace.target_change.description = description.clone();
            }
        }
        Ok(workspace)
    }
}

impl CommandServices for FakeServices {
    fn workspace_log(&self, annotations: &[LogBookmarkAnnotation]) -> Result<String, JjError> {
        self.workspace_log_annotations
            .borrow_mut()
            .push(annotations.to_vec());
        Ok(self.workspace_log.clone())
    }

    fn current_diff(&self, _current_dir: &Path, options: &DiffOptions) -> Result<String, JjError> {
        let tool = match &options.tool {
            DiffToolInvocation::Plain => "plain".to_owned(),
            DiffToolInvocation::External(tool) => format!(
                "external command={} args={}",
                tool.command,
                tool.args.join(",")
            ),
            DiffToolInvocation::Pipe(tool) => format!(
                "pipe producer={} command={} args={}",
                tool.producer_args.join(","),
                tool.command,
                tool.args.join(",")
            ),
        };
        let revision = options
            .revision
            .as_ref()
            .map(|revision| format!(" revision={revision}"))
            .unwrap_or_default();
        let paths = if options.paths.is_empty() {
            String::new()
        } else {
            format!(" paths={}", options.paths.join(","))
        };
        Ok(format!(
            "diff:{revision}{paths} no_tests={} tool={tool}\n",
            options.no_tests
        ))
    }

    fn previous_commit_log(&self, _current_dir: &Path) -> Result<String, JjError> {
        Ok(self.previous_commit_log.clone())
    }

    fn next_commit_log(&self, _current_dir: &Path) -> Result<String, JjError> {
        Ok(self.next_commit_log.clone())
    }

    fn clone_repository(&self, _current_dir: &Path, plan: &ClonePlan) -> Result<(), JjError> {
        if let Some((expected_remote, expected_destination)) = &self.expected_clone {
            assert_eq!(&plan.remote_url, expected_remote);
            assert_eq!(&plan.destination, expected_destination);
        }
        Ok(())
    }

    fn init_repository(&self, current_dir: &Path) -> Result<(), JjError> {
        if let Some(expected) = &self.expected_init_repository {
            assert_eq!(current_dir, expected);
        }
        self.init_repository_calls
            .set(self.init_repository_calls.get() + 1);
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, current_dir))
            .map(|_| ())
            .map_err(|error| JjError::InitFailed {
                status: error.to_string(),
            })
    }

    fn add_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceAddOptions,
    ) -> Result<(), JjError> {
        if let Some(expected) = &self.expected_workspace_add_current_dir {
            assert_eq!(current_dir, expected);
        }
        if let Some(expected) = &self.expected_workspace_add {
            assert_eq!(options, expected);
        }
        if let Some(status) = &self.workspace_add_error {
            return Err(JjError::WorkspaceAddFailed {
                status: status.clone(),
            });
        }
        if let Some(destination) = &self.workspace_add_metadata_blocker {
            fs::create_dir_all(destination).expect("create simulated workspace destination");
            fs::write(destination.join(".jx"), "not a directory").expect("create metadata blocker");
        }
        if let Some(path) = &self.workspace_add_existing_shared_path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create existing shared-path parent");
            }
            fs::write(path, "existing content").expect("create existing shared path");
        }
        Ok(())
    }

    fn workspace_entries(&self, _current_dir: &Path) -> Result<Vec<WorkspaceEntry>, JjError> {
        Ok(self.workspaces.clone())
    }

    fn current_workspace_entry(&self, _current_dir: &Path) -> Result<WorkspaceEntry, JjError> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.is_current)
            .cloned()
            .ok_or_else(|| JjError::WorkspaceLoad {
                message: "test current workspace missing".to_owned(),
            })
    }

    fn remove_workspace(
        &self,
        current_dir: &Path,
        options: &WorkspaceRemoveOptions,
    ) -> Result<(), JjError> {
        self.workspace_delete_events
            .borrow_mut()
            .push(format!("remove:{}", options.name));
        self.workspace_remove_current_dirs
            .borrow_mut()
            .push(current_dir.to_path_buf());
        self.workspace_removes.borrow_mut().push(options.clone());
        Ok(())
    }

    fn initial_publish_target(
        &self,
        _workspace_root: &Path,
    ) -> Result<InitialPublishTarget, JjError> {
        Ok(self.initial_publish_target.clone())
    }

    fn prepare_initial_publish_target(
        &self,
        _workspace_root: &Path,
        target: &InitialPublishTarget,
    ) -> Result<InitialPublishTarget, JjError> {
        self.prepare_initial_publish_calls
            .set(self.prepare_initial_publish_calls.get() + 1);
        Ok(self
            .prepared_initial_publish_target
            .clone()
            .unwrap_or_else(|| target.clone()))
    }

    fn create_repository(
        &self,
        _context: &LocalRepositoryContext,
        repository: &GitHubRepository,
    ) -> Result<RepositoryCreation, WorkflowError> {
        self.create_repository_calls
            .set(self.create_repository_calls.get() + 1);
        let mut created = self.created_repository.clone();
        created.repository = repository.clone();
        created.html_url = repository.https_url();
        Ok(created)
    }

    fn bootstrap_origin_main(
        &self,
        _workspace_root: &Path,
        remote_url: &str,
        target: &InitialPublishTarget,
    ) -> Result<BootstrapPushOutcome, JjError> {
        if let Some((expected_remote, expected_target)) = &self.expected_bootstrap {
            assert_eq!(remote_url, expected_remote);
            assert_eq!(target, expected_target);
        }
        Ok(self.bootstrap_push.clone())
    }

    fn workspace_status(
        &self,
        _current_dir: &Path,
        revision: Option<&str>,
        _color: bool,
    ) -> Result<WorkspaceStatus, JjError> {
        self.workspace_status_requests
            .borrow_mut()
            .push(revision.map(str::to_owned));
        Ok(self.workspace_status.clone())
    }

    fn rewrite_commit_description(
        &self,
        _context: &RepositoryContext,
        target_commit_id: &str,
        description: &str,
    ) -> Result<CommitDescriptionRewrite, JjError> {
        self.description_rewrites
            .borrow_mut()
            .push((target_commit_id.to_owned(), description.to_owned()));
        Ok(CommitDescriptionRewrite {
            commit_id: "feedfacecafebeef".to_owned(),
            changed: true,
        })
    }

    fn workspace_facts(
        &self,
        _context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.fake_workspace_facts(revision)
    }

    fn push_workspace_facts(
        &self,
        _context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<WorkspaceFacts, JjError> {
        self.fake_workspace_facts(revision)
    }

    fn working_copy_snapshot(
        &self,
        _context: &RepositoryContext,
    ) -> Result<WorkingCopySnapshot, JjError> {
        let mut snapshots = self.check_snapshots.borrow_mut();
        if snapshots.is_empty() {
            Ok(WorkingCopySnapshot {
                commit_id: "snapshot".to_owned(),
            })
        } else {
            Ok(snapshots.remove(0))
        }
    }

    fn run_check_command(
        &self,
        _context: &RepositoryContext,
        check: &RepoCheckConfig,
    ) -> io::Result<CheckCommandOutput> {
        self.check_command_calls.borrow_mut().push(check.clone());
        let mut outputs = self.check_command_outputs.borrow_mut();
        if outputs.is_empty() {
            Ok(CheckCommandOutput::success())
        } else {
            Ok(outputs.remove(0))
        }
    }

    fn run_hook_command(
        &self,
        workspace_root: &Path,
        hook: &RepoHook,
    ) -> io::Result<HookCommandOutput> {
        self.workspace_delete_events
            .borrow_mut()
            .push(format!("hook:{}", hook.id));
        self.hook_command_calls
            .borrow_mut()
            .push((workspace_root.to_path_buf(), hook.clone()));
        let mut outputs = self.hook_command_outputs.borrow_mut();
        if outputs.is_empty() {
            Ok(HookCommandOutput::success())
        } else {
            Ok(outputs.remove(0))
        }
    }

    fn check_readiness(
        &self,
        _context: &RepositoryContext,
        _workspace: WorkspaceFacts,
    ) -> Result<CheckReport, WorkflowError> {
        Ok(self.check.clone())
    }

    fn status_workspace_facts(
        &self,
        _context: &RepositoryContext,
    ) -> Result<StatusWorkspaceFacts, JjError> {
        if let Some(message) = &self.status_workspace_error {
            return Err(JjError::StatusFailed {
                status: message.clone(),
            });
        }

        Ok(self.status_workspace.clone())
    }

    fn status_report(
        &self,
        context: &RepositoryContext,
        _workspace: StatusWorkspaceFacts,
    ) -> Result<StatusReport, WorkflowError> {
        let mut status = self.status.clone();
        if self.status_uses_context_remotes {
            for remote in &mut status.remotes {
                if let Some(context_remote) = context
                    .github_remotes
                    .iter()
                    .find(|context_remote| context_remote.name == remote.name)
                {
                    remote.url = context_remote.url.clone();
                    remote.github_url = context_remote.github.https_url();
                }
            }
        }
        if self
            .clean_status_repos
            .iter()
            .any(|repo| repo == &context.origin.github.name)
        {
            for remote in &mut status.remotes {
                remote.local_ahead_by = 0;
                remote.comparison = StatusComparison {
                    state: StatusState::UpToDate,
                    github_ahead_by: 0,
                    github_behind_by: 0,
                    counts_exact: true,
                };
            }
            if let Some(fork) = &mut status.fork {
                fork.comparison = ForkStatusComparison {
                    state: ForkStatusState::Synced,
                    source_ahead_by: 0,
                    fork_ahead_by: 0,
                };
            }
        }
        Ok(status)
    }

    fn stack_trunk_status_report(
        &self,
        context: &RepositoryContext,
        workspace: StatusWorkspaceFacts,
    ) -> Result<RemoteStatusReport, WorkflowError> {
        domain::stack_trunk_status_report_from_branch_head(
            context,
            workspace,
            &self.stack_trunk_branch_head_sha,
        )
    }

    fn origin_can_push(&self, context: &RepositoryContext) -> Result<bool, WorkflowError> {
        Ok(self
            .origin_push_access_roots
            .as_ref()
            .is_none_or(|roots| roots.contains(&context.workspace_root)))
    }

    fn authenticated_login(&self, _token_source: &TokenSource) -> Result<String, WorkflowError> {
        Ok(self.github_login.clone())
    }

    fn pull_request_bookmarks(&self, _context: &RepositoryContext) -> Result<Vec<String>, JjError> {
        self.pull_request_bookmark_calls
            .set(self.pull_request_bookmark_calls.get() + 1);
        Ok(self.pull_request_bookmarks.clone())
    }

    fn pull_request_candidate_bookmarks(
        &self,
        _context: &RepositoryContext,
        selector: Option<&str>,
    ) -> Result<Vec<String>, JjError> {
        self.open_pull_request_selectors
            .borrow_mut()
            .push(selector.map(str::to_owned));
        Ok(self.open_pull_request_candidates.clone())
    }

    fn find_authored_open_pull_request_for_head(
        &self,
        _context: &RepositoryContext,
        branch: &str,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.authored_open_pull_request_head_calls
            .borrow_mut()
            .push((branch.to_owned(), author.to_owned()));
        Ok(self
            .authored_open_pull_requests_by_head
            .get(branch)
            .cloned())
    }

    fn authored_open_pull_requests(
        &self,
        _context: &RepositoryContext,
        author: &str,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.authored_open_pull_request_calls
            .borrow_mut()
            .push(author.to_owned());
        Ok(self.authored_open_pull_requests.clone())
    }

    fn find_pull_request_for_head(
        &self,
        _context: &RepositoryContext,
        branch: &str,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.pull_request_head_calls
            .borrow_mut()
            .push(branch.to_owned());
        Ok(self.pull_requests_by_head.get(branch).cloned())
    }

    fn find_pull_request_by_number(
        &self,
        _context: &RepositoryContext,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, WorkflowError> {
        self.pull_request_number_calls.borrow_mut().push(number);
        Ok(self.pull_requests_by_number.get(&number).cloned())
    }

    fn pull_request_statuses(
        &self,
        _context: &RepositoryContext,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        self.pull_request_status_calls
            .borrow_mut()
            .push(numbers.to_vec());
        Ok(numbers
            .iter()
            .filter_map(|number| self.pull_request_statuses.get(number).cloned())
            .collect())
    }

    fn review_requests(
        &self,
        _token_source: &TokenSource,
    ) -> Result<PullRequestReviewRequests, WorkflowError> {
        if self.review_requests_saml_enforced {
            return Err(WorkflowError::GitHub(GitHubError::GraphQl {
                operation: "search review requests",
                message: "Resource protected by organization SAML enforcement. You must grant your Personal Access token access to this organization.".to_owned(),
            }));
        }

        Ok(PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: self.github_login.clone(),
            },
            requests: self.review_requests.clone(),
        })
    }

    fn review_requests_for_repositories(
        &self,
        _token_source: &TokenSource,
        repositories: &[GitHubRepository],
    ) -> Result<PullRequestReviewRequests, WorkflowError> {
        let repositories = repositories.iter().collect::<BTreeSet<_>>();
        Ok(PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: self.github_login.clone(),
            },
            requests: self
                .review_requests
                .iter()
                .filter(|request| repositories.contains(&request.repository))
                .cloned()
                .collect(),
        })
    }

    fn github_user_display_names(
        &self,
        _token_source: &TokenSource,
        logins: &[String],
    ) -> BTreeMap<String, String> {
        logins
            .iter()
            .filter_map(|login| {
                self.github_user_display_names
                    .get(login)
                    .map(|name| (login.clone(), name.clone()))
            })
            .collect()
    }

    fn pull_request_statuses_for_repository(
        &self,
        _token_source: &TokenSource,
        _repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, WorkflowError> {
        self.pull_request_status_calls
            .borrow_mut()
            .push(numbers.to_vec());
        Ok(numbers
            .iter()
            .filter_map(|number| self.pull_request_statuses.get(number).cloned())
            .collect())
    }

    fn pull_requests_with_history_for_repository(
        &self,
        _token_source: &TokenSource,
        _repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestWithHistory>, WorkflowError> {
        self.pull_request_status_calls
            .borrow_mut()
            .push(numbers.to_vec());
        Ok(numbers
            .iter()
            .filter_map(|number| {
                self.pull_requests_with_history
                    .get(number)
                    .cloned()
                    .or_else(|| {
                        self.pull_request_statuses
                            .get(number)
                            .cloned()
                            .map(|status| PullRequestWithHistory {
                                status,
                                history: Vec::new(),
                                actions: Vec::new(),
                            })
                    })
            })
            .collect())
    }

    fn open_url(&self, url: &str) -> io::Result<()> {
        self.opened_urls.borrow_mut().push(url.to_owned());
        Ok(())
    }

    fn global_fetch_ready(&self, context: &RepositoryContext) -> Result<bool, JjError> {
        Ok(self
            .global_fetch_ready_roots
            .as_ref()
            .is_none_or(|roots| roots.contains(&context.workspace_root)))
    }

    fn fetch_origin(&self, context: &RepositoryContext) -> Result<FetchOutcome, JjError> {
        self.fetch_origin_with_options(context, FetchOptions::default())
    }

    fn fetch_origin_with_options(
        &self,
        context: &RepositoryContext,
        options: FetchOptions,
    ) -> Result<FetchOutcome, JjError> {
        self.fetch_origin_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        self.fetch_options.borrow_mut().push(options);
        let remaining_failures = self.fetch_origin_failures_before_success.get();
        if remaining_failures > 0 {
            self.fetch_origin_failures_before_success
                .set(remaining_failures - 1);
            return Err(JjError::Fetch {
                message: "transient fetch failure".to_owned(),
            });
        }

        Ok(self.fetch.clone())
    }

    fn move_stack(
        &self,
        _context: &RepositoryContext,
        revisions: &[String],
        target: &StackMoveTarget,
    ) -> Result<StackMoveOutcome, JjError> {
        self.stack_move_requests
            .borrow_mut()
            .push((revisions.to_vec(), target.clone()));
        Ok(self.stack_move.clone())
    }

    fn local_stack_branches(
        &self,
        _context: &RepositoryContext,
    ) -> Result<Vec<LocalStackBranch>, JjError> {
        let mut branches = self.local_stack_branches.borrow_mut();
        if branches.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(branches.remove(0))
        }
    }

    fn stack_publish_facts(
        &self,
        _context: &RepositoryContext,
        selection: &StackPublishSelection,
    ) -> Result<StackPublishFacts, JjError> {
        self.stack_publish_selections
            .borrow_mut()
            .push(selection.clone());
        if let StackPublishSelection::ExplicitRevisions { revisions } = selection {
            if let [revision] = revisions.as_slice() {
                if let Some(facts) = self.stack_publish_facts_by_revision.get(revision) {
                    return Ok(facts.clone());
                }
            }
        }
        Ok(self.stack_publish_facts.clone().unwrap_or_else(|| {
            let mut workspace = self.workspace.clone();
            if let Some((_, description)) = self.description_rewrites.borrow().last() {
                workspace.target_change.commit_id = "feedfacecafebeef".to_owned();
                workspace.target_change.short_commit_id = "feedface".to_owned();
                workspace.target_change.description = description.clone();
            }
            StackPublishFacts {
                nodes: vec![crate::jj::StackPublishNodeFacts {
                    workspace,
                    parent_index: None,
                }],
                publish_indexes: vec![0],
                anchor_index: match selection {
                    StackPublishSelection::InferredStack { .. } => Some(0),
                    StackPublishSelection::ExplicitRevisions { .. } => None,
                },
                metrics: StackPublishMetrics::default(),
            }
        }))
    }

    fn stack_plan_facts(
        &self,
        _context: &RepositoryContext,
        selection: &StackPlanSelection,
    ) -> Result<StackPlanFacts, JjError> {
        self.stack_plan_selections
            .borrow_mut()
            .push(selection.clone());
        Ok(self
            .stack_plan_facts
            .clone()
            .unwrap_or_else(|| StackPlanFacts {
                trunk: self.workspace.trunk.clone(),
                nodes: vec![crate::jj::StackPlanNodeFacts {
                    workspace: self.workspace.clone(),
                    parent_index: None,
                }],
                selected_indexes: vec![0],
                anchor_index: match selection {
                    StackPlanSelection::InferredStack { .. } => Some(0),
                    StackPlanSelection::ExplicitRevisions { .. } => None,
                },
            }))
    }

    fn ensure_bookmark(
        &self,
        _context: &RepositoryContext,
        branch: &str,
        _target_commit_id: &str,
    ) -> Result<BookmarkUpdate, JjError> {
        let mut update = self.bookmark_update.clone();
        update.branch = branch.to_owned();
        Ok(update)
    }

    fn push_bookmark(
        &self,
        _context: &RepositoryContext,
        branch: &str,
    ) -> Result<PushOutcome, JjError> {
        self.push_bookmark_calls
            .borrow_mut()
            .push(branch.to_owned());
        let mut push = self.push.clone();
        push.branch = branch.to_owned();
        Ok(push)
    }

    fn advance_trunk_for_sync(
        &self,
        _context: &RepositoryContext,
    ) -> Result<AdvanceTrunkOutcome, JjError> {
        self.advance_trunk_calls
            .set(self.advance_trunk_calls.get() + 1);
        Ok(self.advance_trunk.clone())
    }

    fn push_tracked(&self, context: &RepositoryContext) -> Result<TrackedPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        if self.up_to_date_sync_roots.contains(&context.workspace_root) {
            return Ok(TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            });
        }
        Ok(self.tracked_push.clone())
    }

    fn changed_files_for_tracked_push(
        &self,
        _context: &RepositoryContext,
    ) -> Result<Vec<String>, JjError> {
        Ok(self.tracked_changed_files.clone())
    }

    fn changed_files_for_bookmarks(
        &self,
        _context: &RepositoryContext,
        branches: &[String],
    ) -> Result<Vec<String>, JjError> {
        self.changed_file_bookmark_requests
            .borrow_mut()
            .push(branches.to_vec());
        Ok(self.bookmark_changed_files.clone())
    }

    fn push_syncable_revision(
        &self,
        context: &RepositoryContext,
        revision: Option<&str>,
    ) -> Result<SyncPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        self.push_syncable_revision_requests
            .borrow_mut()
            .push(revision.map(str::to_owned));
        Ok(SyncPushOutcome {
            pushed: self.tracked_push.clone(),
            skipped_conflicted_bookmarks: self.sync_conflicted_bookmarks.clone(),
        })
    }

    fn push_syncable_tracked(
        &self,
        context: &RepositoryContext,
    ) -> Result<SyncPushOutcome, JjError> {
        self.push_tracked_roots
            .borrow_mut()
            .push(context.workspace_root.clone());
        let remaining_rejections = self.push_rejections_before_success.get();
        if remaining_rejections > 0 {
            self.push_rejections_before_success
                .set(remaining_rejections - 1);
            return Err(JjError::PushRejected {
                branch: "tracked bookmarks".to_owned(),
                message: "lease rejected".to_owned(),
            });
        }
        let pushed = if self.up_to_date_sync_roots.contains(&context.workspace_root) {
            TrackedPushOutcome {
                pushed_refs: 0,
                bookmarks: Vec::new(),
                pushed_commits: Vec::new(),
            }
        } else {
            self.tracked_push.clone()
        };
        Ok(SyncPushOutcome {
            pushed,
            skipped_conflicted_bookmarks: self.sync_conflicted_bookmarks.clone(),
        })
    }

    fn sync_pull_requests(
        &self,
        _context: &RepositoryContext,
        push: &TrackedPushOutcome,
        stack_metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, WorkflowError> {
        self.sync_pull_request_pushes
            .borrow_mut()
            .push(push.clone());
        self.sync_pull_request_metadata
            .borrow_mut()
            .push(stack_metadata.clone());
        Ok(self.sync_pull_requests.clone())
    }

    fn pull_request_plan(
        &self,
        _context: &RepositoryContext,
        workspace: WorkspaceFacts,
        task_id: Option<String>,
        labels: Vec<String>,
        readiness: PullRequestReadiness,
    ) -> Result<PullRequestPlan, WorkflowError> {
        self.pull_request_plan_task_ids
            .borrow_mut()
            .push(task_id.clone());
        if let Some(expected) = &self.expected_task_id {
            assert_eq!(&task_id, expected);
        }
        let short = workspace
            .target_change
            .change_id
            .chars()
            .take(8)
            .collect::<String>();
        let branch = match task_id.as_deref() {
            Some(task_id) => format!(
                "example-user/{task_id}-{stack_index:02}-{short}",
                stack_index = workspace.stack_index,
            ),
            None => format!(
                "example-user/{stack_index:02}-{short}",
                stack_index = workspace.stack_index,
            ),
        };

        let (title, body) = fake_pull_request_description(&workspace.target_change.description);

        let existing_pull_request = self
            .pull_requests_by_head
            .get(&branch)
            .cloned()
            .or_else(|| self.existing_pull_request.clone());
        let draft = readiness.desired_draft(existing_pull_request.as_ref());

        Ok(PullRequestPlan {
            repository: self.check.repository.clone(),
            task_id,
            bookmark: BookmarkPlan {
                branch: branch.clone(),
                action: BookmarkAction::Create,
            },
            target_commit_id: workspace.target_change.commit_id.clone(),
            title,
            body,
            changed_files: workspace.changed_files,
            change_lines: workspace.change_lines,
            base: workspace
                .nearest_ancestor_bookmark
                .clone()
                .unwrap_or(workspace.trunk.branch),
            base_pull_request: workspace
                .nearest_ancestor_bookmark
                .as_deref()
                .and_then(|branch| self.pull_requests_by_head.get(branch).cloned()),
            head: PullRequestHead::same_repository("example-owner", branch),
            labels,
            draft,
            existing_pull_request,
            reviewer_candidates: self.reviewer_candidates.clone(),
            reviewers: ReviewerSelection::default(),
        })
    }

    fn publish_pull_request(
        &self,
        _context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
        options: PullRequestPublishOptions,
    ) -> Result<PullRequestReport, WorkflowError> {
        self.assert_publish_plan(&plan, self.published_pull_request_count.get() as usize);
        self.published_plans.borrow_mut().push(plan.clone());

        let number = 42 + self.published_pull_request_count.get();
        self.published_pull_request_count
            .set(self.published_pull_request_count.get() + 1);
        let pull_request = self.pull_request_record_for_plan(&plan, number);

        Ok(PullRequestReport {
            repository: plan.repository,
            task_id: plan.task_id,
            bookmark: plan.bookmark,
            bookmark_update,
            push,
            action: self.pull_request_action,
            pull_request,
            base: plan.base,
            base_pull_request: plan.base_pull_request,
            head: plan.head,
            labels: None,
            reviewers: None,
            event_effects: if options.event_handlers {
                self.pull_request_event_effects.clone()
            } else {
                Vec::new()
            },
        })
    }

    fn publish_pull_request_metadata_only(
        &self,
        _context: &RepositoryContext,
        plan: PullRequestPlan,
        bookmark_update: BookmarkUpdate,
        push: PushOutcome,
    ) -> Result<PullRequestReport, WorkflowError> {
        self.assert_publish_plan(&plan, self.metadata_only_pull_request_count.get() as usize);
        self.metadata_only_plans.borrow_mut().push(plan.clone());

        let number = plan
            .existing_pull_request
            .as_ref()
            .map(|pull_request| pull_request.number)
            .unwrap_or(42 + self.metadata_only_pull_request_count.get());
        self.metadata_only_pull_request_count
            .set(self.metadata_only_pull_request_count.get() + 1);
        let mut pull_request = self.pull_request_record_for_plan(&plan, number);
        if let Some(existing) = &plan.existing_pull_request {
            pull_request.title = existing.title.clone();
            pull_request.body = existing.body.clone();
            pull_request.base_branch = existing.base_branch.clone();
            pull_request.html_url = existing.html_url.clone();
            pull_request.reviewers = existing.reviewers.clone();
            pull_request.merged = existing.merged;
        }

        Ok(PullRequestReport {
            repository: plan.repository,
            task_id: plan.task_id,
            bookmark: plan.bookmark,
            bookmark_update,
            push,
            action: PullRequestAction::Updated,
            pull_request,
            base: plan.base,
            base_pull_request: plan.base_pull_request,
            head: plan.head,
            labels: (!plan.labels.is_empty()).then_some(LabelApplyResult {
                labels: plan.labels.clone(),
            }),
            reviewers: (!plan.reviewers.is_empty()).then_some(ReviewerSyncResult {
                requested_users: plan.reviewers.users.clone(),
                requested_teams: plan.reviewers.teams.clone(),
                removed_users: Vec::new(),
                removed_teams: Vec::new(),
            }),
            event_effects: Vec::new(),
        })
    }
}

fn create_jj_workspace_marker(root: &Path) {
    fs::create_dir_all(root.join(".jj")).expect("create jj workspace marker");
}

fn project_workspaces(workspace: &TestWorkspace) -> Vec<WorkspaceEntry> {
    vec![
        WorkspaceEntry {
            name: "default".to_owned(),
            root: workspace.home.join("projects/jx"),
            is_current: true,
        },
        WorkspaceEntry {
            name: "fix".to_owned(),
            root: workspace.home.join("projects/.work/jx/fix"),
            is_current: false,
        },
    ]
}

fn expected_workspace_status() -> String {
    "Working copy  (@) : a1b2c3d4 abcdef12\nParent commit (@-): 11112222 33334444 main | parent change\n\nexample change\n\nM src/main.rs\n".to_owned()
}

fn workspace_status() -> WorkspaceStatus {
    WorkspaceStatus {
        commit_lines: vec![
            "Working copy  (@) : a1b2c3d4 abcdef12".to_owned(),
            "Parent commit (@-): 11112222 33334444 main | parent change".to_owned(),
        ],
        description: "example change".to_owned(),
        change_lines: vec!["M src/main.rs".to_owned()],
        extra_lines: Vec::new(),
    }
}

fn workspace_facts() -> WorkspaceFacts {
    WorkspaceFacts {
        workspace_root: "/workspace".into(),
        target_change: ChangeSummary {
            change_id: "zzzzzzzz".to_owned(),
            commit_id: "a1b2c3d4e5f6".to_owned(),
            short_commit_id: "a1b2c3d4".to_owned(),
            description: "example change".to_owned(),
            is_empty: false,
        },
        trunk: TrunkSummary {
            branch: "main".to_owned(),
            commit_id: "1111222233334444".to_owned(),
            short_commit_id: "11112222".to_owned(),
        },
        trunk_git_commit_sha: "1111222233334444".to_owned(),
        origin_branch: "main".to_owned(),
        local_bookmarks: Vec::new(),
        local_bookmarks_at_target: Vec::new(),
        nearest_ancestor_bookmark: Some("example-user/01-ancestor".to_owned()),
        changed_files: vec!["src/main.rs".to_owned()],
        change_lines: vec!["M src/main.rs".to_owned()],
        stack_index: 2,
    }
}

fn status_workspace_facts() -> StatusWorkspaceFacts {
    StatusWorkspaceFacts {
        remotes: vec![StatusRemoteFacts {
            remote: "origin".to_owned(),
            branch: "main".to_owned(),
            trunk_git_commit_sha: "1111222233334444".to_owned(),
            trunk_short_commit_id: "11112222".to_owned(),
            local_ahead_by: 0,
        }],
    }
}

fn test_settings() -> UserSettings {
    UserSettings::from_config(StackedConfig::with_defaults()).expect("test settings")
}

fn fake_pull_request_description(description: &str) -> (String, String) {
    let trimmed = description.trim();
    let title = trimmed
        .lines()
        .find_map(|line| {
            let line = line.trim();
            (!line.is_empty()).then_some(line.to_owned())
        })
        .unwrap_or_else(|| "example change".to_owned());
    let body = trimmed
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .skip(1)
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    (title, body)
}

fn test_config_remotes(contents: &str) -> Vec<(String, String)> {
    let mut current_remote = None;
    let mut remotes = Vec::new();

    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current_remote = line
                .strip_prefix(r#"[remote "#)
                .and_then(|section| section.strip_prefix('"'))
                .and_then(|section| section.strip_suffix(r#""]"#))
                .map(str::to_owned);
            continue;
        }
        let Some(remote_name) = current_remote.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        if key.trim() == "url" {
            remotes.push((
                remote_name.to_owned(),
                value.trim().trim_matches('"').to_owned(),
            ));
        }
    }

    remotes
}

struct TestWorkspace {
    home: PathBuf,
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        Self::new_under("")
    }

    fn new_under(relative_path: &str) -> Self {
        let workspace = Self::new_uninitialized_under(relative_path);
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, &workspace.root))
            .expect("initialize jj workspace");
        workspace
    }

    fn new_uninitialized_under(relative_path: &str) -> Self {
        let unique = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let home =
            std::env::temp_dir().join(format!("jx-command-test-{}-{unique}", std::process::id()));
        let root = if relative_path.is_empty() {
            home.clone()
        } else {
            home.join(relative_path)
        };
        fs::create_dir_all(&root).expect("create workspace root");
        Self { home, root }
    }

    fn path(&self) -> PathBuf {
        self.root.clone()
    }

    fn home_environment(&self) -> [(String, String); 2] {
        [
            ("HOME".to_owned(), self.home.to_string_lossy().into_owned()),
            ("JX_PERF_LOG".to_owned(), "off".to_owned()),
        ]
    }

    fn write_file(&self, relative_path: &str, contents: &str) {
        let path = self.root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write test file");
    }

    fn write_home_file(&self, relative_path: &str, contents: &str) {
        let path = self.home.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, contents).expect("write home test file");
    }

    fn create_jj_workspace(&self, relative_path: &str) -> PathBuf {
        let root = self.home.join(relative_path);
        fs::create_dir_all(&root).expect("create workspace root");
        let settings = test_settings();
        pollster::block_on(Workspace::init_internal_git(&settings, &root))
            .expect("initialize jj workspace");
        root
    }

    fn write_git_config(&self, contents: &str) {
        Self::write_git_config_at(&self.root, contents);
    }

    fn write_git_config_at(root: &Path, contents: &str) {
        for (name, url) in test_config_remotes(contents) {
            let settings = test_settings();
            let store_factories = StoreFactories::default();
            let working_copy_factories = default_working_copy_factories();
            let workspace =
                Workspace::load(&settings, root, &store_factories, &working_copy_factories)
                    .expect("load jj workspace");
            let repo =
                pollster::block_on(workspace.repo_loader().load_at_head()).expect("load jj repo");
            let mut tx = repo.start_transaction();

            git::add_remote(
                tx.repo_mut(),
                RemoteName::new(&name),
                &url,
                None,
                gix::remote::fetch::Tags::None,
            )
            .expect("add remote");
            pollster::block_on(tx.commit(format!("arrange test remote {name}")))
                .expect("commit remote");
        }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}
