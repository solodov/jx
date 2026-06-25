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

    /// Returns the current head commit SHA for a repository branch.
    async fn branch_head_sha(
        &self,
        repository: &GitHubRepository,
        branch: &str,
    ) -> Result<String, GitHubError>;

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

    /// Finds open pull requests authored by a user in this repository.
    async fn authored_open_pull_requests(
        &self,
        _repository: &GitHubRepository,
        _author: &str,
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        Ok(Vec::new())
    }

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

    /// Finds several pull requests by durable repository-local PR number in batches when supported.
    async fn find_pull_requests_by_numbers(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        let mut pull_requests = Vec::new();
        for number in unique_pull_request_numbers(numbers) {
            if let Some(pull_request) = self.find_pull_request_by_number(repository, number).await?
            {
                pull_requests.push(pull_request);
            }
        }
        Ok(pull_requests)
    }

    /// Loads read-only status facts for several repository-local pull requests in batches.
    async fn pull_request_statuses(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError>;

    /// Searches open pull requests requesting review from the authenticated viewer.
    async fn review_requests(&self) -> Result<PullRequestReviewRequests, GitHubError> {
        Ok(PullRequestReviewRequests {
            viewer: AuthenticatedUser {
                login: String::new(),
            },
            requests: Vec::new(),
        })
    }

    /// Loads public profile names for GitHub logins.
    async fn user_profiles(
        &self,
        _logins: &[String],
    ) -> Result<Vec<GitHubUserProfile>, GitHubError> {
        Ok(Vec::new())
    }

    /// Loads GitHub's suggested user reviewers for a pull request.
    async fn pull_request_suggested_reviewers(
        &self,
        _repository: &GitHubRepository,
        _number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        Ok(Vec::new())
    }

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

    /// Marks an existing draft pull request ready for review.
    async fn mark_pull_request_ready(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError>;

    /// Converts an existing ready pull request back to draft.
    async fn convert_pull_request_to_draft(
        &self,
        repository: &GitHubRepository,
        number: u64,
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

    async fn set_pull_request_draft_state(
        &self,
        repository: &GitHubRepository,
        number: u64,
        draft: bool,
    ) -> Result<PullRequestRecord, GitHubError> {
        let operation = if draft {
            "convert pull request to draft"
        } else {
            "mark pull request ready"
        };
        let pull_request_id = self
            .pull_request_graphql_id(repository, number, operation)
            .await?;
        let variables = PullRequestReadinessVariables {
            pull_request_id: &pull_request_id,
        };

        if draft {
            let data: ConvertPullRequestToDraftData = self
                .graphql(CONVERT_PULL_REQUEST_TO_DRAFT_MUTATION, variables, operation)
                .await?;
            Ok(map_graphql_pull_request(
                data.convert_pull_request_to_draft.pull_request,
            ))
        } else {
            let data: MarkPullRequestReadyData = self
                .graphql(MARK_PULL_REQUEST_READY_MUTATION, variables, operation)
                .await?;
            Ok(map_graphql_pull_request(
                data.mark_pull_request_ready_for_review.pull_request,
            ))
        }
    }

    async fn pull_request_graphql_id(
        &self,
        repository: &GitHubRepository,
        number: u64,
        operation: &'static str,
    ) -> Result<String, GitHubError> {
        let number = i64::try_from(number).map_err(|_| GitHubError::GraphQl {
            operation,
            message: "pull request number is too large for GitHub GraphQL".to_owned(),
        })?;
        let variables = PullRequestIdVariables {
            owner: &repository.owner,
            name: &repository.name,
            number,
        };
        let data: PullRequestIdQueryData = self
            .graphql(PULL_REQUEST_ID_QUERY, variables, operation)
            .await?;
        data.repository
            .and_then(|repository| repository.pull_request)
            .map(|pull_request| pull_request.id)
            .ok_or_else(|| GitHubError::GraphQl {
                operation,
                message: format!("pull request #{number} was not found"),
            })
    }

    async fn graphql<T, V>(
        &self,
        query: &str,
        variables: V,
        operation: &'static str,
    ) -> Result<T, GitHubError>
    where
        T: for<'de> Deserialize<'de>,
        V: Serialize,
    {
        let response: GraphQlResponse<T> = self
            .crab
            .post("/graphql", Some(&GraphQlRequest { query, variables }))
            .await
            .map_err(|source| api_error(operation, source))?;
        response.into_data(operation)
    }

    async fn pull_requests_by_numbers_chunk(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        if numbers.is_empty() {
            return Ok(Vec::new());
        }
        let query = pull_request_record_query(numbers);
        let variables = PullRequestsByNumberVariables {
            owner: &repository.owner,
            name: &repository.name,
        };
        let data: PullRequestsByNumberQueryData = self
            .graphql(&query, variables, "load pull requests by number")
            .await?;
        let repository = data.repository.ok_or_else(|| GitHubError::GraphQl {
            operation: "load pull requests by number",
            message: "repository was not found".to_owned(),
        })?;

        Ok(numbers
            .iter()
            .enumerate()
            .filter_map(|(index, _)| repository.get(&pull_request_record_alias(index)))
            .filter_map(|pull_request| pull_request.clone())
            .map(map_graphql_pull_request)
            .collect())
    }

    async fn pull_request_statuses_chunk(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError> {
        if numbers.is_empty() {
            return Ok(Vec::new());
        }
        let query = pull_request_status_query(numbers);
        let variables = PullRequestStatusesVariables {
            owner: &repository.owner,
            name: &repository.name,
        };
        let data: PullRequestStatusesQueryData = self
            .graphql(&query, variables, "load pull request statuses")
            .await?;
        let repository = data.repository.ok_or_else(|| GitHubError::GraphQl {
            operation: "load pull request statuses",
            message: "repository was not found".to_owned(),
        })?;

        Ok(numbers
            .iter()
            .enumerate()
            .filter_map(|(index, _)| repository.get(&pull_request_status_alias(index)))
            .filter_map(|pull_request| pull_request.clone())
            .map(map_graphql_pull_request_status)
            .collect())
    }

    /// Builds an authenticated octocrab client from a token value.
    pub fn from_token(token: impl Into<String>) -> Result<Self, GitHubError> {
        let crab = Octocrab::builder()
            .personal_token(token.into())
            .build()
            .map_err(|source| GitHubError::ClientBuild {
                source: Box::new(source),
            })?;

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
        let response = self
            .crab
            ._get_with_headers("/user", None)
            .await
            .map_err(|source| api_error("load authenticated user", source))?;
        let status = response.status();
        let body = self
            .crab
            .body_to_string(response)
            .await
            .map_err(|source| api_error("load authenticated user", source))?;
        if !status.is_success() {
            return Err(api_response_error(
                "load authenticated user",
                status.as_u16(),
                &body,
            ));
        }

        let user: AuthenticatedUserResponse =
            serde_json::from_str(&body).map_err(|source| GitHubError::ResponseDecode {
                operation: "load authenticated user",
                source,
            })?;

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

    async fn branch_head_sha(
        &self,
        repository: &GitHubRepository,
        branch: &str,
    ) -> Result<String, GitHubError> {
        let route = format!(
            "/repos/{owner}/{repo}/git/ref/heads/{branch}",
            owner = repository.owner,
            repo = repository.name,
        );
        let reference: GitRefResponse = self
            .crab
            .get(route, Option::<&()>::None)
            .await
            .map_err(|source| api_error("get branch head", source))?;

        Ok(reference.object.sha)
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

    async fn authored_open_pull_requests(
        &self,
        repository: &GitHubRepository,
        author: &str,
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        let mut pull_requests = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = None::<String>;
        loop {
            let query = format!(
                "is:pr is:open repo:{}/{} author:{}",
                repository.owner, repository.name, author
            );
            let variables = AuthoredOpenPullRequestsVariables {
                query: &query,
                cursor: cursor.as_deref(),
            };
            let data: AuthoredOpenPullRequestsQueryData = self
                .graphql(
                    AUTHORED_OPEN_PULL_REQUESTS_QUERY,
                    variables,
                    "search authored open pull requests",
                )
                .await?;
            for node in data.search.nodes.into_iter().flatten() {
                if node.head_repository_owner.login != repository.owner {
                    continue;
                }
                let pull_request = PullRequestRecord {
                    number: node.number,
                    title: node.title,
                    body: (!node.body.is_empty()).then_some(node.body),
                    head_branch: node.head_ref_name,
                    base_branch: node.base_ref_name,
                    html_url: Some(node.url),
                    draft: node.is_draft,
                    merged: node.merged,
                    reviewers: ReviewerSelection::default(),
                };
                if seen.insert(pull_request.number) {
                    pull_requests.push(pull_request);
                }
            }
            if !data.search.page_info.has_next_page {
                break;
            }
            let Some(next_cursor) = data.search.page_info.end_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Ok(pull_requests)
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

    async fn find_pull_requests_by_numbers(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestRecord>, GitHubError> {
        let mut pull_requests = Vec::new();
        for chunk in unique_pull_request_numbers(numbers).chunks(PULL_REQUEST_RECORD_BATCH_SIZE) {
            pull_requests.extend(
                self.pull_requests_by_numbers_chunk(repository, chunk)
                    .await?,
            );
        }
        Ok(pull_requests)
    }

    async fn pull_request_statuses(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, GitHubError> {
        let mut statuses = Vec::new();
        for chunk in unique_pull_request_numbers(numbers).chunks(PULL_REQUEST_STATUS_BATCH_SIZE) {
            statuses.extend(self.pull_request_statuses_chunk(repository, chunk).await?);
        }
        Ok(statuses)
    }

    async fn review_requests(&self) -> Result<PullRequestReviewRequests, GitHubError> {
        let mut viewer = None::<AuthenticatedUser>;
        let mut requests = Vec::new();
        let mut seen = BTreeSet::new();
        for &query in REVIEW_REQUEST_SEARCH_QUERIES {
            let mut cursor = None::<String>;
            loop {
                let variables = ReviewRequestsVariables {
                    query,
                    cursor: cursor.as_deref(),
                };
                let data: ReviewRequestsQueryData = self
                    .graphql(REVIEW_REQUESTS_QUERY, variables, "search review requests")
                    .await?;
                if viewer.is_none() {
                    viewer = Some(AuthenticatedUser {
                        login: data.viewer.login,
                    });
                }
                for node in data.search.nodes.into_iter().flatten() {
                    let request = PullRequestReviewRequest {
                        repository: GitHubRepository {
                            owner: node.repository.owner.login,
                            name: node.repository.name,
                        },
                        number: node.number,
                    };
                    if seen.insert((request.repository.clone(), request.number)) {
                        requests.push(request);
                    }
                }
                if !data.search.page_info.has_next_page {
                    break;
                }
                let Some(next_cursor) = data.search.page_info.end_cursor else {
                    break;
                };
                cursor = Some(next_cursor);
            }
        }
        Ok(PullRequestReviewRequests {
            viewer: viewer.unwrap_or(AuthenticatedUser {
                login: String::new(),
            }),
            requests,
        })
    }

    async fn user_profiles(
        &self,
        logins: &[String],
    ) -> Result<Vec<GitHubUserProfile>, GitHubError> {
        let logins = unique_user_profile_logins(logins);
        let mut profiles = Vec::new();
        for chunk in logins.chunks(USER_PROFILE_QUERY_CHUNK_SIZE) {
            let query = user_profiles_query(chunk);
            let variables = user_profiles_variables(chunk);
            let response: GraphQlResponse<BTreeMap<String, Option<GraphQlUserProfile>>> = self
                .crab
                .post(
                    "/graphql",
                    Some(&GraphQlRequest {
                        query: &query,
                        variables,
                    }),
                )
                .await
                .map_err(|source| api_error("load user profiles", source))?;
            profiles.extend(user_profiles_from_graphql_response(chunk, response)?);
        }
        Ok(profiles)
    }

    async fn pull_request_suggested_reviewers(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Vec<String>, GitHubError> {
        let variables = PullRequestSuggestedReviewersVariables {
            owner: &repository.owner,
            name: &repository.name,
            number: number as i64,
        };
        let data: PullRequestSuggestedReviewersQueryData = self
            .graphql(
                PULL_REQUEST_SUGGESTED_REVIEWERS_QUERY,
                variables,
                "load suggested reviewers",
            )
            .await?;
        let repository = data.repository.ok_or_else(|| GitHubError::GraphQl {
            operation: "load suggested reviewers",
            message: "repository was not found".to_owned(),
        })?;

        Ok(repository
            .pull_request
            .map(|pull_request| suggested_reviewers_from_graphql(pull_request.suggested_reviewers))
            .unwrap_or_default())
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

    async fn mark_pull_request_ready(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.set_pull_request_draft_state(repository, number, false)
            .await
    }

    async fn convert_pull_request_to_draft(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<PullRequestRecord, GitHubError> {
        self.set_pull_request_draft_state(repository, number, true)
            .await
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

const PULL_REQUEST_ID_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      id
    }
  }
}
"#;

pub(super) const PULL_REQUEST_SUGGESTED_REVIEWERS_QUERY: &str = r#"
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      suggestedReviewers {
        reviewer {
          login
        }
      }
    }
  }
}
"#;

pub(super) const REVIEW_REQUEST_SEARCH_QUERIES: &[&str] = &[
    "is:pr is:open review-requested:@me -author:@me",
    "is:pr is:open reviewed-by:@me -author:@me",
];

const REVIEW_REQUESTS_QUERY: &str = r#"
query($query: String!, $cursor: String) {
  viewer {
    login
  }
  search(type: ISSUE, query: $query, first: 100, after: $cursor) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      ... on PullRequest {
        number
        repository {
          name
          owner {
            login
          }
        }
      }
    }
  }
}
"#;

const AUTHORED_OPEN_PULL_REQUESTS_QUERY: &str = r#"
query($query: String!, $cursor: String) {
  search(type: ISSUE, query: $query, first: 100, after: $cursor) {
    pageInfo {
      hasNextPage
      endCursor
    }
    nodes {
      ... on PullRequest {
        number
        title
        body
        headRefName
        baseRefName
        url
        isDraft
        merged
        headRepositoryOwner {
          login
        }
      }
    }
  }
}
"#;

const MARK_PULL_REQUEST_READY_MUTATION: &str = r#"
mutation($pullRequestId: ID!) {
  markPullRequestReadyForReview(input: { pullRequestId: $pullRequestId }) {
    pullRequest {
      number
      title
      body
      headRefName
      baseRefName
      url
      isDraft
      merged
    }
  }
}
"#;

const CONVERT_PULL_REQUEST_TO_DRAFT_MUTATION: &str = r#"
mutation($pullRequestId: ID!) {
  convertPullRequestToDraft(input: { pullRequestId: $pullRequestId }) {
    pullRequest {
      number
      title
      body
      headRefName
      baseRefName
      url
      isDraft
      merged
    }
  }
}
"#;

const PULL_REQUEST_RECORD_BATCH_SIZE: usize = 50;
const PULL_REQUEST_STATUS_BATCH_SIZE: usize = 10;

pub(super) fn pull_request_record_query(numbers: &[u64]) -> String {
    let fields = numbers
        .iter()
        .enumerate()
        .map(|(index, number)| {
            format!(
                "    {}: pullRequest(number: {number}) {{\n      ...PullRequestRecordFields\n    }}",
                pull_request_record_alias(index)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"query($owner: String!, $name: String!) {{
  repository(owner: $owner, name: $name) {{
{fields}
  }}
}}

fragment PullRequestRecordFields on PullRequest {{
  number
  title
  body
  headRefName
  baseRefName
  url
  isDraft
  merged
}}
"#
    )
}

fn pull_request_record_alias(index: usize) -> String {
    format!("pr{index}")
}

pub(super) fn pull_request_status_query(numbers: &[u64]) -> String {
    let fields = numbers
        .iter()
        .enumerate()
        .map(|(index, number)| {
            format!(
                "    {}: pullRequest(number: {number}) {{\n      ...PullRequestStatusFields\n    }}",
                pull_request_status_alias(index)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"query($owner: String!, $name: String!) {{
  repository(owner: $owner, name: $name) {{
{fields}
  }}
}}

fragment PullRequestStatusFields on PullRequest {{
  number
  title
  url
  createdAt
  headRefName
  baseRefName
  author {{
    login
  }}
  isDraft
  merged
  closed
  mergedAt
  closedAt
  reviewDecision
  reviewRequests(first: 100) {{
    totalCount
    nodes {{
      requestedReviewer {{
        __typename
        ... on User {{
          login
        }}
      }}
    }}
  }}
  suggestedReviewers {{
    reviewer {{
      login
    }}
  }}
  labels(first: 100) {{
    nodes {{
      name
      color
    }}
  }}
  latestReviews(first: 100) {{
    nodes {{
      state
      submittedAt
      author {{
        login
      }}
    }}
  }}
  reviews(first: 100) {{
    nodes {{
      state
      submittedAt
      author {{
        login
      }}
    }}
  }}
  reviewThreads(first: 100) {{
    nodes {{
      isResolved
      isOutdated
      comments(first: 100) {{
        nodes {{
          author {{
            login
          }}
          createdAt
        }}
      }}
    }}
  }}
  timelineItems(last: 100, itemTypes: [READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT, REVIEW_REQUESTED_EVENT]) {{
    nodes {{
      __typename
      ... on ReadyForReviewEvent {{
        createdAt
      }}
      ... on ConvertToDraftEvent {{
        createdAt
      }}
      ... on ReviewRequestedEvent {{
        createdAt
        requestedReviewer {{
          __typename
          ... on User {{
            login
          }}
        }}
      }}
    }}
  }}
  commits(last: 1) {{
    nodes {{
      commit {{
        oid
        statusCheckRollup {{
          state
          contexts(first: 100) {{
            nodes {{
              __typename
              ... on CheckRun {{
                name
                status
                conclusion
              }}
              ... on StatusContext {{
                context
                state
              }}
            }}
          }}
        }}
      }}
    }}
  }}
}}
"#
    )
}

fn pull_request_status_alias(index: usize) -> String {
    format!("pr{index}")
}

const USER_PROFILE_QUERY_CHUNK_SIZE: usize = 50;

pub(super) fn user_profiles_query(logins: &[String]) -> String {
    let variables = (0..logins.len())
        .map(|index| format!("$login{index}: String!"))
        .collect::<Vec<_>>()
        .join(", ");
    let fields = (0..logins.len())
        .map(|index| {
            format!(
                "  {}: user(login: $login{index}) {{\n    login\n    name\n  }}",
                user_profile_alias(index)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("query({variables}) {{\n{fields}\n}}\n")
}

fn user_profiles_variables(logins: &[String]) -> BTreeMap<String, &str> {
    logins
        .iter()
        .enumerate()
        .map(|(index, login)| (format!("login{index}"), login.as_str()))
        .collect()
}

pub(super) fn user_profiles_from_graphql_response(
    logins: &[String],
    response: GraphQlResponse<BTreeMap<String, Option<GraphQlUserProfile>>>,
) -> Result<Vec<GitHubUserProfile>, GitHubError> {
    let data = user_profiles_data_from_graphql_response(response)?;
    Ok(logins
        .iter()
        .enumerate()
        .map(|(index, requested_login)| {
            let alias = user_profile_alias(index);
            data.get(&alias)
                .and_then(|profile| profile.as_ref())
                .map_or_else(
                    || GitHubUserProfile {
                        login: requested_login.clone(),
                        name: None,
                    },
                    |profile| GitHubUserProfile {
                        login: profile.login.clone(),
                        name: normalize_user_profile_name(profile.name.as_deref()),
                    },
                )
        })
        .collect())
}

fn user_profiles_data_from_graphql_response(
    response: GraphQlResponse<BTreeMap<String, Option<GraphQlUserProfile>>>,
) -> Result<BTreeMap<String, Option<GraphQlUserProfile>>, GitHubError> {
    let GraphQlResponse { data, errors } = response;
    let fatal_errors = errors
        .into_iter()
        .filter(|error| !is_missing_user_profile_error(error))
        .map(|error| error.message)
        .collect::<Vec<_>>();
    if !fatal_errors.is_empty() {
        return Err(GitHubError::GraphQl {
            operation: "load user profiles",
            message: fatal_errors.join("; "),
        });
    }
    data.ok_or_else(|| GitHubError::GraphQl {
        operation: "load user profiles",
        message: "missing GraphQL data".to_owned(),
    })
}

fn is_missing_user_profile_error(error: &GraphQlError) -> bool {
    error
        .message
        .starts_with("Could not resolve to a User with the login of ")
}

fn user_profile_alias(index: usize) -> String {
    format!("user{index}")
}

fn unique_user_profile_logins(logins: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    logins
        .iter()
        .map(|login| login.trim())
        .filter(|login| !login.is_empty())
        .filter(|login| seen.insert((*login).to_owned()))
        .map(str::to_owned)
        .collect()
}

fn normalize_user_profile_name(name: Option<&str>) -> Option<String> {
    name.map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn unique_pull_request_numbers(numbers: &[u64]) -> Vec<u64> {
    let mut seen = BTreeSet::new();
    numbers
        .iter()
        .copied()
        .filter(|number| seen.insert(*number))
        .collect()
}

#[derive(Debug, Serialize)]
struct GraphQlRequest<'a, V> {
    query: &'a str,
    variables: V,
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphQlResponse<T> {
    pub(super) data: Option<T>,
    #[serde(default)]
    pub(super) errors: Vec<GraphQlError>,
}

impl<T> GraphQlResponse<T> {
    fn into_data(self, operation: &'static str) -> Result<T, GitHubError> {
        if !self.errors.is_empty() {
            return Err(GitHubError::GraphQl {
                operation,
                message: self
                    .errors
                    .into_iter()
                    .map(|error| error.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            });
        }
        self.data.ok_or_else(|| GitHubError::GraphQl {
            operation,
            message: "missing GraphQL data".to_owned(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphQlError {
    pub(super) message: String,
}

#[derive(Debug, Serialize)]
struct PullRequestIdVariables<'a> {
    owner: &'a str,
    name: &'a str,
    number: i64,
}

#[derive(Debug, Deserialize)]
struct PullRequestIdQueryData {
    repository: Option<PullRequestIdRepository>,
}

#[derive(Debug, Deserialize)]
struct PullRequestIdRepository {
    #[serde(rename = "pullRequest")]
    pull_request: Option<PullRequestIdNode>,
}

#[derive(Debug, Deserialize)]
struct PullRequestIdNode {
    id: String,
}

#[derive(Debug, Serialize)]
struct PullRequestReadinessVariables<'a> {
    #[serde(rename = "pullRequestId")]
    pull_request_id: &'a str,
}

#[derive(Debug, Serialize)]
struct PullRequestsByNumberVariables<'a> {
    owner: &'a str,
    name: &'a str,
}

#[derive(Debug, Deserialize)]
struct PullRequestsByNumberQueryData {
    repository: Option<BTreeMap<String, Option<GraphQlPullRequest>>>,
}

#[derive(Debug, Serialize)]
struct PullRequestStatusesVariables<'a> {
    owner: &'a str,
    name: &'a str,
}

#[derive(Debug, Deserialize)]
struct PullRequestStatusesQueryData {
    repository: Option<BTreeMap<String, Option<GraphQlPullRequestStatus>>>,
}

#[derive(Debug, Serialize)]
struct PullRequestSuggestedReviewersVariables<'a> {
    owner: &'a str,
    name: &'a str,
    number: i64,
}

#[derive(Debug, Serialize)]
struct ReviewRequestsVariables<'a> {
    query: &'a str,
    cursor: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct ReviewRequestsQueryData {
    viewer: GraphQlViewer,
    search: GraphQlReviewRequestSearch,
}

#[derive(Debug, Deserialize)]
struct GraphQlViewer {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewRequestSearch {
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
    nodes: Vec<Option<GraphQlReviewRequestPullRequest>>,
}

#[derive(Debug, Serialize)]
struct AuthoredOpenPullRequestsVariables<'a> {
    query: &'a str,
    cursor: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct AuthoredOpenPullRequestsQueryData {
    search: GraphQlAuthoredOpenPullRequestSearch,
}

#[derive(Debug, Deserialize)]
struct GraphQlAuthoredOpenPullRequestSearch {
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
    nodes: Vec<Option<GraphQlAuthoredOpenPullRequest>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlAuthoredOpenPullRequest {
    number: u64,
    title: String,
    body: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    url: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    merged: bool,
    #[serde(rename = "headRepositoryOwner")]
    head_repository_owner: GraphQlReviewRequestOwner,
}

#[derive(Debug, Deserialize)]
struct GraphQlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewRequestPullRequest {
    number: u64,
    repository: GraphQlReviewRequestRepository,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewRequestRepository {
    name: String,
    owner: GraphQlReviewRequestOwner,
}

#[derive(Debug, Deserialize)]
struct GraphQlReviewRequestOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct PullRequestSuggestedReviewersQueryData {
    repository: Option<PullRequestSuggestedReviewersRepository>,
}

#[derive(Debug, Deserialize)]
struct PullRequestSuggestedReviewersRepository {
    #[serde(rename = "pullRequest")]
    pull_request: Option<GraphQlPullRequestSuggestedReviewers>,
}

#[derive(Debug, Deserialize)]
struct GraphQlPullRequestSuggestedReviewers {
    #[serde(rename = "suggestedReviewers")]
    suggested_reviewers: Vec<GraphQlSuggestedReviewer>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlPullRequestStatus {
    pub(super) number: u64,
    pub(super) title: String,
    pub(super) url: String,
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
    #[serde(rename = "headRefName")]
    pub(super) head_ref_name: String,
    #[serde(rename = "baseRefName")]
    pub(super) base_ref_name: String,
    pub(super) author: Option<GraphQlReviewAuthor>,
    #[serde(rename = "isDraft")]
    pub(super) is_draft: bool,
    pub(super) merged: bool,
    pub(super) closed: bool,
    #[serde(rename = "mergedAt")]
    pub(super) merged_at: Option<String>,
    #[serde(rename = "closedAt")]
    pub(super) closed_at: Option<String>,
    #[serde(rename = "reviewDecision")]
    pub(super) review_decision: Option<String>,
    #[serde(rename = "reviewRequests")]
    pub(super) review_requests: GraphQlReviewRequests,
    #[serde(rename = "suggestedReviewers")]
    pub(super) suggested_reviewers: Vec<GraphQlSuggestedReviewer>,
    pub(super) labels: GraphQlLabels,
    #[serde(rename = "latestReviews")]
    pub(super) latest_reviews: GraphQlReviews,
    pub(super) reviews: GraphQlReviews,
    #[serde(rename = "reviewThreads")]
    pub(super) review_threads: GraphQlReviewThreads,
    #[serde(rename = "timelineItems")]
    pub(super) timeline_items: GraphQlTimelineItems,
    pub(super) commits: GraphQlPullRequestStatusCommits,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlPullRequestStatusCommits {
    pub(super) nodes: Vec<GraphQlPullRequestStatusCommitNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlPullRequestStatusCommitNode {
    pub(super) commit: GraphQlPullRequestStatusCommit,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlPullRequestStatusCommit {
    pub(super) oid: String,
    #[serde(rename = "statusCheckRollup")]
    pub(super) status_check_rollup: Option<GraphQlStatusCheckRollup>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlStatusCheckRollup {
    pub(super) state: String,
    pub(super) contexts: GraphQlStatusCheckContexts,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlStatusCheckContexts {
    pub(super) nodes: Vec<GraphQlStatusCheckContextNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlStatusCheckContextNode {
    #[serde(rename = "__typename")]
    pub(super) type_name: String,
    pub(super) name: Option<String>,
    pub(super) context: Option<String>,
    pub(super) status: Option<String>,
    pub(super) conclusion: Option<String>,
    pub(super) state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MarkPullRequestReadyData {
    #[serde(rename = "markPullRequestReadyForReview")]
    mark_pull_request_ready_for_review: PullRequestMutationPayload,
}

#[derive(Debug, Deserialize)]
struct ConvertPullRequestToDraftData {
    #[serde(rename = "convertPullRequestToDraft")]
    convert_pull_request_to_draft: PullRequestMutationPayload,
}

#[derive(Debug, Deserialize)]
struct PullRequestMutationPayload {
    #[serde(rename = "pullRequest")]
    pull_request: GraphQlPullRequest,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlPullRequest {
    number: u64,
    title: String,
    body: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    url: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    merged: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlSuggestedReviewer {
    pub(super) reviewer: Option<GraphQlSuggestedReviewerUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlSuggestedReviewerUser {
    pub(super) login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlUserProfile {
    pub(super) login: String,
    pub(super) name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewRequests {
    #[serde(rename = "totalCount")]
    pub(super) total_count: usize,
    pub(super) nodes: Vec<GraphQlReviewRequestNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewRequestNode {
    #[serde(rename = "requestedReviewer")]
    pub(super) requested_reviewer: Option<GraphQlRequestedReviewer>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlRequestedReviewer {
    #[serde(rename = "__typename")]
    pub(super) type_name: String,
    pub(super) login: Option<String>,
    pub(super) slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlLabels {
    pub(super) nodes: Vec<GraphQlLabelNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlLabelNode {
    pub(super) name: String,
    pub(super) color: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviews {
    pub(super) nodes: Vec<GraphQlReviewNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewNode {
    pub(super) state: String,
    #[serde(rename = "submittedAt")]
    pub(super) submitted_at: Option<String>,
    pub(super) author: Option<GraphQlReviewAuthor>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewThreads {
    pub(super) nodes: Vec<GraphQlReviewThreadNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewThreadNode {
    #[serde(rename = "isResolved")]
    pub(super) is_resolved: bool,
    #[serde(rename = "isOutdated")]
    pub(super) is_outdated: bool,
    pub(super) comments: GraphQlReviewThreadComments,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewThreadComments {
    pub(super) nodes: Vec<GraphQlReviewThreadCommentNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewThreadCommentNode {
    pub(super) author: Option<GraphQlReviewAuthor>,
    #[serde(rename = "createdAt")]
    pub(super) created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlReviewAuthor {
    pub(super) login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GraphQlTimelineItems {
    pub(super) nodes: Vec<GraphQlTimelineItemNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "__typename")]
pub(super) enum GraphQlTimelineItemNode {
    #[serde(rename = "ReadyForReviewEvent")]
    ReadyForReview {
        #[serde(rename = "createdAt")]
        created_at: String,
    },
    #[serde(rename = "ConvertToDraftEvent")]
    ConvertToDraft {
        #[serde(rename = "createdAt")]
        created_at: String,
    },
    #[serde(rename = "ReviewRequestedEvent")]
    ReviewRequested {
        #[serde(rename = "createdAt")]
        created_at: String,
        #[serde(rename = "requestedReviewer")]
        requested_reviewer: Option<GraphQlRequestedReviewer>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct AuthenticatedUserResponse {
    login: String,
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

#[derive(Debug, Deserialize)]
struct GitRefResponse {
    object: GitRefObject,
}

#[derive(Debug, Deserialize)]
struct GitRefObject {
    sha: String,
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
    let reviewers = ReviewerSelection::new(
        pull.requested_reviewers
            .unwrap_or_default()
            .into_iter()
            .map(|user| user.login),
        pull.requested_teams
            .unwrap_or_default()
            .into_iter()
            .map(|team| team.slug),
    );

    PullRequestRecord {
        number: pull.number,
        title: pull.title.unwrap_or_default(),
        body: pull.body,
        head_branch: pull.head.ref_field,
        base_branch: pull.base.ref_field,
        html_url: pull.html_url.map(|url| url.to_string()),
        draft: pull.draft.unwrap_or(false),
        merged: pull.merged.unwrap_or(false) || pull.merged_at.is_some(),
        reviewers,
    }
}

fn map_graphql_pull_request(pull: GraphQlPullRequest) -> PullRequestRecord {
    PullRequestRecord {
        number: pull.number,
        title: pull.title,
        body: (!pull.body.is_empty()).then_some(pull.body),
        head_branch: pull.head_ref_name,
        base_branch: pull.base_ref_name,
        html_url: Some(pull.url),
        draft: pull.is_draft,
        merged: pull.merged,
        reviewers: ReviewerSelection::default(),
    }
}

pub(super) fn map_graphql_pull_request_status(
    pull: GraphQlPullRequestStatus,
) -> PullRequestStatusRecord {
    let requested_reviewer_count = pull.review_requests.total_count;
    let pull_request_author = pull.author.as_ref().map(|author| author.login.as_str());
    let requested_reviewers = reviewer_selection_from_graphql(pull.review_requests.nodes);
    let suggested_reviewers = suggested_reviewers_from_graphql(pull.suggested_reviewers);
    let latest_review_nodes = pull.latest_reviews.nodes;
    let review_nodes = pull.reviews.nodes;
    let review_thread_nodes = pull.review_threads.nodes;
    let review_activity = review_activity_from_graphql(
        &latest_review_nodes,
        &review_nodes,
        &review_thread_nodes,
        pull_request_author,
    );
    let approved_reviewers =
        approved_reviewers_from_graphql(&latest_review_nodes, pull_request_author);
    let dismissed_reviewers =
        dismissed_reviewers_from_graphql(&latest_review_nodes, pull_request_author);
    let thread_reviewers =
        review_thread_reviewers_from_graphql(review_thread_nodes, pull_request_author);
    let commented_reviewers = active_commented_reviewers(
        thread_reviewers.commented.clone(),
        commented_reviewers_from_graphql(review_nodes, &approved_reviewers, pull_request_author),
        &thread_reviewers.seen,
    );
    let addressed_reviewers = thread_reviewers.addressed;
    let timeline_events = timeline_events_from_graphql(pull.timeline_items.nodes);
    let labels = labels_from_graphql(pull.labels.nodes);
    let latest_commit = pull
        .commits
        .nodes
        .into_iter()
        .last()
        .map(|node| node.commit);
    let check_status = latest_commit
        .as_ref()
        .and_then(|commit| commit.status_check_rollup.as_ref())
        .map_or(PullRequestCheckStatus::Missing, |rollup| {
            map_check_status(&rollup.state)
        });
    let checks = latest_commit
        .as_ref()
        .and_then(|commit| commit.status_check_rollup.as_ref())
        .map(|rollup| checks_from_graphql(rollup.contexts.nodes.clone()))
        .unwrap_or_default();
    let review_status = map_review_status(
        pull.review_decision.as_deref(),
        requested_reviewer_count > 0,
    );

    PullRequestStatusRecord {
        number: pull.number,
        title: pull.title,
        url: Some(pull.url),
        created_at: Some(pull.created_at),
        head_branch: pull.head_ref_name,
        base_branch: pull.base_ref_name,
        author: pull.author.map(|author| author.login),
        draft: pull.is_draft,
        merged: pull.merged,
        closed: pull.closed,
        merged_at: pull.merged_at,
        closed_at: pull.closed_at,
        check_status,
        checks,
        review_status,
        requested_reviewers,
        suggested_reviewers,
        approved_reviewers,
        commented_reviewers,
        addressed_reviewers,
        dismissed_reviewers,
        review_activity,
        timeline_events,
        labels,
        latest_commit_oid: latest_commit.map(|commit| commit.oid),
    }
}

fn map_check_status(state: &str) -> PullRequestCheckStatus {
    match state {
        "SUCCESS" => PullRequestCheckStatus::Passing,
        "FAILURE" | "ERROR" => PullRequestCheckStatus::Failing,
        "PENDING" | "EXPECTED" => PullRequestCheckStatus::Pending,
        _ => PullRequestCheckStatus::Unknown,
    }
}

fn checks_from_graphql(nodes: Vec<GraphQlStatusCheckContextNode>) -> Vec<PullRequestCheck> {
    nodes.into_iter().filter_map(check_from_graphql).collect()
}

fn check_from_graphql(node: GraphQlStatusCheckContextNode) -> Option<PullRequestCheck> {
    let name = match node.type_name.as_str() {
        "CheckRun" => node.name,
        "StatusContext" => node.context,
        _ => node.name.or(node.context),
    }?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let status = match node.type_name.as_str() {
        "CheckRun" => map_check_run_status(node.status.as_deref(), node.conclusion.as_deref()),
        "StatusContext" => node
            .state
            .as_deref()
            .map_or(PullRequestCheckStatus::Unknown, map_check_status),
        _ => PullRequestCheckStatus::Unknown,
    };

    Some(PullRequestCheck {
        name: name.to_owned(),
        status,
    })
}

fn map_check_run_status(status: Option<&str>, conclusion: Option<&str>) -> PullRequestCheckStatus {
    match status {
        Some("COMPLETED") => match conclusion {
            Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => PullRequestCheckStatus::Passing,
            Some(
                "FAILURE" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "CANCELLED"
                | "STALE",
            ) => PullRequestCheckStatus::Failing,
            Some(_) | None => PullRequestCheckStatus::Unknown,
        },
        Some("QUEUED" | "IN_PROGRESS" | "REQUESTED" | "WAITING" | "PENDING") => {
            PullRequestCheckStatus::Pending
        }
        Some(_) | None => PullRequestCheckStatus::Unknown,
    }
}

fn map_review_status(decision: Option<&str>, has_review_requests: bool) -> PullRequestReviewStatus {
    match decision {
        Some("APPROVED") => PullRequestReviewStatus::Approved,
        Some("CHANGES_REQUESTED") => PullRequestReviewStatus::ChangesRequested,
        Some("REVIEW_REQUIRED") => PullRequestReviewStatus::ReviewRequired,
        Some(_) => PullRequestReviewStatus::Unknown,
        None if has_review_requests => PullRequestReviewStatus::ReviewRequested,
        None => PullRequestReviewStatus::NotReviewed,
    }
}

fn reviewer_selection_from_graphql(nodes: Vec<GraphQlReviewRequestNode>) -> ReviewerSelection {
    let mut users = Vec::new();
    let mut teams = Vec::new();
    for node in nodes.into_iter().filter_map(|node| node.requested_reviewer) {
        match node.type_name.as_str() {
            "User" => users.extend(node.login),
            "Team" => teams.extend(node.slug),
            _ => {}
        }
    }
    ReviewerSelection::new(users, teams)
}

pub(super) fn suggested_reviewers_from_graphql(
    nodes: Vec<GraphQlSuggestedReviewer>,
) -> Vec<String> {
    let mut reviewers = Vec::new();
    let mut seen = BTreeSet::new();
    for login in nodes
        .into_iter()
        .filter_map(|node| node.reviewer.map(|reviewer| reviewer.login))
        .map(|login| login.trim().to_owned())
        .filter(|login| !login.is_empty())
    {
        if seen.insert(login.clone()) {
            reviewers.push(login);
        }
    }
    reviewers
}

fn timeline_events_from_graphql(
    nodes: Vec<GraphQlTimelineItemNode>,
) -> Vec<PullRequestTimelineEvent> {
    nodes
        .into_iter()
        .filter_map(|node| match node {
            GraphQlTimelineItemNode::ReadyForReview { created_at } => {
                Some(PullRequestTimelineEvent {
                    kind: PullRequestTimelineEventKind::ReadyForReview,
                    created_at,
                    reviewer: None,
                })
            }
            GraphQlTimelineItemNode::ConvertToDraft { created_at } => {
                Some(PullRequestTimelineEvent {
                    kind: PullRequestTimelineEventKind::ConvertToDraft,
                    created_at,
                    reviewer: None,
                })
            }
            GraphQlTimelineItemNode::ReviewRequested {
                created_at,
                requested_reviewer,
            } => Some(PullRequestTimelineEvent {
                kind: PullRequestTimelineEventKind::ReviewRequested,
                created_at,
                reviewer: requested_reviewer.and_then(timeline_requested_reviewer_name),
            }),
            GraphQlTimelineItemNode::Other => None,
        })
        .collect()
}

fn timeline_requested_reviewer_name(reviewer: GraphQlRequestedReviewer) -> Option<String> {
    match reviewer.type_name.as_str() {
        "User" => reviewer.login,
        "Team" => reviewer.slug.map(|slug| format!("team/{slug}")),
        _ => None,
    }
}

fn review_activity_from_graphql(
    latest_reviews: &[GraphQlReviewNode],
    reviews: &[GraphQlReviewNode],
    review_threads: &[GraphQlReviewThreadNode],
    pull_request_author: Option<&str>,
) -> Vec<PullRequestReviewActivity> {
    let mut reviewed_at_by_reviewer = BTreeMap::<String, String>::new();
    for node in latest_reviews.iter().chain(reviews.iter()) {
        let Some(author) = &node.author else {
            continue;
        };
        let Some(reviewed_at) = node.submitted_at.as_deref() else {
            continue;
        };
        record_review_activity(
            &mut reviewed_at_by_reviewer,
            author.login.as_str(),
            reviewed_at,
            pull_request_author,
        );
    }
    for comment in review_threads
        .iter()
        .flat_map(|thread| thread.comments.nodes.iter())
    {
        let Some(author) = &comment.author else {
            continue;
        };
        record_review_activity(
            &mut reviewed_at_by_reviewer,
            author.login.as_str(),
            comment.created_at.as_str(),
            pull_request_author,
        );
    }

    reviewed_at_by_reviewer
        .into_iter()
        .map(|(reviewer, reviewed_at)| PullRequestReviewActivity {
            reviewer,
            reviewed_at,
        })
        .collect()
}

fn record_review_activity(
    reviewed_at_by_reviewer: &mut BTreeMap<String, String>,
    login: &str,
    reviewed_at: &str,
    pull_request_author: Option<&str>,
) {
    let login = login.trim();
    if login.is_empty() || pull_request_author == Some(login) {
        return;
    }
    let current = reviewed_at_by_reviewer.entry(login.to_owned()).or_default();
    if current.as_str() < reviewed_at {
        *current = reviewed_at.to_owned();
    }
}

fn approved_reviewers_from_graphql(
    nodes: &[GraphQlReviewNode],
    pull_request_author: Option<&str>,
) -> Vec<String> {
    review_state_authors_from_graphql(nodes, "APPROVED", pull_request_author)
}

fn dismissed_reviewers_from_graphql(
    nodes: &[GraphQlReviewNode],
    pull_request_author: Option<&str>,
) -> Vec<String> {
    review_state_authors_from_graphql(nodes, "DISMISSED", pull_request_author)
}

fn review_state_authors_from_graphql(
    nodes: &[GraphQlReviewNode],
    state: &str,
    pull_request_author: Option<&str>,
) -> Vec<String> {
    let mut reviewers = Vec::new();
    let mut seen = BTreeSet::new();
    for node in nodes {
        if node.state == state {
            let Some(author) = &node.author else {
                continue;
            };
            let login = author.login.trim();
            if !login.is_empty()
                && pull_request_author != Some(login)
                && seen.insert(login.to_owned())
            {
                reviewers.push(login.to_owned());
            }
        }
    }
    reviewers
}

fn commented_reviewers_from_graphql(
    nodes: Vec<GraphQlReviewNode>,
    approved_reviewers: &[String],
    pull_request_author: Option<&str>,
) -> Vec<String> {
    let approved = approved_reviewers
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut reviewers = Vec::new();
    let mut seen = BTreeSet::new();
    for node in nodes {
        if node.state == "COMMENTED" {
            let Some(author) = node.author else {
                continue;
            };
            let login = author.login.trim();
            if !login.is_empty()
                && pull_request_author != Some(login)
                && !approved.contains(login)
                && seen.insert(login.to_owned())
            {
                reviewers.push(login.to_owned());
            }
        }
    }
    reviewers
}

#[derive(Debug, Default)]
struct ReviewThreadReviewers {
    commented: Vec<String>,
    addressed: Vec<String>,
    seen: BTreeSet<String>,
}

fn review_thread_reviewers_from_graphql(
    nodes: Vec<GraphQlReviewThreadNode>,
    pull_request_author: Option<&str>,
) -> ReviewThreadReviewers {
    let mut result = ReviewThreadReviewers::default();
    for thread in nodes {
        let mut latest_author_comment_at: Option<String> = None;
        let mut reviewer_comments: Vec<(String, String)> = Vec::new();
        for comment in thread.comments.nodes {
            let Some(author) = comment.author else {
                continue;
            };
            let login = author.login.trim();
            if login.is_empty() {
                continue;
            }
            if pull_request_author == Some(login) {
                replace_if_newer(&mut latest_author_comment_at, &comment.created_at);
                continue;
            }

            result.seen.insert(login.to_owned());
            if let Some((_, created_at)) = reviewer_comments
                .iter_mut()
                .find(|(reviewer, _)| reviewer == login)
            {
                if created_at.as_str() < comment.created_at.as_str() {
                    *created_at = comment.created_at;
                }
            } else {
                reviewer_comments.push((login.to_owned(), comment.created_at));
            }
        }

        if thread.is_resolved || thread.is_outdated {
            continue;
        }

        for (reviewer, reviewer_comment_at) in reviewer_comments {
            if latest_author_comment_at
                .as_ref()
                .is_some_and(|author_comment_at| author_comment_at > &reviewer_comment_at)
            {
                push_unique(&mut result.addressed, reviewer);
            } else {
                push_unique(&mut result.commented, reviewer);
            }
        }
    }

    let commented = result
        .commented
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    result
        .addressed
        .retain(|reviewer| !commented.contains(reviewer.as_str()));
    result
}

fn active_commented_reviewers(
    thread_commented_reviewers: Vec<String>,
    historical_commented_reviewers: Vec<String>,
    thread_reviewers: &BTreeSet<String>,
) -> Vec<String> {
    let mut reviewers = Vec::new();
    for reviewer in thread_commented_reviewers {
        push_unique(&mut reviewers, reviewer);
    }
    for reviewer in historical_commented_reviewers {
        if !thread_reviewers.contains(reviewer.as_str()) {
            push_unique(&mut reviewers, reviewer);
        }
    }
    reviewers
}

fn replace_if_newer(current: &mut Option<String>, candidate: &str) {
    if current
        .as_deref()
        .is_none_or(|existing| existing < candidate)
    {
        *current = Some(candidate.to_owned());
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn labels_from_graphql(nodes: Vec<GraphQlLabelNode>) -> Vec<PullRequestLabel> {
    nodes
        .into_iter()
        .map(|node| PullRequestLabel {
            name: node.name,
            color: node.color,
        })
        .collect()
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
