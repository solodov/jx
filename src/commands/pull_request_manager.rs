use super::*;

/// Command-side integration boundary for pull-request stack state.
pub(super) struct PullRequestStackManager<'a> {
    context: &'a RepositoryContext,
    services: &'a dyn CommandServices,
}

/// PRs whose generated stack context was refreshed after publishing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PullRequestStackPublishUpdate {
    pub(super) pull_requests: Vec<PullRequestRecord>,
}

impl PullRequestStackPublishUpdate {
    pub(super) fn is_empty(&self) -> bool {
        self.pull_requests.is_empty()
    }
}

impl<'a> PullRequestStackManager<'a> {
    pub(super) fn new(context: &'a RepositoryContext, services: &'a dyn CommandServices) -> Self {
        Self { context, services }
    }

    /// Loads the locally stored stack snapshot without requiring GitHub access.
    pub(super) fn stored_snapshot(
        &self,
        selection: PullRequestStackSelection,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        self.snapshot_from_live_pull_requests(Vec::new(), selection)
    }

    /// Tracks authored open PRs attached to local bookmarks and writes durable stack state.
    pub(super) fn track_authored_open_pull_requests(
        &self,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let local_branches = self.local_pull_request_branches()?;
        if local_branches.is_empty() {
            let metadata = StackMetadata::default();
            self.write_metadata(&metadata)?;
            return Ok(PullRequestStackSnapshot::from_metadata(
                &metadata,
                &local_branches,
                &[],
                PullRequestStackSelection::default(),
            ));
        }

        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let live_pull_requests = self.authored_open_pull_requests_for_branches(&local_branches)?;
        let metadata = stack_metadata_from_pull_requests(&live_pull_requests, &metadata);
        self.write_metadata(&metadata)?;

        Ok(PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &live_pull_requests,
            PullRequestStackSelection::default(),
        ))
    }

    /// Builds the stack snapshot used by interactive PR opening.
    pub(super) fn interactive_open_snapshot(
        &self,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let metadata = self.read_metadata()?;
        let local_branches = self.local_pull_request_branches()?;
        if metadata.nodes.is_empty() && local_branches.is_empty() {
            return Err(missing_local_bookmark_pull_requests(self.context).into());
        }

        let selected_branches = self
            .services
            .pull_request_candidate_bookmarks(self.context, None)?;
        let live_pull_requests = if local_branches.is_empty() {
            Vec::new()
        } else {
            self.authored_open_pull_requests_for_branches(&local_branches)?
        };
        let selection = selected_branches
            .first()
            .map(|branch| PullRequestStackSelection::branch(branch.clone()))
            .unwrap_or_default();
        let snapshot = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &live_pull_requests,
            selection,
        );
        let component = snapshot.component_for_branches(&selected_branches);
        if stack_snapshot_has_openable_pull_request(&component) {
            return Ok(component);
        }
        if stack_snapshot_has_openable_pull_request(&snapshot) {
            return Ok(snapshot);
        }

        Err(missing_local_bookmark_pull_requests(self.context).into())
    }

    /// Selects local stack component branches for a jj revision/bookmark selector.
    pub(super) fn local_component_branches_for_selector(
        &self,
        selector: Option<&str>,
    ) -> Result<Vec<String>, CommandError> {
        let selected_branches = self
            .services
            .pull_request_candidate_bookmarks(self.context, selector)?;
        self.local_component_branches_for(&selected_branches)
    }

    /// Returns local stack component branches in merge order for one or more selected branches.
    pub(super) fn local_component_branches_for(
        &self,
        selected_branches: &[String],
    ) -> Result<Vec<String>, CommandError> {
        Ok(self
            .stored_snapshot(PullRequestStackSelection::default())?
            .local_component_branches_for(selected_branches))
    }

    /// Upserts a newly published PR and refreshes stack context for its component.
    pub(super) fn update_after_publish(
        &self,
        report: &PullRequestReport,
    ) -> Result<PullRequestStackPublishUpdate, CommandError> {
        let selection = PullRequestStackSelection::pull_request(report.pull_request.number);
        let mut seed_pull_requests = Vec::new();
        if let Some(base_pull_request) = &report.base_pull_request {
            seed_pull_requests.push(base_pull_request.clone());
        }
        seed_pull_requests.push(report.pull_request.clone());

        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let metadata = upsert_stack_metadata_pull_requests(&seed_pull_requests, &metadata);
        self.write_metadata(&metadata)?;

        let snapshot = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &[],
            &seed_pull_requests,
            selection.clone(),
        );
        let component = snapshot.component_for_selection(selection.clone());
        if component.nodes.len() <= 1 {
            return Ok(PullRequestStackPublishUpdate::default());
        }

        let component_pull_requests =
            self.pull_requests_for_component(&component, &seed_pull_requests)?;
        let metadata = refresh_stack_metadata_pull_requests(&component_pull_requests, &metadata);
        self.write_metadata(&metadata)?;

        let refreshed_snapshot = PullRequestStackSnapshot::from_metadata(
            &metadata,
            &[],
            &component_pull_requests,
            selection.clone(),
        );
        let refreshed_component = refreshed_snapshot.component_for_selection(selection);
        let push = stack_context_sync_push(&refreshed_component, &component_pull_requests);
        let pull_requests = self.sync_pull_requests_with_metadata(&push, &metadata)?;

        Ok(PullRequestStackPublishUpdate { pull_requests })
    }

    /// Syncs PR descriptions using the currently stored stack metadata.
    pub(super) fn sync_pull_requests(
        &self,
        push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let metadata = self.sync_metadata()?;
        if push.bookmarks.is_empty() {
            return Ok(Vec::new());
        }
        self.sync_pull_requests_with_metadata(push, &metadata)
    }

    /// Removes durable stack state while preserving generated metadata ignore rules.
    pub(super) fn reset(&self) -> Result<(), CommandError> {
        reset_stack_metadata(&self.context.repository_root).map_err(Into::into)
    }

    fn snapshot_from_live_pull_requests(
        &self,
        live_pull_requests: Vec<PullRequestRecord>,
        selection: PullRequestStackSelection,
    ) -> Result<PullRequestStackSnapshot, CommandError> {
        let metadata = self.read_metadata()?;
        let local_branches = self.local_pull_request_branches()?;
        Ok(PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &live_pull_requests,
            selection,
        ))
    }

    fn authored_open_pull_requests_for_branches(
        &self,
        branches: &[String],
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let author = self
            .services
            .authenticated_login(&self.context.token_source)?;
        let mut pull_requests = Vec::new();
        let mut seen_numbers = BTreeSet::new();
        for branch in branches {
            let Some(pull_request) = self.services.find_authored_open_pull_request_for_head(
                self.context,
                branch,
                &author,
            )?
            else {
                continue;
            };
            if seen_numbers.insert(pull_request.number) {
                pull_requests.push(pull_request);
            }
        }
        Ok(pull_requests)
    }

    fn local_pull_request_branches(&self) -> Result<Vec<String>, CommandError> {
        self.services
            .pull_request_bookmarks(self.context)
            .map_err(Into::into)
    }

    fn pull_requests_for_component(
        &self,
        component: &PullRequestStackSnapshot,
        seed_pull_requests: &[PullRequestRecord],
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let mut pull_requests_by_branch = seed_pull_requests
            .iter()
            .map(|pull_request| (pull_request.head_branch.clone(), pull_request.clone()))
            .collect::<BTreeMap<_, _>>();

        for node in &component.nodes {
            if pull_requests_by_branch.contains_key(&node.branch) {
                continue;
            }
            let pull_request = match self
                .services
                .find_pull_request_for_head(self.context, &node.branch)?
            {
                Some(pull_request) => Some(pull_request),
                None => node
                    .pull_request_number()
                    .map(|number| {
                        self.services
                            .find_pull_request_by_number(self.context, number)
                    })
                    .transpose()?
                    .flatten(),
            };
            let Some(pull_request) = pull_request else {
                continue;
            };
            pull_requests_by_branch.insert(node.branch.clone(), pull_request);
        }

        Ok(component
            .nodes
            .iter()
            .filter_map(|node| pull_requests_by_branch.get(&node.branch).cloned())
            .collect())
    }

    fn sync_pull_requests_with_metadata(
        &self,
        push: &TrackedPushOutcome,
        metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        Ok(self
            .services
            .sync_pull_requests(self.context, push, metadata)?)
    }

    fn sync_metadata(&self) -> Result<StackMetadata, CommandError> {
        let metadata = self.refresh_metadata_by_number(self.read_metadata()?)?;
        let pruned = prune_merged_stack_metadata_trees(&metadata);
        if pruned != metadata {
            self.write_metadata(&pruned)?;
        }
        Ok(pruned)
    }

    fn refresh_metadata_by_number(
        &self,
        metadata: StackMetadata,
    ) -> Result<StackMetadata, CommandError> {
        let pull_requests = self.pull_requests_for_metadata_numbers(&metadata)?;
        if pull_requests.is_empty() {
            return Ok(metadata);
        }

        let refreshed = refresh_stack_metadata_pull_requests(&pull_requests, &metadata);
        if refreshed != metadata {
            self.write_metadata(&refreshed)?;
        }
        Ok(refreshed)
    }

    fn pull_requests_for_metadata_numbers(
        &self,
        metadata: &StackMetadata,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        let mut pull_requests = Vec::new();
        let mut seen_numbers = BTreeSet::new();
        for number in metadata.nodes.iter().filter_map(|node| node.pull_request) {
            if !seen_numbers.insert(number) {
                continue;
            }
            let Some(pull_request) = self
                .services
                .find_pull_request_by_number(self.context, number)?
            else {
                continue;
            };
            pull_requests.push(pull_request);
        }
        Ok(pull_requests)
    }

    fn read_metadata(&self) -> Result<StackMetadata, CommandError> {
        read_stack_metadata(&self.context.repository_root).map_err(Into::into)
    }

    fn write_metadata(&self, metadata: &StackMetadata) -> Result<(), CommandError> {
        write_stack_metadata(&self.context.repository_root, metadata).map_err(Into::into)
    }
}

