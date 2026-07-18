use super::*;

impl JjWorkspace {
    /// Renders the default jj log restricted to commits reachable from the current workspace.
    pub fn current_workspace_log(
        current_dir: &Path,
        annotations: &[LogBookmarkAnnotation],
    ) -> Result<String, JjError> {
        let workspace_root = find_jj_workspace_root(current_dir)?;
        let (workspace, repo) = load_workspace_for_log(&workspace_root)?;

        render_current_workspace_log(&workspace, repo.as_ref(), current_dir, annotations)
    }

    /// Renders caller-provided content through jj's workspace formatter and color rules.
    pub fn render_workspace_formatted_output(
        current_dir: &Path,
        render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
    ) -> Result<String, JjError> {
        let workspace_root = find_jj_workspace_root(current_dir)?;
        render_workspace_formatted_output(&workspace_root, render)
    }
}

#[cfg(test)]
pub(super) fn user_settings() -> Result<UserSettings, JjError> {
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(default_config_layers());
    jj_lib::config::migrate(&mut config, &default_config_migrations()).map_err(log_error)?;
    UserSettings::from_config(config).map_err(|error| JjError::Settings {
        message: error.to_string(),
    })
}

pub(super) fn find_jj_workspace_root(start: &Path) -> Result<PathBuf, JjError> {
    for candidate in start.ancestors() {
        if candidate.join(".jj").is_dir() {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(JjError::WorkspaceLoad {
        message: "No jj workspace found. Run `jx` from inside a jj workspace.".to_owned(),
    })
}

pub(super) fn load_workspace_for_log(
    workspace_root: &Path,
) -> Result<(Workspace, Arc<ReadonlyRepo>), JjError> {
    let ui = Ui::null();
    let loader = DefaultWorkspaceLoaderFactory
        .create(workspace_root)
        .map_err(log_error)?;
    let config = resolved_workspace_config_for_log(&ui, loader.as_ref())?;
    let settings = UserSettings::from_config(config).map_err(|error| JjError::Settings {
        message: error.to_string(),
    })?;
    let store_factories = StoreFactories::default();
    let working_copy_factories = default_working_copy_factories();
    let workspace = loader
        .load(&settings, &store_factories, &working_copy_factories)
        .map_err(|error| JjError::WorkspaceLoad {
            message: error.to_string(),
        })?;
    let repo = pollster::block_on(workspace.repo_loader().load_at_head()).map_err(|error| {
        JjError::RepoLoad {
            message: error.to_string(),
        }
    })?;

    Ok((workspace, repo))
}

pub(super) fn render_workspace_formatted_output(
    workspace_root: &Path,
    render: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
) -> Result<String, JjError> {
    let ui = Ui::null();
    let loader = DefaultWorkspaceLoaderFactory
        .create(workspace_root)
        .map_err(render_error)?;
    let config = resolved_workspace_config_for_render(&ui, loader.as_ref())?;
    let ui = Ui::with_config(&config).map_err(render_command_error)?;
    let mut output = Vec::new();

    {
        let mut formatter = ui.new_formatter(&mut output);
        render(formatter.as_mut()).map_err(render_error)?;
    }

    String::from_utf8(output).map_err(render_error)
}

pub(super) fn resolved_workspace_config_for_log(
    ui: &Ui,
    loader: &dyn WorkspaceLoader,
) -> Result<StackedConfig, JjError> {
    resolved_workspace_config(ui, loader, log_error, log_command_error)
}

pub(super) fn resolved_workspace_config_for_render(
    ui: &Ui,
    loader: &dyn WorkspaceLoader,
) -> Result<StackedConfig, JjError> {
    resolved_workspace_config(ui, loader, render_error, render_command_error)
}

pub(super) fn resolved_workspace_config_for_workspace_load(
    ui: &Ui,
    loader: &dyn WorkspaceLoader,
) -> Result<StackedConfig, JjError> {
    resolved_workspace_config(ui, loader, settings_error, workspace_config_command_error)
}

pub(super) fn resolved_workspace_config(
    ui: &Ui,
    loader: &dyn WorkspaceLoader,
    map_error: impl Fn(String) -> JjError,
    map_command_error: impl Fn(jj_cli::command_error::CommandError) -> JjError,
) -> Result<StackedConfig, JjError> {
    let mut raw_config = config_from_environment(jx_default_config_layers());
    let mut config_env = jj_cli::config::ConfigEnv::from_environment();
    config_env
        .reload_user_config(&mut raw_config)
        .map_err(|error| map_error(error.to_string()))?;
    config_env.reset_repo_path(loader.repo_path());
    config_env
        .reload_repo_config(ui, &mut raw_config)
        .map_err(&map_command_error)?;
    config_env.reset_workspace_path(loader.workspace_root());
    config_env
        .reload_workspace_config(ui, &mut raw_config)
        .map_err(&map_command_error)?;
    let mut config = config_env
        .resolve_config(&raw_config)
        .map_err(|error| map_error(error.to_string()))?;
    jj_lib::config::migrate(&mut config, &default_config_migrations())
        .map_err(|error| map_error(error.to_string()))?;
    Ok(config)
}

pub(super) fn jx_default_config_layers() -> Vec<ConfigLayer> {
    let mut layers = default_config_layers();
    layers.push(
        ConfigLayer::parse(
            ConfigSource::Default,
            r#"
[templates]
log = "jx_builtin_log_compact"

[template-aliases]
jx_builtin_log_compact = "jx_builtin_log_compact(self)"
'jx_builtin_log_compact(commit)' = '''
if(commit.root(),
  format_jx_root_commit(commit),
  label(
    separate(" ",
      if(commit.current_working_copy(), "working_copy"),
      if(commit.immutable(), "immutable", "mutable"),
      if(commit.conflict(), "conflicted"),
    ),
    concat(
      format_jx_short_commit_header(commit) ++ "\n",
      separate(" ",
        if(commit.empty(), empty_commit_marker),
        if(commit.description(),
          commit.description().first_line(),
          label(if(commit.empty(), "empty"), description_placeholder),
        ),
      ) ++ "\n",
    ),
  )
)
'''
'format_jx_short_commit_header(commit)' = '''
separate(" ",
  format_short_change_id_with_change_offset(commit),
  format_short_signature(commit.author()),
  format_timestamp(commit_timestamp(commit)),
  commit.bookmarks(),
  commit.tags(),
  commit.working_copies(),
  format_commit_labels(commit),
  if(config("ui.show-cryptographic-signatures").as_boolean(),
    format_short_cryptographic_signature(commit.signature())
  ),
)
'''
'format_jx_root_commit(commit)' = '''
label("root", "root()") ++ "\n"
'''

[colors]
link = { underline = true }
"#,
        )
        .expect("jx default log template config is valid"),
    );
    layers
}

pub(super) fn render_current_workspace_log(
    workspace: &Workspace,
    repo: &ReadonlyRepo,
    current_dir: &Path,
    annotations: &[LogBookmarkAnnotation],
) -> Result<String, JjError> {
    // Reuse jj-cli's graph and template machinery so `jx` keeps user aliases and
    // graph behavior while owning a compact default log template.
    let settings = workspace.settings();
    let ui = Ui::with_config(settings.config()).map_err(log_command_error)?;
    let fileset_aliases_map =
        load_fileset_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let revset_aliases_map =
        load_revset_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let template_aliases_map =
        load_template_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let revset_extensions = Arc::new(RevsetExtensions::default());
    let path_converter = RepoPathUiConverter::Fs {
        cwd: current_dir.to_path_buf(),
        base: workspace.workspace_root().to_path_buf(),
    };
    let workspace_context = RevsetWorkspaceContext {
        path_converter: &path_converter,
        workspace_name: workspace.workspace_name(),
    };
    let revset_context = revset_parse_context(
        settings,
        repo,
        &fileset_aliases_map,
        &revset_aliases_map,
        &revset_extensions,
        Some(workspace_context),
    )?;
    let id_prefix_context =
        log_id_prefix_context(settings, &ui, &revset_context, revset_extensions.clone())?;
    let revset = log_revset(
        settings,
        &ui,
        repo,
        &revset_context,
        &id_prefix_context,
        &revset_extensions,
    )?;
    let prioritize_revset = graph_prioritize_revset(
        settings,
        &ui,
        repo,
        &revset_context,
        &id_prefix_context,
        &revset_extensions,
    )?;
    let immutable_expression = immutable_expression(&ui, &revset_context)?;
    let conflict_marker_style = settings
        .get("ui.conflict-marker-style")
        .map_err(log_error)?;
    let language = CommitTemplateLanguage::new(
        repo,
        &path_converter,
        workspace.workspace_name(),
        revset_context.clone(),
        &id_prefix_context,
        immutable_expression,
        conflict_marker_style,
        &[] as &[Box<dyn CommitTemplateLanguageExtension>],
    );
    let template = parse_log_template(
        &ui,
        &language,
        &template_aliases_map,
        &settings.get_string("templates.log").map_err(log_error)?,
    )?
    .labeled(["log", "commit"]);
    let node_template = parse_log_template(
        &ui,
        &language,
        &template_aliases_map,
        &settings
            .get_string("templates.log_node")
            .map_err(log_error)?,
    )?
    .labeled(["log", "commit", "node"]);

    render_log_graph(
        &ui,
        settings,
        repo,
        revset,
        prioritize_revset,
        LogGraphTemplates {
            commit: template,
            node: node_template,
        },
        annotations,
    )
}

pub(super) fn render_commit_ids_log(
    workspace: &Workspace,
    repo: &ReadonlyRepo,
    current_dir: &Path,
    commit_ids: Vec<CommitId>,
) -> Result<String, JjError> {
    let settings = workspace.settings();
    let ui = Ui::with_config(settings.config()).map_err(log_command_error)?;
    let fileset_aliases_map =
        load_fileset_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let revset_aliases_map =
        load_revset_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let template_aliases_map =
        load_template_aliases(&ui, settings.config()).map_err(log_command_error)?;
    let revset_extensions = Arc::new(RevsetExtensions::default());
    let path_converter = RepoPathUiConverter::Fs {
        cwd: current_dir.to_path_buf(),
        base: workspace.workspace_root().to_path_buf(),
    };
    let workspace_context = RevsetWorkspaceContext {
        path_converter: &path_converter,
        workspace_name: workspace.workspace_name(),
    };
    let revset_context = revset_parse_context(
        settings,
        repo,
        &fileset_aliases_map,
        &revset_aliases_map,
        &revset_extensions,
        Some(workspace_context),
    )?;
    let id_prefix_context =
        log_id_prefix_context(settings, &ui, &revset_context, revset_extensions.clone())?;
    let revset = ResolvedRevsetExpression::commits(commit_ids)
        .evaluate(repo)
        .map_err(log_error)?;
    let prioritize_revset = graph_prioritize_revset(
        settings,
        &ui,
        repo,
        &revset_context,
        &id_prefix_context,
        &revset_extensions,
    )?;
    let immutable_expression = immutable_expression(&ui, &revset_context)?;
    let conflict_marker_style = settings
        .get("ui.conflict-marker-style")
        .map_err(log_error)?;
    let language = CommitTemplateLanguage::new(
        repo,
        &path_converter,
        workspace.workspace_name(),
        revset_context.clone(),
        &id_prefix_context,
        immutable_expression,
        conflict_marker_style,
        &[] as &[Box<dyn CommitTemplateLanguageExtension>],
    );
    let template = parse_log_template(
        &ui,
        &language,
        &template_aliases_map,
        &settings.get_string("templates.log").map_err(log_error)?,
    )?
    .labeled(["log", "commit"]);
    let node_template = parse_log_template(
        &ui,
        &language,
        &template_aliases_map,
        &settings
            .get_string("templates.log_node")
            .map_err(log_error)?,
    )?
    .labeled(["log", "commit", "node"]);

    render_log_graph(
        &ui,
        settings,
        repo,
        revset,
        prioritize_revset,
        LogGraphTemplates {
            commit: template,
            node: node_template,
        },
        &[],
    )
}

pub(super) fn revset_parse_context<'a>(
    settings: &'a UserSettings,
    repo: &'a ReadonlyRepo,
    fileset_aliases_map: &'a jj_lib::fileset::FilesetAliasesMap,
    revset_aliases_map: &'a jj_lib::revset::RevsetAliasesMap,
    revset_extensions: &'a RevsetExtensions,
    workspace: Option<RevsetWorkspaceContext<'a>>,
) -> Result<RevsetParseContext<'a>, JjError> {
    let now = if let Some(timestamp) = settings.commit_timestamp() {
        Local
            .timestamp_millis_opt(timestamp.timestamp.0)
            .single()
            .ok_or_else(|| JjError::Log {
                message: "Configured commit timestamp is outside the supported date range"
                    .to_owned(),
            })?
    } else {
        Local::now()
    };

    Ok(RevsetParseContext {
        aliases_map: revset_aliases_map,
        local_variables: HashMap::new(),
        user_email: settings.user_email(),
        date_pattern_context: now.into(),
        default_ignored_remote: default_ignored_remote_name(repo.store()),
        fileset_aliases_map,
        use_glob_by_default: settings
            .get("ui.revsets-use-glob-by-default")
            .map_err(log_error)?,
        extensions: revset_extensions,
        workspace,
    })
}

