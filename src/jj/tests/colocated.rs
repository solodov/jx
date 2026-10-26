use super::*;

#[test]
fn bookmark_mutations_preserve_colocated_workspace_git_heads() {
    // An older jj-lib dropped the child workspace's HEAD record on every transaction.
    // The next jj status then created a new change and snapshotted the same files into it.
    let fixture = TestWorkspace::new("colocated-workspace-heads");
    let primary = fixture.path().join("primary");
    let topic = fixture.path().join("topic");
    fs::create_dir_all(&primary).expect("create primary workspace");
    let settings = user_settings().expect("settings");
    pollster::block_on(Workspace::init_colocated_git(
        &settings,
        &primary,
        gix::hash::Kind::Sha1,
    ))
    .expect("initialize colocated workspace");
    run_jj(&primary, &["describe", "-m", "base"]);
    run_jj(&primary, &["new", "-m", "primary work"]);
    run_jj(
        &primary,
        &[
            "workspace",
            "add",
            "--colocate",
            "--name",
            "topic",
            "-r",
            "@",
            "../topic",
        ],
    );
    fs::write(topic.join("topic.txt"), "local work\n").expect("write topic file");
    let mut subject =
        JjWorkspace::load_after_working_copy_snapshot(&topic).expect("load child workspace");
    let current = subject.current_commit().expect("topic working copy");
    let default_head = subject
        .repo
        .view()
        .git_head(WorkspaceName::new("default"))
        .clone();
    let topic_head = subject
        .repo
        .view()
        .git_head(WorkspaceName::new("topic"))
        .clone();
    assert!(default_head.is_present());
    assert!(topic_head.is_present());
    assert_ne!(default_head, topic_head);

    for branch in ["topic/first", "topic/second"] {
        subject
            .ensure_bookmark(branch, &current.id().hex())
            .expect("create bookmark through jx");
        // Reload from the operation store, not just the in-memory transaction.
        subject = JjWorkspace::load(&topic).expect("reload saved operation");
        assert_eq!(
            subject.repo.view().git_head(WorkspaceName::new("default")),
            &default_head,
        );
        assert_eq!(
            subject.repo.view().git_head(WorkspaceName::new("topic")),
            &topic_head,
        );

        subject = JjWorkspace::load_after_working_copy_snapshot(&topic)
            .expect("snapshot after jx mutation");
        let after = subject
            .current_commit()
            .expect("working copy after snapshot");
        assert_eq!(
            after.id(),
            current.id(),
            "status must not create a duplicate"
        );
        assert_eq!(after.tree_ids(), current.tree_ids());
        assert_eq!(
            subject.local_bookmark_commit_id(branch).expect("bookmark"),
            after.id().hex(),
        );
        assert_eq!(
            fs::read_to_string(topic.join("topic.txt")).expect("read topic file"),
            "local work\n",
        );
    }
}

/// Runs the installed jj CLI against an isolated test workspace.
fn run_jj(root: &Path, args: &[&str]) {
    let output = Command::new("jj")
        .args(["--no-pager", "--color=never"])
        .args(args)
        .current_dir(root)
        .output()
        .expect("run jj");
    assert!(
        output.status.success(),
        "jj {args:?}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}
