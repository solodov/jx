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

    /// Returns effective work-item settings for this repository.
    pub fn work_items_for(&self, repository: &GitHubRepository) -> RepoWorkItemsConfig {
        let mut config = self.base.work_items.clone();
        for rule in self.matching_rules(repository) {
            config.apply_layer(rule.policy.work_items.clone());
        }
        config
    }

    /// Returns effective work-item handlers for this repository in deterministic execution order.
    pub fn work_item_handlers_for(
        &self,
        repository: &GitHubRepository,
    ) -> Vec<RepoWorkItemHandler> {
        let mut handlers = Vec::new();
        apply_work_item_handler_configs(&mut handlers, &self.base.work_item_handlers);
        for rule in self.matching_rules(repository) {
            apply_work_item_handler_configs(&mut handlers, &rule.policy.work_item_handlers);
        }
        handlers
    }

    /// Returns whether any lifecycle check is configured for the trigger after repo overrides.
    pub fn has_checks_for_trigger(
        &self,
        repository: &GitHubRepository,
        before: RepoCheckTrigger,
    ) -> bool {
        self.effective_checks_for(repository)
            .iter()
            .any(|check| check.before.contains(&before))
    }

    /// Returns lifecycle checks selected by repo policy, event, and changed files.
    pub fn checks_for(
        &self,
        repository: &GitHubRepository,
        before: RepoCheckTrigger,
        changed_files: &[String],
    ) -> Vec<RepoCheckConfig> {
        self.effective_checks_for(repository)
            .into_iter()
            .filter(|check| check.matches(before, changed_files))
            .collect()
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

    /// Returns repo-level reviewers offered for `jx stack publish --reviewer` completion.
    pub fn reviewer_completion_for(&self, repository: &GitHubRepository) -> Vec<ReviewerTarget> {
        let mut reviewers = Vec::new();
        merge_reviewers(&mut reviewers, self.base.reviewers.clone());
        for rule in self.matching_rules(repository) {
            merge_reviewers(&mut reviewers, rule.policy.reviewers.clone());
        }
        reviewers
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

    fn effective_checks_for(&self, repository: &GitHubRepository) -> Vec<RepoCheckConfig> {
        let mut checks = Vec::new();
        apply_check_configs(&mut checks, &self.base.checks);
        for rule in self.matching_rules(repository) {
            apply_check_configs(&mut checks, &rule.policy.checks);
        }
        checks
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
    pub work_items: RepoWorkItemsConfig,
    pub work_item_handlers: Vec<RepoWorkItemHandlerConfig>,
    pub checks: Vec<RepoCheckConfig>,
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
        self.work_items.apply_layer(layer.work_items);
        merge_work_item_handler_configs(&mut self.work_item_handlers, &layer.work_item_handlers);
        merge_check_configs(&mut self.checks, &layer.checks);
        merge_reviewers(&mut self.reviewers, layer.reviewers);
        self.reviewer_rules.extend(layer.reviewer_rules);
        merge_workspace_shared_paths(
            &mut self.workspace_shared_paths,
            &layer.workspace_shared_paths,
        );
        self.stack_status.apply_layer(layer.stack_status);
    }
}

/// Work-item side-effect behavior that can vary by repository policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoWorkItemsConfig {
    pub apply_on_stack_status: Option<bool>,
}

impl RepoWorkItemsConfig {
    fn apply_layer(&mut self, layer: RepoWorkItemsConfig) {
        if layer.apply_on_stack_status.is_some() {
            self.apply_on_stack_status = layer.apply_on_stack_status;
        }
    }

    /// Returns whether stack-status should apply configured work-item side effects.
    pub fn apply_on_stack_status(&self) -> bool {
        self.apply_on_stack_status.unwrap_or(false)
    }
}

/// Stack status behavior that can vary by repository policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStackStatusConfig {
    pub review_gate_checks: Vec<ReviewGateCheckConfig>,
    pub ignored_checks: Vec<IgnoredCheckConfig>,
    pub ignored_labels: Vec<IgnoredLabelConfig>,
    pub ignored_labels_when_merged: Vec<IgnoredLabelConfig>,
    pub ignored_reviewers: Vec<IgnoredReviewerConfig>,
    pub title_rewrites: Vec<TitleRewriteConfig>,
    pub review_wait_threshold_seconds: Option<u64>,
}

