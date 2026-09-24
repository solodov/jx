use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn dashboard_defaults_and_global_layers_replace_only_named_operations() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let config = WorkflowConfig::discover_global(&environment).unwrap();
    assert_eq!(
        config.ui.dashboard_keys.labels(DashboardCommand::Refresh),
        ["g r"]
    );
    assert_eq!(
        config.ui.dashboard_keys.labels(DashboardCommand::Quit),
        ["q"]
    );
    workspace.write_file(
        ".config/jx/10-keys.toml",
        "[ui.dashboard.keys]\nrefresh=['Ctrl-r']\nquit=['x']\n",
    );
    workspace.write_file(
        ".config/jx/20-keys.toml",
        "[ui.dashboard.keys]\nrefresh=['F5']\nlast=[]\n",
    );
    let config = WorkflowConfig::discover_global(&environment).unwrap();
    assert_eq!(
        config.ui.dashboard_keys.labels(DashboardCommand::Refresh),
        ["F5"]
    );
    assert_eq!(
        config.ui.dashboard_keys.labels(DashboardCommand::Quit),
        ["x"]
    );
    assert_eq!(
        config.ui.dashboard_keys.labels(DashboardCommand::First),
        ["g g", "Home"]
    );
    assert!(config
        .ui
        .dashboard_keys
        .labels(DashboardCommand::Last)
        .is_empty());
}

#[test]
fn key_conflicts_are_checked_after_merging_all_global_layers() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    workspace.write_file(
        ".config/jx/10-keys.toml",
        "[ui.dashboard.keys]\nrefresh=['j']\n",
    );
    assert!(WorkflowConfig::discover_global(&environment)
        .unwrap_err()
        .to_string()
        .contains("ambiguous dashboard bindings"));
    workspace.write_file(
        ".config/jx/20-keys.toml",
        "[ui.dashboard.keys]\ndown=['Down']\n",
    );
    assert_eq!(
        WorkflowConfig::discover_global(&environment)
            .unwrap()
            .ui
            .dashboard_keys
            .labels(DashboardCommand::Refresh),
        ["j"]
    );
}

#[test]
fn repository_local_keymaps_are_rejected_without_changing_other_ui_preferences() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    workspace.write_file(".jx/config.toml", "[ui.dashboard.keys]\nrefresh=['F5']\n");
    let error = WorkflowConfig::discover(&environment)
        .unwrap_err()
        .to_string();
    assert!(error.contains("user preference"), "{error}");
    assert!(WorkflowConfig::discover_global(&environment).is_ok());
    workspace.write_file(".jx/config.toml", "[ui]\ndefault_command=['status']\n");
    assert_eq!(
        WorkflowConfig::discover(&environment)
            .unwrap()
            .ui
            .default_command,
        ["status"]
    );
}

#[test]
fn invalid_keymaps_fail_with_an_actionable_config_error() {
    let workspace = TestWorkspace::new();
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    for config in [
        "[ui.dashboard.keys]\nrefresh=['g']", // conflicts with default gg
        "[ui.dashboard.keys]\nrefresh=['r','r']",
        "[ui.dashboard.keys]\nrefresh=['z','z r']",
        "[ui.dashboard.keys]\nrefresh=['']",
        "[ui.dashboard.keys]\nrefresh=['NotAKey']",
        "[ui.dashboard.keys]\nrefresh=['Ctrl-C']",
        "[ui.dashboard.keys]\nrefresh=['z Ctrl-c']",
        "[ui.dashboard.keys]\nquit=['Esc']",
        "[ui.dashboard.keys]\nquit=['Ctrl-Esc']",
        "[ui.dashboard.keys]\nrefresh=['Ctrl-Ctrl-r']",
        "[ui.dashboard.keys]\nrefresh=['F99']",
        "[ui.dashboard.keys]\nrefresh=['Shift-r', 'R']",
        "[ui.dashboard.keys]\nrefresh=['Ctrl-R', 'Ctrl-r']",
        "[ui.dashboard.keys]\nrefresh='r'",
        "[ui.dashboard.keys]\nrefresh=[1]",
        "[ui.dashboard.keys]\nunknown=['z']",
        "[ui.dashboard]\nkeys=[]",
        "[ui.dashboard]\nunknown=[]",
        "[ui]\ndashboard=[]",
    ] {
        workspace.write_file(".config/jx/keys.toml", config);
        assert!(
            WorkflowConfig::discover_global(&environment).is_err(),
            "{config}"
        );
    }
}

#[test]
fn configured_keystrokes_match_terminal_normalization() {
    for (spelling, event, label) in [
        (
            "G",
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT),
            "G",
        ),
        (
            "Shift-g",
            KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE),
            "G",
        ),
        (
            "?",
            KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT),
            "?",
        ),
        (
            "Ctrl-R",
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            "Ctrl-r",
        ),
        (
            "Alt-r",
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
            "Alt-r",
        ),
        (
            "BackTab",
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            "Shift-Tab",
        ),
        (
            "Shift-Tab",
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            "Shift-Tab",
        ),
        (
            "Space",
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            "Space",
        ),
        (
            "Ctrl--",
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::CONTROL),
            "Ctrl--",
        ),
    ] {
        let key = DashboardKey::parse(spelling).unwrap();
        assert_eq!(Some(key.clone()), DashboardKey::from_event(event));
        assert_eq!(key.label(), label);
    }
    assert!(
        DashboardKey::from_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::SUPER)).is_none()
    );
}