pub(super) fn log_id_prefix_context(
    settings: &UserSettings,
    ui: &Ui,
    revset_context: &RevsetParseContext<'_>,
    revset_extensions: Arc<RevsetExtensions>,
) -> Result<IdPrefixContext, JjError> {
    let revset_string = settings
        .get_string("revsets.short-prefixes")
        .optional()
        .map_err(log_error)?
        .map_or_else(|| settings.get_string("revsets.log"), Ok)
        .map_err(log_error)?;
    if revset_string.is_empty() {
        return Ok(IdPrefixContext::new(revset_extensions));
    }

    let mut diagnostics = RevsetDiagnostics::new();
    let expression =
        revset::parse(&mut diagnostics, &revset_string, revset_context).map_err(log_error)?;
    print_parse_diagnostics(ui, "In `revsets.short-prefixes`", &diagnostics).map_err(log_error)?;
    Ok(IdPrefixContext::new(revset_extensions).disambiguate_within(expression))
}

pub(super) fn log_revset<'repo>(
    settings: &UserSettings,
    ui: &Ui,
    repo: &'repo ReadonlyRepo,
    revset_context: &RevsetParseContext<'_>,
    id_prefix_context: &'repo IdPrefixContext,
    revset_extensions: &Arc<RevsetExtensions>,
) -> Result<Box<dyn jj_lib::revset::Revset + 'repo>, JjError> {
    let mut diagnostics = RevsetDiagnostics::new();
    let expression = revset::parse(
        &mut diagnostics,
        &settings.get_string("revsets.log").map_err(log_error)?,
        revset_context,
    )
    .map_err(log_error)?;
    print_parse_diagnostics(ui, "In `revsets.log`", &diagnostics).map_err(log_error)?;
    // Keep the configured jj log behavior, but scope it to the active workspace
    // so sibling workspace heads do not appear in the default `jx` view.
    let current_workspace = RevsetExpression::working_copy(
        revset_context
            .workspace
            .expect("workspace context is present")
            .workspace_name
            .to_owned(),
    )
    .ancestors();
    let expression = expression.intersection(&current_workspace);

    RevsetExpressionEvaluator::new(
        repo,
        revset_extensions.clone(),
        id_prefix_context,
        expression,
    )
    .evaluate()
    .map_err(log_error)
}