fn stack_snapshot_has_openable_pull_request(snapshot: &PullRequestStackSnapshot) -> bool {
    snapshot
        .nodes
        .iter()
        .any(|node| node.pull_request_number().is_some())
}

fn missing_local_bookmark_pull_requests(context: &RepositoryContext) -> WorkflowError {
    WorkflowError::MissingLocalBookmarkPullRequests {
        repository: context.origin.github.slug(),
    }
}

fn stack_context_sync_push(
    component: &PullRequestStackSnapshot,
    pull_requests: &[PullRequestRecord],
) -> TrackedPushOutcome {
    let pull_requests_by_branch = pull_requests
        .iter()
        .map(|pull_request| (pull_request.head_branch.as_str(), pull_request))
        .collect::<BTreeMap<_, _>>();
    let pull_requests_by_number = pull_requests
        .iter()
        .map(|pull_request| (pull_request.number, pull_request))
        .collect::<BTreeMap<_, _>>();
    let bookmarks = component
        .nodes
        .iter()
        .filter_map(|node| {
            node.pull_request_number()
                .and_then(|number| pull_requests_by_number.get(&number).copied())
                .or_else(|| pull_requests_by_branch.get(node.branch.as_str()).copied())
        })
        .map(|pull_request| PushedBookmarkSummary {
            branch: pull_request.head_branch.clone(),
            old_short_commit_id: None,
            new_short_commit_id: None,
            old_description: None,
            new_description: Some(pull_request.title.clone()),
            pull_request_description: Some(pull_request_description(pull_request)),
            pull_request_base: Some(pull_request.base_branch.clone()),
            new_workspace_visibility: WorkspaceVisibility::default(),
        })
        .collect::<Vec<_>>();

    TrackedPushOutcome {
        pushed_refs: 0,
        bookmarks,
        pushed_commits: Vec::new(),
    }
}

fn pull_request_description(pull_request: &PullRequestRecord) -> String {
    match pull_request
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
    {
        Some(body) => format!("{}\n\n{body}", pull_request.title),
        None => pull_request.title.clone(),
    }
}
