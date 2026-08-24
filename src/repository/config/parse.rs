use super::*;

#[derive(Debug, Default)]
pub(super) struct WorkflowConfigLayer {
    pub(super) path: PathBuf,
    pub(super) layout: Option<LayoutConfigLayer>,
    pub(super) repo: Option<RepoConfig>,
    pub(super) diff: Option<DiffConfig>,
    pub(super) auth: Option<AuthConfig>,
    pub(super) shell: Option<ShellConfigLayer>,
    pub(super) ui: Option<UiConfigLayer>,
}

pub(super) fn parse_workflow_config_layer(
    path: PathBuf,
    contents: &str,
) -> Result<WorkflowConfigLayer, RepositoryError> {
    let file = config_file_label(&path);
    let table =
        toml::from_str::<toml::Table>(contents).map_err(|source| RepositoryError::ConfigParse {
            file: file.clone(),
            source,
        })?;

    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "layout" | "repo" | "diff" | "auth" | "shell" | "ui"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file,
                key: key.clone(),
            });
        }
    }

    let layout = table
        .get("layout")
        .map(|value| parse_layout_config(&file, value))
        .transpose()?;
    let repo = table
        .get("repo")
        .map(|value| parse_repo_config(&file, value))
        .transpose()?;
    let diff = table
        .get("diff")
        .map(|value| parse_diff_config(&file, value))
        .transpose()?;
    let auth = table
        .get("auth")
        .map(|value| parse_auth_config(&file, value))
        .transpose()?;
    let shell = table
        .get("shell")
        .map(|value| parse_shell_config(&file, value))
        .transpose()?;
    let ui = table
        .get("ui")
        .map(|value| parse_ui_config(&file, value))
        .transpose()?;

    Ok(WorkflowConfigLayer {
        path,
        layout,
        repo,
        diff,
        auth,
        shell,
        ui,
    })
}

fn parse_layout_config(
    file: &str,
    value: &toml::Value,
) -> Result<LayoutConfigLayer, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`layout` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "default_source" | "default_root" | "workspace_dir" | "sources" | "default" | "rules"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("layout.{key}"),
            });
        }
    }

    let default_source = table
        .get("default_source")
        .map(|value| parse_layout_name_value(file, "layout.default_source", value))
        .transpose()?;
    let default_root = table
        .get("default_root")
        .map(|value| parse_non_empty_string_value(file, "layout.default_root", value))
        .transpose()?;
    let workspace_dir = table
        .get("workspace_dir")
        .map(|value| parse_layout_path_segment_value(file, "layout.workspace_dir", value))
        .transpose()?;
    let sources = table
        .get("sources")
        .map(|value| parse_layout_sources(file, value))
        .transpose()?
        .unwrap_or_default();
    let default_path = table
        .get("default")
        .map(|value| parse_layout_default(file, value))
        .transpose()?;
    let rules = table
        .get("rules")
        .map(|value| parse_layout_rules(file, value))
        .transpose()?
        .unwrap_or_default();

    Ok(LayoutConfigLayer {
        default_source,
        default_root,
        workspace_dir,
        sources,
        default_path,
        rules,
    })
}

fn parse_layout_sources(
    file: &str,
    value: &toml::Value,
) -> Result<Vec<LayoutSourceConfig>, RepositoryError> {
    let Some(sources) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`layout.sources` must be an array of tables".to_owned(),
        });
    };

    sources
        .iter()
        .enumerate()
        .map(|(index, value)| parse_layout_source(file, index, value))
        .collect()
}

fn parse_layout_source(
    file: &str,
    index: usize,
    value: &toml::Value,
) -> Result<LayoutSourceConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`layout.sources[{index}]` must be a table"),
        });
    };
    let key_prefix = format!("layout.sources[{index}]");

    for key in table.keys() {
        if !matches!(key.as_str(), "name" | "provider" | "host" | "clone_url") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key_prefix}.{key}"),
            });
        }
    }

    let name = parse_layout_name_value(
        file,
        &format!("{key_prefix}.name"),
        required_value(file, table, &format!("{key_prefix}.name"))?,
    )?;
    let provider = parse_layout_provider(
        file,
        &format!("{key_prefix}.provider"),
        required_value(file, table, &format!("{key_prefix}.provider"))?,
    )?;
    let host = normalize_host(&parse_non_empty_string_value(
        file,
        &format!("{key_prefix}.host"),
        required_value(file, table, &format!("{key_prefix}.host"))?,
    )?)?;
    let clone_url = parse_clone_url_format(
        file,
        &format!("{key_prefix}.clone_url"),
        required_value(file, table, &format!("{key_prefix}.clone_url"))?,
    )?;

    Ok(LayoutSourceConfig {
        name,
        provider,
        host,
        clone_url,
    })
}

fn parse_layout_default(file: &str, value: &toml::Value) -> Result<String, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`layout.default` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if key != "path" {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("layout.default.{key}"),
            });
        }
    }

    required_non_empty_string(file, table, "layout.default.path")
}

fn parse_layout_rules(
    file: &str,
    value: &toml::Value,
) -> Result<Vec<LayoutRuleConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`layout.rules` must be an array of tables".to_owned(),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_layout_rule(file, index, value))
        .collect()
}

fn parse_layout_rule(
    file: &str,
    index: usize,
    value: &toml::Value,
) -> Result<LayoutRuleConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`layout.rules[{index}]` must be a table"),
        });
    };
    let key_prefix = format!("layout.rules[{index}]");

    for key in table.keys() {
        if !matches!(key.as_str(), "source" | "owner" | "repo" | "root" | "path") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key_prefix}.{key}"),
            });
        }
    }

    let source = parse_layout_name_value(
        file,
        &format!("{key_prefix}.source"),
        required_value(file, table, &format!("{key_prefix}.source"))?,
    )?;
    let owner = table
        .get("owner")
        .map(|value| parse_repo_component_value(file, &format!("{key_prefix}.owner"), value))
        .transpose()?;
    let repo = table
        .get("repo")
        .map(|value| parse_repo_component_value(file, &format!("{key_prefix}.repo"), value))
        .transpose()?;
    let root = table
        .get("root")
        .map(|value| parse_non_empty_string_value(file, &format!("{key_prefix}.root"), value))
        .transpose()?;
    let path = table
        .get("path")
        .map(|value| parse_non_empty_string_value(file, &format!("{key_prefix}.path"), value))
        .transpose()?;

    Ok(LayoutRuleConfig {
        source,
        owner,
        repo,
        root,
        path,
    })
}

fn parse_layout_provider(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<LayoutProvider, RepositoryError> {
    let provider = parse_non_empty_string_value(file, key, value)?;
    match provider.as_str() {
        "github" => Ok(LayoutProvider::GitHub),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be `github`"),
        }),
    }
}

