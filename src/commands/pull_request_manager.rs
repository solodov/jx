use super::*;

/// Command-side integration boundary for pull-request stack state.
pub(super) struct PullRequestStackManager<'a> {
    context: &'a RepositoryContext,
    services: &'a dyn CommandServices,
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

        let live_pull_requests = self.authored_open_pull_requests_for_branches(&local_branches)?;
        let metadata =
            stack_metadata_from_pull_requests(&live_pull_requests, &self.read_metadata()?);
        self.write_metadata(&metadata)?;

        Ok(PullRequestStackSnapshot::from_metadata(
            &metadata,
            &local_branches,
            &live_pull_requests,
            PullRequestStackSelection::default(),
        ))
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

    /// Syncs PR descriptions using the currently stored stack metadata.
    pub(super) fn sync_pull_requests(
        &self,
        push: &TrackedPushOutcome,
    ) -> Result<Vec<PullRequestRecord>, CommandError> {
        if push.bookmarks.is_empty() {
            return Ok(Vec::new());
        }
        let metadata = self.read_metadata()?;
        Ok(self
            .services
            .sync_pull_requests(self.context, push, &metadata)?)
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

    fn read_metadata(&self) -> Result<StackMetadata, CommandError> {
        read_stack_metadata(&self.context.repository_root).map_err(Into::into)
    }

    fn write_metadata(&self, metadata: &StackMetadata) -> Result<(), CommandError> {
        write_stack_metadata(&self.context.repository_root, metadata).map_err(Into::into)
    }
}
