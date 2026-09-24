use super::dashboard_keys::DashboardKeyBindingsLayer;
use super::*;

/// Terminal and dispatch preferences loaded from optional config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiConfig {
    pub default_command: Vec<String>,
    pub dashboard_keys: DashboardKeyBindings,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            default_command: vec!["log".to_owned()],
            dashboard_keys: DashboardKeyBindings::default(),
        }
    }
}

impl UiConfig {
    pub(super) fn apply_layer(&mut self, layer: UiConfigLayer) {
        if let Some(default_command) = layer.default_command {
            self.default_command = default_command;
        }
        if let Some(bindings) = layer.dashboard_keys {
            self.dashboard_keys.apply_layer(bindings);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct UiConfigLayer {
    pub(super) default_command: Option<Vec<String>>,
    pub(super) dashboard_keys: Option<DashboardKeyBindingsLayer>,
}