fn parse_clone_url_format(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<CloneUrlFormat, RepositoryError> {
    let format = parse_non_empty_string_value(file, key, value)?;
    match format.as_str() {
        "ssh" => Ok(CloneUrlFormat::Ssh),
        "https" => Ok(CloneUrlFormat::Https),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be `ssh` or `https`"),
        }),
    }
}

fn parse_layout_name_value(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<String, RepositoryError> {
    let name = parse_non_empty_string_value(file, key, value)?;
    if is_valid_layout_name(&name) {
        Ok(name)
    } else {
        Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must contain only letters, numbers, `_`, or `-`"),
        })
    }
}

fn parse_layout_path_segment_value(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<String, RepositoryError> {
    let segment = parse_non_empty_string_value(file, key, value)?;
    validate_single_path_segment(key, &segment)?;
    Ok(segment)
}

fn parse_repo_component_value(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<String, RepositoryError> {
    let component = parse_non_empty_string_value(file, key, value)?;
    normalize_repo_component(&component, key).map_err(|error| match error {
        RepositoryError::InvalidCloneRepository { message, .. } => RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` is invalid: {message}"),
        },
        other => other,
    })
}

fn parse_diff_config(file: &str, value: &toml::Value) -> Result<DiffConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`diff` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(key.as_str(), "default_tool" | "tools") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("diff.{key}"),
            });
        }
    }

    let default_tool = table
        .get("default_tool")
        .map(|value| parse_non_empty_string_value(file, "diff.default_tool", value))
        .transpose()?;
    let tools = table
        .get("tools")
        .map(|value| parse_diff_tools(file, value))
        .transpose()?
        .unwrap_or_default();

    Ok(DiffConfig {
        default_tool,
        tools,
    })
}

fn parse_diff_tools(
    file: &str,
    value: &toml::Value,
) -> Result<BTreeMap<String, DiffToolConfig>, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`diff.tools` must be a table".to_owned(),
        });
    };

    table
        .iter()
        .map(|(name, value)| {
            if name.trim().is_empty() {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: "`diff.tools` must not contain empty tool names".to_owned(),
                });
            }
            Ok((name.clone(), parse_diff_tool(file, name, value)?))
        })
        .collect()
}

fn parse_diff_tool(
    file: &str,
    name: &str,
    value: &toml::Value,
) -> Result<DiffToolConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`diff.tools.{name}` must be a table"),
        });
    };
    let key_prefix = format!("diff.tools.{name}");
    let mode = required_non_empty_string(file, table, &format!("{key_prefix}.mode"))?;

    match mode.as_str() {
        "external" => parse_external_diff_tool(file, &key_prefix, table),
        "pipe" => parse_pipe_diff_tool(file, &key_prefix, table),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key_prefix}.mode` must be `external` or `pipe`"),
        }),
    }
}

fn parse_external_diff_tool(
    file: &str,
    key_prefix: &str,
    table: &toml::Table,
) -> Result<DiffToolConfig, RepositoryError> {
    for key in table.keys() {
        if !matches!(key.as_str(), "mode" | "command" | "args") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key_prefix}.{key}"),
            });
        }
    }

    Ok(DiffToolConfig::External(ExternalDiffToolConfig {
        command: required_non_empty_string(file, table, &format!("{key_prefix}.command"))?,
        args: optional_string_array(file, table, &format!("{key_prefix}.args"))?
            .unwrap_or_default(),
    }))
}

fn parse_pipe_diff_tool(
    file: &str,
    key_prefix: &str,
    table: &toml::Table,
) -> Result<DiffToolConfig, RepositoryError> {
    for key in table.keys() {
        if !matches!(key.as_str(), "mode" | "producer_args" | "command" | "args") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key_prefix}.{key}"),
            });
        }
    }

    Ok(DiffToolConfig::Pipe(PipeDiffToolConfig {
        producer_args: optional_string_array(file, table, &format!("{key_prefix}.producer_args"))?
            .unwrap_or_default(),
        command: required_non_empty_string(file, table, &format!("{key_prefix}.command"))?,
        args: optional_string_array(file, table, &format!("{key_prefix}.args"))?
            .unwrap_or_default(),
    }))
}

fn parse_repo_config(file: &str, value: &toml::Value) -> Result<RepoConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`repo` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "advance_trunk"
                | "sync"
                | "event_handlers"
                | "work_items"
                | "work_item_handlers"
                | "pull_request_handlers"
                | "hooks"
                | "checks"
                | "reviewers"
                | "path_reviewers"
                | "reviewer_rules"
                | "workspace_shared_paths"
                | "stack_status"
                | "review"
                | "rules"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("repo.{key}"),
            });
        }
    }

    let base = parse_repo_policy(file, "repo", table)?;
    let rules = table
        .get("rules")
        .map(|value| parse_repo_rules(file, value))
        .transpose()?
        .unwrap_or_default();

    Ok(RepoConfig { base, rules })
}

fn parse_repo_rules(
    file: &str,
    value: &toml::Value,
) -> Result<Vec<RepoRuleConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`repo.rules` must be an array of tables".to_owned(),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_repo_rule(file, index, value))
        .collect()
}

fn parse_repo_rule(
    file: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoRuleConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`repo.rules[{index}]` must be a table"),
        });
    };

    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "repo"
                | "advance_trunk"
                | "sync"
                | "event_handlers"
                | "work_items"
                | "work_item_handlers"
                | "pull_request_handlers"
                | "hooks"
                | "checks"
                | "reviewers"
                | "path_reviewers"
                | "reviewer_rules"
                | "workspace_shared_paths"
                | "stack_status"
                | "review"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("repo.rules[{index}].{key}"),
            });
        }
    }

    let repo = required_non_empty_string(file, table, &format!("repo.rules[{index}].repo"))?;
    validate_repo_rule_pattern(file, &format!("repo.rules[{index}].repo"), &repo)?;
    let policy = parse_repo_policy(file, &format!("repo.rules[{index}]"), table)?;

    Ok(RepoRuleConfig { repo, policy })
}

