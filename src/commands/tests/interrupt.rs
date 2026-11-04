use super::*;
use std::{
    io::{BufRead as _, BufReader},
    os::unix::process::CommandExt as _,
    process::{Child, Command, ExitStatus},
    time::Instant,
};

#[test]
fn interrupt_finishes_active_operation_and_releases_git_locks() {
    assert_graceful_interrupt("success");
}

#[test]
fn interrupt_unwinds_failed_operation_and_releases_git_locks() {
    assert_graceful_interrupt("error");
}

#[test]
fn interrupt_stops_global_sync_before_the_next_repository() {
    assert_graceful_interrupt("all");
}

#[test]
fn interrupt_waits_for_git_subprocess_cleanup() {
    assert_graceful_interrupt("subprocess");
}

#[test]
fn second_interrupt_can_force_exit() {
    let mut child = InterruptChild::spawn("force");
    child.wait_for("ready");
    child.signal();
    child.wait_for("interrupted");
    assert!(child.process.try_wait().expect("poll child").is_none());
    child.signal();
    assert_eq!(child.wait().code(), Some(130));
}

#[test]
fn dashboard_consumes_interrupt_without_cancelling_later_operations() {
    let mut child = InterruptChild::spawn("dashboard");
    child.wait_for("ready");
    child.signal();
    assert!(child.wait().success());
}

fn assert_graceful_interrupt(mode: &str) {
    let mut child = InterruptChild::spawn(mode);
    child.wait_for("ready");
    child.signal();
    child.wait_for("interrupted");
    assert!(child.process.try_wait().expect("poll child").is_none());
    for lock in LOCKS {
        assert!(
            child.root.path().join(lock).exists(),
            "operation still owns {lock}"
        );
    }
    fs::write(child.root.path().join("release"), b"").expect("release operation");
    assert_eq!(child.wait().code(), Some(130));
    for lock in LOCKS {
        assert!(!child.root.path().join(lock).exists(), "released {lock}");
        let resource = child.root.path().join(lock.strip_suffix(".lock").unwrap());
        let _lock = gix::lock::File::acquire_to_update_resource(
            &resource,
            gix::lock::acquire::Fail::Immediately,
            None,
        )
        .expect("next operation can acquire the Git lock");
    }
    if mode == "subprocess" {
        assert!(child.root.path().join("git-reaped").exists());
        assert!(!child
            .root
            .path()
            .join(".git/refs/heads/child.lock")
            .exists());
    }
}

const LOCKS: [&str; 2] = [
    ".git/packed-refs.lock",
    ".git/refs/heads/solodov/topic.lock",
];

struct InterruptChild {
    process: Child,
    root: tempfile::TempDir,
}

impl InterruptChild {
    fn spawn(mode: &str) -> Self {
        let root = tempfile::tempdir().expect("temporary directory");
        let process = Command::new(env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "commands::tests::interrupt::interrupt_test_child",
                "--ignored",
                "--nocapture",
            ])
            .env("JX_INTERRUPT_TEST_ROOT", root.path())
            .env("JX_INTERRUPT_TEST_MODE", mode)
            .process_group(0)
            .spawn()
            .expect("spawn isolated signal test");
        Self { process, root }
    }

    fn signal(&self) {
        // SAFETY: the child has its own process group, separate from the test runner.
        assert_eq!(
            unsafe { libc::kill(-(self.process.id() as i32), libc::SIGINT) },
            0
        );
    }

    fn wait_for(&mut self, name: &str) {
        wait_until(|| {
            if self.root.path().join(name).exists() {
                return true;
            }
            assert!(
                self.process.try_wait().expect("poll child").is_none(),
                "child exited before {name}"
            );
            false
        });
    }

    fn wait(&mut self) -> ExitStatus {
        let mut result = None;
        wait_until(|| {
            result = self.process.try_wait().expect("poll child exit");
            result.is_some()
        });
        result.unwrap()
    }
}

