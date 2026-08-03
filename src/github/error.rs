use super::*;

/// GitHub boundary failures with operation-specific diagnostics.
#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("No GitHub token found. Set JX_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, or configure [auth.keychain].")]
    MissingToken,
    #[error(transparent)]
    TokenRead(#[from] TokenReadError),
    #[error("Could not initialize GitHub client: {}", octocrab_error_message(.source))]
    ClientBuild { source: Box<octocrab::Error> },
    #[error("GitHub authentication failed while trying to {operation}: {message}")]
    AuthenticationFailed {
        operation: &'static str,
        message: String,
    },
    #[error(
        "GitHub could not compare `{base}` with `{head}` because one target was not found: {message}. Run `jx fetch` if the branch moved, or ensure the local trunk commit exists on GitHub."
    )]
    ComparisonTargetNotFound {
        base: String,
        head: String,
        message: String,
    },
    #[error("Could not decode GitHub response while trying to {operation}: {source}")]
    ResponseDecode {
        operation: &'static str,
        source: serde_json::Error,
    },
    #[error("Could not {operation} through GitHub: HTTP {status}: {message}")]
    ApiResponse {
        operation: &'static str,
        status: u16,
        message: String,
    },
    #[error("Could not {operation} through GitHub GraphQL: {message}")]
    GraphQl {
        operation: &'static str,
        message: String,
    },
    #[error("Timed out after {timeout_ms}ms while trying to {operation} through GitHub")]
    Timeout {
        operation: &'static str,
        timeout_ms: u128,
    },
    #[error("Could not {operation} through GitHub: {}", octocrab_error_message(.source))]
    Api {
        operation: &'static str,
        source: Box<octocrab::Error>,
    },
}

impl GitHubError {
    /// Returns whether this GraphQL failure is GitHub's organization SAML gate for an operation.
    pub(crate) fn is_graphql_saml_enforcement_for(&self, operation: &'static str) -> bool {
        match self {
            Self::GraphQl {
                operation: error_operation,
                message,
            } => {
                *error_operation == operation
                    && message
                        .to_ascii_lowercase()
                        .contains("resource protected by organization saml enforcement")
            }
            _ => false,
        }
    }
}

pub(super) fn api_error(operation: &'static str, source: octocrab::Error) -> GitHubError {
    if let Some(message) = octocrab_github_error(&source)
        .filter(|error| matches!(error.status_code.as_u16(), 401 | 403))
        .map(|error| error.message.clone())
    {
        return GitHubError::AuthenticationFailed { operation, message };
    }

    GitHubError::Api {
        operation,
        source: Box::new(source),
    }
}

pub(super) fn compare_error(base: &str, head: &str, source: octocrab::Error) -> GitHubError {
    if let Some(message) = octocrab_github_error(&source)
        .filter(|error| error.status_code.as_u16() == 404)
        .map(|error| error.message.clone())
    {
        return GitHubError::ComparisonTargetNotFound {
            base: base.to_owned(),
            head: head.to_owned(),
            message,
        };
    }

    api_error("compare commits", source)
}

pub(super) fn api_not_found(source: &octocrab::Error) -> bool {
    octocrab_github_error(source).is_some_and(|error| error.status_code.as_u16() == 404)
}

fn octocrab_github_error(source: &octocrab::Error) -> Option<&octocrab::GitHubError> {
    match source {
        octocrab::Error::GitHub { source, .. } => Some(source.as_ref()),
        _ => None,
    }
}

fn octocrab_error_message(error: &octocrab::Error) -> String {
    summarize_github_error_message(trim_octocrab_error_backtrace(&error.to_string()))
}

pub(super) fn trim_octocrab_error_backtrace(message: &str) -> &str {
    message
        .split_once("\nFound at")
        .map_or(message, |(summary, _)| summary.trim_end())
}

pub(super) fn summarize_github_error_message(message: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        return "empty response body".to_owned();
    }
    if looks_like_html_response(message) {
        return "HTML response body; GitHub may be temporarily unavailable".to_owned();
    }

    let mut summary = message.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_GITHUB_ERROR_SUMMARY_CHARS: usize = 500;
    if summary.chars().count() > MAX_GITHUB_ERROR_SUMMARY_CHARS {
        summary = summary
            .chars()
            .take(MAX_GITHUB_ERROR_SUMMARY_CHARS)
            .collect::<String>();
        summary.push('…');
    }
    summary
}

fn looks_like_html_response(message: &str) -> bool {
    let normalized = message.trim_start().to_ascii_lowercase();
    normalized.starts_with('<')
        || normalized.contains("<!doctype html")
        || normalized.contains("<html")
}