fn parse_repo_policy(
    file: &str,
    key_prefix: &str,
    table: &toml::Table,
) -> Result<RepoPolicyConfig, RepositoryError> {
    let advance_trunk = table
        .get("advance_trunk")
        .map(|value| parse_bool_value(file, &format!("{key_prefix}.advance_trunk"), value))
        .transpose()?;
    let sync = table
        .get("sync")
        .map(|value| parse_sync_config(file, &format!("{key_prefix}.sync"), value))
        .transpose()?
        .unwrap_or_default();
    let event_handlers = table
        .get("event_handlers")
        .map(|value| parse_event_handlers(file, &format!("{key_prefix}.event_handlers"), value))
        .transpose()?
        .unwrap_or_default();
    let work_items = table
        .get("work_items")
        .map(|value| parse_work_items(file, &format!("{key_prefix}.work_items"), value))
        .transpose()?
        .unwrap_or_default();
    let work_item_handlers = table
        .get("work_item_handlers")
        .map(|value| {
            parse_work_item_handlers(file, &format!("{key_prefix}.work_item_handlers"), value)
        })
        .transpose()?
        .unwrap_or_default();
    let pull_request_handlers = table
        .get("pull_request_handlers")
        .map(|value| {
            parse_pull_request_handlers(file, &format!("{key_prefix}.pull_request_handlers"), value)
        })
        .transpose()?
        .unwrap_or_default();
    let hooks = table
        .get("hooks")
        .map(|value| parse_repo_hooks(file, &format!("{key_prefix}.hooks"), value))
        .transpose()?
        .unwrap_or_default();
    let checks = table
        .get("checks")
        .map(|value| parse_repo_checks(file, &format!("{key_prefix}.checks"), value))
        .transpose()?
        .unwrap_or_default();
    let reviewers = table
        .get("reviewers")
        .map(|value| parse_reviewers(file, &format!("{key_prefix}.reviewers"), value))
        .transpose()?
        .unwrap_or_default();
    let reviewer_rules = parse_policy_path_reviewers(file, key_prefix, table)?;
    let workspace_shared_paths = table
        .get("workspace_shared_paths")
        .map(|value| {
            parse_workspace_shared_paths(
                file,
                &format!("{key_prefix}.workspace_shared_paths"),
                value,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let stack_status = table
        .get("stack_status")
        .map(|value| parse_stack_status_config(file, &format!("{key_prefix}.stack_status"), value))
        .transpose()?
        .unwrap_or_default();
    let review = table
        .get("review")
        .map(|value| parse_review_config(file, &format!("{key_prefix}.review"), value))
        .transpose()?
        .unwrap_or_default();

    Ok(RepoPolicyConfig {
        advance_trunk,
        sync,
        event_handlers,
        work_items,
        work_item_handlers,
        pull_request_handlers,
        hooks,
        checks,
        reviewers,
        reviewer_rules,
        workspace_shared_paths,
        stack_status,
        review,
    })
}

fn parse_sync_config(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<RepoSyncConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(
            name.as_str(),
            "push_access" | "rebase_strategy" | "rebase_needed_labels"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}.{name}"),
            });
        }
    }

    let push_access = table
        .get("push_access")
        .map(|value| parse_bool_value(file, &format!("{key}.push_access"), value))
        .transpose()?;
    let rebase_strategy = table
        .get("rebase_strategy")
        .map(|value| parse_sync_rebase_strategy(file, &format!("{key}.rebase_strategy"), value))
        .transpose()?;
    let rebase_needed_labels = table
        .get("rebase_needed_labels")
        .map(|value| {
            parse_named_string_rules(
                file,
                &format!("{key}.rebase_needed_labels"),
                value,
                "label name",
            )
        })
        .transpose()?
        .unwrap_or_default();

    Ok(RepoSyncConfig {
        push_access,
        rebase_strategy,
        rebase_needed_labels,
    })
}

fn parse_sync_rebase_strategy(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<RepoSyncRebaseStrategy, RepositoryError> {
    match parse_non_empty_string_value(file, key, value)?.as_str() {
        "always" => Ok(RepoSyncRebaseStrategy::Always),
        "stack_green_pull_requests" => Ok(RepoSyncRebaseStrategy::StackGreenPullRequests),
        strategy => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!(
                "`{key}` strategy `{strategy}` is unsupported; expected `always` or `stack_green_pull_requests`"
            ),
        }),
    }
}

fn parse_stack_status_config(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<RepoStackStatusConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(
            name.as_str(),
            "review_gate_checks"
                | "auto_merge_prerequisite_checks"
                | "ignored_checks"
                | "ignored_labels"
                | "ignored_label_patterns"
                | "ignored_labels_when_merged"
                | "hidden_labels"
                | "auto_merge_labels"
                | "ignored_reviewers"
                | "title_rewrites"
                | "label_rewrites"
                | "review_wait_threshold"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}.{name}"),
            });
        }
    }

    let review_gate_checks = table
        .get("review_gate_checks")
        .map(|value| parse_review_gate_checks(file, &format!("{key}.review_gate_checks"), value))
        .transpose()?
        .unwrap_or_default();
    let auto_merge_prerequisite_checks = table
        .get("auto_merge_prerequisite_checks")
        .map(|value| {
            parse_auto_merge_prerequisite_checks(
                file,
                &format!("{key}.auto_merge_prerequisite_checks"),
                value,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let ignored_checks = table
        .get("ignored_checks")
        .map(|value| parse_ignored_checks(file, &format!("{key}.ignored_checks"), value))
        .transpose()?
        .unwrap_or_default();
    let ignored_labels = table
        .get("ignored_labels")
        .map(|value| parse_ignored_labels(file, &format!("{key}.ignored_labels"), value))
        .transpose()?
        .unwrap_or_default();
    let ignored_label_patterns = table
        .get("ignored_label_patterns")
        .map(|value| {
            parse_ignored_label_patterns(file, &format!("{key}.ignored_label_patterns"), value)
        })
        .transpose()?
        .unwrap_or_default();
    let ignored_labels_when_merged = table
        .get("ignored_labels_when_merged")
        .map(|value| {
            parse_ignored_labels(file, &format!("{key}.ignored_labels_when_merged"), value)
        })
        .transpose()?
        .unwrap_or_default();
    let hidden_labels = table
        .get("hidden_labels")
        .map(|value| parse_hidden_labels(file, &format!("{key}.hidden_labels"), value))
        .transpose()?
        .unwrap_or_default();
    let auto_merge_labels = table
        .get("auto_merge_labels")
        .map(|value| parse_auto_merge_labels(file, &format!("{key}.auto_merge_labels"), value))
        .transpose()?
        .unwrap_or_default();
    let ignored_reviewers = table
        .get("ignored_reviewers")
        .map(|value| parse_ignored_reviewers(file, &format!("{key}.ignored_reviewers"), value))
        .transpose()?
        .unwrap_or_default();
    let title_rewrites = table
        .get("title_rewrites")
        .map(|value| parse_title_rewrites(file, &format!("{key}.title_rewrites"), value))
        .transpose()?
        .unwrap_or_default();
    let label_rewrites = table
        .get("label_rewrites")
        .map(|value| parse_label_rewrites(file, &format!("{key}.label_rewrites"), value))
        .transpose()?
        .unwrap_or_default();
    let review_wait_threshold_seconds = table
        .get("review_wait_threshold")
        .map(|value| {
            parse_review_wait_threshold(file, &format!("{key}.review_wait_threshold"), value)
        })
        .transpose()?;

    Ok(RepoStackStatusConfig {
        review_gate_checks,
        auto_merge_prerequisite_checks,
        ignored_checks,
        ignored_labels,
        ignored_label_patterns,
        ignored_labels_when_merged,
        hidden_labels,
        auto_merge_labels,
        ignored_reviewers,
        title_rewrites,
        label_rewrites,
        review_wait_threshold_seconds,
    })
}

fn parse_review_config(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<RepoReviewConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(
            name.as_str(),
            "ignored_labels"
                | "ignored_label_patterns"
                | "hidden_labels"
                | "ignored_author_response_comments"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}.{name}"),
            });
        }
    }

    let ignored_labels = table
        .get("ignored_labels")
        .map(|value| parse_ignored_labels(file, &format!("{key}.ignored_labels"), value))
        .transpose()?
        .unwrap_or_default();
    let ignored_label_patterns = table
        .get("ignored_label_patterns")
        .map(|value| {
            parse_ignored_label_patterns(file, &format!("{key}.ignored_label_patterns"), value)
        })
        .transpose()?
        .unwrap_or_default();
    let hidden_labels = table
        .get("hidden_labels")
        .map(|value| parse_hidden_labels(file, &format!("{key}.hidden_labels"), value))
        .transpose()?
        .unwrap_or_default();
    let ignored_author_response_comments = table
        .get("ignored_author_response_comments")
        .map(|value| {
            parse_ignored_author_response_comments(
                file,
                &format!("{key}.ignored_author_response_comments"),
                value,
            )
        })
        .transpose()?
        .unwrap_or_default();

    Ok(RepoReviewConfig {
        ignored_labels,
        ignored_label_patterns,
        hidden_labels,
        ignored_author_response_comments,
    })
}

