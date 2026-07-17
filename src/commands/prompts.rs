use super::*;

const MIN_DIALOGUER_TERMINAL_ROWS: usize = 3;

pub(super) struct PromptHandlers<'a> {
    pub(super) pull_request_previewer: &'a dyn PullRequestPreviewer,
    pub(super) pull_request_selector: &'a dyn PullRequestSelector,
    pub(super) reviewer_selector: &'a dyn ReviewerSelector,
    pub(super) pull_request_confirmer: &'a dyn PullRequestConfirmer,
    pub(super) push_confirmer: &'a dyn PushConfirmer,
    pub(super) repository_initialization_confirmer: &'a dyn RepositoryInitializationConfirmer,
    pub(super) repository_creation_confirmer: &'a dyn RepositoryCreationConfirmer,
    pub(super) workspace_remove_confirmer: &'a dyn WorkspaceRemoveConfirmer,
}

/// Shows the operator-facing PR summary before any publishing mutation.
pub(super) trait PullRequestPreviewer {
    fn show_preview(
        &self,
        plan: &PullRequestPlan,
        status: &WorkspaceStatus,
        prepare_effects: &[PullRequestEventEffect],
    );
}

pub(super) struct TerminalPullRequestPreviewer;

impl PullRequestPreviewer for TerminalPullRequestPreviewer {
    fn show_preview(
        &self,
        plan: &PullRequestPlan,
        status: &WorkspaceStatus,
        prepare_effects: &[PullRequestEventEffect],
    ) {
        eprint!(
            "{}",
            render_pull_request_preview_with_style(
                plan,
                status,
                prepare_effects,
                io::stderr().is_terminal(),
            )
        );
    }
}

#[cfg(test)]
pub(super) struct NoPullRequestPreview;

#[cfg(test)]
impl PullRequestPreviewer for NoPullRequestPreview {
    fn show_preview(
        &self,
        _plan: &PullRequestPlan,
        _status: &WorkspaceStatus,
        _prepare_effects: &[PullRequestEventEffect],
    ) {
    }
}

/// Selects an existing pull request to open when the operator requests an interactive list.
pub(super) trait PullRequestSelector {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError>;
}

#[derive(Debug, Error)]
pub enum PullRequestSelectionError {
    #[error("Cannot select a pull request without an interactive terminal")]
    NonInteractive,
    #[error("No pull requests are available to select")]
    NoPullRequests,
    #[error("Pull request selection cancelled")]
    Cancelled,
    #[error("Could not read pull request selection: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalPullRequestSelector;

impl PullRequestSelector for TerminalPullRequestSelector {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PullRequestSelectionError::NonInteractive);
        }
        ensure_dialoguer_prompt_space()
            .map_err(|source| PullRequestSelectionError::Read { source })?;
        if choices.is_empty() {
            return Err(PullRequestSelectionError::NoPullRequests);
        }

        let labels = choices
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>();
        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt("Open pull request:")
            .items(&labels)
            .default(0)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                PullRequestSelectionError::Read { source }
            })?
            .ok_or(PullRequestSelectionError::Cancelled)?;

        Ok(choices[selected].pull_request.clone())
    }
}

#[cfg(test)]
pub(super) struct SelectFirstPullRequest;

#[cfg(test)]
impl PullRequestSelector for SelectFirstPullRequest {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        choices
            .first()
            .map(|choice| choice.pull_request.clone())
            .ok_or(PullRequestSelectionError::NoPullRequests)
    }
}

#[cfg(test)]
pub(super) struct FixedPullRequestSelector {
    pub(super) selected: usize,
}

