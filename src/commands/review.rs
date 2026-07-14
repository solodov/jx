use super::*;
use crate::domain::{apply_pull_request_status_policy, review_request_state};
use clap::error::ErrorKind;
use globset::{Glob, GlobMatcher};
use std::collections::{BTreeMap, BTreeSet};

struct ReviewRepositoryLayout {
    key: String,
    root: PathBuf,
    provider_slug: String,
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
        let loaded =
            load_review_requests_view(request, environment, &services, &progress, &mut span)?;
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
    let loaded = load_review_requests_view(request, environment, services, progress, span)?;
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

fn load_review_requests_view(
    request: ReviewRequest,
    environment: &RuntimeEnvironment,
    services: &dyn CommandServices,
    progress: &dyn ProgressSink,
    span: &mut PerfSpan,
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

    let mut grouped = BTreeMap::<GitHubRepository, Vec<u64>>::new();
    for candidate in inbox.requests {
        let layout = layout_by_repository.get(&candidate.repository);
        if review_repository_matches(&candidate.repository, layout, &filter_matchers) {
            grouped
                .entry(candidate.repository)
                .or_default()
                .push(candidate.number);
        }
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
        let mut statuses =
            services.pull_request_statuses_for_repository(&token_source, &repository, &numbers)?;
        let policy = config.repo.stack_status_for(&repository);
        statuses = statuses
            .into_iter()
            .map(|status| apply_pull_request_status_policy(status, &policy))
            .filter(|status| status.author.as_deref() != Some(viewer.as_str()))
            .collect();
        statuses.sort_by_key(|status| std::cmp::Reverse(status.number));
        if statuses.is_empty() {
            continue;
        }
        let layout = layout_by_repository.get(&repository);
        repositories.push(ReviewRequestRepositoryView {
            repository: repository.clone(),
            layout_key: layout.map(|layout| layout.key.clone()),
            root: layout.map(|layout| layout.root.clone()),
            display_root: layout.map(|layout| display_path(&layout.root, environment)),
            external: layout.is_none(),
            review_wait_threshold_seconds: policy.review_wait_threshold_seconds,
            rows: statuses
                .into_iter()
                .map(|status| ReviewRequestRowView {
                    state: review_request_state(&status, &viewer),
                    status,
                })
                .collect(),
        });
        progress.percentage(
            "Loading pull request details",
            index + 1,
            grouped_repo_count,
        );
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