fn parse_review_wait_threshold(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<u64, RepositoryError> {
    let raw = parse_non_empty_string_value(file, key, value)?;
    let Some(unit) = raw.chars().last() else {
        return Err(invalid_review_wait_threshold(file, key));
    };
    let amount = &raw[..raw.len() - unit.len_utf8()];
    if amount.is_empty() {
        return Err(invalid_review_wait_threshold(file, key));
    }
    let multiplier = match unit {
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return Err(invalid_review_wait_threshold(file, key)),
    };
    let amount = amount
        .parse::<u64>()
        .map_err(|_| invalid_review_wait_threshold(file, key))?;
    if amount == 0 {
        return Err(invalid_review_wait_threshold(file, key));
    }
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| invalid_review_wait_threshold(file, key))
}

fn invalid_review_wait_threshold(file: &str, key: &str) -> RepositoryError {
    RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message: format!("`{key}` must be a duration such as `30m`, `4h`, or `2d`"),
    }
}

fn parse_review_gate_checks(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<ReviewGateCheckConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "check-name regex").map(|rules| {
        rules
            .into_iter()
            .map(|name| ReviewGateCheckConfig { name })
            .collect()
    })
}

fn parse_auto_merge_prerequisite_checks(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<AutoMergePrerequisiteCheckConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "check-name regex").map(|rules| {
        rules
            .into_iter()
            .map(|name| AutoMergePrerequisiteCheckConfig { name })
            .collect()
    })
}

fn parse_title_rewrites(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<TitleRewriteConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_title_rewrite(file, key, index, value))
        .collect()
}

fn parse_title_rewrite(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<TitleRewriteConfig, RepositoryError> {
    let rewrite = parse_rewrite_rule(file, key, index, value)?;
    Ok(TitleRewriteConfig {
        pattern: rewrite.pattern,
        replace: rewrite.replace,
    })
}

fn parse_label_rewrites(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<LabelRewriteConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_label_rewrite(file, key, index, value))
        .collect()
}

fn parse_label_rewrite(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<LabelRewriteConfig, RepositoryError> {
    let rewrite = parse_rewrite_rule(file, key, index, value)?;
    Ok(LabelRewriteConfig {
        pattern: rewrite.pattern,
        replace: rewrite.replace,
    })
}

struct ParsedRewriteRule {
    pattern: String,
    replace: String,
}

fn parse_rewrite_rule(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<ParsedRewriteRule, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "pattern" | "replace") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let pattern_key = format!("{key}[{index}].pattern");
    let pattern = required_non_empty_string(file, table, &pattern_key)?;
    regex::Regex::new(&pattern).map_err(|source| RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message: format!("`{pattern_key}` must be a valid regex: {source}"),
    })?;
    let replace = required_non_empty_string(file, table, &format!("{key}[{index}].replace"))?;

    Ok(ParsedRewriteRule { pattern, replace })
}

fn parse_ignored_checks(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<IgnoredCheckConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "check-name regex").map(|rules| {
        rules
            .into_iter()
            .map(|name| IgnoredCheckConfig { name })
            .collect()
    })
}

fn parse_ignored_labels(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<IgnoredLabelConfig>, RepositoryError> {
    parse_named_string_rules(file, key, value, "label name").map(|rules| {
        rules
            .into_iter()
            .map(|name| IgnoredLabelConfig { name })
            .collect()
    })
}

fn parse_ignored_label_patterns(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<IgnoredLabelPatternConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "label-name regex").map(|rules| {
        rules
            .into_iter()
            .map(|name| IgnoredLabelPatternConfig { name })
            .collect()
    })
}

fn parse_hidden_labels(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<HiddenLabelConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of label visibility rules"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_hidden_label(file, key, index, value))
        .collect()
}

fn parse_hidden_label(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<HiddenLabelConfig, RepositoryError> {
    let rule = parse_conditioned_label_rule(file, key, index, value, "hidden-label")?;
    Ok(HiddenLabelConfig {
        label: rule.label,
        when: rule.when,
    })
}

fn parse_auto_merge_labels(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<AutoMergeLabelConfig>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of auto-merge label rules"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_auto_merge_label(file, key, index, value))
        .collect()
}

fn parse_auto_merge_label(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<AutoMergeLabelConfig, RepositoryError> {
    let rule = parse_conditioned_label_rule(file, key, index, value, "auto-merge label")?;
    Ok(AutoMergeLabelConfig {
        label: rule.label,
        when: rule.when,
    })
}

struct ConditionedLabelRule {
    label: String,
    when: Vec<HiddenLabelCondition>,
}

fn parse_conditioned_label_rule(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
    description: &str,
) -> Result<ConditionedLabelRule, RepositoryError> {
    if value.is_str() {
        let item_key = format!("{key}[{index}]");
        let label = parse_non_empty_string_value(file, &item_key, value)?;
        return Ok(ConditionedLabelRule {
            label,
            when: vec![HiddenLabelCondition::Always],
        });
    }

    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a {description} string or table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "label" | "when") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let label_key = format!("{key}[{index}].label");
    let label = required_non_empty_string(file, table, &label_key)?;
    let when = table
        .get("when")
        .map(|value| parse_hidden_label_conditions(file, &format!("{key}[{index}].when"), value))
        .transpose()?
        .unwrap_or_else(|| vec![HiddenLabelCondition::Always]);

    Ok(ConditionedLabelRule { label, when })
}

fn parse_hidden_label_conditions(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<HiddenLabelCondition>, RepositoryError> {
    let conditions = parse_string_array(file, key, value)?;
    if conditions.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must include at least one label visibility condition"),
        });
    }
    conditions
        .into_iter()
        .map(|condition| parse_hidden_label_condition(file, key, &condition))
        .collect()
}

