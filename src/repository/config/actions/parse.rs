use super::*;

/// Parses actions without flattening base definitions into repository rules.
pub(in crate::repository::config) fn parse_pr_action_layer(
    file: &str,
    repo: Option<&toml::Value>,
) -> Result<PrActionLayer, RepositoryError> {
    let Some(repo) = repo.and_then(toml::Value::as_table) else {
        return Ok(PrActionLayer::default());
    };
    let base = parse_actions(file, "repo.actions", repo.get("actions"))?;
    let mut rules = Vec::new();
    if let Some(values) = repo.get("rules").and_then(toml::Value::as_array) {
        for (index, value) in values.iter().enumerate() {
            // The ordinary repo parser has already validated the enclosing rule and its pattern.
            let table = value.as_table().expect("validated repository rule");
            let actions = parse_actions(
                file,
                &format!("repo.rules[{index}].actions"),
                table.get("actions"),
            )?;
            if !actions.is_empty() {
                rules.push(PrActionRule {
                    repository: table["repo"]
                        .as_str()
                        .expect("validated repository pattern")
                        .trim()
                        .to_owned(),
                    actions,
                });
            }
        }
    }
    Ok(PrActionLayer { base, rules })
}

fn parse_actions(
    file: &str,
    key: &str,
    value: Option<&toml::Value>,
) -> Result<Vec<PrActionOverride>, RepositoryError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| invalid(file, format!("`{key}` must be an array of tables")))?;
    let mut seen = BTreeSet::new();
    let mut actions = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let key = format!("{key}[{index}]");
        let table = value
            .as_table()
            .ok_or_else(|| invalid(file, format!("`{key}` must be a table")))?;
        for name in table.keys() {
            if !matches!(
                name.as_str(),
                "id" | "title" | "command" | "cwd" | "enabled"
            ) {
                return Err(RepositoryError::UnsupportedConfigKey {
                    file: file.to_owned(),
                    key: format!("{key}.{name}"),
                });
            }
        }
        let id = required_string(file, &key, table, "id")?;
        if !seen.insert(id.clone()) {
            return Err(invalid(
                file,
                format!("duplicate action id `{id}` in `{key}`"),
            ));
        }
        let enabled = table
            .get("enabled")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| invalid(file, format!("`{key}.enabled` must be a boolean")))
            })
            .transpose()?
            .unwrap_or(true);
        if !enabled {
            if table
                .keys()
                .any(|key| !matches!(key.as_str(), "id" | "enabled"))
            {
                return Err(invalid(
                    file,
                    format!("disabled action `{id}` accepts only `id` and `enabled`"),
                ));
            }
            actions.push(PrActionOverride::Disable(id));
            continue;
        }
        let title = required_string(file, &key, table, "title")?;
        let command = table
            .get("command")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| {
                invalid(
                    file,
                    format!("`{key}.command` must be a non-empty argument array"),
                )
            })?;
        let command = command
            .iter()
            .map(|value| {
                let arg = value.as_str().ok_or_else(|| {
                    invalid(file, format!("`{key}.command` arguments must be strings"))
                })?;
                if arg.contains('\0') {
                    return Err(invalid(
                        file,
                        format!("`{key}.command` cannot contain NUL bytes"),
                    ));
                }
                render_pr_action_argument(arg, |_| Ok(String::new()))
                    .map_err(|message| invalid(file, format!("`{key}.command`: {message}")))?;
                Ok(arg.to_owned())
            })
            .collect::<Result<Vec<_>, RepositoryError>>()?;
        if command
            .first()
            .is_none_or(|program| program.trim().is_empty())
        {
            return Err(invalid(
                file,
                format!("`{key}.command` must name an executable"),
            ));
        }
        let cwd = match table.get("cwd") {
            None => PrActionWorkingDirectory::Repository,
            Some(value) if value.as_str() == Some("repository") => {
                PrActionWorkingDirectory::Repository
            }
            Some(value) if value.as_str() == Some("caller") => PrActionWorkingDirectory::Caller,
            Some(_) => {
                return Err(invalid(
                    file,
                    format!("`{key}.cwd` must be `repository` or `caller`"),
                ))
            }
        };
        actions.push(PrActionOverride::Define(PrAction {
            id,
            title,
            command,
            cwd,
        }));
    }
    Ok(actions)
}

fn required_string(
    file: &str,
    key: &str,
    table: &toml::Table,
    field: &str,
) -> Result<String, RepositoryError> {
    table
        .get(field)
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(file, format!("`{key}.{field}` must be a non-empty string")))
}

fn invalid(file: &str, message: String) -> RepositoryError {
    RepositoryError::InvalidConfig {
        file: file.to_owned(),
        message,
    }
}
