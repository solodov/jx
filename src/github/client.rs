use super::*;

/// GitHub API operations exposed to command/domain services.
#[async_trait]
pub trait GitHubClient: Send + Sync {
    /// Returns the authenticated GitHub user identity.
    async fn authenticated_user(&self) -> Result<AuthenticatedUser, GitHubError>;

    /// Verifies repository access and returns high-level repository facts.
    async fn repository_access(
        &self,
        repository: &GitHubRepository,
    ) -> Result<RepositoryAccess, GitHubError>;

    /// Returns source repository metadata when the repository is a GitHub fork.
    async fn repository_fork(
        &self,
        repository: &GitHubRepository,
    ) -> Result<Option<RepositoryFork>, GitHubError>;

    /// Creates a private or public GitHub repository without initializing contents.
    async fn create_repository(
        &self,
        repository: &GitHubRepository,
        private: bool,
    ) -> Result<RepositoryCreation, GitHubError>;

    /// Compares two Git refs or SHAs through GitHub's repository comparison API.
    async fn compare_commits(
        &self,
        repository: &GitHubRepository,
        base: &str,
        head: &str,
    ) -> Result<CommitComparison, GitHubError>;

    /// Finds an open pull request by same-repository head branch and author.
    async fn find_authored_open_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, GitHubError>;

    /// Finds an open pull request by same-repository head branch.
    async fn find_open_pull_request(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError>;

    /// Finds the most recent pull request, open or closed, by same-repository head branch.
    async fn find_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError>;

    /// Finds a pull request by durable repository-local PR number.
    async fn find_pull_request_by_number(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, GitHubError>;

    /// Creates a pull request from domain input.
    async fn create_pull_request(
        &self,
        repository: &GitHubRepository,
        request: PullRequestCreate,
    ) -> Result<PullRequestRecord, GitHubError>;

    /// Updates an existing pull request from domain input.
    async fn update_pull_request(
        &self,
        repository: &GitHubRepository,
        number: u64,
        request: PullRequestUpdate,
    ) -> Result<PullRequestRecord, GitHubError>;

    /// Lists labels currently attached to a pull request's backing issue.
    async fn pull_request_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError>;

    /// Adds labels to a pull request's backing issue.
    async fn add_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
        labels: Vec<String>,
    ) -> Result<LabelApplyResult, GitHubError>;

    /// Synchronizes requested reviewers to the desired user/team sets.
    async fn sync_reviewers(
        &self,
        repository: &GitHubRepository,
        number: u64,
        desired: ReviewerSelection,
    ) -> Result<ReviewerSyncResult, GitHubError>;
}

/// Concrete GitHub integration implemented with octocrab.
pub struct OctocrabGitHubClient {
    crab: Octocrab,
}

impl OctocrabGitHubClient {
    /// Wraps an existing octocrab client. This is mainly useful for integration tests.
    pub fn new(crab: Octocrab) -> Self {
        Self { crab }
    }

    async fn find_pull_request_for_head_with_state(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        state: params::State,
        operation: &'static str,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        self.find_pull_request_for_head_with_state_and_author(
            repository, head, state, operation, None,
        )
        .await
    }

    async fn find_pull_request_for_head_with_state_and_author(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        state: params::State,
        operation: &'static str,
        author: Option<&str>,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        let mut page = self
            .crab
            .pulls(&repository.owner, &repository.name)
            .list()
            .state(state)
            .head(head.label())
            .per_page(if author.is_some() { 100 } else { 1 })
            .send()
            .await
            .map_err(|source| api_error(operation, source))?;

        loop {
            let next = page.next.clone();
            if let Some(pull) = page.items.into_iter().find(|pull| {
                author.is_none_or(|author| {
                    pull.user
                        .as_ref()
                        .is_some_and(|user| user.login.as_str() == author)
                })
            }) {
                return Ok(Some(map_pull_request(pull)));
            }
            if author.is_none() {
                return Ok(None);
            }

            let Some(next_page) = self
                .crab
                .get_page::<models::pulls::PullRequest>(&next)
                .await
                .map_err(|source| api_error(operation, source))?
            else {
                return Ok(None);
            };
            page = next_page;
        }
    }

    /// Builds an authenticated octocrab client from a token value.
    pub fn from_token(token: impl Into<String>) -> Result<Self, GitHubError> {
        let crab = Octocrab::builder()
            .personal_token(token.into())
            .build()
            .map_err(|source| GitHubError::ClientBuild { source })?;

        Ok(Self::new(crab))
    }

    /// Builds an authenticated client from the token source discovered in repository context.
    pub fn from_token_source(
        token_source: &TokenSource,
        environment: &RuntimeEnvironment,
    ) -> Result<Self, GitHubError> {
        let token = token_source
            .token(environment)?
            .ok_or(GitHubError::MissingToken)?;

        Self::from_token(token)
    }
}