pub(super) fn graph_prioritize_revset<'repo>(
    settings: &UserSettings,
    ui: &Ui,
    repo: &'repo ReadonlyRepo,
    revset_context: &RevsetParseContext<'_>,
    id_prefix_context: &'repo IdPrefixContext,
    revset_extensions: &Arc<RevsetExtensions>,
) -> Result<RevsetExpressionEvaluator<'repo>, JjError> {
    let mut diagnostics = RevsetDiagnostics::new();
    let expression = revset::parse(
        &mut diagnostics,
        &settings
            .get_string("revsets.log-graph-prioritize")
            .map_err(log_error)?,
        revset_context,
    )
    .map_err(log_error)?;
    print_parse_diagnostics(ui, "In `revsets.log-graph-prioritize`", &diagnostics)
        .map_err(log_error)?;

    Ok(RevsetExpressionEvaluator::new(
        repo,
        revset_extensions.clone(),
        id_prefix_context,
        expression,
    ))
}

pub(super) fn immutable_expression(
    ui: &Ui,
    revset_context: &RevsetParseContext<'_>,
) -> Result<Arc<jj_lib::revset::UserRevsetExpression>, JjError> {
    let mut diagnostics = RevsetDiagnostics::new();
    let expression =
        parse_immutable_heads_expression(&mut diagnostics, revset_context).map_err(log_error)?;
    print_parse_diagnostics(ui, "In `revset-aliases.immutable_heads()`", &diagnostics)
        .map_err(log_error)?;

    Ok(expression.ancestors())
}

