use super::*;

#[test]
fn current_workspace_log_snapshots_pending_disk_changes() {
    // Verifies: the public log entrypoint refreshes disk changes before reading via jj-lib.
    if !jj_cli_is_available() {
        eprintln!("skipping log snapshot test because jj CLI is unavailable");
        return;
    }

    let fixture = TestWorkspace::new("workspace-log-snapshot");
    let settings = log_test_settings().expect("settings");
    pollster::block_on(Workspace::init_internal_git(&settings, fixture.path()))
        .expect("initialize jj workspace");
    fs::write(fixture.path().join("README.md"), "pending\n").expect("write pending file");

    let log = JjWorkspace::current_workspace_log(fixture.path(), &[]).expect("log renders");
    let workspace = JjWorkspace::load(fixture.path()).expect("workspace reloads");
    let current = workspace.current_commit().expect("current commit");
    let is_empty = pollster::block_on(current.is_empty(workspace.repo.as_ref()))
        .expect("current commit emptiness");

    assert!(!is_empty, "{log}");
}

#[test]
fn short_commit_ids_are_eight_hex_characters() {
    // Verifies: Short commit IDs are eight hex characters.
    let commit_id = CommitId::from_hex("0123456789abcdef");

    assert_eq!(short_commit_id(&commit_id), "01234567");
}

#[test]
fn workspace_log_preserves_configured_jj_revset() {
    // Verifies: jx log does not add workspace-head filtering on top of jj's revset.
    let fixture = TestWorkspace::new("workspace-log");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current workspace change").await;
        let other = write_child(tx.repo_mut(), &trunk, "other workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        tx.repo_mut()
            .set_wc_commit(WorkspaceNameBuf::from("other"), other.id().clone())
            .expect("set other working-copy change");

        let repo = tx
            .commit("arrange multi-workspace log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains("current workspace change"), "{log}");
    assert!(log.contains("main trunk"), "{log}");
    assert!(log.contains("other workspace change"), "{log}");
}

#[test]
fn compact_relative_time_abbreviates_log_ages() {
    // Verifies: Compact log ages use one short unit and reserve `mo` for months.
    const MINUTE_MS: i64 = 60_000;
    const HOUR_MS: i64 = 60 * MINUTE_MS;
    const DAY_MS: i64 = 24 * HOUR_MS;
    const WEEK_MS: i64 = 7 * DAY_MS;
    const MONTH_MS: i64 = 30 * DAY_MS;
    const YEAR_MS: i64 = 365 * DAY_MS;
    let now = test_timestamp(YEAR_MS * 5);

    assert_eq!(compact_relative_time(now, now), "now");
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 + MINUTE_MS), now),
        "now"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 2 * MINUTE_MS), now),
        "2m"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 3 * HOUR_MS), now),
        "3h"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 4 * DAY_MS), now),
        "4d"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 2 * WEEK_MS), now),
        "2w"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 3 * MONTH_MS), now),
        "3mo"
    );
    assert_eq!(
        compact_relative_time(test_timestamp(now.timestamp.0 - 2 * YEAR_MS), now),
        "2y"
    );
}

fn test_timestamp(milliseconds: i64) -> jj_lib::backend::Timestamp {
    jj_lib::backend::Timestamp {
        timestamp: jj_lib::backend::MillisSinceEpoch(milliseconds),
        tz_offset: 0,
    }
}

#[test]
fn log_description_line_ellipsizes_to_content_width() {
    // Verifies: log-only ellipsizing truncates the description line with a Unicode ellipsis.
    let mut buffer =
        b"header\nBRA4-350: Move InboundApplicationDeferredAcceptanceTask tail-marker\nfooter\n"
            .to_vec();

    ellipsize_log_description_line(&mut buffer, 20).expect("description ellipsizes");
    let rendered = String::from_utf8(buffer).expect("log output is UTF-8");
    let description = rendered.lines().nth(1).expect("description line renders");

    assert!(description.ends_with('…'), "{description:?}");
    assert!(!description.contains("..."), "{description:?}");
    assert!(!description.contains("tail-marker"), "{description:?}");
    assert!(rendered_visible_width(description) <= 20, "{description:?}");
    assert!(rendered.starts_with("header\n"));
    assert!(rendered.ends_with("\nfooter\n"));
}

#[test]
fn log_description_line_ellipsizing_preserves_color_reset() {
    // Verifies: truncating a colored description does not leak style into later graph output.
    let mut buffer = b"header\n\x1b[38;5;2mVery long colored description\x1b[39m\n".to_vec();

    ellipsize_log_description_line(&mut buffer, 8).expect("description ellipsizes");
    let rendered = String::from_utf8(buffer).expect("log output is UTF-8");
    let description = rendered.lines().nth(1).expect("description line renders");

    assert!(description.contains('…'), "{description:?}");
    assert!(description.ends_with("\x1b[0m"), "{description:?}");
    assert!(rendered_visible_width(description) <= 8, "{description:?}");
}

