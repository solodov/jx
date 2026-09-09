use super::*;

impl LayoutConfig {
    /// Plans bounded discovery paths; final identity checks still enforce rule precedence.
    pub(crate) fn discovery_paths(
        &self,
        environment: &RuntimeEnvironment,
    ) -> Result<Vec<LayoutDiscoveryPath>, RepositoryError> {
        let mut paths = Vec::new();
        for source in self.sources.values() {
            if let Some(path) = layout_discovery_path(
                source,
                &self.default_root,
                &self.default.path,
                (None, None),
                environment,
            )? {
                paths.push(path);
            }
            for rule in self.rules.iter().filter(|rule| rule.source == source.name) {
                if let Some(path) = layout_discovery_path(
                    source,
                    rule.root.as_deref().unwrap_or(&self.default_root),
                    rule.path.as_deref().unwrap_or(&self.default.path),
                    (rule.owner.as_deref(), rule.repo.as_deref()),
                    environment,
                )? {
                    paths.push(path);
                }
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// One primary-checkout pattern. `None` matches exactly one directory, never a subtree.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LayoutDiscoveryPath {
    pub(crate) root: PathBuf,
    pub(crate) components: Vec<Option<String>>,
}

fn layout_discovery_path(
    source: &LayoutSourceConfig,
    root: &str,
    template: &str,
    rule_identity: (Option<&str>, Option<&str>),
    environment: &RuntimeEnvironment,
) -> Result<Option<LayoutDiscoveryPath>, RepositoryError> {
    let root = resolve_config_root(root, environment)?;
    let Some(components) = template_path_components(template) else {
        return Ok(None);
    };
    let components = components
        .into_iter()
        .map(|component| match component.as_str() {
            "{source}" => Some(source.name.clone()),
            "{host}" => Some(source.host.clone()),
            "{owner}" => rule_identity.0.map(str::to_owned),
            "{repo}" => rule_identity.1.map(str::to_owned),
            _ => Some(component),
        })
        .collect();
    Ok(Some(LayoutDiscoveryPath { root, components }))
}