impl RepoStackStatusConfig {
    fn apply_layer(&mut self, layer: RepoStackStatusConfig) {
        merge_review_gate_checks(&mut self.review_gate_checks, layer.review_gate_checks);
        merge_ignored_checks(&mut self.ignored_checks, layer.ignored_checks);
        merge_ignored_labels(&mut self.ignored_labels, layer.ignored_labels);
        merge_ignored_labels(
            &mut self.ignored_labels_when_merged,
            layer.ignored_labels_when_merged,
        );
        merge_ignored_reviewers(&mut self.ignored_reviewers, layer.ignored_reviewers);
        self.title_rewrites.extend(layer.title_rewrites);
        if layer.review_wait_threshold_seconds.is_some() {
            self.review_wait_threshold_seconds = layer.review_wait_threshold_seconds;
        }
    }

    /// Returns whether a GitHub check should be omitted from stack/review status health.
    pub fn ignores_check(&self, check: &str) -> bool {
        self.ignored_checks.iter().any(|rule| rule.matches(check))
    }

    /// Returns whether a GitHub check contributes to the repository-specific review gate.
    pub fn matches_review_gate_check(&self, check: &str) -> bool {
        self.review_gate_checks
            .iter()
            .any(|rule| rule.matches(check))
    }

    /// Returns whether a pull-request label should be omitted from stack/review status views.
    pub fn ignores_label(&self, label: &str) -> bool {
        self.ignored_labels.iter().any(|rule| rule.matches(label))
    }

    /// Returns whether a pull-request label should be omitted after the PR has merged.
    pub fn ignores_label_when_merged(&self, label: &str) -> bool {
        self.ignored_labels_when_merged
            .iter()
            .any(|rule| rule.matches(label))
    }

    /// Returns whether a reviewer token should be omitted from stack/review status views.
    pub fn ignores_reviewer(&self, reviewer: &str) -> bool {
        self.ignored_reviewers
            .iter()
            .any(|rule| rule.matches(reviewer))
    }

    /// Applies repository-specific title presentation rewrites in configured order.
    pub fn rewrite_title(&self, title: &str) -> String {
        self.title_rewrites
            .iter()
            .fold(title.to_owned(), |title, rule| rule.rewrite(&title))
    }
}

/// Regex-based pull-request title rewrite applied before display ellipsizing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleRewriteConfig {
    pub pattern: String,
    pub replace: String,
}

impl TitleRewriteConfig {
    /// Applies this rewrite once using Rust regex replacement syntax such as `$1`.
    pub fn rewrite(&self, title: &str) -> String {
        regex::Regex::new(&self.pattern)
            .ok()
            .map(|regex| regex.replace(title, self.replace.as_str()).into_owned())
            .unwrap_or_else(|| title.to_owned())
    }
}

/// Check-name regex omitted from stack/review status health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredCheckConfig {
    pub name: String,
}

impl IgnoredCheckConfig {
    /// Returns whether this regex matches a GitHub check run or status context name.
    pub fn matches(&self, check_name: &str) -> bool {
        regex::Regex::new(&self.name)
            .ok()
            .is_some_and(|regex| regex.is_match(check_name))
    }
}

/// Label-name glob hidden from stack/review status presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredLabelConfig {
    pub name: String,
}

impl IgnoredLabelConfig {
    /// Returns whether this rule matches a GitHub label name.
    pub fn matches(&self, label_name: &str) -> bool {
        Glob::new(&self.name)
            .ok()
            .map(|glob| glob.compile_matcher().is_match(label_name))
            .unwrap_or(false)
    }
}

/// Reviewer-name glob hidden from stack/review status presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredReviewerConfig {
    pub name: String,
}

impl IgnoredReviewerConfig {
    /// Returns whether this rule matches a user, team slug, or `team/<slug>` reviewer token.
    pub fn matches(&self, reviewer: &str) -> bool {
        Glob::new(&self.name)
            .ok()
            .map(|glob| glob.compile_matcher().is_match(reviewer))
            .unwrap_or(false)
    }
}

/// Check-name glob whose matching status contexts encode repository approval policy.
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

/// One configured work-item handler or a disabling override by handler id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoWorkItemHandlerConfig {
    Handler(RepoWorkItemHandler),
    Disable { id: String },
}

/// Check-only command that can block selected lifecycle operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCheckConfig {
    pub id: String,
    pub before: Vec<RepoCheckTrigger>,
    pub paths: Vec<String>,
    pub command: Vec<String>,
}