fn parse_hidden_label_condition(
    file: &str,
    key: &str,
    value: &str,
) -> Result<HiddenLabelCondition, RepositoryError> {
    match value {
        "ALWAYS" => Ok(HiddenLabelCondition::Always),
        "DRAFT" => Ok(HiddenLabelCondition::Draft),
        "NOT_DRAFT" => Ok(HiddenLabelCondition::NotDraft),
        "OPEN" => Ok(HiddenLabelCondition::Open),
        "CLOSED" => Ok(HiddenLabelCondition::Closed),
        "MERGED" => Ok(HiddenLabelCondition::Merged),
        "NOT_MERGED" => Ok(HiddenLabelCondition::NotMerged),
        "TARGETS_DEFAULT_BRANCH" => Ok(HiddenLabelCondition::TargetsDefaultBranch),
        "TARGETS_NON_DEFAULT_BRANCH" => Ok(HiddenLabelCondition::TargetsNonDefaultBranch),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` contains unsupported label visibility condition `{value}`"),
        }),
    }
}

fn parse_ignored_author_response_comments(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<IgnoredAuthorResponseCommentConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "author-response comment regex").map(|rules| {
        rules
            .into_iter()
            .map(|pattern| IgnoredAuthorResponseCommentConfig { pattern })
            .collect()
    })
}

fn parse_ignored_reviewers(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<IgnoredReviewerConfig>, RepositoryError> {
    parse_named_regex_rules(file, key, value, "reviewer-name regex").map(|rules| {
        rules
            .into_iter()
            .map(|name| IgnoredReviewerConfig { name })
            .collect()
    })
}

fn parse_named_regex_rules(
    file: &str,
    key: &str,
    value: &toml::Value,
    description: &str,
) -> Result<Vec<String>, RepositoryError> {
    parse_named_rules(file, key, value, description, validate_named_regex_rule)
}

fn parse_named_string_rules(
    file: &str,
    key: &str,
    value: &toml::Value,
    description: &str,
) -> Result<Vec<String>, RepositoryError> {
    parse_named_rules(file, key, value, description, validate_named_string_rule)
}

fn parse_named_rules(
    file: &str,
    key: &str,
    value: &toml::Value,
    description: &str,
    validate: fn(&str, &str, String, &str) -> Result<String, RepositoryError>,
) -> Result<Vec<String>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of {description}s"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_named_rule(file, key, index, value, description, validate))
        .collect()
}

fn parse_named_rule(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
    description: &str,
    validate: fn(&str, &str, String, &str) -> Result<String, RepositoryError>,
) -> Result<String, RepositoryError> {
    if value.is_str() {
        let item_key = format!("{key}[{index}]");
        let name = parse_non_empty_string_value(file, &item_key, value)?;
        return validate(file, &item_key, name, description);
    }

    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a {description} string or table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "name") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let item_key = format!("{key}[{index}].name");
    let name = required_non_empty_string(file, table, &item_key)?;
    validate(file, &item_key, name, description)
}

fn validate_named_regex_rule(
    file: &str,
    key: &str,
    name: String,
    description: &str,
) -> Result<String, RepositoryError> {
    regex::Regex::new(&name).map_err(|source| RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message: format!("`{key}` must be a valid {description}: {source}"),
    })?;
    Ok(name)
}

fn validate_named_string_rule(
    _file: &str,
    _key: &str,
    name: String,
    _description: &str,
) -> Result<String, RepositoryError> {
    Ok(name)
}

fn parse_repo_checks(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<RepoCheckConfig>, RepositoryError> {
    let Some(checks) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    checks
        .iter()
        .enumerate()
        .map(|(index, value)| parse_repo_check(file, key, index, value))
        .collect()
}

fn parse_repo_check(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoCheckConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "id" | "before" | "paths" | "command") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let id = required_non_empty_string(file, table, &format!("{key}[{index}].id"))?;
    let before = parse_repo_check_triggers(
        file,
        &format!("{key}[{index}].before"),
        &required_string_array(file, table, &format!("{key}[{index}].before"))?,
    )?;
    let paths = required_string_array(file, table, &format!("{key}[{index}].paths"))?;
    if paths.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].paths` must not be empty"),
        });
    }
    for pattern in &paths {
        validate_glob_pattern(file, &format!("{key}[{index}].paths"), pattern)?;
    }
    let command = required_string_array(file, table, &format!("{key}[{index}].command"))?;
    if command.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].command` must not be empty"),
        });
    }

    Ok(RepoCheckConfig {
        id,
        before,
        paths,
        command,
    })
}

fn parse_repo_check_triggers(
    file: &str,
    key: &str,
    values: &[String],
) -> Result<Vec<RepoCheckTrigger>, RepositoryError> {
    if values.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must not be empty"),
        });
    }

    let mut triggers = Vec::new();
    for value in values {
        let trigger = match value.as_str() {
            "pull_request" => RepoCheckTrigger::PullRequest,
            "push" => RepoCheckTrigger::Push,
            "sync" => RepoCheckTrigger::Sync,
            _ => {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!(
                        "`{key}` contains unsupported trigger `{value}`; use `pull_request`, `push`, or `sync`"
                    ),
                });
            }
        };
        if !triggers.contains(&trigger) {
            triggers.push(trigger);
        }
    }

    Ok(triggers)
}

fn validate_glob_pattern(file: &str, key: &str, pattern: &str) -> Result<(), RepositoryError> {
    Glob::new(pattern)
        .map(drop)
        .map_err(|error| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` glob `{pattern}` is invalid: {error}"),
        })
}

fn parse_policy_path_reviewers(
    file: &str,
    key_prefix: &str,
    table: &toml::Table,
) -> Result<Vec<ReviewerPathRule>, RepositoryError> {
    match (table.get("path_reviewers"), table.get("reviewer_rules")) {
        (Some(value), None) => {
            parse_reviewer_path_rules(file, &format!("{key_prefix}.path_reviewers"), value)
        }
        (None, Some(value)) => {
            parse_reviewer_path_rules(file, &format!("{key_prefix}.reviewer_rules"), value)
        }
        (Some(_), Some(_)) => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!(
                "configure `{key_prefix}.path_reviewers` or legacy `{key_prefix}.reviewer_rules`, not both"
            ),
        }),
        (None, None) => Ok(Vec::new()),
    }
}

