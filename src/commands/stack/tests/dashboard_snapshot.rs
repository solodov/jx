use super::*;
use crate::domain::RepositorySummary;

#[test]
fn failed_repository_entries_reject_both_partial_and_wholly_failed_refreshes() {
    for partial in [false, true] {
        let mut entries = vec![
            GlobalStackStatusEntry {
                key: Some("offline".to_owned()),
                root: PathBuf::from("/offline"),
                display_root: "~/offline".to_owned(),
                repository: Some(
                    GitHubRepository::parse("https://github.com/owner/offline").unwrap(),
                ),
                result: Err("network down\nrequest details".to_owned()),
            },
            GlobalStackStatusEntry {
                key: Some("missing-checkout".to_owned()),
                root: PathBuf::from("/missing"),
                display_root: "~/missing".to_owned(),
                repository: None,
                result: Err("cannot open checkout".to_owned()),
            },
        ];
        if partial {
            entries.push(healthy_entry());
        }
        let loaded = LoadedGlobalStackStatusView {
            total_repositories: entries.len(),
            entries: entries.clone(),
            display_names: BTreeMap::new(),
        };
        let error = snapshot(loaded, Path::new("/caller"))
            .err()
            .expect("failed refresh must not produce a snapshot")
            .to_string();
        assert!(error.contains("2 repository refreshes failed"));
        assert!(error.contains("owner/offline (~/offline): network down\nrequest details"));
        assert!(error.contains("missing-checkout (~/missing): cannot open checkout"));

        // One-shot output still includes per-repository errors and any successful rows.
        let text = render_global_stack_status(
            &entries,
            entries.len(),
            Path::new("/caller"),
            false,
            None,
            PullRequestTableLayout::Flow,
            &BTreeMap::new(),
        )
        .text;
        assert!(text.contains("error: network down"));
        assert!(text.contains("error: cannot open checkout"));
        assert_eq!(text.contains("owner/healthy"), partial);
        let json = render_stack_status_json(&entries);
        assert!(json.contains("network down"));
        assert!(json.contains("cannot open checkout"));
    }
}

#[test]
fn healthy_and_empty_results_can_still_replace_the_dashboard() {
    for entries in [vec![healthy_entry()], Vec::new()] {
        let empty = entries.is_empty();
        let loaded = LoadedGlobalStackStatusView {
            total_repositories: entries.len(),
            entries,
            display_names: BTreeMap::new(),
        };
        let snapshot = snapshot(loaded, Path::new("/caller")).unwrap();
        let frame = snapshot
            .render(DashboardRenderOptions {
                color: false,
                terminal_width: Some(80),
            })
            .unwrap();
        assert_eq!(frame.text.is_empty(), empty);
        if !empty {
            assert!(frame.text.contains("owner/healthy"));
        }
    }
}

fn healthy_entry() -> GlobalStackStatusEntry {
    GlobalStackStatusEntry::current(
        PathBuf::from("/healthy"),
        &PullRequestStackStatusReport {
            repository: RepositorySummary {
                origin_name: "origin",
                origin_url: "https://github.com/owner/healthy".to_owned(),
                github_slug: "owner/healthy".to_owned(),
                github_url: "https://github.com/owner/healthy".to_owned(),
                token_source: "test".to_owned(),
                config: "test",
                default_reviewers: String::new(),
            },
            snapshot: PullRequestStackSnapshot::default(),
            statuses: BTreeMap::new(),
            trunk: None,
            review_wait_threshold_seconds: None,
        },
    )
}