#[cfg(test)]
impl PullRequestSelector for FixedPullRequestSelector {
    fn select_pull_request(
        &self,
        choices: &[PullRequestChoice],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        choices
            .get(self.selected)
            .map(|choice| choice.pull_request.clone())
            .ok_or(PullRequestSelectionError::NoPullRequests)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PullRequestChoice {
    pub(super) pull_request: PullRequestRecord,
    pub(super) label: String,
}

pub(super) fn pull_request_choice_rows(
    snapshot: &PullRequestStackSnapshot,
) -> Vec<PullRequestChoice> {
    snapshot
        .rows()
        .into_iter()
        .filter_map(|row| {
            let pull_request = pull_request_record_from_stack_node(row.node)?;
            Some(PullRequestChoice {
                pull_request,
                label: render_stack_row_label(row, true),
            })
        })
        .collect()
}

#[cfg(test)]
pub(super) fn pull_request_choice_label(pull_request: &PullRequestRecord) -> String {
    let snapshot = PullRequestStackSnapshot::from_metadata(
        &StackMetadata::default(),
        std::slice::from_ref(&pull_request.head_branch),
        std::slice::from_ref(pull_request),
        PullRequestStackSelection::default(),
    );
    pull_request_choice_rows(&snapshot)
        .into_iter()
        .next()
        .map(|choice| choice.label)
        .unwrap_or_default()
}

fn pull_request_record_from_stack_node(node: &PullRequestStackNode) -> Option<PullRequestRecord> {
    let pull_request = node.pull_request.as_ref()?;
    Some(PullRequestRecord {
        number: pull_request.number,
        title: node.title.clone(),
        body: None,
        head_branch: node.branch.clone(),
        base_branch: node.base_branch.clone(),
        html_url: pull_request.url.clone(),
        draft: node.draft,
        merged: node.merged,
        reviewers: ReviewerSelection::default(),
    })
}

/// Confirms whether a planned PR should proceed to bookmark, push, and GitHub mutation.
pub(super) trait PullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError>;
}

#[derive(Debug, Error)]
pub enum PullRequestConfirmationError {
    #[error("Cannot confirm pull request publishing without an interactive terminal")]
    NonInteractive,
    #[error("Could not read pull request confirmation: {source}")]
    Read { source: dialoguer::Error },
}

fn ensure_dialoguer_prompt_space() -> Result<(), dialoguer::Error> {
    let rows = dialoguer::console::Term::stderr().size().0 as usize;
    if rows < MIN_DIALOGUER_TERMINAL_ROWS {
        return Err(dialoguer::Error::from(io::Error::other(format!(
            "terminal is too short for interactive prompts ({rows} rows; need at least {MIN_DIALOGUER_TERMINAL_ROWS}); resize and retry"
        ))));
    }
    Ok(())
}

/// Plain, low-noise prompt theme for reviewer and confirmation selectors.
pub(super) struct PlainPromptTheme;

impl Theme for PlainPromptTheme {
    fn format_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write!(f, "{prompt}")
    }

    fn format_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: &str,
    ) -> fmt::Result {
        write!(f, "{prompt} {selection}")
    }

    fn format_multi_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        write!(f, "{prompt}")
    }

    fn format_multi_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selections: &[&str],
    ) -> fmt::Result {
        write!(f, "{prompt}")?;
        if !selections.is_empty() {
            write!(f, " {}", selections.join(", "))?;
        }
        Ok(())
    }

    fn format_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        active: bool,
    ) -> fmt::Result {
        write!(f, "{} {text}", if active { "❯" } else { " " })
    }

    fn format_multi_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        checked: bool,
        active: bool,
    ) -> fmt::Result {
        let cursor = if active { "❯" } else { " " };
        let marker = if checked { "[x]" } else { "[ ]" };
        write!(f, "{cursor} {marker} {text}")
    }
}

pub(super) struct TerminalPullRequestConfirmer;

impl PullRequestConfirmer for TerminalPullRequestConfirmer {
    fn confirm_pull_request(
        &self,
        plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PullRequestConfirmationError::NonInteractive);
        }
        ensure_dialoguer_prompt_space()
            .map_err(|source| PullRequestConfirmationError::Read { source })?;

        eprintln!();
        let theme = PlainPromptTheme;
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(pull_request_confirmation_prompt(plan))
            .default(true)
            .show_default(false)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                PullRequestConfirmationError::Read { source }
            })?;

        Ok(confirmed.unwrap_or(false))
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmPullRequest;