fn parse_work_items(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<RepoWorkItemsConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "apply_on_stack_status") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}.{name}"),
            });
        }
    }

    let apply_on_stack_status = table
        .get("apply_on_stack_status")
        .map(|value| parse_bool_value(file, &format!("{key}.apply_on_stack_status"), value))
        .transpose()?;

    Ok(RepoWorkItemsConfig {
        apply_on_stack_status,
    })
}

fn parse_work_item_handlers(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<RepoWorkItemHandlerConfig>, RepositoryError> {
    let Some(handlers) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    handlers
        .iter()
        .enumerate()
        .map(|(index, value)| parse_work_item_handler(file, key, index, value))
        .collect()
}

fn parse_work_item_handler(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoWorkItemHandlerConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "id" | "enabled" | "on" | "command") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let id = table
        .get("id")
        .map(|value| parse_non_empty_string_value(file, &format!("{key}[{index}].id"), value))
        .transpose()?;
    let enabled = table
        .get("enabled")
        .map(|value| parse_bool_value(file, &format!("{key}[{index}].enabled"), value))
        .transpose()?
        .unwrap_or(true);
    if !enabled {
        let Some(id) = id else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}[{index}].id` is required when disabling a handler"),
            });
        };
        return Ok(RepoWorkItemHandlerConfig::Disable { id });
    }

    let on = parse_work_item_event(file, key, index, required_value(file, table, "on")?)?;
    let command_key = format!("{key}[{index}].command");
    let command = parse_string_array(file, &command_key, required_value(file, table, "command")?)?;
    if command.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{command_key}` must not be empty"),
        });
    }

    Ok(RepoWorkItemHandlerConfig::Handler(RepoWorkItemHandler {
        id,
        on,
        command,
    }))
}

fn parse_work_item_event(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoWorkItemEvent, RepositoryError> {
    match parse_non_empty_string_value(file, &format!("{key}[{index}].on"), value)?.as_str() {
        "work_item.fixed" => Ok(RepoWorkItemEvent::Fixed),
        event => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("unsupported work item handler event `{event}`"),
        }),
    }
}

fn parse_pull_request_handlers(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<RepoPullRequestHandlerConfig>, RepositoryError> {
    let Some(handlers) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    handlers
        .iter()
        .enumerate()
        .map(|(index, value)| parse_pull_request_handler(file, key, index, value))
        .collect()
}

fn parse_pull_request_handler(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoPullRequestHandlerConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "id" | "enabled" | "on" | "command") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let id = table
        .get("id")
        .map(|value| parse_non_empty_string_value(file, &format!("{key}[{index}].id"), value))
        .transpose()?;
    let enabled = table
        .get("enabled")
        .map(|value| parse_bool_value(file, &format!("{key}[{index}].enabled"), value))
        .transpose()?
        .unwrap_or(true);
    if !enabled {
        let Some(id) = id else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}[{index}].id` is required when disabling a handler"),
            });
        };
        return Ok(RepoPullRequestHandlerConfig::Disable { id });
    }

    let on =
        parse_pull_request_handler_event(file, key, index, required_value(file, table, "on")?)?;
    let command_key = format!("{key}[{index}].command");
    let command = parse_string_array(file, &command_key, required_value(file, table, "command")?)?;
    if command.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{command_key}` must not be empty"),
        });
    }

    Ok(RepoPullRequestHandlerConfig::Handler(
        RepoPullRequestHandler { id, on, command },
    ))
}

fn parse_pull_request_handler_event(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoPullRequestEvent, RepositoryError> {
    match parse_non_empty_string_value(file, &format!("{key}[{index}].on"), value)?.as_str() {
        "pull_request.merged" => Ok(RepoPullRequestEvent::Merged),
        event => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("unsupported pull request handler event `{event}`"),
        }),
    }
}

fn parse_repo_hooks(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<RepoHookConfig>, RepositoryError> {
    let Some(hooks) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    hooks
        .iter()
        .enumerate()
        .map(|(index, value)| parse_repo_hook(file, key, index, value))
        .collect()
}

fn parse_repo_hook(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoHookConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "id" | "enabled" | "on" | "command") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let id = required_non_empty_string(file, table, &format!("{key}[{index}].id"))?;
    let enabled = table
        .get("enabled")
        .map(|value| parse_bool_value(file, &format!("{key}[{index}].enabled"), value))
        .transpose()?
        .unwrap_or(true);
    if !enabled {
        return Ok(RepoHookConfig::Disable { id });
    }

    let on = parse_repo_hook_event(file, key, index, required_value(file, table, "on")?)?;
    let command_key = format!("{key}[{index}].command");
    let command = parse_string_array(file, &command_key, required_value(file, table, "command")?)?;
    if command.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{command_key}` must not be empty"),
        });
    }

    Ok(RepoHookConfig::Hook(RepoHook { id, on, command }))
}

fn parse_repo_hook_event(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoHookEvent, RepositoryError> {
    match parse_non_empty_string_value(file, &format!("{key}[{index}].on"), value)?.as_str() {
        "workspace.delete.before" => Ok(RepoHookEvent::WorkspaceDeleteBefore),
        event => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("unsupported repo hook event `{event}`"),
        }),
    }
}

fn parse_event_handlers(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<RepoEventHandlerConfig>, RepositoryError> {
    let Some(handlers) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    handlers
        .iter()
        .enumerate()
        .map(|(index, value)| parse_event_handler(file, key, index, value))
        .collect()
}

fn parse_event_handler(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<RepoEventHandlerConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(
            name.as_str(),
            "id" | "enabled" | "on" | "when" | "run" | "labels"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let enabled = table
        .get("enabled")
        .map(|value| parse_bool_value(file, &format!("{key}[{index}].enabled"), value))
        .transpose()?
        .unwrap_or(true);
    let id = table
        .get("id")
        .map(|value| parse_non_empty_string_value(file, &format!("{key}[{index}].id"), value))
        .transpose()?;

    if !enabled {
        let Some(id) = id else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}[{index}].id` is required when `enabled = false`"),
            });
        };
        return Ok(RepoEventHandlerConfig::Disable { id });
    }

    let on = parse_event_handler_event(
        file,
        &format!("{key}[{index}].on"),
        &required_non_empty_string(file, table, &format!("{key}[{index}].on"))?,
    )?;
    let when = table
        .get("when")
        .map(|value| {
            parse_pull_request_event_query(
                file,
                &format!("{key}[{index}].when"),
                &parse_string_value(file, &format!("{key}[{index}].when"), value)?,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let run = parse_event_handler_run(
        file,
        key,
        index,
        table,
        on,
        &required_non_empty_string(file, table, &format!("{key}[{index}].run"))?,
    )?;

    Ok(RepoEventHandlerConfig::Handler(RepoEventHandler {
        id,
        on,
        when,
        run,
    }))
}

fn parse_event_handler_event(
    file: &str,
    key: &str,
    value: &str,
) -> Result<RepoEvent, RepositoryError> {
    match value {
        "pull_request.prepare" => Ok(RepoEvent::PullRequestPrepare),
        "pull_request.created" => Ok(RepoEvent::PullRequestCreated),
        "pull_request.updated" => Ok(RepoEvent::PullRequestUpdated),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!(
                "`{key}` must be one of `pull_request.prepare`, `pull_request.created`, or `pull_request.updated`"
            ),
        }),
    }
}

