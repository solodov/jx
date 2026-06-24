use super::*;

/// GitHub boundary failures with operation-specific diagnostics.
#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("No GitHub token found. Set JX_GITHUB_TOKEN, GH_TOKEN, GITHUB_TOKEN, or configure [auth.keychain].")]
    MissingToken,
    #[error(transparent)]
    TokenRead(#[from] TokenReadError),
    #[error("Could not initialize GitHub client: {source}")]
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
    #[error("Could not {operation} through GitHub: {source}")]
    Api {
        operation: &'static str,
        source: Box<octocrab::Error>,
    },
}

pub(super) fn api_response_error(operation: &'static str, status: u16, body: &str) -> GitHubError {
    let message = format!("HTTP {status}: {}", github_error_response_message(body));
    if matches!(status, 401 | 403) {
        return GitHubError::AuthenticationFailed { operation, message };
    }

    GitHubError::ApiResponse {
        operation,
        status,
        message: github_error_response_message(body),
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

fn github_error_response_message(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return "empty response body".to_owned();
    }

    serde_json::from_str::<GitHubErrorResponse>(body)
        .ok()
        .and_then(|response| response.message)
        .map(|message| message.trim().to_owned())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| body.to_owned())
}

#[derive(Debug, Deserialize)]
struct GitHubErrorResponse {
    message: Option<String>,
}