#[test]
fn workspace_log_omits_commit_ids_from_jx_default_header() {
    // Verifies: jx's default log header prioritizes the operator-facing change id.
    let fixture = TestWorkspace::new("workspace-log-compact-header");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo, current_change_id, current_commit_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current workspace change").await;
        let current_change_id = short_change_id(&current);
        let current_commit_id = short_commit_id(current.id());

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange compact log").await.expect("commit");
        (workspace, repo, current_change_id, current_commit_id)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains(&current_change_id), "{log}");
    assert!(!log.contains(&current_commit_id), "{log}");
}

#[test]
fn workspace_log_renders_compact_commit_age() {
    // Verifies: jx's default log header renders age as a compact relative unit.
    let fixture = TestWorkspace::new("workspace-log-compact-age");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let mut signature = root.committer().clone();
        let now = jj_lib::backend::Timestamp::now();
        signature.timestamp = jj_lib::backend::Timestamp {
            timestamp: jj_lib::backend::MillisSinceEpoch(now.timestamp.0 - 2 * 60_000),
            tz_offset: now.tz_offset,
        };
        let current = tx
            .repo_mut()
            .new_commit(vec![root.id().clone()], root.tree())
            .set_description("compact age change")
            .set_author(signature.clone())
            .set_committer(signature)
            .write()
            .await
            .expect("write compact age commit");

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange compact age log").await.expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains(" 2m"), "{log}");
    assert!(!log.contains("minutes ago"), "{log}");
}

#[test]
fn workspace_log_ellipsizes_long_description_line() {
    // Verifies: default jx log descriptions stay on one terminal-width-bound line.
    let fixture = TestWorkspace::new("workspace-log-ellipsized-description");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let description = format!(
            "BRA4-350: Move InboundApplicationDeferredAcceptanceTask {} tail-marker",
            "long".repeat(2_500)
        );
        let current = write_child(tx.repo_mut(), &root, &description).await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange ellipsized description log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");
    let description = log
        .lines()
        .find(|line| line.contains("BRA4-350"))
        .expect("description line renders");

    assert!(description.contains('…'), "{description:?}");
    assert!(!description.contains("..."), "{description:?}");
    assert!(!log.contains("tail-marker"), "{log}");
}

#[test]
fn workspace_log_renders_current_user_author_as_me() {
    // Verifies: jx's default log abbreviates commits authored with the resolved jj user.email.
    let fixture = TestWorkspace::new("workspace-log-current-user-author");
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(jx_default_config_layers());
    config.extend_layers([ConfigLayer::parse(
        ConfigSource::User,
        r#"
[user]
name = "Current User"
email = "me@example.com"
"#,
    )
    .expect("user config parses")]);
    jj_lib::config::migrate(&mut config, &default_config_migrations()).expect("config migrates");
    let settings = UserSettings::from_config(config).expect("settings load");
    let (workspace, repo, mine_change_id) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let mut mine_author = root.author().clone();
        mine_author.name = "Current User".to_owned();
        mine_author.email = "me@example.com".to_owned();
        let mine = tx
            .repo_mut()
            .new_commit(vec![root.id().clone()], root.tree())
            .set_description("current user change")
            .set_author(mine_author.clone())
            .write()
            .await
            .expect("write current user commit");
        let mine_change_id = short_change_id(&mine);
        let mut other_author = mine_author;
        other_author.name = "Other User".to_owned();
        other_author.email = "other@example.com".to_owned();
        let other = tx
            .repo_mut()
            .new_commit(vec![mine.id().clone()], mine.tree())
            .set_description("other user change")
            .set_author(other_author)
            .write()
            .await
            .expect("write other user commit");

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), other.id().clone())
            .expect("set current working-copy change");

        let repo = tx.commit("arrange current user log").await.expect("commit");
        (workspace, repo, mine_change_id)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains(&format!("{mine_change_id} me ")), "{log}");
    assert!(!log.contains("me@example.com"), "{log}");
    assert!(log.contains("other@example.com"), "{log}");
}

