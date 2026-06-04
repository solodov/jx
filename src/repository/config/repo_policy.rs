use super::*;

/// Repository-matched workflow policy loaded from optional config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoConfig {
    pub base: RepoPolicyConfig,
    pub rules: Vec<RepoRuleConfig>,
}

impl RepoConfig {
    pub(super) fn apply_layer(&mut self, layer: RepoConfig) {
        self.base.apply_layer(layer.base);
        self.rules.extend(layer.rules);
    }

    /// Returns whether sync should move the local trunk bookmark for this repository.
    pub fn advance_trunk_enabled_for(&self, repository: &GitHubRepository) -> bool {
        let mut enabled = self.base.advance_trunk.unwrap_or(false);
        for rule in self.matching_rules(repository) {
            if let Some(value) = rule.policy.advance_trunk {
                enabled = value;
            }
        }
        enabled
    }

    /// Returns effective event handlers for this repository in deterministic execution order.
    pub fn event_handlers_for(&self, repository: &GitHubRepository) -> Vec<RepoEventHandler> {
        let mut handlers = Vec::new();
        apply_event_handler_configs(&mut handlers, &self.base.event_handlers);
        for rule in self.matching_rules(repository) {
            apply_event_handler_configs(&mut handlers, &rule.policy.event_handlers);
        }
        handlers
    }

    /// Returns reviewer candidates selected by repo policy and matching file rules.
    pub fn reviewer_candidates_for(
        &self,
        repository: &GitHubRepository,
        changed_files: &[String],
    ) -> Vec<ReviewerCandidate> {
        let mut candidates = Vec::new();
        add_repo_policy_reviewer_candidates(&mut candidates, &self.base, changed_files);
        for rule in self.matching_rules(repository) {
            add_repo_policy_reviewer_candidates(&mut candidates, &rule.policy, changed_files);
        }
        candidates
    }

    /// Returns normalized shared workspace paths from base policy and matching repo rules.
    pub fn workspace_shared_paths_for(
        &self,
        repository: &GitHubRepository,
    ) -> Result<Vec<String>, RepositoryError> {
        let mut paths = self.base.workspace_shared_paths.clone();
        for rule in self.matching_rules(repository) {
            paths.extend(rule.policy.workspace_shared_paths.clone());
        }

        normalize_workspace_shared_path_set("jx config", "workspace_shared_paths", &paths)
    }

    /// Returns effective stack status policy for this repository.
    pub fn stack_status_for(&self, repository: &GitHubRepository) -> RepoStackStatusConfig {
        let mut config = self.base.stack_status.clone();
        for rule in self.matching_rules(repository) {
            config.apply_layer(rule.policy.stack_status.clone());
        }
        config
    }

    pub(super) fn validate(&self) -> Result<(), RepositoryError> {
        validate_workspace_shared_path_set(
            "jx config",
            "repo.workspace_shared_paths",
            &self.base.workspace_shared_paths,
        )?;
        for (index, rule) in self.rules.iter().enumerate() {
            validate_workspace_shared_path_set(
                "jx config",
                &format!("repo.rules[{index}].workspace_shared_paths"),
                &rule.policy.workspace_shared_paths,
            )?;
        }

        Ok(())
    }

    pub(super) fn reviewer_summary_for(&self, repository: &GitHubRepository) -> String {
        let mut reviewers = BTreeSet::new();
        reviewers.extend(self.base.reviewers.iter().cloned());
        let mut rules = self.base.reviewer_rules.len();

        for rule in self.matching_rules(repository) {
            reviewers.extend(rule.policy.reviewers.iter().cloned());
            rules += rule.policy.reviewer_rules.len();
        }

        match (reviewers.len(), rules) {
            (0, 0) => "none".to_owned(),
            (base, 0) => reviewer_count_summary(base),
            (0, rules) => reviewer_rule_count_summary(rules),
            (base, rules) => format!(
                "{}, {}",
                reviewer_count_summary(base),
                reviewer_rule_count_summary(rules)
            ),
        }
    }

    fn matching_rules(&self, repository: &GitHubRepository) -> Vec<&RepoRuleConfig> {
        let slug = repository.slug();
        self.rules
            .iter()
            .filter(|rule| rule.matches(&slug))
            .collect()
    }
}

/// One layer of repo-scoped workflow behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoPolicyConfig {
    pub advance_trunk: Option<bool>,
    pub event_handlers: Vec<RepoEventHandlerConfig>,
    pub reviewers: Vec<ReviewerTarget>,
    pub reviewer_rules: Vec<ReviewerPathRule>,
    pub workspace_shared_paths: Vec<String>,
    pub stack_status: RepoStackStatusConfig,
}

