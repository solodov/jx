use super::*;
use crate::domain::{apply_review_request_status_policy, review_request_state, ReviewRequestState};
use clap::error::ErrorKind;
use globset::{Glob, GlobMatcher};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

const REVIEW_DISMISSALS_LOG_FILE: &str = "review-dismissals.log";
const REVIEW_DISMISSALS_LEGACY_LOG_FILE: &str = "review-dismissals.log.jsonl";
const REVIEW_DISMISSAL_LOG_VERSION: u32 = 1;

struct ReviewRepositoryLayout {
    key: String,
    root: PathBuf,
    provider_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ReviewPullRequestKey {
    repository: String,
    number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewDismissal {
    repository: String,
    number: u64,
    source: ReviewDismissalSource,
    reason: String,
    latest_commit_oid: String,
    viewer_response_at: Option<String>,
    approval_reviewer: Option<String>,
    dismissed_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ReviewDismissalSource {
    #[default]
    Manual,
    Automatic,
}

#[derive(serde::Serialize)]
struct ReviewDismissalLogEvent {
    version: u32,
    at: String,
    action: &'static str,
    reason: &'static str,
    source: &'static str,
    repository: String,
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dismissed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dismissed_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dismissed_head_oid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_head_oid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dismissed_viewer_response_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_viewer_response_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_reviewer: Option<String>,
}

fn run_review_dashboard(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
) -> Result<CommandResult, CommandError> {
    let environment = environment.clone();
    let loader_request = request.clone();
    let loader: DashboardFrameLoader = std::sync::Arc::new(move || {
        load_review_dashboard_snapshot(loader_request.clone(), &environment)
            .map_err(|error| error.to_string())
    });
    run_interactive_dashboard(request.refresh_seconds, loader)
}

fn load_review_dashboard_snapshot(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
) -> Result<DashboardFrameSnapshot, CommandError> {
    let services = ProductionServices::new(environment)?;
    let progress = SilentProgress;
    let perf = PerfLog::from_environment(environment);
    let mut span = perf.start(
        "review.dashboard_frame",
        [perf_attr("filter_count", request.repo_filters.len())],
    );
    let result = (|| {
        let loaded = load_review_requests_view(
            &request,
            environment,
            &services,
            &progress,
            &mut span,
            ReviewDismissalMode::Apply,
        )?;
        Ok::<_, CommandError>(DashboardFrameSnapshot::new(move |options| {
            Ok(render_review_requests(
                &loaded.view,
                options.color,
                options.terminal_width,
                PullRequestTableLayout::FitTerminal,
                &loaded.display_names,
            ))
        }))
    })();
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

pub(super) fn handle_review(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
) -> Result<String, CommandError> {
    match &request.action {
        ReviewAction::Dismiss { selector, until } => {
            return handle_review_dismiss(selector, until, environment, services, progress);
        }
        ReviewAction::Undismiss { selector } => {
            return handle_review_undismiss(selector, environment, services, progress);
        }
        ReviewAction::History { selector } => {
            return handle_review_history(selector, environment, request.format);
        }
        ReviewAction::Show | ReviewAction::Dismissed => {}
    }
    if request.interactive {
        return run_review_dashboard(request, environment).map(|result| result.stdout);
    }

    let perf = PerfLog::from_environment(environment);
    let mut span = perf.start(
        "review.run",
        [
            perf_attr("filter_count", request.repo_filters.len()),
            perf_attr("format", review_format_name(request.format)),
        ],
    );
    let result = handle_review_traced(request, environment, services, progress, output, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_review_traced(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    output: OutputMode,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let format = request.format;
    let dismissal_mode = match request.action {
        ReviewAction::Dismissed => ReviewDismissalMode::Only,
        ReviewAction::Show
        | ReviewAction::Dismiss { .. }
        | ReviewAction::History { .. }
        | ReviewAction::Undismiss { .. } => ReviewDismissalMode::Apply,
    };
    let loaded = load_review_requests_view(
        &request,
        environment,
        services,
        progress,
        span,
        dismissal_mode,
    )?;
    progress.finish();

    span.measure("review.render", Vec::new(), || {
        Ok::<_, CommandError>(match format {
            ReviewFormat::Human => render_review_requests(
                &loaded.view,
                output.color,
                output.terminal_width,
                PullRequestTableLayout::Flow,
                &loaded.display_names,
            ),
            ReviewFormat::Json => render_review_requests_json(&loaded.view, &loaded.display_names),
        })
    })
}

struct LoadedReviewRequestsView {
    view: ReviewRequestsView,
    display_names: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewDismissalMode {
    Apply,
    Ignore,
    Only,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewCleanupMode {
    Record,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewDecision {
    state: ReviewRequestState,
    visible: bool,
    action_resurface_reason: Option<&'static str>,
    automatic_hide_reason: Option<&'static str>,
    viewer_signal: ReviewRequestViewerSignal,
    visible_since_unix: Option<i64>,
    dismissal: Option<ReviewRequestDismissalView>,
}

struct ReviewDecisionInput<'a> {
    status: &'a PullRequestStatusRecord,
    history: &'a [PullRequestHistoryRecord],
    actions: &'a [PullRequestActionRecord],
    viewer: &'a str,
    dismissal_mode: ReviewDismissalMode,
}

struct ReviewViewContext<'a> {
    environment: &'a RuntimeEnvironment,
    config: &'a WorkflowConfig,
    layout_by_repository: &'a BTreeMap<GitHubRepository, ReviewRepositoryLayout>,
    dismissal_mode: ReviewDismissalMode,
}

fn load_review_requests_view(
    request: &ReviewRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
    dismissal_mode: ReviewDismissalMode,
) -> Result<LoadedReviewRequestsView, CommandError> {
    progress.status("Loading review context…");
    let config = span.measure("review.discover_config", Vec::new(), || {
        WorkflowConfig::discover_for_clone(environment)
    })?;
    let token_source = TokenSource::discover(environment, &config);
    let layout_repositories = span.measure_with_result_attrs(
        "review.discover_layout",
        Vec::new(),
        || global_work_repositories(&config, environment),
        |result| {
            result
                .as_ref()
                .map(|repositories| vec![perf_attr("layout_repo_count", repositories.len())])
                .unwrap_or_default()
        },
    )?;
    let layout_by_repository = layout_repositories
        .iter()
        .map(|repository| {
            (
                repository.github_repository(),
                ReviewRepositoryLayout {
                    key: repository.key.clone(),
                    root: repository.root.clone(),
                    provider_slug: repository.provider_slug(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let filter_matchers = review_filter_matchers(&request.repo_filters)?;
    let view_context = ReviewViewContext {
        environment,
        config: &config,
        layout_by_repository: &layout_by_repository,
        dismissal_mode,
    };
    if request.cached {
        return load_cached_review_requests_view(
            &view_context,
            services,
            progress,
            span,
            &filter_matchers,
        );
    }

    progress.status("Loading review requests…");
    let inbox = match span.measure_with_result_attrs(
        "review.fetch_candidates",
        Vec::new(),
        || services.review_requests(&token_source),
        review_candidate_result_attrs,
    ) {
        Ok(inbox) => inbox,
        Err(error) if review_candidate_search_saml_enforced(&error) => {
            span.set([perf_attr("candidate_search_saml_enforced", true)]);
            let fallback_repositories =
                review_candidate_fallback_repositories(&layout_by_repository, &filter_matchers);
            span.measure_with_result_attrs(
                "review.fetch_configured_candidates",
                [perf_attr("repo_count", fallback_repositories.len())],
                || services.review_requests_for_repositories(&token_source, &fallback_repositories),
                review_candidate_result_attrs,
            )?
        }
        Err(error) => return Err(error.into()),
    };
    let inbox_for_cache = inbox.clone();
    let viewer = inbox.viewer.login;
    let (candidate_keys, mut grouped) =
        group_review_candidates(inbox.requests, &layout_by_repository, &filter_matchers);

    if dismissal_mode != ReviewDismissalMode::Ignore {
        add_action_dismissed_pull_requests_to_review_fetch(
            environment,
            &mut grouped,
            &layout_by_repository,
            &filter_matchers,
        )?;
    }
    span.set([perf_attr("filtered_repo_count", grouped.len())]);

    let grouped_repo_count = grouped.len();
    if grouped_repo_count > 0 {
        progress.percentage("Loading pull request details", 0, grouped_repo_count);
    }
    let mut repositories = Vec::new();
    let mut detail_pr_count = 0usize;
    let fetch_details = span.start_step(
        "review.fetch_pull_request_details",
        [perf_attr("repo_count", grouped_repo_count)],
    );
    for (index, (repository, mut numbers)) in grouped.into_iter().enumerate() {
        numbers.sort_unstable();
        numbers.dedup();
        detail_pr_count += numbers.len();
        let pull_requests = services.pull_requests_with_history_for_repository(
            &token_source,
            &repository,
            &numbers,
        )?;
        if let Some(repository_view) = build_review_repository_view(
            &view_context,
            &repository,
            pull_requests,
            &candidate_keys,
            &viewer,
            ReviewCleanupMode::Record,
        )? {
            repositories.push(repository_view);
            progress.percentage(
                "Loading pull request details",
                index + 1,
                grouped_repo_count,
            );
        }
    }
    span.finish_step(
        fetch_details,
        [perf_attr("detail_pr_count", detail_pr_count)],
        Option::<&WorkflowError>::None,
    );
    span.measure(
        "review.record_candidates",
        [
            perf_attr("candidate_count", inbox_for_cache.requests.len()),
            perf_attr("viewer", &inbox_for_cache.viewer.login),
        ],
        || {
            PullRequestStore::open(environment)?.record_review_inbox_snapshot(&inbox_for_cache)?;
            Ok::<(), CommandError>(())
        },
    )?;

    let view = review_requests_view(viewer, repositories, span)?;
    let display_names = load_review_display_names(
        &view,
        environment,
        services,
        Some(&token_source),
        progress,
        span,
        false,
    )?;

    Ok(LoadedReviewRequestsView {
        view,
        display_names,
    })
}

fn load_cached_review_requests_view(
    context: &ReviewViewContext<'_>,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
    filter_matchers: &[ReviewFilterMatcher],
) -> Result<LoadedReviewRequestsView, CommandError> {
    progress.status("Loading cached review requests…");
    let store = span.measure("review.open_store", Vec::new(), || {
        PullRequestStore::open(context.environment)
    })?;
    let inbox = span.measure("review.load_cached_candidates", Vec::new(), || {
        store.latest_review_inbox_snapshot()
    })?;
    let inbox = inbox.ok_or_else(|| CommandError::Check {
        message: "no cached review inbox is available; run `jx review` without --cached first"
            .to_owned(),
    })?;
    span.set([
        perf_attr("cached", true),
        perf_attr("cached_candidate_count", inbox.requests.len()),
        perf_attr("cached_observed_at_unix", inbox.observed_at_unix),
        perf_attr("viewer", &inbox.viewer),
    ]);

    let viewer = inbox.viewer;
    let (candidate_keys, mut grouped) = group_review_candidates(
        inbox.requests,
        context.layout_by_repository,
        filter_matchers,
    );
    if context.dismissal_mode != ReviewDismissalMode::Ignore {
        add_action_dismissed_pull_requests_to_review_fetch(
            context.environment,
            &mut grouped,
            context.layout_by_repository,
            filter_matchers,
        )?;
    }
    span.set([perf_attr("filtered_repo_count", grouped.len())]);

    let grouped_repo_count = grouped.len();
    if grouped_repo_count > 0 {
        progress.percentage("Loading cached pull request details", 0, grouped_repo_count);
    }
    let mut repositories = Vec::new();
    let mut detail_pr_count = 0usize;
    let load_details = span.start_step(
        "review.load_cached_pull_request_details",
        [perf_attr("repo_count", grouped_repo_count)],
    );
    for (index, (repository, mut numbers)) in grouped.into_iter().enumerate() {
        numbers.sort_unstable();
        numbers.dedup();
        detail_pr_count += numbers.len();
        let pull_requests = store.latest_pull_requests_with_history(&repository, &numbers)?;
        if let Some(repository_view) = build_review_repository_view(
            context,
            &repository,
            pull_requests,
            &candidate_keys,
            &viewer,
            ReviewCleanupMode::ReadOnly,
        )? {
            repositories.push(repository_view);
            progress.percentage(
                "Loading cached pull request details",
                index + 1,
                grouped_repo_count,
            );
        }
    }
    span.finish_step(
        load_details,
        [perf_attr("detail_pr_count", detail_pr_count)],
        Option::<&WorkflowError>::None,
    );

    let view = review_requests_view(viewer, repositories, span)?;
    let display_names = load_review_display_names(
        &view,
        context.environment,
        services,
        None,
        progress,
        span,
        true,
    )?;

    Ok(LoadedReviewRequestsView {
        view,
        display_names,
    })
}

fn group_review_candidates(
    candidates: Vec<PullRequestReviewRequest>,
    layout_by_repository: &BTreeMap<GitHubRepository, ReviewRepositoryLayout>,
    filter_matchers: &[ReviewFilterMatcher],
) -> (
    BTreeSet<ReviewPullRequestKey>,
    BTreeMap<GitHubRepository, Vec<u64>>,
) {
    let mut candidate_keys = BTreeSet::new();
    let mut grouped = BTreeMap::<GitHubRepository, Vec<u64>>::new();
    for candidate in candidates {
        let layout = layout_by_repository.get(&candidate.repository);
        if review_repository_matches(&candidate.repository, layout, filter_matchers) {
            candidate_keys.insert(review_key(&candidate.repository, candidate.number));
            grouped
                .entry(candidate.repository)
                .or_default()
                .push(candidate.number);
        }
    }
    (candidate_keys, grouped)
}

fn build_review_repository_view(
    context: &ReviewViewContext<'_>,
    repository: &GitHubRepository,
    pull_requests: Vec<PullRequestWithHistory>,
    candidate_keys: &BTreeSet<ReviewPullRequestKey>,
    viewer: &str,
    cleanup_mode: ReviewCleanupMode,
) -> Result<Option<ReviewRequestRepositoryView>, CommandError> {
    let stack_status_policy = context.config.repo.stack_status_for(repository);
    let review_policy = context.config.repo.review_for(repository);
    let mut rows = Vec::new();
    for pull_request in pull_requests.into_iter().map(|mut pull_request| {
        pull_request.status = apply_review_request_status_policy(
            pull_request.status,
            &stack_status_policy,
            &review_policy,
        );
        pull_request
    }) {
        let status = pull_request.status;
        let key = review_key(repository, status.number);
        if status.author.as_deref() == Some(viewer) || !candidate_keys.contains(&key) {
            if cleanup_mode == ReviewCleanupMode::Record {
                let reason = if status.author.as_deref() == Some(viewer) {
                    "authored_by_viewer"
                } else {
                    "left_review_inbox"
                };
                record_review_action_cleanup(
                    context.environment,
                    repository,
                    &status,
                    &pull_request.actions,
                    viewer,
                    reason,
                )?;
            }
            continue;
        }
        let decision = decide_review_request(ReviewDecisionInput {
            status: &status,
            history: &pull_request.history,
            actions: &pull_request.actions,
            viewer,
            dismissal_mode: context.dismissal_mode,
        });
        if let Some(reason) = decision.action_resurface_reason {
            if cleanup_mode == ReviewCleanupMode::Record {
                record_review_action_cleanup(
                    context.environment,
                    repository,
                    &status,
                    &pull_request.actions,
                    viewer,
                    reason,
                )?;
            }
        }
        if !decision.visible {
            continue;
        }
        rows.push(ReviewRequestRowView {
            state: decision.state,
            status,
            viewer_signal: decision.viewer_signal,
            lag_since_unix: decision.visible_since_unix,
            dismissal: decision.dismissal,
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.status.number));
    if rows.is_empty() {
        return Ok(None);
    }
    let layout = context.layout_by_repository.get(repository);
    Ok(Some(ReviewRequestRepositoryView {
        repository: repository.clone(),
        layout_key: layout.map(|layout| layout.key.clone()),
        root: layout.map(|layout| layout.root.clone()),
        display_root: layout.map(|layout| display_path(&layout.root, context.environment)),
        external: layout.is_none(),
        review_wait_threshold_seconds: stack_status_policy.review_wait_threshold_seconds,
        rows,
    }))
}

fn review_requests_view(
    viewer: String,
    mut repositories: Vec<ReviewRequestRepositoryView>,
    span: &mut PerfSpan,
) -> Result<ReviewRequestsView, CommandError> {
    span.measure("review.group_repositories", Vec::new(), || {
        sort_review_repositories(&mut repositories);
        Ok::<(), CommandError>(())
    })?;
    span.set([
        perf_attr("repo_count", repositories.len()),
        perf_attr(
            "external_repo_count",
            repositories
                .iter()
                .filter(|repository| repository.external)
                .count(),
        ),
        perf_attr(
            "detail_pr_count",
            repositories
                .iter()
                .map(|repository| repository.rows.len())
                .sum::<usize>(),
        ),
    ]);

    Ok(ReviewRequestsView {
        viewer,
        repositories,
    })
}

fn load_review_display_names(
    view: &ReviewRequestsView,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    token_source: Option<&TokenSource>,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
    cached: bool,
) -> Result<BTreeMap<String, String>, CommandError> {
    let mut display_name_logins = review_request_user_logins(view);
    if !view.repositories.is_empty() {
        display_name_logins.push(view.viewer.clone());
    }
    span.measure_with_result_attrs(
        "review.load_display_names",
        [
            perf_attr("login_count", display_name_logins.len()),
            perf_attr("cached", cached),
        ],
        || {
            Ok::<_, CommandError>(if display_name_logins.is_empty() {
                BTreeMap::new()
            } else if cached {
                cached_review_display_names(environment, &display_name_logins)?
            } else {
                progress.status("Loading reviewer names…");
                services.github_user_display_names(
                    token_source.expect("live review display names have a token source"),
                    &display_name_logins,
                )
            })
        },
        |result| {
            result
                .as_ref()
                .map(|display_names| vec![perf_attr("display_name_count", display_names.len())])
                .unwrap_or_default()
        },
    )
}

fn cached_review_display_names(
    environment: &RuntimeEnvironment,
    logins: &[String],
) -> Result<BTreeMap<String, String>, CommandError> {
    let cache = read_github_user_name_cache(environment)?;
    Ok(logins
        .iter()
        .filter_map(|login| {
            cache
                .cached_name(login)
                .flatten()
                .map(|name| (login.clone(), name))
        })
        .collect())
}

fn review_candidate_search_saml_enforced(error: &WorkflowError) -> bool {
    matches!(
        error,
        WorkflowError::GitHub(error)
            if error.is_graphql_saml_enforcement_for("search review requests")
    )
}

fn review_candidate_result_attrs(
    result: &Result<PullRequestReviewRequests, WorkflowError>,
) -> Vec<PerfAttr> {
    result
        .as_ref()
        .map(|inbox| {
            let repo_count = inbox
                .requests
                .iter()
                .map(|request| &request.repository)
                .collect::<BTreeSet<_>>()
                .len();
            vec![
                perf_attr("candidate_count", inbox.requests.len()),
                perf_attr("repo_count", repo_count),
                perf_attr("viewer", &inbox.viewer.login),
            ]
        })
        .unwrap_or_default()
}

fn review_candidate_fallback_repositories(
    layout_by_repository: &BTreeMap<GitHubRepository, ReviewRepositoryLayout>,
    filter_matchers: &[ReviewFilterMatcher],
) -> Vec<GitHubRepository> {
    layout_by_repository
        .iter()
        .filter(|(repository, layout)| {
            review_repository_matches(repository, Some(*layout), filter_matchers)
        })
        .map(|(repository, _)| repository.clone())
        .collect()
}

fn review_request_user_logins(view: &ReviewRequestsView) -> Vec<String> {
    pull_request_status_user_logins(
        view.repositories
            .iter()
            .flat_map(|repository| repository.rows.iter())
            .map(|row| &row.status),
    )
}

struct ReviewFilterMatcher {
    raw: String,
    glob: Option<GlobMatcher>,
}

fn review_filter_matchers(filters: &[String]) -> Result<Vec<ReviewFilterMatcher>, CommandError> {
    filters
        .iter()
        .map(|filter| {
            Ok(ReviewFilterMatcher {
                raw: filter.clone(),
                glob: if has_glob_meta(filter) {
                    Some(
                        Glob::new(filter)
                            .map_err(|error| {
                                CommandError::Usage(clap::Error::raw(
                                    ErrorKind::InvalidValue,
                                    format!("invalid repository filter `{filter}`: {error}"),
                                ))
                            })?
                            .compile_matcher(),
                    )
                } else {
                    None
                },
            })
        })
        .collect()
}

fn review_repository_matches(
    repository: &GitHubRepository,
    layout: Option<&ReviewRepositoryLayout>,
    filters: &[ReviewFilterMatcher],
) -> bool {
    if filters.is_empty() {
        return true;
    }
    let labels = review_repository_filter_labels(repository, layout);
    filters.iter().any(|filter| {
        labels.iter().any(|label| {
            filter
                .glob
                .as_ref()
                .map_or_else(|| label.contains(&filter.raw), |glob| glob.is_match(label))
        })
    })
}

fn review_repository_filter_labels(
    repository: &GitHubRepository,
    layout: Option<&ReviewRepositoryLayout>,
) -> Vec<String> {
    let mut labels = vec![
        repository.slug(),
        repository.name.clone(),
        repository.https_url(),
        format!("github.com/{}", repository.slug()),
    ];
    if let Some(layout) = layout {
        labels.push(layout.key.clone());
        labels.push(layout.provider_slug.clone());
    }
    labels
}

fn sort_review_repositories(repositories: &mut [ReviewRequestRepositoryView]) {
    repositories.sort_by(|left, right| match (left.external, right.external) {
        (false, true) => std::cmp::Ordering::Less,
        (true, false) => std::cmp::Ordering::Greater,
        _ => review_repository_sort_key(left).cmp(&review_repository_sort_key(right)),
    });
}

fn review_repository_sort_key(repository: &ReviewRequestRepositoryView) -> String {
    repository
        .layout_key
        .clone()
        .unwrap_or_else(|| repository.repository.slug())
}

fn has_glob_meta(value: &str) -> bool {
    value.contains(['*', '?', '[', '{'])
}

fn handle_review_dismiss(
    selector: &str,
    until: &ReviewDismissUntil,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment)
        .start("review.dismiss", [perf_attr("selector", selector)]);
    let result =
        handle_review_dismiss_traced(selector, until, environment, services, progress, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_review_dismiss_traced(
    selector: &str,
    until: &ReviewDismissUntil,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let request = ReviewRequest {
        action: ReviewAction::Show,
        repo_filters: Vec::new(),
        interactive: false,
        refresh_seconds: 300,
        format: ReviewFormat::Human,
        cached: false,
    };
    let loaded = load_review_requests_view(
        &request,
        environment,
        services,
        progress,
        span,
        ReviewDismissalMode::Ignore,
    )?;
    progress.finish();

    let target = parse_review_dismiss_target(selector)?;
    let matches = review_dismiss_matches(&loaded.view, &target);
    let (repository, row) = match matches.as_slice() {
        [(repository, row)] => (repository, row),
        [] => {
            return Err(review_dismiss_usage_error(format!(
                "no review pull request matched `{selector}`"
            )));
        }
        matches => {
            let choices = matches
                .iter()
                .map(|(repository, row)| format!("{}#{}", repository.slug(), row.status.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(review_dismiss_usage_error(format!(
                "review pull request `{selector}` is ambiguous; matches: {choices}; use a longer repo suffix such as owner/repo#number"
            )));
        }
    };
    let status = &row.status;

    let Some(latest_commit_oid) = status.latest_commit_oid.clone() else {
        return Err(review_dismiss_usage_error(format!(
            "{}#{} cannot be dismissed because GitHub did not return its latest commit oid",
            repository.slug(),
            status.number
        )));
    };

    let reason = review_manual_dismissal_reason(status, until);
    let approval_reviewer = review_dismissal_approval_reviewer(until);
    let dismissal = ReviewDismissal {
        repository: repository.slug(),
        number: status.number,
        source: ReviewDismissalSource::Manual,
        reason: reason.to_owned(),
        latest_commit_oid,
        viewer_response_at: review_viewer_response_at(status, &loaded.view.viewer)
            .map(str::to_owned),
        approval_reviewer,
        dismissed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    append_review_dismissal_log(
        environment,
        review_dismissal_log_event(
            "dismiss",
            reason,
            &dismissal,
            Some(status),
            &loaded.view.viewer,
            Some(selector),
        ),
    )?;
    record_review_dismissal_action(
        environment,
        "dismiss",
        reason,
        &dismissal,
        Some(status),
        &loaded.view.viewer,
        Some(selector),
    )?;

    let prefix = "Dismissed";
    Ok(format!(
        "{prefix} {} {}\n",
        review_dismiss_pull_request_link(repository, status),
        review_dismissal_until_message(&dismissal),
    ))
}

fn handle_review_undismiss(
    selector: &str,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment)
        .start("review.undismiss", [perf_attr("selector", selector)]);
    let result =
        handle_review_undismiss_traced(selector, environment, services, progress, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_review_undismiss_traced(
    selector: &str,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
) -> Result<String, CommandError> {
    let request = ReviewRequest {
        action: ReviewAction::Dismissed,
        repo_filters: Vec::new(),
        interactive: false,
        refresh_seconds: 300,
        format: ReviewFormat::Human,
        cached: false,
    };
    let loaded = load_review_requests_view(
        &request,
        environment,
        services,
        progress,
        span,
        ReviewDismissalMode::Only,
    )?;
    progress.finish();

    let target = parse_review_dismiss_target(selector)?;
    let matches = review_dismiss_matches(&loaded.view, &target);
    let (repository, row) = match matches.as_slice() {
        [(repository, row)] => (repository, row),
        [] => {
            return Err(review_dismiss_usage_error(format!(
                "no dismissed review pull request matched `{selector}`"
            )));
        }
        matches => {
            let choices = matches
                .iter()
                .map(|(repository, row)| format!("{}#{}", repository.slug(), row.status.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(review_dismiss_usage_error(format!(
                "dismissed review pull request `{selector}` is ambiguous; matches: {choices}; use a longer repo suffix such as owner/repo#number"
            )));
        }
    };
    let status = &row.status;

    let dismissal = review_dismissal_from_row(repository, row, &loaded.view.viewer);
    append_review_dismissal_log(
        environment,
        review_dismissal_log_event(
            "undismiss",
            "manual",
            &dismissal,
            Some(status),
            &loaded.view.viewer,
            Some(selector),
        ),
    )?;
    record_review_dismissal_action(
        environment,
        "undismiss",
        "manual",
        &dismissal,
        Some(status),
        &loaded.view.viewer,
        Some(selector),
    )?;

    Ok(format!(
        "Undismissed {}\n",
        review_dismiss_pull_request_link(repository, status),
    ))
}

fn handle_review_history(
    selector: &str,
    environment: &RuntimeEnvironment,
    format: ReviewFormat,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment)
        .start("review.history", [perf_attr("selector", selector)]);
    let result = handle_review_history_traced(selector, environment, format);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_review_history_traced(
    selector: &str,
    environment: &RuntimeEnvironment,
    format: ReviewFormat,
) -> Result<String, CommandError> {
    let store = PullRequestStore::open(environment)?;
    let target = parse_review_dismiss_target(selector)?;
    let config = WorkflowConfig::discover_for_clone(environment)?;
    let layout_repositories = global_work_repositories(&config, environment)?;
    let layout_by_repository = layout_repositories
        .iter()
        .map(|repository| (repository.github_repository(), repository.key.clone()))
        .collect::<BTreeMap<_, _>>();
    let matches = store
        .stored_pull_request_identities()?
        .into_iter()
        .filter(|identity| {
            review_history_identity_matches(identity, &target, &layout_by_repository)
        })
        .collect::<Vec<_>>();
    let identity = match matches.as_slice() {
        [identity] => identity,
        [] => {
            return Err(review_dismiss_usage_error(format!(
                "no stored pull request matched `{selector}`"
            )));
        }
        matches => {
            let choices = matches
                .iter()
                .map(|identity| format!("{}#{}", identity.repository.slug(), identity.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(review_dismiss_usage_error(format!(
                "stored pull request `{selector}` is ambiguous; matches: {choices}; use a longer repo suffix such as owner/repo#number"
            )));
        }
    };
    let timeline = store
        .stored_pull_request_timeline(&identity.repository, identity.number)?
        .ok_or_else(|| {
            review_dismiss_usage_error(format!("no stored pull request matched `{selector}`"))
        })?;

    Ok(match format {
        ReviewFormat::Human => render_review_history(&timeline),
        ReviewFormat::Json => render_review_history_json(&timeline),
    })
}

fn review_history_identity_matches(
    identity: &StoredPullRequestIdentity,
    target: &ReviewDismissTarget,
    layout_by_repository: &BTreeMap<GitHubRepository, String>,
) -> bool {
    identity.number == target.number
        && review_repository_suffix_matches(
            &identity.repository,
            layout_by_repository
                .get(&identity.repository)
                .map(String::as_str),
            &target.repository_suffix,
        )
}

fn render_review_history(timeline: &StoredPullRequestTimeline) -> String {
    let mut output = String::new();
    let title = timeline
        .status
        .as_ref()
        .map(|status| status.title.as_str())
        .unwrap_or("no snapshot");
    output.push_str(&format!(
        "{}#{} {title}\n",
        timeline.repository.slug(),
        timeline.number
    ));
    for entry in review_history_entries(timeline) {
        output.push_str(&entry.render_human());
    }
    output
}

fn render_review_history_json(timeline: &StoredPullRequestTimeline) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "repository": timeline.repository.slug(),
        "number": timeline.number,
        "title": timeline.status.as_ref().map(|status| status.title.as_str()),
        "entries": review_history_entries(timeline)
            .into_iter()
            .map(|entry| entry.render_json())
            .collect::<Vec<_>>(),
    }))
    .expect("review history JSON serializes")
        + "\n"
}

#[derive(Debug, Clone, Copy)]
enum ReviewHistoryEntry<'a> {
    History(&'a PullRequestHistoryRecord),
    Action(&'a PullRequestActionRecord),
}

impl ReviewHistoryEntry<'_> {
    fn changed_at_unix(self) -> i64 {
        match self {
            Self::History(event) => event.changed_at_unix,
            Self::Action(action) => action.changed_at_unix,
        }
    }

    fn render_human(self) -> String {
        match self {
            Self::History(event) => format!(
                "  {}  history  {}  old={} new={} details={}\n",
                format_review_history_timestamp(event.changed_at_unix),
                event.kind,
                review_history_json_cell(event.old_json.as_ref()),
                review_history_json_cell(event.new_json.as_ref()),
                review_history_json_cell(Some(&event.details_json)),
            ),
            Self::Action(action) => format!(
                "  {}  action   {} source={} reason={} details={}\n",
                format_review_history_timestamp(action.changed_at_unix),
                action.action,
                action.source,
                action.reason.as_deref().unwrap_or("-"),
                review_history_json_cell(Some(&action.details_json)),
            ),
        }
    }

    fn render_json(self) -> serde_json::Value {
        match self {
            Self::History(event) => serde_json::json!({
                "type": "history",
                "kind": event.kind,
                "changedAtUnix": event.changed_at_unix,
                "changedAt": format_review_history_timestamp(event.changed_at_unix),
                "old": event.old_json,
                "new": event.new_json,
                "details": event.details_json,
            }),
            Self::Action(action) => serde_json::json!({
                "type": "action",
                "action": action.action,
                "source": action.source,
                "reason": action.reason,
                "changedAtUnix": action.changed_at_unix,
                "changedAt": format_review_history_timestamp(action.changed_at_unix),
                "details": action.details_json,
            }),
        }
    }
}

fn review_history_entries(timeline: &StoredPullRequestTimeline) -> Vec<ReviewHistoryEntry<'_>> {
    let mut entries = timeline
        .history
        .iter()
        .map(ReviewHistoryEntry::History)
        .chain(timeline.actions.iter().map(ReviewHistoryEntry::Action))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| (entry.changed_at_unix(), review_history_entry_order(*entry)));
    entries
}

fn review_history_entry_order(entry: ReviewHistoryEntry<'_>) -> u8 {
    match entry {
        ReviewHistoryEntry::History(_) => 0,
        ReviewHistoryEntry::Action(_) => 1,
    }
}

fn format_review_history_timestamp(value: i64) -> String {
    chrono::DateTime::from_timestamp(value, 0)
        .map(|timestamp| {
            timestamp
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| value.to_string())
}

fn review_history_json_cell(value: Option<&serde_json::Value>) -> String {
    value
        .map(|value| serde_json::to_string(value).expect("review history JSON value serializes"))
        .unwrap_or_else(|| "null".to_owned())
}

fn review_dismissal_from_row(
    repository: &GitHubRepository,
    row: &ReviewRequestRowView,
    viewer: &str,
) -> ReviewDismissal {
    let status = &row.status;
    let reason = row
        .dismissal
        .as_ref()
        .map(|dismissal| dismissal.reason.clone())
        .unwrap_or_else(|| {
            review_manual_dismissal_reason(status, &ReviewDismissUntil::Attention).to_owned()
        });
    ReviewDismissal {
        repository: repository.slug(),
        number: status.number,
        source: ReviewDismissalSource::Manual,
        reason,
        latest_commit_oid: status.latest_commit_oid.clone().unwrap_or_default(),
        viewer_response_at: review_viewer_response_at(status, viewer).map(str::to_owned),
        approval_reviewer: None,
        dismissed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    }
}

fn review_manual_dismissal_reason(
    status: &PullRequestStatusRecord,
    until: &ReviewDismissUntil,
) -> &'static str {
    match until {
        ReviewDismissUntil::Attention if status.draft => "draft",
        ReviewDismissUntil::Attention => "manual",
        ReviewDismissUntil::PeerApproval => "peer_approval",
        ReviewDismissUntil::UserApproval { .. } => "user_approval",
    }
}

fn review_dismissal_approval_reviewer(until: &ReviewDismissUntil) -> Option<String> {
    match until {
        ReviewDismissUntil::UserApproval { login } => Some(login.clone()),
        ReviewDismissUntil::Attention | ReviewDismissUntil::PeerApproval => None,
    }
}

fn review_dismissal_until_message(dismissal: &ReviewDismissal) -> String {
    match dismissal.reason.as_str() {
        "draft" => "until it is ready for review, a fresh review request, a mention, or a new author response".to_owned(),
        "peer_approval" => "until another reviewer approves".to_owned(),
        "user_approval" => dismissal
            .approval_reviewer
            .as_ref()
            .map(|reviewer| format!("until {reviewer} approves"))
            .unwrap_or_else(|| "until the selected reviewer approves".to_owned()),
        _ => "until new commits, a fresh review request, or a new author response".to_owned(),
    }
}

fn review_dismiss_pull_request_link(
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
) -> String {
    osc8_link(
        &review_request_url(repository, status),
        &format!("{}#{}", repository.slug(), status.number),
    )
}

fn review_dismiss_matches<'a>(
    view: &'a ReviewRequestsView,
    target: &ReviewDismissTarget,
) -> Vec<(&'a GitHubRepository, &'a ReviewRequestRowView)> {
    view.repositories
        .iter()
        .flat_map(|repository| {
            repository
                .rows
                .iter()
                .filter(|row| {
                    row.status.number == target.number
                        && review_dismiss_repository_matches(repository, &target.repository_suffix)
                })
                .map(move |row| (&repository.repository, row))
        })
        .collect()
}

fn review_dismiss_repository_matches(
    repository: &ReviewRequestRepositoryView,
    repository_suffix: &[String],
) -> bool {
    review_repository_suffix_matches(
        &repository.repository,
        repository.layout_key.as_deref(),
        repository_suffix,
    )
}

fn review_repository_suffix_matches(
    repository: &GitHubRepository,
    layout_key: Option<&str>,
    repository_suffix: &[String],
) -> bool {
    if repository_suffix.is_empty() {
        return true;
    }
    review_repository_identities(repository, layout_key)
        .iter()
        .any(|identity| review_dismiss_identity_matches(identity, repository_suffix))
}

fn review_repository_identities(
    repository: &GitHubRepository,
    layout_key: Option<&str>,
) -> Vec<Vec<String>> {
    let mut identities = Vec::new();
    if let Some(key) = layout_key {
        identities.push(review_dismiss_repository_components(key));
    }
    identities.push(vec![repository.name.clone()]);
    identities.push(vec![repository.owner.clone(), repository.name.clone()]);
    identities.push(vec![
        "github.com".to_owned(),
        repository.owner.clone(),
        repository.name.clone(),
    ]);
    identities
}

fn review_dismiss_identity_matches(identity: &[String], suffix: &[String]) -> bool {
    identity.len() >= suffix.len()
        && identity[identity.len() - suffix.len()..]
            .iter()
            .zip(suffix)
            .all(|(candidate, selector)| candidate.eq_ignore_ascii_case(selector))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewDismissTarget {
    repository_suffix: Vec<String>,
    number: u64,
}

fn parse_review_dismiss_target(selector: &str) -> Result<ReviewDismissTarget, CommandError> {
    let selector = selector.trim().trim_end_matches('/');
    if selector.is_empty() {
        return Err(review_dismiss_usage_error("pull request selector is empty"));
    }

    if let Some((repository_suffix, number)) = parse_github_pull_request_url(selector) {
        return Ok(ReviewDismissTarget {
            repository_suffix,
            number,
        });
    }
    if let Some((repository, number)) = selector.rsplit_once('#') {
        return Ok(ReviewDismissTarget {
            repository_suffix: review_dismiss_repository_components(repository),
            number: parse_review_pull_request_number(number)?,
        });
    }

    Ok(ReviewDismissTarget {
        repository_suffix: Vec::new(),
        number: parse_review_pull_request_number(selector)?,
    })
}

fn parse_github_pull_request_url(selector: &str) -> Option<(Vec<String>, u64)> {
    let (_, path) = selector.split_once("github.com/")?;
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if parts.next()? != "pull" {
        return None;
    }
    let number = parse_review_pull_request_number(parts.next()?).ok()?;
    Some((
        vec!["github.com".to_owned(), owner.to_owned(), repo.to_owned()],
        number,
    ))
}

fn review_dismiss_repository_components(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_matches('/')
        .split('/')
        .map(str::trim)
        .filter(|component| !component.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_review_pull_request_number(raw: &str) -> Result<u64, CommandError> {
    let raw = raw
        .trim()
        .trim_start_matches('#')
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    raw.parse::<u64>()
        .map_err(|_| review_dismiss_usage_error(format!("invalid pull request number `{raw}`")))
}

fn review_dismiss_usage_error(message: impl Into<String>) -> CommandError {
    CommandError::Usage(clap::Error::raw(ErrorKind::InvalidValue, message.into()))
}

fn add_action_dismissed_pull_requests_to_review_fetch(
    environment: &RuntimeEnvironment,
    grouped: &mut BTreeMap<GitHubRepository, Vec<u64>>,
    layout_by_repository: &BTreeMap<GitHubRepository, ReviewRepositoryLayout>,
    filter_matchers: &[ReviewFilterMatcher],
) -> Result<BTreeSet<ReviewPullRequestKey>, CommandError> {
    let mut keys = BTreeSet::new();
    for dismissal in PullRequestStore::open(environment)?.action_dismissed_pull_requests()? {
        let layout = layout_by_repository.get(&dismissal.repository);
        if !review_repository_matches(&dismissal.repository, layout, filter_matchers) {
            continue;
        }
        let key = review_key(&dismissal.repository, dismissal.number);
        keys.insert(key);
        grouped
            .entry(dismissal.repository)
            .or_default()
            .push(dismissal.number);
    }
    Ok(keys)
}

fn review_key(repository: &GitHubRepository, number: u64) -> ReviewPullRequestKey {
    ReviewPullRequestKey {
        repository: repository.slug(),
        number,
    }
}

fn review_viewer_response_at<'a>(
    status: &'a PullRequestStatusRecord,
    viewer: &str,
) -> Option<&'a str> {
    status
        .reviewer_responses
        .iter()
        .filter(|response| response.reviewer == viewer)
        .map(|response| response.responded_at.as_str())
        .max()
}

fn review_viewer_active_response_at<'a>(
    status: &'a PullRequestStatusRecord,
    viewer: &str,
) -> Option<&'a str> {
    let response_at = review_viewer_response_at(status, viewer)?;
    review_timestamp_after(Some(response_at), review_viewer_activity_at(status, viewer))
        .then_some(response_at)
}

fn review_viewer_activity_at<'a>(
    status: &'a PullRequestStatusRecord,
    viewer: &str,
) -> Option<&'a str> {
    status
        .review_activity
        .iter()
        .filter(|activity| activity.reviewer == viewer)
        .map(|activity| activity.reviewed_at.as_str())
        .max()
}

fn review_viewer_mention_at<'a>(
    status: &'a PullRequestStatusRecord,
    viewer: &str,
) -> Option<&'a str> {
    status
        .reviewer_mentions
        .iter()
        .find(|mention| mention.reviewer == viewer)
        .map(|mention| mention.mentioned_at.as_str())
}

/// Decides whether a review row should be shown from current PR facts and local history.
fn decide_review_request(input: ReviewDecisionInput<'_>) -> ReviewDecision {
    let state = review_request_state(input.status, input.viewer);
    let action_resurface_reason = (input.dismissal_mode != ReviewDismissalMode::Ignore)
        .then(|| {
            review_action_resurface_reason(
                input.actions,
                input.history,
                input.status,
                input.viewer,
                state,
            )
        })
        .flatten();
    let hidden_by_dismissal = action_resurface_reason == Some("no_longer_hidden");
    let resurface_reason = (!hidden_by_dismissal)
        .then_some(action_resurface_reason)
        .flatten();
    let auto_redismiss_allowed = resurface_reason
        .map(review_dismissal_allows_auto_redismiss)
        .unwrap_or(true);
    let automatic_hide_reason =
        if input.dismissal_mode == ReviewDismissalMode::Ignore || hidden_by_dismissal {
            None
        } else if action_resurface_reason != Some("not_dismissed")
            && review_request_targets_non_default_branch(input.status)
        {
            Some("non_default_branch")
        } else if auto_redismiss_allowed {
            review_request_auto_dismiss_reason(input.status, input.viewer, state)
        } else {
            None
        };
    let visible = match input.dismissal_mode {
        ReviewDismissalMode::Apply => !hidden_by_dismissal && automatic_hide_reason.is_none(),
        ReviewDismissalMode::Only => hidden_by_dismissal || automatic_hide_reason.is_some(),
        ReviewDismissalMode::Ignore => true,
    };
    let dismissal = (input.dismissal_mode == ReviewDismissalMode::Only && visible)
        .then(|| {
            if hidden_by_dismissal {
                review_request_action_dismissal_view(input.actions)
            } else {
                automatic_hide_reason.map(review_request_automatic_dismissal_view)
            }
        })
        .flatten();

    ReviewDecision {
        state,
        visible,
        action_resurface_reason: resurface_reason.filter(|reason| *reason != "not_dismissed"),
        automatic_hide_reason,
        viewer_signal: review_request_viewer_signal(
            input.actions,
            input.history,
            input.status,
            input.viewer,
        ),
        visible_since_unix: review_visible_since_unix(
            input.history,
            input.actions,
            input.viewer,
            state,
        ),
        dismissal,
    }
}

fn review_request_viewer_signal(
    actions: &[PullRequestActionRecord],
    history: &[PullRequestHistoryRecord],
    status: &PullRequestStatusRecord,
    viewer: &str,
) -> ReviewRequestViewerSignal {
    if status
        .approved_reviewers
        .iter()
        .any(|reviewer| reviewer == viewer)
        || !status
            .dismissed_reviewers
            .iter()
            .any(|reviewer| reviewer == viewer)
    {
        return ReviewRequestViewerSignal::None;
    }

    if review_actions_have_dismissed_reason(actions, "approved")
        || review_history_viewer_lost_approval(history, viewer)
    {
        ReviewRequestViewerSignal::DismissedApproval
    } else {
        ReviewRequestViewerSignal::None
    }
}

fn review_actions_have_dismissed_reason(actions: &[PullRequestActionRecord], reason: &str) -> bool {
    actions.iter().any(|action| {
        action.action == "dismiss"
            && (action.reason.as_deref() == Some(reason)
                || review_action_json_str(action, "dismissedReason") == Some(reason))
    })
}

fn review_history_viewer_lost_approval(history: &[PullRequestHistoryRecord], viewer: &str) -> bool {
    history.iter().any(|event| {
        event.kind == "review_state_changed"
            && review_history_event_reviewer(event) == Some(viewer)
            && review_history_json_str(event.old_json.as_ref(), "state") == Some("approved")
            && review_history_json_str(event.new_json.as_ref(), "state") == Some("dismissed")
    })
}

fn review_request_auto_dismiss_reason(
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> Option<&'static str> {
    review_request_auto_dismiss_reason_after(status, viewer, state, None)
}

/// Returns whether a PR is stacked behind a non-trunk base and should wait off-inbox.
fn review_request_targets_non_default_branch(status: &PullRequestStatusRecord) -> bool {
    status
        .default_branch
        .as_deref()
        .is_some_and(|default_branch| {
            !default_branch.is_empty() && status.base_branch != default_branch
        })
}

fn review_request_auto_dismiss_reason_after(
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
    active_response_after: Option<&str>,
) -> Option<&'static str> {
    if status.draft {
        return (!draft_review_needs_attention(status, viewer, active_response_after))
            .then_some("draft");
    }
    if status
        .requested_reviewers
        .users
        .iter()
        .any(|reviewer| reviewer == viewer)
    {
        return None;
    }
    if review_timestamp_after(
        review_viewer_active_response_at(status, viewer),
        active_response_after,
    ) {
        return None;
    }
    match state {
        ReviewRequestState::Approved => Some("approved"),
        ReviewRequestState::ChangesRequested | ReviewRequestState::Commented => Some("commented"),
        ReviewRequestState::New | ReviewRequestState::Answered | ReviewRequestState::Again => None,
    }
}

fn review_dismissal_allows_auto_redismiss(reason: &str) -> bool {
    !matches!(
        reason,
        "author_response"
            | "fresh_review_request"
            | "mentioned"
            | "not_dismissed"
            | "peer_approval"
            | "ready_for_review"
            | "user_approval"
    )
}

fn draft_review_needs_attention(
    status: &PullRequestStatusRecord,
    viewer: &str,
    dismissed_at: Option<&str>,
) -> bool {
    draft_fresh_review_request_at(status, viewer, dismissed_at).is_some()
        || review_timestamp_after(review_viewer_mention_at(status, viewer), dismissed_at)
        || review_timestamp_after(
            review_viewer_active_response_at(status, viewer),
            dismissed_at,
        )
}

fn draft_fresh_review_request_at<'a>(
    status: &'a PullRequestStatusRecord,
    viewer: &str,
    dismissed_at: Option<&str>,
) -> Option<&'a str> {
    let dismissed_at = dismissed_at?;
    status
        .timeline_events
        .iter()
        .filter(|event| {
            event.kind == PullRequestTimelineEventKind::ReviewRequested
                && event.reviewer.as_deref() == Some(viewer)
                && event.created_at.as_str() > dismissed_at
        })
        .map(|event| event.created_at.as_str())
        .max()
}

fn review_timestamp_after(timestamp: Option<&str>, after: Option<&str>) -> bool {
    match (timestamp, after) {
        (Some(timestamp), Some(after)) => timestamp > after,
        (Some(_), None) => true,
        _ => false,
    }
}

fn review_visible_since_unix(
    history: &[PullRequestHistoryRecord],
    actions: &[PullRequestActionRecord],
    viewer: &str,
    state: ReviewRequestState,
) -> Option<i64> {
    history
        .iter()
        .filter(|event| review_history_starts_visible_epoch(event, viewer, state))
        .map(|event| event.changed_at_unix)
        .chain(
            actions
                .iter()
                .filter(|action| action.action == "undismiss")
                .map(|action| action.changed_at_unix),
        )
        .max()
}

fn review_history_starts_visible_epoch(
    event: &PullRequestHistoryRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> bool {
    match event.kind.as_str() {
        "first_seen" | "head_changed" | "reopened" => true,
        "draft_changed" => {
            review_history_json_bool(event.new_json.as_ref(), "draft") == Some(false)
        }
        "reviewer_requested" | "author_response" | "reviewer_mentioned" => {
            review_history_event_reviewer(event) == Some(viewer)
        }
        "review_state_changed" => {
            review_history_event_reviewer(event) == Some(viewer)
                && matches!(
                    review_history_json_str(event.new_json.as_ref(), "state"),
                    Some("dismissed" | "changes_requested")
                )
                && !matches!(state, ReviewRequestState::Approved)
        }
        _ => false,
    }
}

fn review_history_event_reviewer(event: &PullRequestHistoryRecord) -> Option<&str> {
    review_history_json_str(event.new_json.as_ref(), "reviewer")
        .or_else(|| review_history_json_str(event.new_json.as_ref(), "login"))
}

fn review_history_json_str<'a>(value: Option<&'a serde_json::Value>, key: &str) -> Option<&'a str> {
    value?.get(key)?.as_str()
}

fn review_history_json_bool(value: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    value?.get(key)?.as_bool()
}

fn review_action_resurface_reason(
    actions: &[PullRequestActionRecord],
    history: &[PullRequestHistoryRecord],
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> Option<&'static str> {
    let action = latest_review_visibility_action(actions)?;
    match action.action.as_str() {
        "undismiss" if action.source == "manual" => Some("not_dismissed"),
        "undismiss" => None,
        "dismiss" => Some(review_action_dismissal_resurface_reason(
            action, history, status, viewer, state,
        )),
        _ => None,
    }
}

fn latest_review_visibility_action(
    actions: &[PullRequestActionRecord],
) -> Option<&PullRequestActionRecord> {
    actions
        .iter()
        .rev()
        .find(|action| matches!(action.action.as_str(), "dismiss" | "undismiss"))
}

fn review_action_dismissal_resurface_reason(
    action: &PullRequestActionRecord,
    history: &[PullRequestHistoryRecord],
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> &'static str {
    if action.reason.as_deref() == Some("draft") {
        return draft_action_dismissal_resurface_reason(action, history, status, viewer);
    }
    if action.reason.as_deref() == Some("peer_approval") {
        return if review_has_peer_approval(status, viewer) {
            "peer_approval"
        } else {
            "no_longer_hidden"
        };
    }
    if action.reason.as_deref() == Some("user_approval") {
        return if review_action_json_str(action, "approvalReviewer").is_some_and(|reviewer| {
            status
                .approved_reviewers
                .iter()
                .any(|user| user == reviewer)
        }) {
            "user_approval"
        } else {
            "no_longer_hidden"
        };
    }
    if review_history_has_reviewer_requested_after_unix(history, viewer, action.changed_at_unix) {
        return "fresh_review_request";
    }
    if review_history_has_event_after_unix(history, "head_changed", action.changed_at_unix)
        || review_action_json_str(action, "dismissedHeadOid").is_some_and(|dismissed_head| {
            status.latest_commit_oid.as_deref() != Some(dismissed_head)
        })
    {
        return "head_changed";
    }
    if review_action_has_new_author_response(action, history, status, viewer) {
        return "author_response";
    }
    if action.source == "automatic"
        && review_request_auto_dismiss_reason_after(
            status,
            viewer,
            state,
            review_action_json_str(action, "dismissedViewerResponseAt"),
        )
        .is_none()
    {
        return "viewer_review_state_changed";
    }
    "no_longer_hidden"
}

fn review_has_peer_approval(status: &PullRequestStatusRecord, viewer: &str) -> bool {
    status
        .approved_reviewers
        .iter()
        .any(|reviewer| reviewer != viewer)
}

fn draft_action_dismissal_resurface_reason(
    action: &PullRequestActionRecord,
    history: &[PullRequestHistoryRecord],
    status: &PullRequestStatusRecord,
    viewer: &str,
) -> &'static str {
    if review_history_has_ready_for_review_after_unix(history, action.changed_at_unix)
        || !status.draft
    {
        return "ready_for_review";
    }
    if review_history_has_reviewer_requested_after_unix(history, viewer, action.changed_at_unix) {
        return "fresh_review_request";
    }
    if review_history_has_reviewer_mention_after_unix(history, viewer, action.changed_at_unix) {
        return "mentioned";
    }
    if review_action_has_new_author_response(action, history, status, viewer) {
        return "author_response";
    }
    "no_longer_hidden"
}

fn review_action_has_new_author_response(
    action: &PullRequestActionRecord,
    history: &[PullRequestHistoryRecord],
    status: &PullRequestStatusRecord,
    viewer: &str,
) -> bool {
    let dismissed_response_at = review_action_json_str(action, "dismissedViewerResponseAt");
    if dismissed_response_at.is_some() {
        review_history_has_author_response_after(history, viewer, dismissed_response_at)
            || review_timestamp_after(
                review_viewer_active_response_at(status, viewer),
                dismissed_response_at,
            )
    } else {
        review_history_has_author_response_after_unix(history, viewer, action.changed_at_unix)
    }
}

fn review_action_json_str<'a>(action: &'a PullRequestActionRecord, key: &str) -> Option<&'a str> {
    action.details_json.get(key)?.as_str()
}

fn review_history_has_event_after_unix(
    history: &[PullRequestHistoryRecord],
    kind: &str,
    after_unix: i64,
) -> bool {
    history
        .iter()
        .any(|event| event.kind == kind && event.changed_at_unix > after_unix)
}

fn review_history_has_ready_for_review_after_unix(
    history: &[PullRequestHistoryRecord],
    after_unix: i64,
) -> bool {
    history.iter().any(|event| {
        event.kind == "draft_changed"
            && event.changed_at_unix > after_unix
            && review_history_json_bool(event.new_json.as_ref(), "draft") == Some(false)
    })
}

fn review_history_has_reviewer_requested_after_unix(
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    after_unix: i64,
) -> bool {
    history.iter().any(|event| {
        event.kind == "reviewer_requested"
            && event.changed_at_unix > after_unix
            && review_history_event_reviewer(event) == Some(viewer)
    })
}

fn review_history_has_reviewer_mention_after_unix(
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    after_unix: i64,
) -> bool {
    history.iter().any(|event| {
        event.kind == "reviewer_mentioned"
            && event.changed_at_unix > after_unix
            && review_history_event_reviewer(event) == Some(viewer)
    })
}

fn review_history_has_author_response_after(
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    after: Option<&str>,
) -> bool {
    history.iter().any(|event| {
        event.kind == "author_response"
            && review_history_event_after(event, after)
            && event
                .new_json
                .as_ref()
                .and_then(|value| value.get("reviewer"))
                .and_then(serde_json::Value::as_str)
                == Some(viewer)
    })
}

fn review_history_has_author_response_after_unix(
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    after_unix: i64,
) -> bool {
    history.iter().any(|event| {
        event.kind == "author_response"
            && event.changed_at_unix > after_unix
            && review_history_event_reviewer(event) == Some(viewer)
    })
}

fn review_history_event_after(event: &PullRequestHistoryRecord, after: Option<&str>) -> bool {
    let Some(after) = after.and_then(review_timestamp_unix) else {
        return true;
    };
    event.changed_at_unix > after
}

fn review_timestamp_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}

fn record_review_action_cleanup(
    environment: &RuntimeEnvironment,
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
    actions: &[PullRequestActionRecord],
    viewer: &str,
    reason: &'static str,
) -> Result<(), CommandError> {
    let Some(action) = latest_review_visibility_action(actions) else {
        return Ok(());
    };
    if action.action != "dismiss" {
        return Ok(());
    }
    let dismissal = ReviewDismissal {
        repository: repository.slug(),
        number: status.number,
        source: ReviewDismissalSource::Automatic,
        reason: action.reason.clone().unwrap_or_else(|| "manual".to_owned()),
        latest_commit_oid: review_action_json_str(action, "dismissedHeadOid")
            .map(str::to_owned)
            .or_else(|| status.latest_commit_oid.clone())
            .unwrap_or_default(),
        viewer_response_at: review_action_json_str(action, "dismissedViewerResponseAt")
            .map(str::to_owned)
            .or_else(|| review_viewer_response_at(status, viewer).map(str::to_owned)),
        approval_reviewer: review_action_json_str(action, "approvalReviewer").map(str::to_owned),
        dismissed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    append_review_dismissal_log(
        environment,
        review_dismissal_log_event("undismiss", reason, &dismissal, Some(status), viewer, None),
    )?;
    record_review_dismissal_action(
        environment,
        "undismiss",
        reason,
        &dismissal,
        Some(status),
        viewer,
        None,
    )
}

fn record_review_dismissal_action(
    environment: &RuntimeEnvironment,
    action: &'static str,
    reason: &'static str,
    dismissal: &ReviewDismissal,
    status: Option<&PullRequestStatusRecord>,
    viewer: &str,
    selector: Option<&str>,
) -> Result<(), CommandError> {
    let Some(repository) = review_repository_from_slug(&dismissal.repository) else {
        return Ok(());
    };
    PullRequestStore::open(environment)?.record_pull_request_action(
        &repository,
        dismissal.number,
        action,
        review_dismissal_action_source(action, dismissal, selector),
        Some(reason),
        serde_json::json!({
            "selector": selector,
            "dismissedAt": dismissal.dismissed_at,
            "dismissedReason": review_dismissal_reason(dismissal),
            "dismissedHeadOid": dismissal.latest_commit_oid,
            "currentHeadOid": status.and_then(|status| status.latest_commit_oid.as_deref()),
            "dismissedViewerResponseAt": dismissal.viewer_response_at,
            "currentViewerResponseAt": status.and_then(|status| review_viewer_response_at(status, viewer)),
            "approvalReviewer": dismissal.approval_reviewer.clone(),
        }),
    )?;
    Ok(())
}

fn review_repository_from_slug(slug: &str) -> Option<GitHubRepository> {
    let (owner, name) = slug.split_once('/')?;
    Some(GitHubRepository {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn review_dismissal_log_event(
    action: &'static str,
    reason: &'static str,
    dismissal: &ReviewDismissal,
    status: Option<&PullRequestStatusRecord>,
    viewer: &str,
    selector: Option<&str>,
) -> ReviewDismissalLogEvent {
    ReviewDismissalLogEvent {
        version: REVIEW_DISMISSAL_LOG_VERSION,
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        action,
        reason,
        source: review_dismissal_action_source(action, dismissal, selector),
        repository: dismissal.repository.clone(),
        number: dismissal.number,
        selector: selector.map(str::to_owned),
        dismissed_at: Some(dismissal.dismissed_at.clone()),
        dismissed_reason: Some(review_dismissal_reason(dismissal)),
        dismissed_head_oid: Some(dismissal.latest_commit_oid.clone()),
        current_head_oid: status.and_then(|status| status.latest_commit_oid.clone()),
        dismissed_viewer_response_at: dismissal.viewer_response_at.clone(),
        current_viewer_response_at: status
            .and_then(|status| review_viewer_response_at(status, viewer))
            .map(str::to_owned),
        approval_reviewer: dismissal.approval_reviewer.clone(),
    }
}

fn review_dismissal_action_source(
    action: &str,
    dismissal: &ReviewDismissal,
    selector: Option<&str>,
) -> &'static str {
    if selector.is_some()
        || (action == "dismiss" && matches!(dismissal.source, ReviewDismissalSource::Manual))
    {
        "manual"
    } else {
        "automatic"
    }
}

fn review_request_action_dismissal_view(
    actions: &[PullRequestActionRecord],
) -> Option<ReviewRequestDismissalView> {
    let action = latest_review_visibility_action(actions)?;
    (action.action == "dismiss").then(|| ReviewRequestDismissalView {
        source: action.source.clone(),
        reason: action.reason.clone().unwrap_or_else(|| "manual".to_owned()),
    })
}

fn review_request_automatic_dismissal_view(reason: &'static str) -> ReviewRequestDismissalView {
    ReviewRequestDismissalView {
        source: "automatic".to_owned(),
        reason: reason.to_owned(),
    }
}

fn review_dismissal_reason(dismissal: &ReviewDismissal) -> String {
    if dismissal.reason.is_empty() {
        "manual".to_owned()
    } else {
        dismissal.reason.clone()
    }
}

fn append_review_dismissal_log(
    environment: &RuntimeEnvironment,
    event: ReviewDismissalLogEvent,
) -> Result<(), CommandError> {
    let file = review_dismissals_log_file(environment)?;
    migrate_legacy_review_dismissals_log(&file)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|source| RepositoryError::CacheWrite {
            file: parent.to_path_buf(),
            source,
        })?;
    }
    let mut output = serde_json::to_string(&event).expect("review dismissal log event serializes");
    output.push('\n');
    let mut file_handle = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .map_err(|source| RepositoryError::CacheWrite {
            file: file.clone(),
            source,
        })?;
    file_handle
        .write_all(output.as_bytes())
        .map_err(|source| RepositoryError::CacheWrite { file, source })?;
    Ok(())
}

fn migrate_legacy_review_dismissals_log(file: &Path) -> Result<(), CommandError> {
    let legacy = file.with_file_name(REVIEW_DISMISSALS_LEGACY_LOG_FILE);
    if !legacy.exists() || file.exists() {
        return Ok(());
    }
    fs::rename(&legacy, file).map_err(|source| RepositoryError::CacheWrite {
        file: file.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn review_dismissals_log_file(environment: &RuntimeEnvironment) -> Result<PathBuf, CommandError> {
    let root = environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: "environment".to_owned(),
            message: "HOME or XDG_STATE_HOME must be set to store review dismissal audit logs"
                .to_owned(),
        })?;
    Ok(root.join("jx").join(REVIEW_DISMISSALS_LOG_FILE))
}