#[cfg(test)]
impl PullRequestConfirmer for AlwaysConfirmPullRequest {
    fn confirm_pull_request(
        &self,
        _plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        Ok(true)
    }
}

/// Shared non-interactive yes implementation for explicit batch confirmation mode.
pub(super) struct YesConfirmer;

impl PullRequestConfirmer for YesConfirmer {
    fn confirm_pull_request(
        &self,
        _plan: &PullRequestPlan,
    ) -> Result<bool, PullRequestConfirmationError> {
        Ok(true)
    }
}

/// Confirms whether a generated push bookmark should be created and pushed.
pub(super) trait PushConfirmer {
    fn confirm_push(&self, plan: &PushPlan) -> Result<bool, PushConfirmationError>;
}

#[derive(Debug, Error)]
pub enum PushConfirmationError {
    #[error("Cannot confirm bookmark creation without an interactive terminal")]
    NonInteractive,
    #[error("Could not read push confirmation: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalPushConfirmer;

impl PushConfirmer for TerminalPushConfirmer {
    fn confirm_push(&self, plan: &PushPlan) -> Result<bool, PushConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PushConfirmationError::NonInteractive);
        }
        ensure_dialoguer_prompt_space().map_err(|source| PushConfirmationError::Read { source })?;

        eprintln!();
        let theme = PlainPromptTheme;
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(push_confirmation_prompt(plan))
            .default(false)
            .show_default(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                PushConfirmationError::Read { source }
            })?;

        Ok(confirmed.unwrap_or(false))
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmPush;

#[cfg(test)]
impl PushConfirmer for AlwaysConfirmPush {
    fn confirm_push(&self, _plan: &PushPlan) -> Result<bool, PushConfirmationError> {
        Ok(true)
    }
}

impl PushConfirmer for YesConfirmer {
    fn confirm_push(&self, _plan: &PushPlan) -> Result<bool, PushConfirmationError> {
        Ok(true)
    }
}

/// Confirms whether a layout path should be initialized as a local jj repository.
pub(super) trait RepositoryInitializationConfirmer {
    fn confirm_repository_initialization(
        &self,
        workspace_root: &Path,
    ) -> Result<bool, RepositoryInitializationConfirmationError>;
}

#[derive(Debug, Error)]
pub enum RepositoryInitializationConfirmationError {
    #[error("Cannot confirm repository initialization without an interactive terminal")]
    NonInteractive,
    #[error("Could not read repository initialization confirmation: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalRepositoryInitializationConfirmer;

impl RepositoryInitializationConfirmer for TerminalRepositoryInitializationConfirmer {
    fn confirm_repository_initialization(
        &self,
        workspace_root: &Path,
    ) -> Result<bool, RepositoryInitializationConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(RepositoryInitializationConfirmationError::NonInteractive);
        }
        ensure_dialoguer_prompt_space()
            .map_err(|source| RepositoryInitializationConfirmationError::Read { source })?;

        let theme = PlainPromptTheme;
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(format!(
                "Initialize jj repository at {}?",
                workspace_root.display()
            ))
            .default(false)
            .show_default(false)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                RepositoryInitializationConfirmationError::Read { source }
            })?;

        Ok(confirmed.unwrap_or(false))
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmRepositoryInitialization;

#[cfg(test)]
impl RepositoryInitializationConfirmer for AlwaysConfirmRepositoryInitialization {
    fn confirm_repository_initialization(
        &self,
        _workspace_root: &Path,
    ) -> Result<bool, RepositoryInitializationConfirmationError> {
        Ok(true)
    }
}

