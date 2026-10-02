use super::*;

impl JjWorkspace {
    /// Returns shortest usable change-ID prefixes for exact local bookmarks using the log's prefix rules.
    /// Missing/conflicted bookmarks and divergent changes are omitted rather than returned as ambiguous selectors.
    pub fn local_bookmark_change_aliases(
        &self,
        bookmarks: &[String],
    ) -> Result<BTreeMap<String, String>, JjError> {
        let mut commits = Vec::new();
        for bookmark in bookmarks {
            let target = self.repo.view().get_local_bookmark(RefName::new(bookmark));
            let Some(commit_id) = target.as_normal() else {
                continue;
            };
            let commit = self.load_commit(commit_id)?;
            let targets = self
                .repo
                .resolve_change_id(commit.change_id())
                .map_err(|error| JjError::Index {
                    message: error.to_string(),
                })?;
            if targets
                .is_some_and(|targets| targets.has_visible(commit_id) && !targets.is_divergent())
            {
                commits.push((bookmark, commit));
            }
        }
        if commits.is_empty() {
            return Ok(BTreeMap::new());
        }

        let settings = self.workspace.settings();
        let ui = Ui::null();
        let fileset_aliases =
            load_fileset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let revset_aliases =
            load_revset_aliases(&ui, settings.config()).map_err(log_command_error)?;
        let extensions = Arc::new(RevsetExtensions::default());
        let path_converter = RepoPathUiConverter::Fs {
            cwd: self.workspace.workspace_root().to_path_buf(),
            base: self.workspace.workspace_root().to_path_buf(),
        };
        let context = revset_parse_context(
            settings,
            self.repo.as_ref(),
            &fileset_aliases,
            &revset_aliases,
            &extensions,
            Some(RevsetWorkspaceContext {
                path_converter: &path_converter,
                workspace_name: self.workspace.workspace_name(),
            }),
        )?;
        let prefix_context = log_id_prefix_context(settings, &ui, &context, extensions.clone())?;
        let prefix_index = prefix_context
            .populate(self.repo.as_ref())
            .map_err(|error| JjError::Index {
                message: error.to_string(),
            })?;

        commits
            .into_iter()
            .map(|(bookmark, commit)| {
                let length = prefix_index
                    .shortest_change_prefix_len(self.repo.as_ref(), commit.change_id())
                    .map_err(|error| JjError::Index {
                        message: error.to_string(),
                    })?
                    .max(1);
                let alias = commit
                    .change_id()
                    .reverse_hex()
                    .chars()
                    .take(length)
                    .collect();
                Ok((bookmark.clone(), alias))
            })
            .collect()
    }
}