#[async_trait]
impl GitHubClient for OctocrabGitHubClient {
    async fn authenticated_user(&self) -> Result<AuthenticatedUser, GitHubError> {
        let user = self
            .crab
            .current()
            .user()
            .await
            .map_err(|source| api_error("load authenticated user", source))?;

        Ok(AuthenticatedUser { login: user.login })
    }

    async fn repository_access(
        &self,
        repository: &GitHubRepository,
    ) -> Result<RepositoryAccess, GitHubError> {
        let octo_repository = self
            .crab
            .repos(&repository.owner, &repository.name)
            .get()
            .await
            .map_err(|source| api_error("check repository access", source))?;
        let permissions = octo_repository.permissions;
        let can_read = permissions
            .as_ref()
            .is_none_or(|permissions| permissions.pull);
        let can_push = permissions
            .as_ref()
            .is_some_and(|permissions| permissions.push);
        let can_admin = permissions
            .as_ref()
            .is_some_and(|permissions| permissions.admin);

        Ok(RepositoryAccess {
            repository: repository.clone(),
            default_branch: octo_repository.default_branch,
            can_read,
            can_push,
            can_admin,
        })
    }

    async fn repository_fork(
        &self,
        repository: &GitHubRepository,
    ) -> Result<Option<RepositoryFork>, GitHubError> {
        let route = format!(
            "/repos/{owner}/{repo}",
            owner = repository.owner,
            repo = repository.name,
        );
        let response: RepositoryForkResponse = self
            .crab
            .get(route, Option::<&()>::None)
            .await
            .map_err(|source| api_error("load repository fork source", source))?;
        let Some(source) = response.source.filter(|_| response.fork) else {
            return Ok(None);
        };

        Ok(Some(RepositoryFork {
            source: GitHubRepository {
                owner: source.owner.login,
                name: source.name,
            },
            source_default_branch: source.default_branch,
        }))
    }

    async fn create_repository(
        &self,
        repository: &GitHubRepository,
        private: bool,
    ) -> Result<RepositoryCreation, GitHubError> {
        let user = self.authenticated_user().await?;
        let route = if user.login == repository.owner {
            "/user/repos".to_owned()
        } else {
            format!("/orgs/{}/repos", repository.owner)
        };
        let request = CreateRepositoryRequest {
            name: &repository.name,
            private,
            auto_init: false,
        };
        let created: CreateRepositoryResponse = self
            .crab
            .post(route, Some(&request))
            .await
            .map_err(|source| api_error("create repository", source))?;

        Ok(RepositoryCreation {
            repository: repository.clone(),
            html_url: created.html_url.unwrap_or_else(|| repository.https_url()),
            private,
        })
    }

    async fn compare_commits(
        &self,
        repository: &GitHubRepository,
        base: &str,
        head: &str,
    ) -> Result<CommitComparison, GitHubError> {
        // Octocrab's full compare model requires nested commit fields that GitHub can omit.
        // jx only needs the top-level relationship counters, so deserialize just those.
        let route = format!(
            "/repos/{owner}/{repo}/compare/{base}...{head}",
            owner = repository.owner,
            repo = repository.name,
        );
        let comparison: CompareCommitsResponse = self
            .crab
            .get(route, Option::<&()>::None)
            .await
            .map_err(|source| compare_error(base, head, source))?;

        Ok(CommitComparison {
            status: map_comparison_status(comparison.status),
            ahead_by: comparison.ahead_by,
            behind_by: comparison.behind_by,
        })
    }