impl RepositoryInitializationConfirmer for YesConfirmer {
    fn confirm_repository_initialization(
        &self,
        _workspace_root: &Path,
    ) -> Result<bool, RepositoryInitializationConfirmationError> {
        Ok(true)
    }
}

#[cfg(test)]
pub(super) struct FixedRepositoryInitializationConfirmer {
    pub(super) confirmed: bool,
}

#[cfg(test)]
impl RepositoryInitializationConfirmer for FixedRepositoryInitializationConfirmer {
    fn confirm_repository_initialization(
        &self,
        _workspace_root: &Path,
    ) -> Result<bool, RepositoryInitializationConfirmationError> {
        Ok(self.confirmed)
    }
}

/// Confirms whether a missing-origin repository should be created on GitHub.
pub(super) trait RepositoryCreationConfirmer {
    fn confirm_repository_creation(
        &self,
        plan: &RepositoryBootstrapPlan,
    ) -> Result<bool, RepositoryCreationConfirmationError>;
}

#[derive(Debug, Error)]
pub enum RepositoryCreationConfirmationError {
    #[error("Cannot confirm repository creation without an interactive terminal")]
    NonInteractive,
    #[error("Could not read repository creation confirmation: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalRepositoryCreationConfirmer;

impl RepositoryCreationConfirmer for TerminalRepositoryCreationConfirmer {
    fn confirm_repository_creation(
        &self,
        plan: &RepositoryBootstrapPlan,
    ) -> Result<bool, RepositoryCreationConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(RepositoryCreationConfirmationError::NonInteractive);
        }
        ensure_dialoguer_prompt_space()
            .map_err(|source| RepositoryCreationConfirmationError::Read { source })?;

        let theme = PlainPromptTheme;
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(format!("Create private {} repository?", plan.remote_url))
            .default(false)
            .show_default(false)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                RepositoryCreationConfirmationError::Read { source }
            })?;

        Ok(confirmed.unwrap_or(false))
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmRepositoryCreation;

#[cfg(test)]
impl RepositoryCreationConfirmer for AlwaysConfirmRepositoryCreation {
    fn confirm_repository_creation(
        &self,
        _plan: &RepositoryBootstrapPlan,
    ) -> Result<bool, RepositoryCreationConfirmationError> {
        Ok(true)
    }
}

impl RepositoryCreationConfirmer for YesConfirmer {
    fn confirm_repository_creation(
        &self,
        _plan: &RepositoryBootstrapPlan,
    ) -> Result<bool, RepositoryCreationConfirmationError> {
        Ok(true)
    }
}

#[cfg(test)]
pub(super) struct FixedRepositoryCreationConfirmer {
    pub(super) confirmed: bool,
}

#[cfg(test)]
impl RepositoryCreationConfirmer for FixedRepositoryCreationConfirmer {
    fn confirm_repository_creation(
        &self,
        _plan: &RepositoryBootstrapPlan,
    ) -> Result<bool, RepositoryCreationConfirmationError> {
        Ok(self.confirmed)
    }
}

/// Confirms whether a managed workspace should be forgotten and deleted.
pub(super) trait WorkspaceRemoveConfirmer {
    fn confirm_workspace_remove(
        &self,
        workspace: &WorkspaceEntry,
        display_root: &str,
    ) -> Result<bool, WorkspaceRemoveConfirmationError>;
}

#[derive(Debug, Error)]
pub enum WorkspaceRemoveConfirmationError {
    #[error("Cannot confirm workspace deletion without an interactive terminal")]
    NonInteractive,
    #[error("Could not read workspace deletion confirmation: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalWorkspaceRemoveConfirmer;

impl WorkspaceRemoveConfirmer for TerminalWorkspaceRemoveConfirmer {
    fn confirm_workspace_remove(
        &self,
        workspace: &WorkspaceEntry,
        display_root: &str,
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(WorkspaceRemoveConfirmationError::NonInteractive);
        }
        ensure_dialoguer_prompt_space()
            .map_err(|source| WorkspaceRemoveConfirmationError::Read { source })?;

