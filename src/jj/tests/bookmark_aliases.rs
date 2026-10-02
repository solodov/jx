use super::*;

#[test]
fn aliases_use_current_local_change_ids_and_skip_unusable_bookmarks() {
    let fixture = TestWorkspace::new("bookmark-change-aliases");
    let settings = alias_settings("bookmarks()");
    let (workspace, repo, current) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .unwrap();
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let old = write_alias_change(
            tx.repo_mut(),
            &root,
            "11100000000000000000000000000000",
            "old",
        )
        .await;
        let current = write_alias_change(
            tx.repo_mut(),
            &root,
            "22200000000000000000000000000000",
            "current",
        )
        .await;
        set_origin_bookmark(tx.repo_mut(), "topic/current", old.id());
        set_local_bookmark(tx.repo_mut(), "topic/current", current.id());
        set_local_bookmark(tx.repo_mut(), "other/topic/missing", current.id());
        set_origin_bookmark(tx.repo_mut(), "topic/missing", old.id());
        tx.repo_mut().set_local_bookmark_target(
            RefName::new("topic/conflicted"),
            RefTarget::from_legacy_form([], [old.id().clone(), current.id().clone()]),
        );
        let divergent = tx
            .repo_mut()
            .new_commit(vec![root.id().clone()], root.tree())
            .set_change_id(old.change_id().clone())
            .set_description("divergent old change")
            .write()
            .await
            .unwrap();
        set_local_bookmark(tx.repo_mut(), "topic/divergent", old.id());
        set_local_bookmark(tx.repo_mut(), "topic/divergent-other", divergent.id());
        let repo = tx.commit("arrange local alias targets").await.unwrap();
        (workspace, repo, current)
    });
    let subject = JjWorkspace { workspace, repo };

    let aliases = subject
        .local_bookmark_change_aliases(
            &[
                "topic/current",
                "topic/missing",
                "topic/conflicted",
                "topic/divergent",
            ]
            .map(str::to_owned),
        )
        .unwrap();

    assert_eq!(aliases.len(), 1);
    assert_eq!(
        aliases["topic/current"],
        current.change_id().reverse_hex()[..1]
    );
    assert_eq!(
        subject
            .resolve_single_revision(&aliases["topic/current"], "alias test")
            .unwrap()
            .id(),
        current.id(),
    );
}

#[test]
fn aliases_follow_configured_log_prefix_scope_and_retain_long_unique_prefixes() {
    for (scope, expected_length) in [("all()", 4), ("bookmarks('topic/selected')", 1)] {
        let fixture = TestWorkspace::new("bookmark-alias-scope");
        let settings = alias_settings(scope);
        let (workspace, repo, selected) = pollster::block_on(async {
            let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
                .await
                .unwrap();
            let root = repo.store().root_commit();
            let mut tx = repo.start_transaction();
            let selected = write_alias_change(
                tx.repo_mut(),
                &root,
                "11100000000000000000000000000000",
                "selected",
            )
            .await;
            let other = write_alias_change(
                tx.repo_mut(),
                &root,
                "11110000000000000000000000000000",
                "other",
            )
            .await;
            set_local_bookmark(tx.repo_mut(), "topic/selected", selected.id());
            set_local_bookmark(tx.repo_mut(), "topic/other", other.id());
            let repo = tx.commit("arrange scoped alias prefixes").await.unwrap();
            (workspace, repo, selected)
        });
        let subject = JjWorkspace { workspace, repo };

        let aliases = subject
            .local_bookmark_change_aliases(&["topic/selected".to_owned()])
            .unwrap();
        let alias = &aliases["topic/selected"];

        if scope == "all()" {
            // The initial working-copy change has a random ID and may require additional characters.
            assert!(alias.len() >= expected_length);
        } else {
            assert_eq!(alias.len(), expected_length);
        }
        assert!(selected.change_id().reverse_hex().starts_with(alias));
        assert_eq!(
            subject
                .resolve_single_revision(alias, "alias test")
                .unwrap()
                .id(),
            selected.id(),
        );
    }
}

#[test]
fn aliases_avoid_prefixes_shadowed_by_local_bookmark_names() {
    let fixture = TestWorkspace::new("bookmark-alias-shadow");
    let settings = alias_settings("bookmarks()");
    let (workspace, repo, selected) = pollster::block_on(async {
        let (workspace, repo) = Workspace::init_internal_git(&settings, fixture.path())
            .await
            .unwrap();
        let root = repo.store().root_commit();
        let mut tx = repo.start_transaction();
        let selected = write_alias_change(
            tx.repo_mut(),
            &root,
            "11100000000000000000000000000000",
            "selected",
        )
        .await;
        let shadow = selected.change_id().reverse_hex()[..1].to_owned();
        set_local_bookmark(tx.repo_mut(), "topic/selected", selected.id());
        set_local_bookmark(tx.repo_mut(), &shadow, root.id());
        let repo = tx.commit("arrange shadowed alias prefix").await.unwrap();
        (workspace, repo, selected)
    });
    let subject = JjWorkspace { workspace, repo };

    let aliases = subject
        .local_bookmark_change_aliases(&["topic/selected".to_owned()])
        .unwrap();
    let alias = &aliases["topic/selected"];

    assert_eq!(alias.len(), 2);
    assert_eq!(
        subject
            .resolve_single_revision(alias, "alias test")
            .unwrap()
            .id(),
        selected.id(),
    );
}

fn alias_settings(scope: &str) -> UserSettings {
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(jx_default_config_layers());
    config.extend_layers([ConfigLayer::parse(
        ConfigSource::User,
        &format!("[revsets]\nshort-prefixes = \"{scope}\"\n"),
    )
    .unwrap()]);
    UserSettings::from_config(config).unwrap()
}

async fn write_alias_change(
    repo: &mut MutableRepo,
    parent: &Commit,
    change_hex: &'static str,
    description: &str,
) -> Commit {
    repo.new_commit(vec![parent.id().clone()], parent.tree())
        .set_change_id(ChangeId::from_hex(change_hex))
        .set_description(description)
        .write()
        .await
        .unwrap()
}