impl RepoPolicyConfig {
    fn apply_layer(&mut self, layer: RepoPolicyConfig) {
        if layer.advance_trunk.is_some() {
            self.advance_trunk = layer.advance_trunk;
        }
        merge_event_handler_configs(&mut self.event_handlers, &layer.event_handlers);
        merge_reviewers(&mut self.reviewers, layer.reviewers);
        self.reviewer_rules.extend(layer.reviewer_rules);
        merge_workspace_shared_paths(
            &mut self.workspace_shared_paths,
            &layer.workspace_shared_paths,
        );
        self.stack_status.apply_layer(layer.stack_status);
    }
}

/// Stack status behavior that can vary by repository policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStackStatusConfig {
    pub review_gate_checks: Vec<ReviewGateCheckConfig>,
}

impl RepoStackStatusConfig {
    fn apply_layer(&mut self, layer: RepoStackStatusConfig) {
        merge_review_gate_checks(&mut self.review_gate_checks, layer.review_gate_checks);
    }
}

/// Check-name glob that reports approval requirements instead of test failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewGateCheckConfig {
    pub name: String,
}

impl ReviewGateCheckConfig {
    /// Returns whether this rule matches a GitHub check run or status context name.
    pub fn matches(&self, check_name: &str) -> bool {
        Glob::new(&self.name)
            .ok()
            .map(|glob| glob.compile_matcher().is_match(check_name))
            .unwrap_or(false)
    }
}

/// One configured repository event handler or a disabling override by handler id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoEventHandlerConfig {
    Handler(RepoEventHandler),
    Disable { id: String },
}

/// Effect to run when a matching repository event is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEventHandler {
    pub id: Option<String>,
    pub on: RepoEvent,
    pub when: PullRequestEventQuery,
    pub run: RepoEventHandlerRun,
}

/// Repository events supported by configured handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoEvent {
    PullRequestPrepare,
    PullRequestCreated,
    PullRequestUpdated,
}

impl RepoEvent {
    /// Stable config label for this event, used in operator-facing handler output.
    pub fn label(self) -> &'static str {
        match self {
            Self::PullRequestPrepare => "pull_request.prepare",
            Self::PullRequestCreated => "pull_request.created",
            Self::PullRequestUpdated => "pull_request.updated",
        }
    }
}

/// Handler action to execute after an event passes its query filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoEventHandlerRun {
    AddLabels { labels: Vec<String> },
    OpenPullRequest,
    PrependTaskId,
}

/// Parsed pull-request event filter. Terms are ANDed and may be negated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestEventQuery {
    pub terms: Vec<PullRequestEventQueryTerm>,
}

/// One pull-request event predicate, optionally negated by a leading `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestEventQueryTerm {
    pub predicate: PullRequestEventPredicate,
    pub negated: bool,
}

/// Pull-request event facts that can be matched by config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullRequestEventPredicate {
    Draft,
    Ready,
    HasReviewers,
    HasTask,
    Label(String),
}

/// Repository glob plus policy overrides for matching `origin` GitHub slugs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRuleConfig {
    pub repo: String,
    pub policy: RepoPolicyConfig,
}

impl RepoRuleConfig {
    fn matches(&self, slug: &str) -> bool {
        Glob::new(&self.repo)
            .ok()
            .map(|glob| glob.compile_matcher().is_match(slug))
            .unwrap_or(false)
    }
}

/// File ownership rule that can add reviewer candidates for matching PR changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerPathRule {
    pub paths: Vec<String>,
    pub reviewers: Vec<ReviewerTarget>,
}

impl ReviewerPathRule {
    fn match_reasons(&self, changed_files: &[String]) -> Vec<String> {
        self.paths
            .iter()
            .filter_map(|pattern| {
                let matcher = Glob::new(pattern).ok()?.compile_matcher();
                let count = changed_files
                    .iter()
                    .filter(|path| matcher.is_match(path))
                    .count();
                (count > 0).then(|| matched_path_reason(pattern, count))
            })
            .collect()
    }
}

fn add_repo_policy_reviewer_candidates(
    candidates: &mut Vec<ReviewerCandidate>,
    policy: &RepoPolicyConfig,
    changed_files: &[String],
) {
    for reviewer in &policy.reviewers {
        add_reviewer_candidate(candidates, reviewer, "repo".to_owned());
    }

    for rule in &policy.reviewer_rules {
        let reasons = rule.match_reasons(changed_files);
        for reviewer in &rule.reviewers {
            for reason in &reasons {
                add_reviewer_candidate(candidates, reviewer, reason.clone());
            }
        }
    }
}

