use super::*;
use std::future::Future;

/// Loads a complete, fresh inventory; partial responses must never imply an empty repository.
pub(super) async fn load_authored_pull_request_inventory(
    github: &OctocrabGitHubClient,
) -> Result<AuthoredPullRequestInventory, GitHubError> {
    collect_inventory(|cursor| async move {
        github
            .graphql(
                AUTHORED_PULL_REQUEST_INVENTORY_QUERY,
                InventoryVariables {
                    cursor: cursor.as_deref(),
                },
                INVENTORY_OPERATION,
            )
            .await
    })
    .await
}

async fn collect_inventory<F, Fut>(
    mut fetch_page: F,
) -> Result<AuthoredPullRequestInventory, GitHubError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<InventoryQueryData, GitHubError>>,
{
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut viewer = None;
    let mut expected_count = None;
    let mut repositories = BTreeMap::<GitHubRepository, Vec<PullRequestRecord>>::new();
    loop {
        let page = fetch_page(cursor).await?.viewer;
        if page.login.is_empty() || viewer.as_ref().is_some_and(|login| login != &page.login) {
            return Err(incomplete_inventory(
                "viewer identity changed or is missing",
            ));
        }
        viewer = Some(page.login);
        let connection = page.pull_requests;
        if expected_count.is_some_and(|count| count != connection.total_count) {
            return Err(incomplete_inventory(
                "open PR count changed during pagination",
            ));
        }
        expected_count = Some(connection.total_count);
        if connection.page_info.has_next_page && connection.nodes.is_empty() {
            return Err(incomplete_inventory("pagination made no progress"));
        }
        for node in connection.nodes {
            let node = node.ok_or_else(|| incomplete_inventory("response contains a null PR"))?;
            let repository = GitHubRepository {
                owner: node.repository.owner.login,
                name: node.repository.name,
            };
            if repository.owner.is_empty() || repository.name.is_empty() {
                return Err(incomplete_inventory("PR repository identity is missing"));
            }
            if !seen.insert((
                repository.slug().to_ascii_lowercase(),
                node.pull_request.number,
            )) {
                return Err(incomplete_inventory("response repeats a PR"));
            }
            if let Some(pull_request) =
                map_authored_open_pull_request(&repository, node.pull_request)
            {
                let key = GitHubRepository {
                    owner: repository.owner.to_ascii_lowercase(),
                    name: repository.name.to_ascii_lowercase(),
                };
                repositories.entry(key).or_default().push(pull_request);
            }
        }
        if seen.len() > connection.total_count {
            return Err(incomplete_inventory(
                "response exceeds the reported PR count",
            ));
        }
        if !connection.page_info.has_next_page {
            if seen.len() != connection.total_count {
                return Err(incomplete_inventory("response is missing reported PRs"));
            }
            return Ok(AuthoredPullRequestInventory {
                viewer: AuthenticatedUser {
                    login: viewer.expect("validated viewer"),
                },
                repositories,
            });
        }
        let next = connection
            .page_info
            .end_cursor
            .filter(|cursor| !cursor.is_empty())
            .ok_or_else(|| incomplete_inventory("next-page cursor is missing"))?;
        if !cursors.insert(next.clone()) {
            return Err(incomplete_inventory("next-page cursor repeats"));
        }
        cursor = Some(next);
    }
}

/// Applies the same head-owner filter to bulk and repository-scoped authored PR discovery.
pub(super) fn map_authored_open_pull_request(
    repository: &GitHubRepository,
    pull: GraphQlAuthoredOpenPullRequest,
) -> Option<PullRequestRecord> {
    if !pull
        .head_repository_owner
        .login
        .eq_ignore_ascii_case(&repository.owner)
    {
        return None;
    }
    Some(PullRequestRecord {
        number: pull.number,
        title: pull.title,
        body: (!pull.body.is_empty()).then_some(pull.body),
        head_branch: pull.head_ref_name,
        base_branch: pull.base_ref_name,
        html_url: Some(pull.url),
        draft: pull.is_draft,
        merged: pull.merged,
        reviewers: ReviewerSelection::default(),
    })
}

fn incomplete_inventory(message: &str) -> GitHubError {
    GitHubError::GraphQl {
        operation: INVENTORY_OPERATION,
        message: format!("incomplete authored PR inventory: {message}"),
    }
}

const INVENTORY_OPERATION: &str = "load authored pull request inventory";

#[derive(Serialize)]
struct InventoryVariables<'a> {
    cursor: Option<&'a str>,
}

#[derive(Deserialize)]
struct InventoryQueryData {
    viewer: InventoryViewer,
}

#[derive(Deserialize)]
struct InventoryViewer {
    login: String,
    #[serde(rename = "pullRequests")]
    pull_requests: InventoryConnection,
}

#[derive(Deserialize)]
struct InventoryConnection {
    #[serde(rename = "totalCount")]
    total_count: usize,
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
    nodes: Vec<Option<InventoryNode>>,
}

#[derive(Deserialize)]
struct InventoryNode {
    repository: GraphQlReviewRequestRepository,
    #[serde(flatten)]
    pull_request: GraphQlAuthoredOpenPullRequest,
}

const AUTHORED_PULL_REQUEST_INVENTORY_QUERY: &str = r#"
query($cursor: String) {
  viewer {
    login
    pullRequests(states: OPEN, first: 100, after: $cursor) {
      totalCount
      pageInfo {
        hasNextPage
        endCursor
      }
      nodes {
        repository {
          name
          owner { login }
        }
        number
        title
        body
        headRefName
        baseRefName
        url
        isDraft
        merged
        headRepositoryOwner { login }
      }
    }
  }
}
"#;

#[cfg(test)]
#[path = "../tests/authored_pull_requests.rs"]
mod tests;