    async fn find_authored_open_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
        author: &str,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        self.find_pull_request_for_head_with_state_and_author(
            repository,
            head,
            params::State::Open,
            "find authored open pull request",
            Some(author),
        )
        .await
    }

    async fn find_open_pull_request(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        self.find_pull_request_for_head_with_state(
            repository,
            head,
            params::State::Open,
            "find open pull request",
        )
        .await
    }

    async fn find_pull_request_for_head(
        &self,
        repository: &GitHubRepository,
        head: &PullRequestHead,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        self.find_pull_request_for_head_with_state(
            repository,
            head,
            params::State::All,
            "find pull request",
        )
        .await
    }

    async fn find_pull_request_by_number(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Option<PullRequestRecord>, GitHubError> {
        match self
            .crab
            .pulls(&repository.owner, &repository.name)
            .get(number)
            .await
        {
            Ok(pull) => Ok(Some(map_pull_request(pull))),
            Err(source) if api_not_found(&source) => Ok(None),
            Err(source) => Err(api_error("find pull request by number", source)),
        }
    }

    async fn create_pull_request(
        &self,
        repository: &GitHubRepository,
        request: PullRequestCreate,
    ) -> Result<PullRequestRecord, GitHubError> {
        let handler = self.crab.pulls(&repository.owner, &repository.name);
        let mut builder = handler
            .create(request.title, request.head.label(), request.base)
            .draft(request.draft);

        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let pull = builder
            .send()
            .await
            .map_err(|source| api_error("create pull request", source))?;

        Ok(map_pull_request(pull))
    }

    async fn update_pull_request(
        &self,
        repository: &GitHubRepository,
        number: u64,
        request: PullRequestUpdate,
    ) -> Result<PullRequestRecord, GitHubError> {
        let handler = self.crab.pulls(&repository.owner, &repository.name);
        let mut builder = handler.update(number);

        if let Some(title) = request.title {
            builder = builder.title(title);
        }
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        if let Some(base) = request.base {
            builder = builder.base(base);
        }

        let pull = builder
            .send()
            .await
            .map_err(|source| api_error("update pull request", source))?;

        Ok(map_pull_request(pull))
    }

    async fn pull_request_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        let page = self
            .crab
            .issues(&repository.owner, &repository.name)
            .list_labels_for_issue(number)
            .per_page(100u8)
            .send()
            .await
            .map_err(|source| api_error("list pull request labels", source))?;

        Ok(normalize_names(
            page.items.into_iter().map(|label| label.name),
        ))
    }

    async fn add_labels(
        &self,
        repository: &GitHubRepository,
        number: u64,
        labels: Vec<String>,
    ) -> Result<LabelApplyResult, GitHubError> {
        let labels = normalize_names(labels);
        if labels.is_empty() {
            return Ok(LabelApplyResult { labels });
        }

        self.crab
            .issues(&repository.owner, &repository.name)
            .add_labels(number, &labels)
            .await
            .map_err(|source| api_error("apply labels", source))?;

        Ok(LabelApplyResult { labels })
    }

    async fn sync_reviewers(
        &self,
        repository: &GitHubRepository,
        number: u64,
        desired: ReviewerSelection,
    ) -> Result<ReviewerSyncResult, GitHubError> {
        let desired = ReviewerSelection::new(desired.users, desired.teams);
        let handler = self.crab.pulls(&repository.owner, &repository.name);
        let pull = handler
            .get(number)
            .await
            .map_err(|source| api_error("load pull request reviewers", source))?;
        let current_users = pull
            .requested_reviewers
            .unwrap_or_default()
            .into_iter()
            .map(|user| user.login)
            .collect::<Vec<_>>();
        let current_teams = pull
            .requested_teams
            .unwrap_or_default()
            .into_iter()
            .map(|team| team.slug)
            .collect::<Vec<_>>();

        let requested_users = difference(&desired.users, &current_users);
        let requested_teams = difference(&desired.teams, &current_teams);
        let removed_users = difference(&current_users, &desired.users);
        let removed_teams = difference(&current_teams, &desired.teams);

        if !removed_users.is_empty() || !removed_teams.is_empty() {
            handler
                .remove_requested_reviewers(number, removed_users.clone(), removed_teams.clone())
                .await
                .map_err(|source| api_error("remove requested reviewers", source))?;
        }

        if !requested_users.is_empty() || !requested_teams.is_empty() {
            handler
                .request_reviews(number, requested_users.clone(), requested_teams.clone())
                .await
                .map_err(|source| api_error("request reviewers", source))?;
        }

        Ok(ReviewerSyncResult {
            requested_users,
            requested_teams,
            removed_users,
            removed_teams,
        })
    }
}

#[derive(Debug, Serialize)]
struct CreateRepositoryRequest<'a> {
    name: &'a str,
    private: bool,
    auto_init: bool,
}

#[derive(Debug, Deserialize)]
struct CreateRepositoryResponse {
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepositoryForkResponse {
    #[serde(default)]
    fork: bool,
    source: Option<RepositoryForkSourceResponse>,
}

#[derive(Debug, Deserialize)]
struct RepositoryForkSourceResponse {
    name: String,
    owner: RepositoryForkOwnerResponse,
    default_branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RepositoryForkOwnerResponse {
    login: String,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct CompareCommitsResponse {
    pub(super) status: CompareCommitsStatus,
    pub(super) ahead_by: i64,
    pub(super) behind_by: i64,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CompareCommitsStatus {
    Ahead,
    Behind,
    Diverged,
    Identical,
    #[serde(other)]
    Unknown,
}

fn map_pull_request(pull: models::pulls::PullRequest) -> PullRequestRecord {
    PullRequestRecord {
        number: pull.number,
        title: pull.title.unwrap_or_default(),
        body: pull.body,
        head_branch: pull.head.ref_field,
        base_branch: pull.base.ref_field,
        html_url: pull.html_url.map(|url| url.to_string()),
        draft: pull.draft.unwrap_or(false),
        merged: pull.merged.unwrap_or(false) || pull.merged_at.is_some(),
    }
}

pub(super) fn map_comparison_status(status: CompareCommitsStatus) -> ComparisonStatus {
    match status {
        CompareCommitsStatus::Ahead => ComparisonStatus::Ahead,
        CompareCommitsStatus::Behind => ComparisonStatus::Behind,
        CompareCommitsStatus::Diverged => ComparisonStatus::Diverged,
        CompareCommitsStatus::Identical => ComparisonStatus::Identical,
        CompareCommitsStatus::Unknown => ComparisonStatus::Unknown,
    }
}
