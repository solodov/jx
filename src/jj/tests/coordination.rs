use super::*;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::Command;

#[test]
fn contending_writers_time_out_without_splitting_the_lock() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mutation.lock");
    let first = RepositoryLock::lock_path(&path, Duration::ZERO, &mut || Ok(())).unwrap();
    assert!(matches!(
        RepositoryLock::lock_path(&path, Duration::from_millis(30), &mut || Ok(())),
        Err(JjError::MutationBusy { .. })
    ));
    drop(first);
    let _next = RepositoryLock::lock_path(&path, Duration::ZERO, &mut || Ok(())).unwrap();
    assert!(path.exists());
}

#[test]
fn cancellation_interrupts_lock_waiting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mutation.lock");
    let _first = RepositoryLock::lock_path(&path, Duration::ZERO, &mut || Ok(())).unwrap();
    assert!(matches!(
        RepositoryLock::lock_path(&path, Duration::from_secs(1), &mut || Err(
            JjError::Interrupted
        )),
        Err(JjError::Interrupted)
    ));
}

#[test]
fn independent_repositories_do_not_block_each_other() {
    let directory = tempfile::tempdir().unwrap();
    let _first =
        RepositoryLock::lock_path(&directory.path().join("first"), Duration::ZERO, &mut || {
            Ok(())
        })
        .unwrap();
    let _second = RepositoryLock::lock_path(
        &directory.path().join("second"),
        Duration::ZERO,
        &mut || Ok(()),
    )
    .unwrap();
}

#[test]
fn publication_rejects_an_operation_that_moved_during_preparation() {
    let root = tempfile::tempdir().unwrap();
    let settings = user_settings().unwrap();
    pollster::block_on(async {
        let (_workspace, base) =
            Workspace::init_internal_git(&settings, root.path(), gix::hash::Kind::Sha1)
                .await
                .unwrap();
        let parent = base.store().root_commit();
        let mut pending = base.start_transaction();
        pending
            .repo_mut()
            .new_commit(vec![parent.id().clone()], parent.tree())
            .set_description("pending")
            .write()
            .await
            .unwrap();
        let mut external = base.start_transaction();
        external
            .repo_mut()
            .new_commit(vec![parent.id().clone()], parent.tree())
            .set_description("external")
            .write()
            .await
            .unwrap();
        let latest = external.commit("external mutation").await.unwrap();
        let error = publish_checked(pending, "must not publish", &base).unwrap_err();
        assert!(matches!(error, JjError::ConcurrentOperations));
        let heads = base.loader().op_heads_store().get_op_heads().await.unwrap();
        assert_eq!(heads, vec![latest.op_id().clone()]);
    });
}

#[test]
fn loading_does_not_reconcile_divergent_operations() {
    let root = tempfile::tempdir().unwrap();
    let settings = user_settings().unwrap();
    let loader = pollster::block_on(async {
        let (workspace, base) =
            Workspace::init_internal_git(&settings, root.path(), gix::hash::Kind::Sha1)
                .await
                .unwrap();
        let parent = base.store().root_commit();
        for message in ["first", "second"] {
            let mut tx = base.start_transaction();
            tx.repo_mut()
                .new_commit(vec![parent.id().clone()], parent.tree())
                .set_description(message)
                .write()
                .await
                .unwrap();
            tx.commit(message).await.unwrap();
        }
        workspace.repo_loader().clone()
    });
    let before = pollster::block_on(loader.op_heads_store().get_op_heads()).unwrap();
    assert_eq!(before.len(), 2);
    assert!(matches!(
        load_single_head(&loader),
        Err(JjError::ConcurrentOperations)
    ));
    assert_eq!(
        pollster::block_on(loader.op_heads_store().get_op_heads()).unwrap(),
        before
    );
}

#[test]
fn process_exit_releases_the_lock() {
    const CHILD_PATH: &str = "JX_TEST_MUTATION_LOCK_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        let _guard =
            RepositoryLock::lock_path(Path::new(&path), Duration::ZERO, &mut || Ok(())).unwrap();
        println!("lock-ready");
        io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(30));
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mutation.lock");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "jj::coordination::tests::process_exit_releases_the_lock",
            "--nocapture",
        ])
        .env(CHILD_PATH, &path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        assert_ne!(
            output.read_line(&mut line).unwrap(),
            0,
            "child exited before locking"
        );
        if line.contains("lock-ready") {
            break;
        }
        line.clear();
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let _guard = RepositoryLock::lock_path(&path, Duration::from_secs(1), &mut || Ok(())).unwrap();
}
