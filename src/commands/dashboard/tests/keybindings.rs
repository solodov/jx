use super::*;

#[test]
fn defaults_refresh_only_with_gr_and_use_gg_without_moving_on_the_prefix() {
    let mut keyboard = keyboard("");
    assert_eq!(press(&mut keyboard, 'r'), DashboardInput::None);
    assert_eq!(press(&mut keyboard, 'g'), DashboardInput::None);
    let hint = keyboard.prefix_hint().unwrap();
    assert!(hint.contains("g …"));
    assert!(hint.contains("r: Refresh"));
    assert!(hint.contains("g: First PR"));
    assert_eq!(
        press(&mut keyboard, 'r'),
        DashboardInput::Command(DashboardCommand::Refresh)
    );
    assert!(keyboard.prefix_hint().is_none());
    assert_eq!(press(&mut keyboard, 'g'), DashboardInput::None);
    assert_eq!(
        press(&mut keyboard, 'g'),
        DashboardInput::Command(DashboardCommand::First)
    );
    assert_eq!(
        press(&mut keyboard, 'G'),
        DashboardInput::Command(DashboardCommand::Last)
    );
}

#[test]
fn escape_never_requests_exit_and_cancels_prefix_or_help_before_running_actions() {
    let mut keyboard = keyboard("");
    let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(keyboard.handle_key(escape), DashboardInput::Cancel);
    assert_eq!(keyboard.handle_key(escape), DashboardInput::Cancel);
    press(&mut keyboard, 'g');
    assert_eq!(keyboard.handle_key(escape), DashboardInput::None);
    assert!(keyboard.prefix_hint().is_none());
    press(&mut keyboard, '?');
    assert!(keyboard.help_open());
    assert_eq!(keyboard.handle_key(escape), DashboardInput::None);
    assert!(!keyboard.help_open());
    assert_eq!(press(&mut keyboard, 'Q'), DashboardInput::None);
    assert_eq!(
        press(&mut keyboard, 'q'),
        DashboardInput::Command(DashboardCommand::Quit)
    );
}

#[test]
fn invalid_continuations_do_not_accidentally_quit_or_reuse_the_old_prefix() {
    let mut keyboard = keyboard("");
    press(&mut keyboard, 'g');
    assert_eq!(press(&mut keyboard, 'q'), DashboardInput::None);
    assert!(keyboard.prefix_hint().is_none());
    assert_eq!(press(&mut keyboard, 'r'), DashboardInput::None);
    assert_eq!(press(&mut keyboard, 'g'), DashboardInput::None);
    assert_eq!(
        press(&mut keyboard, 'r'),
        DashboardInput::Command(DashboardCommand::Refresh)
    );
}

#[test]
fn repeats_only_navigate_and_do_not_complete_a_prefix_or_fire_refresh_or_quit() {
    let mut keyboard = keyboard("");
    for (key, expected) in [
        ('r', DashboardInput::None),
        ('q', DashboardInput::None),
        ('g', DashboardInput::None),
        ('j', DashboardInput::Command(DashboardCommand::Down)),
    ] {
        let mut event = KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE);
        event.kind = KeyEventKind::Repeat;
        assert_eq!(keyboard.handle_key(event), expected);
    }
    press(&mut keyboard, 'g');
    let mut event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    event.kind = KeyEventKind::Repeat;
    assert_eq!(keyboard.handle_key(event), DashboardInput::None);
    assert!(keyboard.prefix_hint().is_some());
    let mut repeat_refresh = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE);
    repeat_refresh.kind = KeyEventKind::Repeat;
    assert_eq!(keyboard.handle_key(repeat_refresh), DashboardInput::None);
    event.kind = KeyEventKind::Release;
    assert_eq!(keyboard.handle_key(event), DashboardInput::None);
    assert!(keyboard.prefix_hint().is_some());
    assert_eq!(
        press(&mut keyboard, 'g'),
        DashboardInput::Command(DashboardCommand::First)
    );
}

#[test]
fn remaps_replace_defaults_and_help_and_prefix_hints_follow_effective_bindings() {
    let mut keyboard =
        keyboard("[ui.dashboard.keys]\nrefresh=['Ctrl-r', 'z z r']\nquit=['x']\nfirst=['Home']\n");
    assert_eq!(press(&mut keyboard, 'r'), DashboardInput::None);
    assert_eq!(press(&mut keyboard, 'q'), DashboardInput::None);
    assert_eq!(
        keyboard.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        DashboardInput::Command(DashboardCommand::Refresh)
    );
    assert_eq!(
        press(&mut keyboard, 'x'),
        DashboardInput::Command(DashboardCommand::Quit)
    );
    assert_eq!(press(&mut keyboard, 'z'), DashboardInput::None);
    assert!(keyboard.prefix_hint().unwrap().contains("z r: Refresh"));
    assert_eq!(press(&mut keyboard, 'z'), DashboardInput::None);
    assert_eq!(
        press(&mut keyboard, 'r'),
        DashboardInput::Command(DashboardCommand::Refresh)
    );
    let help = keyboard.help_lines().join("\n");
    assert!(help.contains("Refresh: Ctrl-r, z z r"));
    assert!(help.contains("Exit dashboard: x"));
    assert!(!help.contains("Exit dashboard: q"));
}

#[test]
fn help_is_bounded_scrollable_and_does_not_dispatch_list_commands() {
    let mut keyboard = keyboard("");
    press(&mut keyboard, '?');
    assert_eq!(press(&mut keyboard, 'g'), DashboardInput::None);
    assert_eq!(press(&mut keyboard, 'r'), DashboardInput::None);
    assert!(keyboard.help_open());
    for (width, height) in [(0, 0), (1, 1), (4, 2), (30, 6), (100, 30)] {
        let screen = keyboard
            .help_screen(DashboardTerminalSize::new(width, height))
            .unwrap();
        assert!(screen.y + screen.lines.len() <= usize::from(height));
        for line in screen.lines {
            assert!(rendered_visible_width(&line) <= usize::from(width));
        }
    }
    for _ in 0..6 {
        keyboard.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    }
    let screen = keyboard
        .help_screen(DashboardTerminalSize::new(35, 8))
        .unwrap();
    assert!(screen.lines.join("\n").contains("confirms"));
    keyboard.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!keyboard.help_open());
}

fn press(keyboard: &mut DashboardKeyboard, key: char) -> DashboardInput {
    keyboard.handle_key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE))
}

fn keyboard(config: &str) -> DashboardKeyboard {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join(".config/jx")).unwrap();
    std::fs::write(root.path().join(".config/jx/keys.toml"), config).unwrap();
    let environment = RuntimeEnvironment::new(
        root.path(),
        [("HOME".to_owned(), root.path().display().to_string())],
    );
    DashboardKeyboard::new(
        WorkflowConfig::discover_global(&environment)
            .unwrap()
            .ui
            .dashboard_keys,
    )
}