fn apply_event_handler_configs(
    target: &mut Vec<RepoEventHandler>,
    configs: &[RepoEventHandlerConfig],
) {
    for config in configs {
        match config {
            RepoEventHandlerConfig::Handler(handler) => {
                if let Some(id) = &handler.id {
                    target.retain(|existing| existing.id.as_deref() != Some(id.as_str()));
                }
                target.push(handler.clone());
            }
            RepoEventHandlerConfig::Disable { id } => {
                target.retain(|handler| handler.id.as_deref() != Some(id.as_str()));
            }
        }
    }
}

fn merge_event_handler_configs(
    target: &mut Vec<RepoEventHandlerConfig>,
    configs: &[RepoEventHandlerConfig],
) {
    for config in configs {
        match config {
            RepoEventHandlerConfig::Handler(handler) => {
                if let Some(id) = &handler.id {
                    target
                        .retain(|existing| event_handler_config_id(existing) != Some(id.as_str()));
                }
                target.push(config.clone());
            }
            RepoEventHandlerConfig::Disable { id } => {
                target.retain(|existing| event_handler_config_id(existing) != Some(id.as_str()));
            }
        }
    }
}

fn event_handler_config_id(config: &RepoEventHandlerConfig) -> Option<&str> {
    match config {
        RepoEventHandlerConfig::Handler(handler) => handler.id.as_deref(),
        RepoEventHandlerConfig::Disable { id } => Some(id.as_str()),
    }
}

fn merge_reviewers(target: &mut Vec<ReviewerTarget>, reviewers: Vec<ReviewerTarget>) {
    let mut seen = target.iter().cloned().collect::<BTreeSet<_>>();
    for reviewer in reviewers {
        if seen.insert(reviewer.clone()) {
            target.push(reviewer);
        }
    }
}

fn merge_workspace_shared_paths(target: &mut Vec<String>, paths: &[String]) {
    let mut seen = target.iter().cloned().collect::<BTreeSet<_>>();
    for path in paths {
        if seen.insert(path.clone()) {
            target.push(path.clone());
        }
    }
}

fn merge_review_gate_checks(
    target: &mut Vec<ReviewGateCheckConfig>,
    checks: Vec<ReviewGateCheckConfig>,
) {
    let mut seen = target
        .iter()
        .map(|check| check.name.clone())
        .collect::<BTreeSet<_>>();
    for check in checks {
        if seen.insert(check.name.clone()) {
            target.push(check);
        }
    }
}

pub(super) fn normalize_workspace_shared_path(
    file: &str,
    key: &str,
    path: &str,
) -> Result<String, RepositoryError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must not contain empty paths"),
        });
    }
    if Path::new(path).is_absolute() || path.starts_with('/') || has_windows_drive_prefix(path) {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` path `{path}` must be repo-relative"),
        });
    }
    if path.contains('\\') {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` path `{path}` must use forward slash separators"),
        });
    }

    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!("`{key}` path `{path}` must not contain `..` components"),
                });
            }
            component => components.push(component),
        }
    }

    if components.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must not contain empty paths"),
        });
    }

    Ok(components.join("/"))
}

pub(super) fn normalize_workspace_shared_path_set(
    file: &str,
    key: &str,
    paths: &[String],
) -> Result<Vec<String>, RepositoryError> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let path = normalize_workspace_shared_path(file, key, path)?;
        if seen.insert(path.clone()) {
            normalized.push(path);
        }
    }

    let mut sorted = normalized.iter().collect::<Vec<_>>();
    sorted.sort();
    for (index, parent) in sorted.iter().enumerate() {
        for child in sorted.iter().skip(index + 1) {
            if workspace_shared_path_is_parent(parent, child) {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!(
                        "`{key}` contains overlapping paths `{parent}` and `{child}`; configure only one of them"
                    ),
                });
            }
        }
    }

    Ok(normalized)
}

pub(super) fn validate_workspace_shared_path_set(
    file: &str,
    key: &str,
    paths: &[String],
) -> Result<(), RepositoryError> {
    normalize_workspace_shared_path_set(file, key, paths).map(drop)
}

fn workspace_shared_path_is_parent(parent: &str, child: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn add_reviewer_candidate(
    candidates: &mut Vec<ReviewerCandidate>,
    reviewer: &ReviewerTarget,
    reason: String,
) {
    if let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| &candidate.target == reviewer)
    {
        if !candidate.reasons.contains(&reason) {
            candidate.reasons.push(reason);
        }
        return;
    }

    candidates.push(ReviewerCandidate::new(reviewer.clone(), vec![reason]));
}

fn matched_path_reason(pattern: &str, count: usize) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!("{pattern} matched {count} {noun}")
}

fn reviewer_count_summary(count: usize) -> String {
    if count == 1 {
        "1 configured reviewer".to_owned()
    } else {
        format!("{count} configured reviewers")
    }
}

fn reviewer_rule_count_summary(count: usize) -> String {
    if count == 1 {
        "1 reviewer rule".to_owned()
    } else {
        format!("{count} reviewer rules")
    }
}
