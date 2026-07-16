use super::*;
use crate::domain::{apply_review_request_status_policy, review_request_state, ReviewRequestState};
use clap::error::ErrorKind;
use globset::{Glob, GlobMatcher};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

const REVIEW_DISMISSALS_FILE: &str = "review-dismissals.toml";
const REVIEW_DISMISSALS_LOG_FILE: &str = "review-dismissals.log";
const REVIEW_DISMISSALS_LEGACY_LOG_FILE: &str = "review-dismissals.log.jsonl";
const REVIEW_DISMISSALS_VERSION: u32 = 1;
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

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct ReviewDismissals {
    #[serde(default)]
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pull_requests: Vec<ReviewDismissal>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct ReviewDismissal {
    repository: String,
    number: u64,
    #[serde(default, skip_serializing_if = "ReviewDismissalSource::is_manual")]
    source: ReviewDismissalSource,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reason: String,
    latest_commit_oid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    viewer_response_at: Option<String>,
    dismissed_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ReviewDismissalSource {
    #[default]
    Manual,
    Automatic,
}

impl ReviewDismissalSource {
    fn is_manual(source: &Self) -> bool {
        matches!(source, Self::Manual)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReviewDismissalHistory {
    latest_reasons: BTreeMap<ReviewPullRequestKey, String>,
}

#[derive(Debug, serde::Deserialize)]
struct ReviewDismissalLogRecord {
    repository: String,
    number: u64,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    dismissed_reason: Option<String>,
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
}

fn run_review_dashboard(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
) -> Result<CommandResult, CommandError> {
    let environment = environment.clone();
    let loader_request = request.clone();
    let loader: DashboardFrameLoader = std::sync::Arc::new(move || {
        render_review_dashboard_frame(loader_request.clone(), &environment)
            .map_err(|error| error.to_string())
    });
    run_interactive_dashboard("jx review", request.refresh_seconds, loader)
}

fn render_review_dashboard_frame(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
) -> Result<String, CommandError> {
    let services = ProductionServices::new(environment)?;
    let progress = SilentProgress;
    let output = OutputMode::from_process();
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
        span.measure("review.render", Vec::new(), || {
            Ok::<_, CommandError>(render_review_requests(
                &loaded.view,
                output.color,
                output.terminal_width,
                &loaded.display_names,
            ))
        })
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
        ReviewAction::Dismiss { selector } => {
            return handle_review_dismiss(selector, environment, services, progress);
        }
        ReviewAction::Undismiss { selector } => {
            return handle_review_undismiss(selector, environment, services, progress);
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
        ReviewAction::Show | ReviewAction::Dismiss { .. } | ReviewAction::Undismiss { .. } => {
            ReviewDismissalMode::Apply
        }
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
    progress.status("Loading review requests…");
    let inbox = span.measure_with_result_attrs(
        "review.fetch_candidates",
        Vec::new(),
        || services.review_requests(&token_source),
        |result| {
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
        },
    )?;
    let viewer = inbox.viewer.login;
    let mut dismissals = if dismissal_mode == ReviewDismissalMode::Ignore {
        ReviewDismissals::default()
    } else {
        read_review_dismissals(environment)?
    };
    let dismissal_history = if dismissal_mode == ReviewDismissalMode::Ignore {
        ReviewDismissalHistory::default()
    } else {
        read_review_dismissal_history(environment)?
    };
    let mut dismissal_state_changed = false;
    let mut candidate_keys = BTreeSet::new();
    let mut grouped = BTreeMap::<GitHubRepository, Vec<u64>>::new();
    for candidate in inbox.requests {
        let layout = layout_by_repository.get(&candidate.repository);
        if review_repository_matches(&candidate.repository, layout, &filter_matchers) {
            candidate_keys.insert(review_key(&candidate.repository, candidate.number));
            if dismissal_mode != ReviewDismissalMode::Only {
                grouped
                    .entry(candidate.repository)
                    .or_default()
                    .push(candidate.number);
            }
        }
    }

    let dismissed_keys_in_scope = if dismissal_mode == ReviewDismissalMode::Ignore {
        BTreeSet::new()
    } else {
        add_dismissed_pull_requests_to_review_fetch(
            &mut grouped,
            &dismissals,
            &layout_by_repository,
            &filter_matchers,
        )
    };
    span.set([perf_attr("filtered_repo_count", grouped.len())]);

    let grouped_repo_count = grouped.len();
    if grouped_repo_count > 0 {
        progress.percentage("Loading pull request details", 0, grouped_repo_count);
    }
    let mut repositories = Vec::new();
    let mut detail_pr_count = 0usize;
    let mut fetched_keys = BTreeSet::new();
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
        let stack_status_policy = config.repo.stack_status_for(&repository);
        let review_policy = config.repo.review_for(&repository);
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
            let key = review_key(&repository, status.number);
            fetched_keys.insert(key.clone());
            if status.author.as_deref() == Some(viewer.as_str()) || !candidate_keys.contains(&key) {
                let reason = if status.author.as_deref() == Some(viewer.as_str()) {
                    "authored_by_viewer"
                } else {
                    "left_review_inbox"
                };
                dismissal_state_changed |= remove_review_dismissal_with_log(
                    environment,
                    &mut dismissals,
                    &key,
                    reason,
                    Some(&status),
                    &viewer,
                    None,
                )?;
                continue;
            }
            let state = review_request_state(&status, &viewer);
            let prior_dismissal = dismissals.get(&key).cloned();
            let hidden_by_dismissal = dismissal_mode != ReviewDismissalMode::Ignore
                && dismissals.hides(&key, &status, &pull_request.history, &viewer, state);
            let mut auto_redismiss_allowed = true;
            if dismissal_mode != ReviewDismissalMode::Ignore && !hidden_by_dismissal {
                let reason = review_dismissal_resurface_reason(
                    &dismissals,
                    &key,
                    &status,
                    &pull_request.history,
                    &viewer,
                    state,
                );
                let removed = remove_review_dismissal_with_log(
                    environment,
                    &mut dismissals,
                    &key,
                    reason,
                    Some(&status),
                    &viewer,
                    None,
                )?;
                dismissal_state_changed |= removed;
                if removed {
                    auto_redismiss_allowed = review_dismissal_allows_auto_redismiss(reason);
                }
            }
            match dismissal_mode {
                ReviewDismissalMode::Apply if hidden_by_dismissal => continue,
                ReviewDismissalMode::Apply => {
                    if let Some(reason) =
                        review_request_auto_dismiss_reason(&status, &viewer, state)
                            .filter(|_| auto_redismiss_allowed)
                    {
                        dismissal_state_changed |= auto_dismiss_review_request(
                            environment,
                            &mut dismissals,
                            &repository,
                            &status,
                            &viewer,
                            reason,
                        )?;
                        continue;
                    }
                }
                ReviewDismissalMode::Only if !hidden_by_dismissal => continue,
                ReviewDismissalMode::Only | ReviewDismissalMode::Ignore => {}
            }
            let dismissal = if dismissal_mode == ReviewDismissalMode::Only {
                dismissals.get(&key).map(|dismissal| {
                    review_request_dismissal_view(dismissal, &status, &viewer, state)
                })
            } else {
                None
            };
            let viewer_signal = review_request_viewer_signal(
                &dismissal_history,
                prior_dismissal.as_ref(),
                &key,
                &status,
                &viewer,
                state,
            );
            rows.push(ReviewRequestRowView {
                state,
                status,
                viewer_signal,
                dismissal,
            });
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.status.number));
        if rows.is_empty() {
            continue;
        }
        let layout = layout_by_repository.get(&repository);
        repositories.push(ReviewRequestRepositoryView {
            repository: repository.clone(),
            layout_key: layout.map(|layout| layout.key.clone()),
            root: layout.map(|layout| layout.root.clone()),
            display_root: layout.map(|layout| display_path(&layout.root, environment)),
            external: layout.is_none(),
            review_wait_threshold_seconds: stack_status_policy.review_wait_threshold_seconds,
            rows,
        });
        progress.percentage(
            "Loading pull request details",
            index + 1,
            grouped_repo_count,
        );
    }
    if dismissal_mode != ReviewDismissalMode::Ignore {
        for dismissal in dismissals.remove_missing(&dismissed_keys_in_scope, &fetched_keys) {
            append_review_dismissal_log(
                environment,
                review_dismissal_log_event(
                    "undismiss",
                    "missing_from_github",
                    &dismissal,
                    None,
                    &viewer,
                    None,
                ),
            )?;
            dismissal_state_changed = true;
        }
        if dismissal_state_changed {
            write_review_dismissals(environment, &dismissals)?;
        }
    }
    span.finish_step(
        fetch_details,
        [perf_attr("detail_pr_count", detail_pr_count)],
        Option::<&WorkflowError>::None,
    );

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

    let view = ReviewRequestsView {
        viewer,
        repositories,
    };
    let mut display_name_logins = review_request_user_logins(&view);
    if !view.repositories.is_empty() {
        display_name_logins.push(view.viewer.clone());
    }
    let display_names = span.measure_with_result_attrs(
        "review.load_display_names",
        [perf_attr("login_count", display_name_logins.len())],
        || {
            Ok::<_, CommandError>(if display_name_logins.is_empty() {
                BTreeMap::new()
            } else {
                progress.status("Loading reviewer names…");
                services.github_user_display_names(&token_source, &display_name_logins)
            })
        },
        |result| {
            result
                .as_ref()
                .map(|display_names| vec![perf_attr("display_name_count", display_names.len())])
                .unwrap_or_default()
        },
    )?;

    Ok(LoadedReviewRequestsView {
        view,
        display_names,
    })
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
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
) -> Result<String, CommandError> {
    let mut span = PerfLog::from_environment(environment)
        .start("review.dismiss", [perf_attr("selector", selector)]);
    let result = handle_review_dismiss_traced(selector, environment, services, progress, &mut span);
    if let Err(error) = &result {
        span.record_error(error);
    }
    span.end();
    result
}

fn handle_review_dismiss_traced(
    selector: &str,
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
    let (repository, status) = match matches.as_slice() {
        [(repository, status)] => (repository, status),
        [] => {
            return Err(review_dismiss_usage_error(format!(
                "no review pull request matched `{selector}`"
            )));
        }
        matches => {
            let choices = matches
                .iter()
                .map(|(repository, status)| format!("{}#{}", repository.slug(), status.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(review_dismiss_usage_error(format!(
                "review pull request `{selector}` is ambiguous; matches: {choices}; use a longer repo suffix such as owner/repo#number"
            )));
        }
    };

    let Some(latest_commit_oid) = status.latest_commit_oid.clone() else {
        return Err(review_dismiss_usage_error(format!(
            "{}#{} cannot be dismissed because GitHub did not return its latest commit oid",
            repository.slug(),
            status.number
        )));
    };

    let reason = review_manual_dismissal_reason(status);
    let mut dismissals = read_review_dismissals(environment)?;
    let key = review_key(repository, status.number);
    let dismissal = ReviewDismissal {
        repository: key.repository.clone(),
        number: key.number,
        source: ReviewDismissalSource::Manual,
        reason: reason.to_owned(),
        latest_commit_oid: latest_commit_oid.clone(),
        viewer_response_at: review_viewer_response_at(status, &loaded.view.viewer)
            .map(str::to_owned),
        dismissed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let changed = dismissals.upsert(dismissal.clone());
    if changed {
        write_review_dismissals(environment, &dismissals)?;
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
    }

    let prefix = if changed {
        "Dismissed"
    } else {
        "Already dismissed"
    };
    Ok(format!(
        "{prefix} {} {}\n",
        review_dismiss_pull_request_link(repository, status),
        review_dismissal_until_message(reason),
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
    let (repository, status) = match matches.as_slice() {
        [(repository, status)] => (repository, status),
        [] => {
            return Err(review_dismiss_usage_error(format!(
                "no dismissed review pull request matched `{selector}`"
            )));
        }
        matches => {
            let choices = matches
                .iter()
                .map(|(repository, status)| format!("{}#{}", repository.slug(), status.number))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(review_dismiss_usage_error(format!(
                "dismissed review pull request `{selector}` is ambiguous; matches: {choices}; use a longer repo suffix such as owner/repo#number"
            )));
        }
    };

    let key = review_key(repository, status.number);
    let mut dismissals = read_review_dismissals(environment)?;
    let removed = remove_review_dismissal_with_log(
        environment,
        &mut dismissals,
        &key,
        "manual",
        Some(status),
        &loaded.view.viewer,
        Some(selector),
    )?;
    if removed {
        write_review_dismissals(environment, &dismissals)?;
    }

    Ok(format!(
        "Undismissed {}\n",
        review_dismiss_pull_request_link(repository, status),
    ))
}

fn review_manual_dismissal_reason(status: &PullRequestStatusRecord) -> &'static str {
    if status.draft {
        "draft"
    } else {
        "manual"
    }
}

fn review_dismissal_until_message(reason: &str) -> &'static str {
    if reason == "draft" {
        "until it is ready for review, a fresh review request, a mention, or a new author response"
    } else {
        "until new commits, a fresh review request, or a new author response"
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
) -> Vec<(&'a GitHubRepository, &'a PullRequestStatusRecord)> {
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
                .map(move |row| (&repository.repository, &row.status))
        })
        .collect()
}

fn review_dismiss_repository_matches(
    repository: &ReviewRequestRepositoryView,
    repository_suffix: &[String],
) -> bool {
    if repository_suffix.is_empty() {
        return true;
    }
    review_dismiss_repository_identities(repository)
        .iter()
        .any(|identity| review_dismiss_identity_matches(identity, repository_suffix))
}

fn review_dismiss_repository_identities(
    repository: &ReviewRequestRepositoryView,
) -> Vec<Vec<String>> {
    let mut identities = Vec::new();
    if let Some(key) = &repository.layout_key {
        identities.push(review_dismiss_repository_components(key));
    }
    identities.push(vec![repository.repository.name.clone()]);
    identities.push(vec![
        repository.repository.owner.clone(),
        repository.repository.name.clone(),
    ]);
    identities.push(vec![
        "github.com".to_owned(),
        repository.repository.owner.clone(),
        repository.repository.name.clone(),
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

fn add_dismissed_pull_requests_to_review_fetch(
    grouped: &mut BTreeMap<GitHubRepository, Vec<u64>>,
    dismissals: &ReviewDismissals,
    layout_by_repository: &BTreeMap<GitHubRepository, ReviewRepositoryLayout>,
    filter_matchers: &[ReviewFilterMatcher],
) -> BTreeSet<ReviewPullRequestKey> {
    let mut keys = BTreeSet::new();
    for dismissal in &dismissals.pull_requests {
        let Some(repository) = repository_from_slug(&dismissal.repository) else {
            continue;
        };
        let layout = layout_by_repository.get(&repository);
        if !review_repository_matches(&repository, layout, filter_matchers) {
            continue;
        }
        let key = review_key(&repository, dismissal.number);
        keys.insert(key);
        grouped
            .entry(repository)
            .or_default()
            .push(dismissal.number);
    }
    keys
}

fn repository_from_slug(slug: &str) -> Option<GitHubRepository> {
    let (owner, name) = slug.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(GitHubRepository {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
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

fn review_request_viewer_signal(
    dismissal_history: &ReviewDismissalHistory,
    prior_dismissal: Option<&ReviewDismissal>,
    key: &ReviewPullRequestKey,
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
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

    let prior_reason = prior_dismissal.map(|dismissal| {
        review_dismissal_reason_for_state(dismissal, Some(status), viewer, Some(state))
    });
    if prior_reason.as_deref() == Some("approved")
        || dismissal_history.latest_reason(key) == Some("approved")
    {
        ReviewRequestViewerSignal::DismissedApproval
    } else {
        ReviewRequestViewerSignal::None
    }
}

fn review_request_auto_dismiss_reason(
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> Option<&'static str> {
    review_request_auto_dismiss_reason_after(status, viewer, state, None)
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
        "author_response" | "fresh_review_request" | "mentioned" | "ready_for_review"
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

fn review_history_has_event_after(
    history: &[PullRequestHistoryRecord],
    kind: &str,
    after: Option<&str>,
) -> bool {
    history
        .iter()
        .any(|event| event.kind == kind && review_history_event_after(event, after))
}

fn review_history_has_ready_for_review_after(
    history: &[PullRequestHistoryRecord],
    after: Option<&str>,
) -> bool {
    history.iter().any(|event| {
        event.kind == "draft_changed"
            && review_history_event_after(event, after)
            && event
                .new_json
                .as_ref()
                .and_then(|value| value.get("draft"))
                .and_then(serde_json::Value::as_bool)
                == Some(false)
    })
}

fn review_history_has_reviewer_requested_after(
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    after: Option<&str>,
) -> bool {
    history.iter().any(|event| {
        event.kind == "reviewer_requested"
            && review_history_event_after(event, after)
            && event
                .new_json
                .as_ref()
                .and_then(|value| value.get("login"))
                .and_then(serde_json::Value::as_str)
                == Some(viewer)
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

fn auto_dismiss_review_request(
    environment: &RuntimeEnvironment,
    dismissals: &mut ReviewDismissals,
    repository: &GitHubRepository,
    status: &PullRequestStatusRecord,
    viewer: &str,
    reason: &'static str,
) -> Result<bool, CommandError> {
    let Some(latest_commit_oid) = status.latest_commit_oid.clone() else {
        return Ok(false);
    };
    let key = review_key(repository, status.number);
    let dismissal = ReviewDismissal {
        repository: key.repository,
        number: key.number,
        source: ReviewDismissalSource::Automatic,
        reason: reason.to_owned(),
        latest_commit_oid,
        viewer_response_at: review_viewer_response_at(status, viewer).map(str::to_owned),
        dismissed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let changed = dismissals.upsert(dismissal.clone());
    if changed {
        append_review_dismissal_log(
            environment,
            review_dismissal_log_event("dismiss", reason, &dismissal, Some(status), viewer, None),
        )?;
        record_review_dismissal_action(
            environment,
            "dismiss",
            reason,
            &dismissal,
            Some(status),
            viewer,
            None,
        )?;
    }
    Ok(changed)
}

fn review_dismissal_resurface_reason(
    dismissals: &ReviewDismissals,
    key: &ReviewPullRequestKey,
    status: &PullRequestStatusRecord,
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    state: ReviewRequestState,
) -> &'static str {
    let Some(dismissal) = dismissals.get(key) else {
        return "not_dismissed";
    };
    review_dismissal_resurface_reason_for(dismissal, status, history, viewer, state)
}

fn review_dismissal_resurface_reason_for(
    dismissal: &ReviewDismissal,
    status: &PullRequestStatusRecord,
    history: &[PullRequestHistoryRecord],
    viewer: &str,
    state: ReviewRequestState,
) -> &'static str {
    if review_dismissal_uses_draft_policy(dismissal, status) {
        return draft_dismissal_resurface_reason(dismissal, status, history, viewer);
    }
    if status
        .requested_reviewers
        .users
        .iter()
        .any(|reviewer| reviewer == viewer)
    {
        return "fresh_review_request";
    }
    if review_history_has_event_after(history, "head_changed", Some(&dismissal.dismissed_at))
        || status.latest_commit_oid.as_deref() != Some(dismissal.latest_commit_oid.as_str())
    {
        return "head_changed";
    }
    if review_history_has_author_response_after(
        history,
        viewer,
        dismissal.viewer_response_at.as_deref(),
    ) || review_timestamp_after(
        review_viewer_active_response_at(status, viewer),
        dismissal.viewer_response_at.as_deref(),
    ) {
        return "author_response";
    }
    if matches!(dismissal.source, ReviewDismissalSource::Automatic)
        && review_request_auto_dismiss_reason_after(
            status,
            viewer,
            state,
            dismissal.viewer_response_at.as_deref(),
        )
        .is_none()
    {
        return "viewer_review_state_changed";
    }
    "no_longer_hidden"
}

fn review_dismissal_uses_draft_policy(
    dismissal: &ReviewDismissal,
    status: &PullRequestStatusRecord,
) -> bool {
    dismissal.reason == "draft"
        || (dismissal.reason.is_empty()
            && matches!(dismissal.source, ReviewDismissalSource::Automatic)
            && status.draft)
}

fn draft_dismissal_resurface_reason(
    dismissal: &ReviewDismissal,
    status: &PullRequestStatusRecord,
    history: &[PullRequestHistoryRecord],
    viewer: &str,
) -> &'static str {
    if review_history_has_ready_for_review_after(history, Some(&dismissal.dismissed_at))
        || !status.draft
    {
        return "ready_for_review";
    }
    if review_history_has_reviewer_requested_after(history, viewer, Some(&dismissal.dismissed_at))
        || draft_fresh_review_request_at(status, viewer, Some(&dismissal.dismissed_at)).is_some()
    {
        return "fresh_review_request";
    }
    if review_timestamp_after(
        review_viewer_mention_at(status, viewer),
        Some(&dismissal.dismissed_at),
    ) {
        return "mentioned";
    }
    if review_history_has_author_response_after(
        history,
        viewer,
        dismissal.viewer_response_at.as_deref(),
    ) || review_timestamp_after(
        review_viewer_active_response_at(status, viewer),
        dismissal.viewer_response_at.as_deref(),
    ) {
        return "author_response";
    }
    "no_longer_hidden"
}

fn remove_review_dismissal_with_log(
    environment: &RuntimeEnvironment,
    dismissals: &mut ReviewDismissals,
    key: &ReviewPullRequestKey,
    reason: &'static str,
    status: Option<&PullRequestStatusRecord>,
    viewer: &str,
    selector: Option<&str>,
) -> Result<bool, CommandError> {
    let Some(dismissal) = dismissals.get(key).cloned() else {
        return Ok(false);
    };
    dismissals.remove(key);
    append_review_dismissal_log(
        environment,
        review_dismissal_log_event("undismiss", reason, &dismissal, status, viewer, selector),
    )?;
    record_review_dismissal_action(
        environment,
        "undismiss",
        reason,
        &dismissal,
        status,
        viewer,
        selector,
    )?;
    Ok(true)
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
            "dismissedReason": review_dismissal_reason(dismissal, status, viewer),
            "dismissedHeadOid": dismissal.latest_commit_oid,
            "currentHeadOid": status.and_then(|status| status.latest_commit_oid.as_deref()),
            "dismissedViewerResponseAt": dismissal.viewer_response_at,
            "currentViewerResponseAt": status.and_then(|status| review_viewer_response_at(status, viewer)),
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
        dismissed_reason: Some(review_dismissal_reason(dismissal, status, viewer)),
        dismissed_head_oid: Some(dismissal.latest_commit_oid.clone()),
        current_head_oid: status.and_then(|status| status.latest_commit_oid.clone()),
        dismissed_viewer_response_at: dismissal.viewer_response_at.clone(),
        current_viewer_response_at: status
            .and_then(|status| review_viewer_response_at(status, viewer))
            .map(str::to_owned),
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

fn review_request_dismissal_view(
    dismissal: &ReviewDismissal,
    status: &PullRequestStatusRecord,
    viewer: &str,
    state: ReviewRequestState,
) -> ReviewRequestDismissalView {
    ReviewRequestDismissalView {
        source: review_dismissal_source_label(dismissal.source).to_owned(),
        reason: review_dismissal_reason_for_state(dismissal, Some(status), viewer, Some(state)),
    }
}

fn review_dismissal_source_label(source: ReviewDismissalSource) -> &'static str {
    match source {
        ReviewDismissalSource::Manual => "manual",
        ReviewDismissalSource::Automatic => "automatic",
    }
}

fn review_dismissal_reason(
    dismissal: &ReviewDismissal,
    status: Option<&PullRequestStatusRecord>,
    viewer: &str,
) -> String {
    let state = status.map(|status| review_request_state(status, viewer));
    review_dismissal_reason_for_state(dismissal, status, viewer, state)
}

fn review_dismissal_reason_for_state(
    dismissal: &ReviewDismissal,
    status: Option<&PullRequestStatusRecord>,
    viewer: &str,
    state: Option<ReviewRequestState>,
) -> String {
    if !dismissal.reason.is_empty() {
        return dismissal.reason.clone();
    }
    match dismissal.source {
        ReviewDismissalSource::Manual => "manual".to_owned(),
        ReviewDismissalSource::Automatic => status
            .zip(state)
            .and_then(|(status, state)| review_request_auto_dismiss_reason(status, viewer, state))
            .unwrap_or("unknown")
            .to_owned(),
    }
}

impl ReviewDismissals {
    fn upsert(&mut self, dismissal: ReviewDismissal) -> bool {
        self.version = REVIEW_DISMISSALS_VERSION;
        let key = ReviewPullRequestKey {
            repository: dismissal.repository.clone(),
            number: dismissal.number,
        };
        if let Some(existing) = self
            .pull_requests
            .iter_mut()
            .find(|existing| existing.key() == key)
        {
            if existing.source == dismissal.source
                && existing.reason == dismissal.reason
                && existing.latest_commit_oid == dismissal.latest_commit_oid
                && existing.viewer_response_at == dismissal.viewer_response_at
            {
                return false;
            }
            *existing = dismissal;
        } else {
            self.pull_requests.push(dismissal);
        }
        self.normalize();
        true
    }

    fn hides(
        &self,
        key: &ReviewPullRequestKey,
        status: &PullRequestStatusRecord,
        history: &[PullRequestHistoryRecord],
        viewer: &str,
        state: ReviewRequestState,
    ) -> bool {
        let Some(dismissal) = self.get(key) else {
            return false;
        };
        review_dismissal_resurface_reason_for(dismissal, status, history, viewer, state)
            == "no_longer_hidden"
    }

    fn get(&self, key: &ReviewPullRequestKey) -> Option<&ReviewDismissal> {
        self.pull_requests
            .iter()
            .find(|dismissal| dismissal.key() == *key)
    }

    fn remove(&mut self, key: &ReviewPullRequestKey) -> bool {
        let before = self.pull_requests.len();
        self.pull_requests
            .retain(|dismissal| dismissal.key() != *key);
        before != self.pull_requests.len()
    }

    fn remove_missing(
        &mut self,
        scoped_keys: &BTreeSet<ReviewPullRequestKey>,
        fetched_keys: &BTreeSet<ReviewPullRequestKey>,
    ) -> Vec<ReviewDismissal> {
        let mut removed = Vec::new();
        self.pull_requests.retain(|dismissal| {
            let key = dismissal.key();
            let keep = !scoped_keys.contains(&key) || fetched_keys.contains(&key);
            if !keep {
                removed.push(dismissal.clone());
            }
            keep
        });
        removed
    }

    fn normalize(&mut self) {
        self.pull_requests.sort_by_key(ReviewDismissal::key);
        self.pull_requests.dedup_by(|left, right| {
            left.repository == right.repository && left.number == right.number
        });
    }
}

impl ReviewDismissal {
    fn key(&self) -> ReviewPullRequestKey {
        ReviewPullRequestKey {
            repository: self.repository.clone(),
            number: self.number,
        }
    }
}

impl ReviewDismissalHistory {
    fn latest_reason(&self, key: &ReviewPullRequestKey) -> Option<&str> {
        self.latest_reasons.get(key).map(String::as_str)
    }

    fn record(&mut self, record: ReviewDismissalLogRecord) {
        let Some(reason) = record.dismissed_reason.or(record.reason) else {
            return;
        };
        self.latest_reasons.insert(
            ReviewPullRequestKey {
                repository: record.repository,
                number: record.number,
            },
            reason,
        );
    }
}

fn read_review_dismissal_history(
    environment: &RuntimeEnvironment,
) -> Result<ReviewDismissalHistory, CommandError> {
    let file = review_dismissals_log_file(environment)?;
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let legacy = file.with_file_name(REVIEW_DISMISSALS_LEGACY_LOG_FILE);
            match fs::read_to_string(&legacy) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Ok(ReviewDismissalHistory::default());
                }
                Err(source) => {
                    return Err(RepositoryError::CacheRead {
                        file: legacy,
                        source,
                    }
                    .into())
                }
            }
        }
        Err(source) => return Err(RepositoryError::CacheRead { file, source }.into()),
    };

    let mut history = ReviewDismissalHistory::default();
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Ok(record) = serde_json::from_str::<ReviewDismissalLogRecord>(line) {
            history.record(record);
        }
    }
    Ok(history)
}

fn read_review_dismissals(
    environment: &RuntimeEnvironment,
) -> Result<ReviewDismissals, CommandError> {
    let file = review_dismissals_file(environment)?;
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReviewDismissals {
                version: REVIEW_DISMISSALS_VERSION,
                pull_requests: Vec::new(),
            });
        }
        Err(source) => return Err(RepositoryError::CacheRead { file, source }.into()),
    };
    let mut dismissals = toml::from_str::<ReviewDismissals>(&contents)
        .map_err(|source| RepositoryError::CacheParse { file, source })?;
    dismissals.version = REVIEW_DISMISSALS_VERSION;
    dismissals.normalize();
    Ok(dismissals)
}

fn write_review_dismissals(
    environment: &RuntimeEnvironment,
    dismissals: &ReviewDismissals,
) -> Result<(), CommandError> {
    let file = review_dismissals_file(environment)?;
    if dismissals.pull_requests.is_empty() {
        match fs::remove_file(&file) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(RepositoryError::CacheWrite { file, source }.into()),
        }
    }

    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|source| RepositoryError::CacheWrite {
            file: parent.to_path_buf(),
            source,
        })?;
    }
    let contents = toml::to_string(dismissals).expect("review dismissals serialize");
    let temporary = file.with_extension("toml.tmp");
    fs::write(&temporary, contents).map_err(|source| RepositoryError::CacheWrite {
        file: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, &file).map_err(|source| RepositoryError::CacheWrite { file, source })?;
    Ok(())
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
    Ok(review_dismissals_file(environment)?.with_file_name(REVIEW_DISMISSALS_LOG_FILE))
}

fn review_dismissals_file(environment: &RuntimeEnvironment) -> Result<PathBuf, CommandError> {
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
            message: "HOME or XDG_STATE_HOME must be set to store review dismissals".to_owned(),
        })?;
    Ok(root.join("jx").join(REVIEW_DISMISSALS_FILE))
}
