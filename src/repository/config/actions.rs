use super::*;

mod parse;
mod template;

pub(super) use parse::parse_pr_action_layer;
pub(crate) use template::render_pr_action_argument;

/// A manually selected command. Arguments are templates, never an implicit shell string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrAction {
    pub id: String,
    pub title: String,
    /// Sort priority before the title; lower values come first and omitted values default to zero.
    pub order: i64,
    pub command: Vec<String>,
    pub cwd: PrActionWorkingDirectory,
    pub on_success: PrActionOnSuccess,
}

/// How a dashboard reloads after a successful action; local reloads are review-only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrActionOnSuccess {
    #[default]
    Refresh,
    RefreshLocal,
}

impl PrActionOnSuccess {
    /// Returns the configuration spelling for previews and action logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::RefreshLocal => "refresh-local",
        }
    }
}

/// Whether an action requires the selected PR's checkout or explicitly uses the caller's directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PrActionWorkingDirectory {
    #[default]
    Repository,
    Caller,
}

/// Trust boundary of the file that supplied an effective action definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrActionConfigScope {
    Global,
    Repository,
}

/// Origin of an action, retained even when it replaces a previously trusted definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrActionSource {
    pub path: PathBuf,
    pub scope: PrActionConfigScope,
}

/// A complete action definition and the configuration file responsible for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPrAction {
    pub action: PrAction,
    pub source: PrActionSource,
}

/// Source-preserving layers for one action set, isolated from other dashboards and repo policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrActionsConfig {
    layers: Vec<(PrActionSource, PrActionLayer)>,
}

impl PrActionsConfig {
    /// Resolves this set's global defaults, matching global rules, then local definitions and rules.
    /// Whole-definition overrides and removals apply before sorting by ascending (order, title).
    pub fn for_repository(&self, repository: &GitHubRepository) -> Vec<ResolvedPrAction> {
        let mut actions = Vec::new();
        let slug = repository.slug();
        for scope in [PrActionConfigScope::Global, PrActionConfigScope::Repository] {
            for (source, layer) in self
                .layers
                .iter()
                .filter(|(source, _)| source.scope == scope)
            {
                apply_actions(&mut actions, &layer.base, source);
            }
            for (source, layer) in self
                .layers
                .iter()
                .filter(|(source, _)| source.scope == scope)
            {
                for rule in &layer.rules {
                    if Glob::new(&rule.repository)
                        .expect("action repository patterns are validated at load time")
                        .compile_matcher()
                        .is_match(&slug)
                    {
                        apply_actions(&mut actions, &rule.actions, source);
                    }
                }
            }
        }
        actions.sort_by(|left, right| {
            left.action
                .order
                .cmp(&right.action.order)
                .then_with(|| left.action.title.cmp(&right.action.title))
        });
        actions
    }

    pub(super) fn push_layer(&mut self, source: PrActionSource, layer: PrActionLayer) {
        if !layer.base.is_empty() || !layer.rules.is_empty() {
            self.layers.push((source, layer));
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct PrActionLayer {
    base: Vec<PrActionOverride>,
    rules: Vec<PrActionRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrActionRule {
    repository: String,
    actions: Vec<PrActionOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrActionOverride {
    Define(PrAction),
    Disable(String),
}

fn apply_actions(
    target: &mut Vec<ResolvedPrAction>,
    definitions: &[PrActionOverride],
    source: &PrActionSource,
) {
    for definition in definitions {
        match definition {
            PrActionOverride::Define(action) => {
                let resolved = ResolvedPrAction {
                    action: action.clone(),
                    source: source.clone(),
                };
                if let Some(existing) = target
                    .iter_mut()
                    .find(|existing| existing.action.id == action.id)
                {
                    *existing = resolved;
                } else {
                    target.push(resolved);
                }
            }
            PrActionOverride::Disable(id) => target.retain(|existing| existing.action.id != *id),
        }
    }
}