        let theme = PlainPromptTheme;
        let confirmed = Confirm::with_theme(&theme)
            .with_prompt(workspace_remove_confirmation_prompt(
                workspace,
                display_root,
            ))
            .default(false)
            .show_default(false)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                WorkspaceRemoveConfirmationError::Read { source }
            })?;

        Ok(confirmed.unwrap_or(false))
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmWorkspaceRemove;

#[cfg(test)]
impl WorkspaceRemoveConfirmer for AlwaysConfirmWorkspaceRemove {
    fn confirm_workspace_remove(
        &self,
        _workspace: &WorkspaceEntry,
        _display_root: &str,
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        Ok(true)
    }
}

impl WorkspaceRemoveConfirmer for YesConfirmer {
    fn confirm_workspace_remove(
        &self,
        _workspace: &WorkspaceEntry,
        _display_root: &str,
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        Ok(true)
    }
}

pub(super) trait ReviewerSelector {
    fn select_reviewers(
        &self,
        candidates: &[ReviewerCandidate],
        preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError>;
}

#[derive(Debug, Error)]
pub enum ReviewerSelectionError {
    #[error("Reviewer selection cancelled")]
    Cancelled,
    #[error("Could not read reviewer selection: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalReviewerSelector;

impl ReviewerSelector for TerminalReviewerSelector {
    fn select_reviewers(
        &self,
        candidates: &[ReviewerCandidate],
        preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        let choices = reviewer_choices(candidates, preselected);
        if choices.is_empty() {
            return Ok(ReviewerSelection::default());
        }

        if !io::stdin().is_terminal()
            || !io::stderr().is_terminal()
            || ensure_dialoguer_prompt_space().is_err()
        {
            return Ok(selection_from_choices(
                choices.iter().filter(|choice| choice.checked),
            ));
        }

        let labels = choices
            .iter()
            .map(ReviewerChoice::label)
            .collect::<Vec<_>>();
        let defaults = choices
            .iter()
            .map(|choice| choice.checked)
            .collect::<Vec<_>>();
        eprintln!();
        let theme = PlainPromptTheme;
        let selected = MultiSelect::with_theme(&theme)
            .with_prompt("Reviewers:")
            .items(&labels)
            .defaults(&defaults)
            .clear(true)
            .report(false)
            .interact_opt()
            .map_err(|source| {
                restore_terminal_cursor();
                ReviewerSelectionError::Read { source }
            })?
            .ok_or(ReviewerSelectionError::Cancelled)?;

        let selected_reviewers = selection_from_indexes(&choices, &selected);
        eprintln!(
            "Reviewers: {}",
            reviewer_selection_summary(&choices, &selected)
        );
        Ok(selected_reviewers)
    }
}

#[cfg(test)]
pub(super) struct SelectAllReviewers;

#[cfg(test)]
impl ReviewerSelector for SelectAllReviewers {
    fn select_reviewers(
        &self,
        candidates: &[ReviewerCandidate],
        preselected: &[ReviewerTarget],
    ) -> Result<ReviewerSelection, ReviewerSelectionError> {
        let choices = reviewer_choices(candidates, preselected);
        Ok(selection_from_choices(choices.iter()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerChoice {
    pub(super) target: ReviewerTarget,
    reasons: Vec<String>,
    pub(super) checked: bool,
}

const REVIEWER_HINT_STYLE: &str = "\x1b[38;5;244m";
const REVIEWER_HINT_RESET_STYLE: &str = "\x1b[0m";

impl ReviewerChoice {
    pub(super) fn label(&self) -> String {
        let name = self.target.display_name();
        match reviewer_reason_hint(&self.reasons) {
            Some(hint) => {
                format!("{name:<24} {REVIEWER_HINT_STYLE}{hint}{REVIEWER_HINT_RESET_STYLE}")
            }
            None => name.to_owned(),
        }
    }
}

pub(super) fn reviewer_choices(
    candidates: &[ReviewerCandidate],
    preselected: &[ReviewerTarget],
) -> Vec<ReviewerChoice> {
    let mut choices: Vec<ReviewerChoice> = Vec::new();

    for target in preselected {
        if choices
            .iter()
            .any(|choice| choice.target.matches_identity(target))
        {
            continue;
        }
        choices.push(ReviewerChoice {
            target: target.clone(),
            reasons: Vec::new(),
            checked: true,
        });
    }

    for candidate in candidates {
        if let Some(choice) = choices
            .iter_mut()
            .find(|choice| choice.target.matches_identity(&candidate.target))
        {
            for reason in &candidate.reasons {
                if !choice.reasons.contains(reason) {
                    choice.reasons.push(reason.clone());
                }
            }
        } else {
            choices.push(ReviewerChoice {
                target: candidate.target.clone(),
                reasons: candidate.reasons.clone(),
                checked: false,
            });
        }
    }

    choices
}

pub(super) fn reviewer_reason_hint(reasons: &[String]) -> Option<String> {
    let mut matched_files = 0;
    let mut hints = Vec::new();

    for reason in reasons {
        if let Some(count) = matched_file_count(reason) {
            matched_files += count;
        } else if !hints.contains(reason) {
            hints.push(reason.clone());
        }
    }

    if matched_files > 0 {
        hints.insert(0, matched_file_count_hint(matched_files));
    }

    (!hints.is_empty()).then(|| hints.join("; "))
}

pub(super) fn matched_file_count(reason: &str) -> Option<usize> {
    let tail = reason
        .strip_prefix("matched ")
        .or_else(|| reason.rsplit_once(" matched ").map(|(_, tail)| tail))?;
    tail.split_whitespace().next()?.parse().ok()
}

pub(super) fn matched_file_count_hint(count: usize) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!("matched {count} {noun}")
}

pub(super) fn selection_from_indexes(
    choices: &[ReviewerChoice],
    selected: &[usize],
) -> ReviewerSelection {
    selection_from_choices(selected.iter().filter_map(|index| choices.get(*index)))
}

fn reviewer_selection_summary(choices: &[ReviewerChoice], selected: &[usize]) -> String {
    let names = selected
        .iter()
        .filter_map(|index| choices.get(*index))
        .map(|choice| choice.target.display_name().to_owned())
        .collect::<Vec<_>>();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// Restores cursor visibility before the process exits from an interrupt.
pub(super) fn install_interrupt_cursor_restore() -> io::Result<()> {
    let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::signal::SIGINT])?;
    std::thread::Builder::new()
        .name("jx-signal-handler".to_owned())
        .spawn(move || {
            if signals.forever().next().is_some() {
                restore_terminal_cursor();
                std::process::exit(130);
            }
        })?;
    Ok(())
}

/// Restores cursor visibility when dialoguer exits through an interrupt error path.
pub(super) fn restore_terminal_cursor() {
    let mut stderr = io::stderr();
    let _ = stderr.write_all(b"\x1b[?25h");
    let _ = stderr.flush();
}

pub(super) fn selection_from_choices<'a>(
    choices: impl IntoIterator<Item = &'a ReviewerChoice>,
) -> ReviewerSelection {
    selection_from_targets(choices.into_iter().map(|choice| &choice.target))
}

pub(super) fn selection_from_targets<'a>(
    targets: impl IntoIterator<Item = &'a ReviewerTarget>,
) -> ReviewerSelection {
    let mut users = Vec::new();
    let mut teams = Vec::new();
    for target in targets {
        match target {
            ReviewerTarget::User { login } => users.push(login.clone()),
            ReviewerTarget::Team { slug, .. } => teams.push(slug.clone()),
        }
    }

    ReviewerSelection::new(users, teams)
}