#[test]
fn workspace_log_colors_current_user_author_like_other_authors() {
    // Verifies: `me` and compact ages keep jj's log colors instead of rendering as plain terminal text.
    let fixture = TestWorkspace::new("workspace-log-current-user-author-color");
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(jx_default_config_layers());
    config.extend_layers([ConfigLayer::parse(
        ConfigSource::User,
        r#"
[user]
name = "Current User"
email = "me@example.com"

[ui]
color = "always"
"#,
    )
    .expect("user config parses")]);
    jj_lib::config::migrate(&mut config, &default_config_migrations()).expect("config migrates");
    let settings = UserSettings::from_config(config).expect("settings load");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let mut author = root.author().clone();
        author.name = "Current User".to_owned();
        author.email = "me@example.com".to_owned();
        let now = jj_lib::backend::Timestamp::now();
        author.timestamp = jj_lib::backend::Timestamp {
            timestamp: jj_lib::backend::MillisSinceEpoch(now.timestamp.0 - 2 * 60_000),
            tz_offset: now.tz_offset,
        };
        let current = tx
            .repo_mut()
            .new_commit(vec![root.id().clone()], root.tree())
            .set_description("colored author")
            .set_author(author.clone())
            .set_committer(author)
            .write()
            .await
            .expect("write current user commit");

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");

        let repo = tx
            .commit("arrange colored current user log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains("me\x1b["), "{log:?}");
    assert!(log.contains("\x1b[38;5;14m2m\x1b["), "{log:?}");
    assert!(!log.contains("me@example.com"), "{log:?}");
}

#[test]
fn workspace_log_ignores_unsynced_backing_git_bookmarks() {
    // Verifies: jx log treats @git as local backing-store state, not remote publish freshness.
    let fixture = TestWorkspace::new("workspace-log-unsynced-git-bookmark");
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(jx_default_config_layers());
    config.extend_layers([ConfigLayer::parse(
        ConfigSource::User,
        r#"
[ui]
color = "always"
"#,
    )
    .expect("color config parses")]);
    jj_lib::config::migrate(&mut config, &default_config_migrations()).expect("config migrates");
    let settings = UserSettings::from_config(config).expect("settings load");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        set_local_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_origin_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_remote_bookmark(
            tx.repo_mut(),
            git::REMOTE_NAME_FOR_LOCAL_GIT_REPO.as_str(),
            "topic/current",
            trunk.id(),
        );

        let repo = tx
            .commit("arrange git-only unsynced bookmark log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains("topic/current"), "{log}");
    assert!(!log.contains("topic/current*"), "{log}");
    assert!(!log.contains("\x1b[48;2;239;232;251m"), "{log}");
}

#[test]
fn workspace_log_highlights_unsynced_local_bookmarks() {
    // Verifies: jx log makes stale local bookmark state visible without relying on operator jj templates.
    let fixture = TestWorkspace::new("workspace-log-unsynced-bookmark");
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(jx_default_config_layers());
    config.extend_layers([ConfigLayer::parse(
        ConfigSource::User,
        r#"
[ui]
color = "always"
"#,
    )
    .expect("color config parses")]);
    jj_lib::config::migrate(&mut config, &default_config_migrations()).expect("config migrates");
    let settings = UserSettings::from_config(config).expect("settings load");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let trunk = write_child(tx.repo_mut(), &root, "main trunk").await;
        let current = write_child(tx.repo_mut(), &trunk, "current workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        set_local_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_origin_bookmark(tx.repo_mut(), "topic/current", trunk.id());

        let repo = tx
            .commit("arrange unsynced bookmark log")
            .await
            .expect("commit");
        (workspace, repo)
    });

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &[])
        .expect("log renders");

    assert!(log.contains("topic/current"), "{log}");
    assert!(!log.contains("topic/current*"), "{log}");
    assert!(log.contains("\x1b[48;2;239;232;251m"), "{log}");
}

#[test]
fn workspace_log_links_pull_request_annotations_for_matching_bookmarks() {
    // Verifies: log annotations attach PR links to commits via local bookmark targets.
    let fixture = TestWorkspace::new("workspace-log-pr-links");
    let settings = log_test_settings().expect("settings");
    let (workspace, repo) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .expect("initialize jj workspace");
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let current = write_child(tx.repo_mut(), &root, "current workspace change").await;

        tx.repo_mut()
            .set_wc_commit(workspace.workspace_name().to_owned(), current.id().clone())
            .expect("set current working-copy change");
        set_local_bookmark(tx.repo_mut(), "topic/current", current.id());

        let repo = tx.commit("arrange annotated log").await.expect("commit");
        (workspace, repo)
    });
    let annotations = [LogBookmarkAnnotation {
        bookmark: "topic/current".to_owned(),
        label: "#42".to_owned(),
        url: Some("https://github.com/example-owner/example-repo/pull/42".to_owned()),
    }];

    let log = render_current_workspace_log(&workspace, repo.as_ref(), fixture.path(), &annotations)
        .expect("log renders");

    assert!(log.contains("topic/current"), "{log}");
    assert!(
        log.contains(
            "\x1b]8;;https://github.com/example-owner/example-repo/pull/42\x1b\\#42\x1b]8;;\x1b\\"
        ),
        "{log}"
    );
}

fn jj_cli_is_available() -> bool {
    Command::new("jj")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}