pub(super) fn parse_log_template<'repo, C: Clone + 'repo>(
    ui: &Ui,
    language: &CommitTemplateLanguage<'repo>,
    aliases: &jj_cli::template_parser::TemplateAliasesMap,
    template_text: &str,
) -> Result<TemplateRenderer<'repo, C>, JjError>
where
    jj_cli::commit_templater::CommitTemplatePropertyKind<'repo>:
        jj_cli::templater::WrapTemplateProperty<'repo, C>,
{
    let mut diagnostics = TemplateDiagnostics::new();
    let template = template_builder::parse(language, &mut diagnostics, template_text, aliases)
        .map_err(log_error)?;
    print_parse_diagnostics(ui, "In template expression", &diagnostics).map_err(log_error)?;
    Ok(template)
}

pub(super) struct LogGraphTemplates<'repo> {
    commit: TemplateRenderer<'repo, Commit>,
    node: TemplateRenderer<'repo, Option<Commit>>,
}

pub(super) fn render_log_graph<'repo>(
    ui: &Ui,
    settings: &UserSettings,
    repo: &'repo ReadonlyRepo,
    revset: Box<dyn jj_lib::revset::Revset + 'repo>,
    prioritize_revset: RevsetExpressionEvaluator<'repo>,
    templates: LogGraphTemplates<'repo>,
    annotations: &[LogBookmarkAnnotation],
) -> Result<String, JjError> {
    let graph_style = GraphStyle::from_settings(settings).map_err(log_error)?;
    let use_elided_nodes = settings
        .get_bool("ui.log-synthetic-elided-nodes")
        .map_err(log_error)?;
    let with_content_format = LogContentFormat::new(ui, settings).map_err(log_error)?;
    let annotations_by_bookmark = log_annotations_by_bookmark(annotations);
    let store = repo.store();
    let mut output = Vec::new();

    {
        let mut formatter = ui.new_formatter(&mut output);
        let mut raw_output = formatter.raw().map_err(log_error)?;
        let mut graph = get_graphlog(graph_style, raw_output.as_mut());
        let mut forward_iter = TopoGroupedGraph::new(revset.stream_graph(), |id| id);
        let has_commit = revset.containing_fn();
        let mut prio_stream = prioritize_revset
            .evaluate_to_commit_ids()
            .map_err(log_error)?;
        while let Some(prio) = pollster::block_on(prio_stream.try_next()).map_err(log_error)? {
            if has_commit(&prio).map_err(log_error)? {
                forward_iter.prioritize_branch(prio);
            }
        }

        let forward_stream = forward_iter.stream();
        futures::pin_mut!(forward_stream);
        while let Some((commit_id, edges)) =
            pollster::block_on(forward_stream.try_next()).map_err(log_error)?
        {
            let mut graphlog_edges = vec![];
            let mut missing_edge_id = None;
            let mut elided_targets = vec![];
            for edge in edges {
                match edge.edge_type {
                    GraphEdgeType::Missing => {
                        missing_edge_id = Some(edge.target);
                    }
                    GraphEdgeType::Direct => {
                        graphlog_edges.push(GraphEdge::direct((edge.target, false)));
                    }
                    GraphEdgeType::Indirect => {
                        if use_elided_nodes {
                            elided_targets.push(edge.target.clone());
                            graphlog_edges.push(GraphEdge::direct((edge.target, true)));
                        } else {
                            graphlog_edges.push(GraphEdge::indirect((edge.target, false)));
                        }
                    }
                }
            }
            if let Some(missing_edge_id) = missing_edge_id {
                graphlog_edges.push(GraphEdge::missing((missing_edge_id, false)));
            }

            let mut buffer = vec![];
            let key = (commit_id, false);
            let commit = store.get_commit(&key.0).map_err(log_error)?;
            let within_graph = with_content_format.sub_width(graph.width(&key, &graphlog_edges));
            pollster::block_on(
                within_graph.write(ui.new_formatter(&mut buffer).as_mut(), async |formatter| {
                    templates.commit.format(&commit, formatter)
                }),
            )
            .map_err(log_error)?;
            let annotations = log_annotations_for_commit(repo, &commit, &annotations_by_bookmark);
            append_log_annotations(ui, &mut buffer, &annotations)?;

            let commit = Some(commit);
            let node_symbol = format_template(ui, &commit, &templates.node);
            graph
                .add_node(
                    &key,
                    &graphlog_edges,
                    &node_symbol,
                    &String::from_utf8_lossy(&buffer),
                )
                .map_err(log_error)?;

            for elided_target in elided_targets {
                let elided_key = (elided_target, true);
                let real_key = (elided_key.0.clone(), false);
                let edges = [GraphEdge::direct(real_key)];
                let mut buffer = vec![];
                let within_graph = with_content_format.sub_width(graph.width(&elided_key, &edges));
                pollster::block_on(
                    within_graph.write(ui.new_formatter(&mut buffer).as_mut(), async |formatter| {
                        writeln!(formatter.labeled("elided"), "(elided revisions)")
                    }),
                )
                .map_err(log_error)?;
                let node_symbol = format_template(ui, &None, &templates.node);
                graph
                    .add_node(
                        &elided_key,
                        &edges,
                        &node_symbol,
                        &String::from_utf8_lossy(&buffer),
                    )
                    .map_err(log_error)?;
            }
        }
    }

    String::from_utf8(output).map_err(log_error)
}

