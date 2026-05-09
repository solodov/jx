use super::*;

/// Diff tool preferences loaded from optional config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffConfig {
    pub default_tool: Option<String>,
    pub tools: BTreeMap<String, DiffToolConfig>,
}

impl DiffConfig {
    pub(super) fn apply_layer(&mut self, layer: DiffConfig) {
        if layer.default_tool.is_some() {
            self.default_tool = layer.default_tool;
        }
        self.tools.extend(layer.tools);
    }
}

/// Configured renderer strategy for `jx diff`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffToolConfig {
    External(ExternalDiffToolConfig),
    Pipe(PipeDiffToolConfig),
}

/// External diff command invoked by jj with generated left/right trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalDiffToolConfig {
    pub command: String,
    pub args: Vec<String>,
}

/// Renderer command that consumes a jj-produced diff stream on stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeDiffToolConfig {
    pub producer_args: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
}