impl Drop for InterruptChild {
    fn drop(&mut self) {
        if self.process.try_wait().ok().flatten().is_none() {
            // SAFETY: only this test's isolated child group is terminated on failure.
            unsafe { libc::kill(-(self.process.id() as i32), libc::SIGKILL) };
            let _ = self.process.wait();
        }
    }
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for signal test"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Runs only in a subprocess so actual SIGINT handling cannot affect parallel tests.
#[test]
#[ignore = "signal-test subprocess entrypoint"]
fn interrupt_test_child() {
    let root = PathBuf::from(env::var_os("JX_INTERRUPT_TEST_ROOT").expect("test root"));
    let mode = env::var("JX_INTERRUPT_TEST_MODE").expect("test mode");
    install_interrupt_handler().expect("install handler");

    if mode == "dashboard" {
        let dashboard = DashboardInterrupts::enter().expect("dashboard interrupts");
        fs::write(root.join("ready"), b"").unwrap();
        wait_until(|| dashboard.take_pending() > 0);
        assert!(check_interrupted().is_ok());
        drop(dashboard);
        assert!(check_interrupted().is_ok());
        return;
    }

    let result = if mode == "subprocess" {
        locked_operation(&root, &mode)
            .map(|()| CommandResult::success(String::new()))
            .map_err(CommandError::from)
    } else {
        run_interrupted_sync(&root, &mode)
    };
    assert!(
        check_jj_interrupted().is_err(),
        "next operation must not start"
    );
    let result = finish_interrupted_command(result).expect("interrupted command result");
    std::process::exit(i32::from(result.exit_code));
}

fn run_interrupted_sync(root: &Path, mode: &str) -> Result<CommandResult, CommandError> {
    let workspace = TestWorkspace::new();
    let all = mode == "all";
    if all {
        workspace.write_home_file(
            ".config/jx/config.toml",
            r#"
[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/projects"
path = "{repo}"
"#,
        );
        for name in ["first", "second"] {
            let repo = workspace.create_jj_workspace(&format!("projects/{name}"));
            TestWorkspace::write_git_config_at(
                &repo,
                &format!(
                    "[remote \"origin\"]\nurl = https://github.com/example-owner/{name}.git\n"
                ),
            );
        }
    } else {
        workspace.write_git_config(
            "[remote \"origin\"]\nurl = https://github.com/example-owner/example-repo.git\n",
        );
    }
    let hook_root = root.to_path_buf();
    let hook_mode = mode.to_owned();
    let services = FakeServices {
        fetch_origin_hook: Some(Box::new(move || locked_operation(&hook_root, &hook_mode))),
        origin_push_access_roots: all.then(BTreeSet::new),
        ..FakeServices::default()
    };
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let args = if all {
        vec!["jx", "sync", "--all"]
    } else {
        vec!["jx", "sync", "-R"]
    };
    let result = run_with_args_and_services(args, &environment, &services);
    assert_eq!(services.fetch_origin_roots.borrow().len(), 1);
    assert!(services.push_tracked_roots.borrow().is_empty());
    assert_eq!(services.advance_trunk_calls.get(), 0);
    assert!(services.sync_pull_request_pushes.borrow().is_empty());
    result
}

fn locked_operation(root: &Path, mode: &str) -> Result<(), JjError> {
    let mut git = if mode == "subprocess" {
        Some(prepare_git_transaction(root))
    } else {
        None
    };
    fs::create_dir_all(root.join(".git/refs/heads/solodov")).unwrap();
    let _locks: Vec<_> = LOCKS
        .iter()
        .map(|lock| {
            gix::lock::File::acquire_to_update_resource(
                root.join(lock.strip_suffix(".lock").unwrap()),
                gix::lock::acquire::Fail::Immediately,
                None,
            )
            .expect("acquire real Git lock")
        })
        .collect();
    fs::write(root.join("ready"), b"").unwrap();
    wait_until(|| check_interrupted().is_err());
    if let Some(git) = &mut git {
        assert!(!git
            .wait()
            .expect("reap interrupted Git transaction")
            .success());
        fs::write(root.join("git-reaped"), b"").unwrap();
    }
    fs::write(root.join("interrupted"), b"").unwrap();
    wait_until(|| root.join("release").exists());
    if mode == "error" {
        return Err(JjError::Export {
            message: "simulated export failure".to_owned(),
        });
    }
    Ok(())
}

fn prepare_git_transaction(root: &Path) -> Child {
    for args in [
        vec!["init", "--quiet"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "test",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }
    let mut child = Command::new("git")
        .args(["update-ref", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start Git reference transaction");
    let input = child.stdin.as_mut().unwrap();
    input
        .write_all(b"start\nupdate refs/heads/child HEAD\nprepare\n")
        .unwrap();
    input.flush().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    for expected in ["start: ok", "prepare: ok"] {
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), expected);
    }
    assert!(root.join(".git/refs/heads/child.lock").exists());
    child
}