fn log_annotations_by_bookmark(
    annotations: &[LogBookmarkAnnotation],
) -> BTreeMap<&str, &LogBookmarkAnnotation> {
    annotations
        .iter()
        .map(|annotation| (annotation.bookmark.as_str(), annotation))
        .collect()
}

fn log_annotations_for_commit<'a>(
    repo: &ReadonlyRepo,
    commit: &Commit,
    annotations_by_bookmark: &'a BTreeMap<&str, &LogBookmarkAnnotation>,
) -> Vec<&'a LogBookmarkAnnotation> {
    repo.view()
        .local_bookmarks_for_commit(commit.id())
        .filter_map(|(bookmark, _)| annotations_by_bookmark.get(bookmark.as_str()).copied())
        .collect()
}

fn append_log_annotations(
    ui: &Ui,
    buffer: &mut Vec<u8>,
    annotations: &[&LogBookmarkAnnotation],
) -> Result<(), JjError> {
    if annotations.is_empty() || buffer.is_empty() {
        return Ok(());
    }

    let mut annotation_buffer = Vec::new();
    {
        let mut formatter = ui.new_formatter(&mut annotation_buffer);
        for annotation in annotations {
            write!(formatter, " ").map_err(log_error)?;
            write_log_annotation(formatter.as_mut(), annotation).map_err(log_error)?;
        }
    }

    let insertion = buffer
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(buffer.len());
    buffer.splice(insertion..insertion, annotation_buffer);
    Ok(())
}

fn write_log_annotation(
    formatter: &mut dyn Formatter,
    annotation: &LogBookmarkAnnotation,
) -> io::Result<()> {
    if let Some(url) = &annotation.url {
        write_osc8(formatter, url, &annotation.label)
    } else {
        write!(formatter, "{}", annotation.label)
    }
}

fn write_osc8(formatter: &mut dyn Formatter, url: &str, label: &str) -> io::Result<()> {
    write!(formatter.raw()?, "\x1b]8;;{url}\x1b\\")?;
    formatter.push_label("link");
    let result = write!(formatter, "{label}");
    formatter.pop_label();
    result?;
    write!(formatter.raw()?, "\x1b]8;;\x1b\\")
}
