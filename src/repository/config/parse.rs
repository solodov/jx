use super::*;

#[derive(Debug, Default)]
pub(super) struct WorkflowConfigLayer {
    pub(super) path: PathBuf,
    pub(super) layout: Option<LayoutConfigLayer>,
    pub(super) repo: Option<RepoConfig>,
    pub(super) diff: Option<DiffConfig>,
    pub(super) auth: Option<AuthConfig>,
    pub(super) shell: Option<ShellConfigLayer>,
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
        if !matches!(key.as_str(), "layout" | "repo" | "diff" | "auth" | "shell") {
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

    Ok(WorkflowConfigLayer {
        path,
        layout,
        repo,
        diff,
        auth,
        shell,
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
            "advance_trunk" | "reviewers" | "reviewer_rules" | "rules"
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
            "repo" | "advance_trunk" | "reviewers" | "reviewer_rules"
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
    let reviewers = table
        .get("reviewers")
        .map(|value| parse_reviewers(file, &format!("{key_prefix}.reviewers"), value))
        .transpose()?
        .unwrap_or_default();
    let reviewer_rules = table
        .get("reviewer_rules")
        .map(|value| {
            parse_reviewer_path_rules(file, &format!("{key_prefix}.reviewer_rules"), value)
        })
        .transpose()?
        .unwrap_or_default();

    Ok(RepoPolicyConfig {
        advance_trunk,
        reviewers,
        reviewer_rules,
    })
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
        if !matches!(key.as_str(), "navigation" | "zoxide") {
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
    let zoxide = table
        .get("zoxide")
        .map(|value| parse_shell_zoxide_mode(file, "shell.zoxide", value))
        .transpose()?;

    Ok(ShellConfigLayer { navigation, zoxide })
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
        _ => Err(RepositoryError::InvalidConfig {
            file: file.to_owned(),
            message: format!("`{key}` must be `auto` or `never`"),
        }),
    }
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