fn parse_event_handler_run(
    file: &str,
    key: &str,
    index: usize,
    table: &toml::Table,
    on: RepoEvent,
    value: &str,
) -> Result<RepoEventHandlerRun, RepositoryError> {
    match value {
        "add_labels" => {
            let labels = optional_string_array(file, table, &format!("{key}[{index}].labels"))?
                .unwrap_or_default();
            let labels = normalize_label_set(labels);
            if labels.is_empty() {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!("`{key}[{index}].labels` is required for `add_labels`"),
                });
            }
            Ok(RepoEventHandlerRun::AddLabels { labels })
        }
        "open_pull_request" => {
            reject_labels_for_non_label_handler(file, key, index, table)?;
            Ok(RepoEventHandlerRun::OpenPullRequest)
        }
        "prepend_task_id" => {
            reject_labels_for_non_label_handler(file, key, index, table)?;
            if on != RepoEvent::PullRequestPrepare {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!(
                        "`{key}[{index}].run = \"prepend_task_id\"` is only supported for `pull_request.prepare`"
                    ),
                });
            }
            Ok(RepoEventHandlerRun::PrependTaskId)
        }
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!(
                "`{key}[{index}].run` must be one of `add_labels`, `open_pull_request`, or `prepend_task_id`"
            ),
        }),
    }
}

fn reject_labels_for_non_label_handler(
    file: &str,
    key: &str,
    index: usize,
    table: &toml::Table,
) -> Result<(), RepositoryError> {
    if table.contains_key("labels") {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].labels` is only supported for `add_labels` handlers"),
        });
    }

    Ok(())
}

fn parse_pull_request_event_query(
    file: &str,
    key: &str,
    value: &str,
) -> Result<PullRequestEventQuery, RepositoryError> {
    let mut terms = Vec::new();
    for token in value.split_whitespace() {
        let (negated, predicate) = token
            .strip_prefix('-')
            .map_or((false, token), |predicate| (true, predicate));
        if predicate.is_empty() {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` contains an empty negated term"),
            });
        }

        let predicate = match predicate {
            "is:draft" => PullRequestEventPredicate::Draft,
            "is:ready" => PullRequestEventPredicate::Ready,
            "has:reviewers" => PullRequestEventPredicate::HasReviewers,
            "has:task" => PullRequestEventPredicate::HasTask,
            predicate if predicate.starts_with("label:") => {
                let label = predicate
                    .strip_prefix("label:")
                    .expect("label predicate prefix was checked");
                if label.is_empty() {
                    return Err(RepositoryError::InvalidConfig {
                        file: file.to_owned(),
                        message: format!("`{key}` contains an empty `label:` term"),
                    });
                }
                PullRequestEventPredicate::Label(label.to_owned())
            }
            _ => {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: format!("`{key}` contains unsupported term `{token}`"),
                });
            }
        };

        terms.push(PullRequestEventQueryTerm { predicate, negated });
    }

    Ok(PullRequestEventQuery { terms })
}

fn normalize_label_set(labels: Vec<String>) -> Vec<String> {
    normalize_string_set(labels)
}

fn normalize_string_set(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && seen.insert(value.to_owned()) {
            normalized.push(value.to_owned());
        }
    }
    normalized
}

fn parse_workspace_shared_paths(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<String>, RepositoryError> {
    let Some(paths) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of path strings"),
        });
    };

    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let Some(path) = path.as_str() else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` must contain only strings"),
            });
        };
        let path = normalize_workspace_shared_path(file, key, path)?;
        if seen.insert(path.clone()) {
            normalized.push(path);
        }
    }

    Ok(normalized)
}

fn validate_repo_rule_pattern(file: &str, key: &str, pattern: &str) -> Result<(), RepositoryError> {
    if !pattern.contains('/') {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must match `owner/repo` slugs"),
        });
    }

    Glob::new(pattern).map_err(|source| RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message: format!("`{key}` must be a valid repository glob: {source}"),
    })?;

    Ok(())
}

fn parse_reviewer_path_rules(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<ReviewerPathRule>, RepositoryError> {
    let Some(rules) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of tables"),
        });
    };

    rules
        .iter()
        .enumerate()
        .map(|(index, value)| parse_reviewer_path_rule(file, key, index, value))
        .collect()
}

fn parse_reviewer_path_rule(
    file: &str,
    key: &str,
    index: usize,
    value: &toml::Value,
) -> Result<ReviewerPathRule, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}]` must be a table"),
        });
    };

    for name in table.keys() {
        if !matches!(name.as_str(), "paths" | "reviewers") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("{key}[{index}].{name}"),
            });
        }
    }

    let paths = table
        .get("paths")
        .map(|value| parse_rule_paths(file, &format!("{key}[{index}].paths"), value))
        .transpose()?
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].paths` is required"),
        })?;
    let reviewers = table
        .get("reviewers")
        .map(|value| parse_reviewers(file, &format!("{key}[{index}].reviewers"), value))
        .transpose()?
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].reviewers` is required"),
        })?;
    if reviewers.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}[{index}].reviewers` must not be empty"),
        });
    }

    Ok(ReviewerPathRule { paths, reviewers })
}

fn parse_rule_paths(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<String>, RepositoryError> {
    let Some(paths) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of glob strings"),
        });
    };

    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let Some(path) = path.as_str() else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` must contain only strings"),
            });
        };
        let path = path.trim();
        if path.is_empty() {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` must not contain empty globs"),
            });
        }
        Glob::new(path).map_err(|source| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{path}` is not a valid reviewer path glob: {source}"),
        })?;
        if seen.insert(path.to_owned()) {
            normalized.push(path.to_owned());
        }
    }

    if normalized.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must not be empty"),
        });
    }

    Ok(normalized)
}