impl RepoCheckConfig {
    fn matches(&self, before: RepoCheckTrigger, changed_files: &[String]) -> bool {
        self.before.contains(&before) && self.matches_changed_files(changed_files)
    }

    fn matches_changed_files(&self, changed_files: &[String]) -> bool {
        self.paths.iter().any(|pattern| {
            Glob::new(pattern)
                .ok()
                .map(|glob| {
                    let matcher = glob.compile_matcher();
                    changed_files.iter().any(|path| matcher.is_match(path))
                })
                .unwrap_or(false)
        })
    }
}

/// Lifecycle points that can run configured repo checks before mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoCheckTrigger {
    PullRequest,
    Push,
    Sync,
}

impl RepoCheckTrigger {
    /// Stable config label for this check trigger.
    pub fn label(self) -> &'static str {
        match self {
            Self::PullRequest => "pull_request",
            Self::Push => "push",
            Self::Sync => "sync",
        }
    }
}

/// Effect to run when a matching repository event is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEventHandler {
    pub id: Option<String>,
    pub on: RepoEvent,
    pub when: PullRequestEventQuery,
    pub run: RepoEventHandlerRun,
}

/// Side effect to run when a matching work-item event is emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoWorkItemHandler {
    pub id: Option<String>,
    pub on: RepoWorkItemEvent,
    pub command: Vec<String>,
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

/// Work-item events supported by configured handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoWorkItemEvent {
    Fixed,
}

impl RepoWorkItemEvent {
    /// Stable config label for this event, used in operator-facing handler output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "work_item.fixed",
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

fn apply_work_item_handler_configs(
    target: &mut Vec<RepoWorkItemHandler>,
    configs: &[RepoWorkItemHandlerConfig],
) {
    for config in configs {
        match config {
            RepoWorkItemHandlerConfig::Handler(handler) => {
                if let Some(id) = &handler.id {
                    target.retain(|existing| existing.id.as_deref() != Some(id.as_str()));
                }
                target.push(handler.clone());
            }
            RepoWorkItemHandlerConfig::Disable { id } => {
                target.retain(|handler| handler.id.as_deref() != Some(id.as_str()));
            }
        }
    }
}

fn merge_work_item_handler_configs(
    target: &mut Vec<RepoWorkItemHandlerConfig>,
    configs: &[RepoWorkItemHandlerConfig],
) {
    for config in configs {
        match config {
            RepoWorkItemHandlerConfig::Handler(handler) => {
                if let Some(id) = &handler.id {
                    target.retain(|existing| {
                        work_item_handler_config_id(existing) != Some(id.as_str())
                    });
                }
                target.push(config.clone());
            }
            RepoWorkItemHandlerConfig::Disable { id } => {
                target
                    .retain(|existing| work_item_handler_config_id(existing) != Some(id.as_str()));
            }
        }
    }
}

fn work_item_handler_config_id(config: &RepoWorkItemHandlerConfig) -> Option<&str> {
    match config {
        RepoWorkItemHandlerConfig::Handler(handler) => handler.id.as_deref(),
        RepoWorkItemHandlerConfig::Disable { id } => Some(id.as_str()),
    }
}

fn apply_check_configs(target: &mut Vec<RepoCheckConfig>, configs: &[RepoCheckConfig]) {
    for config in configs {
        target.retain(|existing| existing.id != config.id);
        target.push(config.clone());
    }
}

fn merge_check_configs(target: &mut Vec<RepoCheckConfig>, configs: &[RepoCheckConfig]) {
    for config in configs {
        target.retain(|existing| existing.id != config.id);
        target.push(config.clone());
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

fn merge_ignored_checks(target: &mut Vec<IgnoredCheckConfig>, checks: Vec<IgnoredCheckConfig>) {
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

fn merge_ignored_labels(target: &mut Vec<IgnoredLabelConfig>, labels: Vec<IgnoredLabelConfig>) {
    let mut seen = target
        .iter()
        .map(|label| label.name.clone())
        .collect::<BTreeSet<_>>();
    for label in labels {
        if seen.insert(label.name.clone()) {
            target.push(label);
        }
    }
}

fn merge_ignored_reviewers(
    target: &mut Vec<IgnoredReviewerConfig>,
    reviewers: Vec<IgnoredReviewerConfig>,
) {
    let mut seen = target
        .iter()
        .map(|reviewer| reviewer.name.clone())
        .collect::<BTreeSet<_>>();
    for reviewer in reviewers {
        if seen.insert(reviewer.name.clone()) {
            target.push(reviewer);
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
