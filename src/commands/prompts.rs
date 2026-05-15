use super::*;

pub(super) struct PromptHandlers<'a> {
    pub(super) pull_request_previewer: &'a dyn PullRequestPreviewer,
    pub(super) pull_request_selector: &'a dyn PullRequestSelector,
    pub(super) reviewer_selector: &'a dyn ReviewerSelector,
    pub(super) pull_request_confirmer: &'a dyn PullRequestConfirmer,
    pub(super) push_confirmer: &'a dyn PushConfirmer,
    pub(super) repository_creation_confirmer: &'a dyn RepositoryCreationConfirmer,
    pub(super) workspace_remove_confirmer: &'a dyn WorkspaceRemoveConfirmer,
}

/// Shows the operator-facing PR summary before any publishing mutation.
pub(super) trait PullRequestPreviewer {
    fn show_preview(&self, plan: &PullRequestPlan, status: &WorkspaceStatus);
}

pub(super) struct TerminalPullRequestPreviewer;

impl PullRequestPreviewer for TerminalPullRequestPreviewer {
    fn show_preview(&self, plan: &PullRequestPlan, status: &WorkspaceStatus) {
        eprint!("{}", render_pull_request_preview(plan, status));
    }
}

#[cfg(test)]
pub(super) struct NoPullRequestPreview;

#[cfg(test)]
impl PullRequestPreviewer for NoPullRequestPreview {
    fn show_preview(&self, _plan: &PullRequestPlan, _status: &WorkspaceStatus) {}
}

/// Selects an existing pull request to open when the operator requests an interactive list.
pub(super) trait PullRequestSelector {
    fn select_pull_request(
        &self,
        pull_requests: &[PullRequestRecord],
    ) -> Result<PullRequestRecord, PullRequestSelectionError>;
}

#[derive(Debug, Error)]
pub enum PullRequestSelectionError {
    #[error("Cannot select a pull request without an interactive terminal")]
    NonInteractive,
    #[error("No pull requests are available to select")]
    NoPullRequests,
    #[error("Could not read pull request selection: {source}")]
    Read { source: dialoguer::Error },
}

pub(super) struct TerminalPullRequestSelector;

impl PullRequestSelector for TerminalPullRequestSelector {
    fn select_pull_request(
        &self,
        pull_requests: &[PullRequestRecord],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(PullRequestSelectionError::NonInteractive);
        }
        if pull_requests.is_empty() {
            return Err(PullRequestSelectionError::NoPullRequests);
        }

        let labels = pull_requests
            .iter()
            .map(pull_request_choice_label)
            .collect::<Vec<_>>();
        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt("Open pull request:")
            .items(&labels)
            .default(0)
            .report(false)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                PullRequestSelectionError::Read { source }
            })?;

        Ok(pull_requests[selected].clone())
    }
}

#[cfg(test)]
pub(super) struct SelectFirstPullRequest;

#[cfg(test)]
impl PullRequestSelector for SelectFirstPullRequest {
    fn select_pull_request(
        &self,
        pull_requests: &[PullRequestRecord],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        pull_requests
            .first()
            .cloned()
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
        pull_requests: &[PullRequestRecord],
    ) -> Result<PullRequestRecord, PullRequestSelectionError> {
        pull_requests
            .get(self.selected)
            .cloned()
            .ok_or(PullRequestSelectionError::NoPullRequests)
    }
}

const PULL_REQUEST_DRAFT_STYLE: &str = "\x1b[2m\x1b[38;2;150;142;132m";
const RESET_STYLE: &str = "\x1b[0m";

pub(super) fn pull_request_choice_label(pull_request: &PullRequestRecord) -> String {
    let title = if pull_request.title.trim().is_empty() {
        "(untitled)"
    } else {
        pull_request.title.trim()
    };
    let label = format!(
        "#{number:<6} {title} [{head} -> {base}]",
        number = pull_request.number,
        head = pull_request.head_branch.as_str(),
        base = pull_request.base_branch.as_str(),
    );
    if pull_request.draft {
        format!("{PULL_REQUEST_DRAFT_STYLE}{label}{RESET_STYLE}")
    } else {
        label
    }
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

        eprintln!();
        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt(pull_request_confirmation_prompt(plan))
            .items(["Yes", "No"])
            .default(1)
            .report(false)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                PullRequestConfirmationError::Read { source }
            })?;

        Ok(selected == 0)
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

        eprintln!();
        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt(push_confirmation_prompt(plan))
            .items(["Yes", "No"])
            .default(1)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                PushConfirmationError::Read { source }
            })?;

        Ok(selected == 0)
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

        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt(format!("Create private {} repository?", plan.remote_url))
            .items(["Yes", "No"])
            .default(1)
            .report(false)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                RepositoryCreationConfirmationError::Read { source }
            })?;

        Ok(selected == 0)
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
    ) -> Result<bool, WorkspaceRemoveConfirmationError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(WorkspaceRemoveConfirmationError::NonInteractive);
        }

        let theme = PlainPromptTheme;
        let selected = Select::with_theme(&theme)
            .with_prompt(workspace_remove_confirmation_prompt(workspace))
            .items(["Yes", "No"])
            .default(1)
            .report(false)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                WorkspaceRemoveConfirmationError::Read { source }
            })?;

        Ok(selected == 0)
    }
}

#[cfg(test)]
pub(super) struct AlwaysConfirmWorkspaceRemove;

#[cfg(test)]
impl WorkspaceRemoveConfirmer for AlwaysConfirmWorkspaceRemove {
    fn confirm_workspace_remove(
        &self,
        _workspace: &WorkspaceEntry,
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
            eprintln!("\nReviewers for the pull request cannot be determined, set them manually in github.");
            return Ok(ReviewerSelection::default());
        }

        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
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
        let theme = PlainPromptTheme;
        eprintln!();
        let selected = MultiSelect::with_theme(&theme)
            .with_prompt("Reviewers:")
            .items(&labels)
            .defaults(&defaults)
            .clear(false)
            .report(false)
            .interact()
            .map_err(|source| {
                restore_terminal_cursor();
                ReviewerSelectionError::Read { source }
            })?;

        Ok(selection_from_indexes(&choices, &selected))
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

impl ReviewerChoice {
    pub(super) fn label(&self) -> String {
        let name = self.target.display_name();
        match reviewer_reason_hint(&self.reasons) {
            Some(hint) => format!("{name:<24} {REVIEWER_HINT_STYLE}{hint}{RESET_STYLE}"),
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
        if choices.iter().any(|choice| &choice.target == target) {
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
            .find(|choice| choice.target == candidate.target)
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
