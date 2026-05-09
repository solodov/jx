use super::*;

/// A GitHub account or team that can be requested for PR review.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewerTarget {
    User { login: String },
    Team { name: String, slug: String },
}

impl ReviewerTarget {
    /// Builds a user reviewer target from a GitHub login.
    pub fn user(login: impl Into<String>) -> Self {
        Self::User {
            login: login.into(),
        }
    }

    /// Builds a team reviewer target from an operator-facing `org/team` name and API slug.
    pub fn team(name: impl Into<String>, slug: impl Into<String>) -> Self {
        Self::Team {
            name: name.into(),
            slug: slug.into(),
        }
    }

    /// Parses a CLI/config reviewer name into the GitHub reviewer target shape.
    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim();
        if let Some((owner, slug)) = name.split_once('/') {
            if is_valid_reviewer_component(owner) && is_valid_reviewer_component(slug) {
                return Some(Self::team(name, slug));
            }
        } else if is_valid_reviewer_component(name) {
            return Some(Self::user(name));
        }

        None
    }

    /// Returns the name shown to the operator in reviewer prompts.
    pub fn display_name(&self) -> &str {
        match self {
            Self::User { login } => login,
            Self::Team { name, .. } => name,
        }
    }
}

fn is_valid_reviewer_component(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['.', '-', '_'])
        && !name.ends_with(['.', '-', '_'])
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// One reviewer offered to the operator with concise ownership reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerCandidate {
    pub target: ReviewerTarget,
    pub reasons: Vec<String>,
}

impl ReviewerCandidate {
    /// Builds a candidate for a target with accumulated ownership reasons.
    pub fn new(target: ReviewerTarget, reasons: Vec<String>) -> Self {
        Self { target, reasons }
    }
}

/// Desired requested-reviewer state for a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewerSelection {
    pub users: Vec<String>,
    pub teams: Vec<String>,
}

impl ReviewerSelection {
    /// Builds a normalized reviewer selection with empty entries removed and duplicates collapsed.
    pub fn new<Users, Teams, User, Team>(users: Users, teams: Teams) -> Self
    where
        Users: IntoIterator<Item = User>,
        Teams: IntoIterator<Item = Team>,
        User: Into<String>,
        Team: Into<String>,
    {
        Self {
            users: normalize_names(users),
            teams: normalize_names(teams),
        }
    }

    /// Returns true when no reviewers are desired.
    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.teams.is_empty()
    }
}

/// Summary of labels applied to a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelApplyResult {
    pub labels: Vec<String>,
}

/// Summary of reviewer changes applied by `sync_reviewers`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewerSyncResult {
    pub requested_users: Vec<String>,
    pub requested_teams: Vec<String>,
    pub removed_users: Vec<String>,
    pub removed_teams: Vec<String>,
}

pub(super) fn normalize_names<Items, Item>(items: Items) -> Vec<String>
where
    Items: IntoIterator<Item = Item>,
    Item: Into<String>,
{
    items
        .into_iter()
        .map(Into::into)
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn difference(left: &[String], right: &[String]) -> Vec<String> {
    let right = right.iter().collect::<BTreeSet<_>>();

    left.iter()
        .filter(|item| !right.contains(item))
        .cloned()
        .collect()
}
