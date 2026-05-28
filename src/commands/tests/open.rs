use super::*;

#[test]
fn open_prints_current_repository_url() {
    // Verifies: Open can resolve the current repository without launching a browser.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "open", "--print"], &environment, &services)
        .expect("open print succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo\n"
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn o_alias_runs_open() {
    // Verifies: The short open alias keeps repository navigation quick.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = ssh://git@github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), []);
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "o", "--print"], &environment, &services)
        .expect("open alias succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo\n"
    );
}

#[test]
fn open_accepts_specific_repository_argument() {
    // Verifies: Open resolves a layout project key and launches that repository URL.
    let workspace = TestWorkspace::new();
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
    let target = workspace.create_jj_workspace("projects/target");
    TestWorkspace::write_git_config_at(
        &target,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/target.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(["jx", "open", "target"], &environment, &services)
        .expect("open project succeeds");

    assert_eq!(
        services.opened_urls.borrow().as_slice(),
        ["https://github.com/example-owner/target".to_owned()]
    );
    assert_eq!(
        result.stdout,
        "Opened: https://github.com/example-owner/target\n"
    );
}

#[test]
fn open_repo_filter_prints_matching_repository_urls() {
    // Verifies: Open uses the same configured project glob matching as global status commands.
    let workspace = TestWorkspace::new();
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
    let api = workspace.create_jj_workspace("projects/api");
    let web = workspace.create_jj_workspace("projects/web");
    TestWorkspace::write_git_config_at(
        &api,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &web,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web.git
"#,
    );
    let environment = RuntimeEnvironment::new(workspace.path(), workspace.home_environment());
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "open", "--print", "--repo", "*"],
        &environment,
        &services,
    )
    .expect("open filtered repos succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/api\nhttps://github.com/example-owner/web\n"
    );
}

#[test]
fn open_prs_builds_authored_pull_request_search_url() {
    // Verifies: PR navigation uses the authenticated login and repo qualifiers for glob matches.
    let workspace = TestWorkspace::new();
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
    let api = workspace.create_jj_workspace("projects/api");
    let web = workspace.create_jj_workspace("projects/web");
    TestWorkspace::write_git_config_at(
        &api,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/api.git
"#,
    );
    TestWorkspace::write_git_config_at(
        &web,
        r#"
[remote "origin"]
    url = https://github.com/example-owner/web.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [
            (
                "HOME".to_owned(),
                workspace.home.to_string_lossy().into_owned(),
            ),
            ("GH_TOKEN".to_owned(), "placeholder-token".to_owned()),
        ],
    );
    let services = FakeServices::default();

    let result = run_with_args_and_services(
        ["jx", "open", "prs", "--print", "--repo", "*"],
        &environment,
        &services,
    )
    .expect("open prs succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/pulls?q=is%3Apr+is%3Aopen+author%3Aexample-user+repo%3Aexample-owner%2Fapi+repo%3Aexample-owner%2Fweb\n"
    );
}

#[test]
fn open_pr_prints_first_candidate_with_pull_request() {
    // Verifies: Open PR follows candidate bookmark order until GitHub has a pull request to open.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        open_pull_request_candidates: vec![
            "topic/without-review".to_owned(),
            "topic/with-review".to_owned(),
        ],
        pull_requests_by_head: BTreeMap::from([(
            "topic/with-review".to_owned(),
            PullRequestRecord {
                number: 24,
                title: "Reviewable change".to_owned(),
                body: None,
                head_branch: "topic/with-review".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/24".to_owned()),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result =
        run_with_args_and_services(["jx", "open", "pr", "--print"], &environment, &services)
            .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/24\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [None]
    );
    assert_eq!(
        services.pull_request_head_calls.borrow().as_slice(),
        ["topic/without-review", "topic/with-review"]
    );
    assert!(services.opened_urls.borrow().is_empty());
}

#[test]
fn open_pr_accepts_positional_commit_or_bookmark_selector() {
    // Verifies: Open PR accepts the selector UX used for commit prefixes and slash bookmarks.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        open_pull_request_candidates: vec!["example-user/00-1977d9cd".to_owned()],
        pull_requests_by_head: BTreeMap::from([(
            "example-user/00-1977d9cd".to_owned(),
            PullRequestRecord {
                number: 1977,
                title: "Selected change".to_owned(),
                body: None,
                head_branch: "example-user/00-1977d9cd".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some(
                    "https://github.com/example-owner/example-repo/pull/1977".to_owned(),
                ),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result = run_with_args_and_services(
        ["jx", "open", "pr", "--print", "example-user/00-1977d9cd"],
        &environment,
        &services,
    )
    .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/1977\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [Some("example-user/00-1977d9cd".to_owned())]
    );
}

#[test]
fn open_pr_accepts_commit_option_as_selector() {
    // Verifies: The legacy --commit option keeps working while using selector resolution.
    let workspace = TestWorkspace::new();
    workspace.write_git_config(
        r#"
[remote "origin"]
    url = https://github.com/example-owner/example-repo.git
"#,
    );
    let environment = RuntimeEnvironment::new(
        workspace.path(),
        [("GH_TOKEN".to_owned(), "placeholder-token".to_owned())],
    );
    let services = FakeServices {
        open_pull_request_candidates: vec!["topic/with-review".to_owned()],
        pull_requests_by_head: BTreeMap::from([(
            "topic/with-review".to_owned(),
            PullRequestRecord {
                number: 25,
                title: "Selected change".to_owned(),
                body: None,
                head_branch: "topic/with-review".to_owned(),
                base_branch: "main".to_owned(),
                html_url: Some("https://github.com/example-owner/example-repo/pull/25".to_owned()),
                draft: false,
                merged: false,
            },
        )]),
        ..Default::default()
    };

    let result = run_with_args_and_services(
        ["jx", "open", "pr", "--print", "--commit", "abc123"],
        &environment,
        &services,
    )
    .expect("open pr succeeds");

    assert_eq!(
        result.stdout,
        "https://github.com/example-owner/example-repo/pull/25\n"
    );
    assert_eq!(
        services.open_pull_request_selectors.borrow().as_slice(),
        [Some("abc123".to_owned())]
    );
}