fn parse_shell_config(
    file: &str,
    value: &toml::Value,
) -> Result<ShellConfigLayer, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`shell` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(
            key.as_str(),
            "navigation"
                | "navigation_tab"
                | "title"
                | "slug_repositories"
                | "title_rewrites"
                | "zoxide"
        ) {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("shell.{key}"),
            });
        }
    }

    let navigation = table
        .get("navigation")
        .map(|value| {
            parse_string_value(file, "shell.navigation", value).map(|value| value.trim().to_owned())
        })
        .transpose()?;
    let navigation_tab = table
        .get("navigation_tab")
        .map(|value| {
            parse_string_value(file, "shell.navigation_tab", value)
                .map(|value| value.trim().to_owned())
        })
        .transpose()?;
    let title = table
        .get("title")
        .map(|value| parse_bool_value(file, "shell.title", value))
        .transpose()?;
    let slug_repositories =
        optional_string_array(file, table, "shell.slug_repositories")?.map(normalize_string_set);
    let title_rewrites = table
        .get("title_rewrites")
        .map(|value| parse_title_rewrites(file, "shell.title_rewrites", value))
        .transpose()?;
    let zoxide = table
        .get("zoxide")
        .map(|value| parse_shell_zoxide_mode(file, "shell.zoxide", value))
        .transpose()?;

    Ok(ShellConfigLayer {
        navigation,
        navigation_tab,
        title,
        slug_repositories,
        title_rewrites,
        zoxide,
    })
}

fn parse_shell_zoxide_mode(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<ShellZoxideMode, RepositoryError> {
    let mode = parse_non_empty_string_value(file, key, value)?;
    match mode.as_str() {
        "auto" => Ok(ShellZoxideMode::Auto),
        "never" => Ok(ShellZoxideMode::Never),
        "prefer" => Ok(ShellZoxideMode::Prefer),
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be `auto`, `never`, or `prefer`"),
        }),
    }
}

fn parse_ui_config(file: &str, value: &toml::Value) -> Result<UiConfigLayer, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`ui` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(key.as_str(), "default_command" | "default-command") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("ui.{key}"),
            });
        }
    }
    if table.contains_key("default_command") && table.contains_key("default-command") {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "configure only one of `ui.default_command` or `ui.default-command`"
                .to_owned(),
        });
    }

    let default_command = table
        .get("default_command")
        .or_else(|| table.get("default-command"))
        .map(|value| {
            let command = parse_string_or_array(file, "ui.default_command", value)?;
            if command.is_empty() {
                return Err(RepositoryError::InvalidConfig {
                    file: file.to_owned(),
                    message: "`ui.default_command` must name a subcommand".to_owned(),
                });
            }
            Ok(command)
        })
        .transpose()?;

    Ok(UiConfigLayer { default_command })
}

fn parse_auth_config(file: &str, value: &toml::Value) -> Result<AuthConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`auth` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if key != "keychain" {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("auth.{key}"),
            });
        }
    }

    let keychain = table
        .get("keychain")
        .map(|value| parse_keychain_config(file, value))
        .transpose()?;

    Ok(AuthConfig { keychain })
}

fn parse_keychain_config(
    file: &str,
    value: &toml::Value,
) -> Result<KeychainConfig, RepositoryError> {
    let Some(table) = value.as_table() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: "`auth.keychain` must be a table".to_owned(),
        });
    };

    for key in table.keys() {
        if !matches!(key.as_str(), "service" | "account") {
            return Err(RepositoryError::UnsupportedConfigKey {
                file: file.to_owned(),
                key: format!("auth.keychain.{key}"),
            });
        }
    }

    Ok(KeychainConfig {
        service: required_non_empty_string(file, table, "auth.keychain.service")?,
        account: required_non_empty_string(file, table, "auth.keychain.account")?,
    })
}

fn required_string_array(
    file: &str,
    table: &toml::Table,
    key: &str,
) -> Result<Vec<String>, RepositoryError> {
    let value = table.get(key.rsplit_once('.').map_or(key, |(_, key)| key));
    let Some(value) = value else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` is required"),
        });
    };

    parse_string_array(file, key, value)
}

fn required_non_empty_string(
    file: &str,
    table: &toml::Table,
    key: &str,
) -> Result<String, RepositoryError> {
    let value = table.get(key.rsplit_once('.').map_or(key, |(_, key)| key));
    let Some(value) = value else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` is required"),
        });
    };

    parse_non_empty_string_value(file, key, value)
}

fn parse_non_empty_string_value(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<String, RepositoryError> {
    let value = parse_string_value(file, key, value)?;
    let value = value.trim();

    if value.is_empty() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must not be empty"),
        });
    }

    Ok(value.to_owned())
}

fn parse_string_value(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<String, RepositoryError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a string"),
        })
}

fn parse_bool_value(file: &str, key: &str, value: &toml::Value) -> Result<bool, RepositoryError> {
    value
        .as_bool()
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a boolean"),
        })
}

fn optional_string_array(
    file: &str,
    table: &toml::Table,
    key: &str,
) -> Result<Option<Vec<String>>, RepositoryError> {
    table
        .get(key.rsplit_once('.').map_or(key, |(_, key)| key))
        .map(|value| parse_string_array(file, key, value))
        .transpose()
}

fn parse_string_or_array(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<String>, RepositoryError> {
    if value.is_str() {
        return parse_non_empty_string_value(file, key, value).map(|value| vec![value]);
    }
    if !value.is_array() {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be a string or array of strings"),
        });
    }

    parse_string_array(file, key, value)
}

fn parse_string_array(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<String>, RepositoryError> {
    let Some(values) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of strings"),
        });
    };

    values
        .iter()
        .map(|value| parse_non_empty_string_value(file, key, value))
        .collect()
}

fn parse_reviewers(
    file: &str,
    key: &str,
    value: &toml::Value,
) -> Result<Vec<ReviewerTarget>, RepositoryError> {
    let Some(reviewers) = value.as_array() else {
        return Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be an array of reviewer names"),
        });
    };

    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();

    for reviewer in reviewers {
        let Some(reviewer) = reviewer.as_str() else {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` must contain only strings"),
            });
        };
        let reviewer = reviewer.trim();

        if reviewer.is_empty() {
            return Err(RepositoryError::InvalidConfig {
                file: file.to_owned(),
                message: format!("`{key}` must not contain empty reviewer names"),
            });
        }

        let reviewer = parse_reviewer_target(file, reviewer)?;

        if seen.insert(reviewer.clone()) {
            normalized.push(reviewer);
        }
    }

    Ok(normalized)
}

fn required_value<'a>(
    file: &str,
    table: &'a toml::Table,
    key: &str,
) -> Result<&'a toml::Value, RepositoryError> {
    table
        .get(key.rsplit_once('.').map_or(key, |(_, key)| key))
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` is required"),
        })
}

fn parse_reviewer_target(file: &str, reviewer: &str) -> Result<ReviewerTarget, RepositoryError> {
    ReviewerTarget::parse(reviewer).ok_or_else(|| RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message: format!(
            "`{reviewer}` is not a valid reviewer name; use a GitHub login or `org/team`"
        ),
    })
}

pub(super) fn config_file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(PROJECT_CONFIG_FILE)
        .to_owned()
}
