use super::*;
use serde_json::{json, Value};
use std::collections::VecDeque;

#[test]
fn inventory_paginates_and_groups_same_owner_prs_without_merging_repository_numbers() {
    let first = page(
        3,
        vec![node("First", 7, "Owner"), node("Second", 7, "owner")],
        true,
        Some("next"),
    );
    let last = page(3, vec![node("First", 8, "fork-owner")], false, None);
    let (inventory, cursors) = collect_pages(vec![Ok(first), Ok(last)]);
    let inventory = inventory.unwrap();

    assert_eq!(cursors, vec![None, Some("next".to_owned())]);
    assert_eq!(inventory.viewer.login, "viewer");
    assert_eq!(inventory.repositories.len(), 2);
    let first = inventory.for_repository(&GitHubRepository {
        owner: "OWNER".to_owned(),
        name: "first".to_owned(),
    });
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].number, 7);
    assert_eq!(first[0].title, "First PR");
    assert_eq!(first[0].body.as_deref(), Some("PR description"));
    assert_eq!(first[0].head_branch, "topic/7");
    assert_eq!(first[0].base_branch, "main");
    assert!(first[0].draft);
    assert!(!first[0].merged);
    assert!(inventory
        .for_repository(&GitHubRepository {
            owner: "Owner".to_owned(),
            name: "empty".to_owned()
        })
        .is_empty());
}

#[test]
fn repository_case_changes_between_pages_do_not_split_the_inventory() {
    let (result, _) = collect_pages(vec![
        Ok(page(2, vec![node("Repo", 1, "Owner")], true, Some("next"))),
        Ok(page(2, vec![node("REPO", 2, "Owner")], false, None)),
    ]);
    let inventory = result.unwrap();
    assert_eq!(inventory.repositories.len(), 1);
    let repository = GitHubRepository {
        owner: "Owner".to_owned(),
        name: "repo".to_owned(),
    };
    assert_eq!(inventory.for_repository(&repository).len(), 2);
}

#[test]
fn a_complete_empty_inventory_is_authoritative() {
    let (result, cursors) = collect_pages(vec![Ok(page(0, vec![], false, None))]);
    assert!(result.unwrap().repositories.is_empty());
    assert_eq!(cursors, vec![None]);
}

#[test]
fn null_nodes_and_missing_or_excess_nodes_reject_the_whole_inventory() {
    for response in [
        page(1, vec![Value::Null], false, None),
        page(1, vec![], true, Some("no-progress")),
        page(2, vec![node("repo", 1, "Owner")], false, None),
        page(0, vec![node("repo", 1, "Owner")], false, None),
    ] {
        let (result, _) = collect_pages(vec![Ok(response)]);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("incomplete authored PR inventory"));
    }
}

#[test]
fn broken_pagination_does_not_return_partial_success() {
    for cursor in [None, Some("")] {
        let (result, _) = collect_pages(vec![Ok(page(
            2,
            vec![node("repo", 1, "Owner")],
            true,
            cursor,
        ))]);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("cursor is missing"));
    }
    let (result, _) = collect_pages(vec![
        Ok(page(3, vec![node("repo", 1, "Owner")], true, Some("same"))),
        Ok(page(3, vec![node("repo", 2, "Owner")], true, Some("same"))),
    ]);
    assert!(result.unwrap_err().to_string().contains("cursor repeats"));
}

#[test]
fn changing_counts_duplicate_prs_and_changing_viewers_reject_the_inventory() {
    let first = page(2, vec![node("repo", 1, "Owner")], true, Some("next"));
    let mut changed_viewer = page(2, vec![node("repo", 2, "Owner")], false, None);
    changed_viewer["viewer"]["login"] = json!("other-viewer");
    for last in [
        page(1, vec![node("repo", 2, "Owner")], false, None),
        page(2, vec![node("REPO", 1, "Owner")], false, None),
        changed_viewer,
    ] {
        let (result, _) = collect_pages(vec![Ok(first.clone()), Ok(last)]);
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("incomplete authored PR inventory"));
    }
}

#[test]
fn later_page_failure_discards_earlier_results() {
    let (result, cursors) = collect_pages(vec![
        Ok(page(2, vec![node("repo", 1, "Owner")], true, Some("next"))),
        Err(GitHubError::Timeout {
            operation: INVENTORY_OPERATION,
            timeout_ms: 15_000,
        }),
    ]);
    assert!(matches!(result, Err(GitHubError::Timeout { .. })));
    assert_eq!(cursors.len(), 2);
}

#[test]
fn graphql_partial_data_and_missing_fields_are_not_complete_inventories() {
    let response = json!({
        "data": page(1, vec![node("repo", 1, "Owner")], false, None),
        "errors": [{ "message": "Resource protected by organization SAML enforcement" }],
    });
    let response: GraphQlResponse<InventoryQueryData> = serde_json::from_value(response).unwrap();
    assert!(response.into_data(INVENTORY_OPERATION).is_err());

    let mut missing_repository = node("repo", 1, "Owner");
    missing_repository["repository"] = Value::Null;
    let mut missing_count = page(1, vec![node("repo", 1, "Owner")], false, None);
    missing_count["viewer"]["pullRequests"]
        .as_object_mut()
        .unwrap()
        .remove("totalCount");
    for response in [
        page(1, vec![missing_repository], false, None),
        missing_count,
    ] {
        assert!(serde_json::from_value::<InventoryQueryData>(response).is_err());
    }
}

#[test]
fn inventory_query_uses_the_viewer_connection_and_requests_pagination_evidence() {
    assert!(AUTHORED_PULL_REQUEST_INVENTORY_QUERY
        .contains("pullRequests(states: OPEN, first: 100, after: $cursor)"));
    for field in [
        "totalCount",
        "hasNextPage",
        "endCursor",
        "repository",
        "headRepositoryOwner",
    ] {
        assert!(AUTHORED_PULL_REQUEST_INVENTORY_QUERY.contains(field));
    }
    assert!(!AUTHORED_PULL_REQUEST_INVENTORY_QUERY.contains("search("));
}

fn collect_pages(
    pages: Vec<Result<Value, GitHubError>>,
) -> (
    Result<AuthoredPullRequestInventory, GitHubError>,
    Vec<Option<String>>,
) {
    let mut pages = VecDeque::from(pages);
    let mut cursors = Vec::new();
    let result = futures::executor::block_on(collect_inventory(|cursor| {
        cursors.push(cursor);
        std::future::ready(
            pages
                .pop_front()
                .expect("unexpected page request")
                .map(|page| serde_json::from_value(page).unwrap()),
        )
    }));
    (result, cursors)
}

fn page(count: usize, nodes: Vec<Value>, has_next: bool, cursor: Option<&str>) -> Value {
    json!({ "viewer": {
        "login": "viewer",
        "pullRequests": {
            "totalCount": count,
            "pageInfo": { "hasNextPage": has_next, "endCursor": cursor },
            "nodes": nodes,
        }
    } })
}

fn node(repository: &str, number: u64, head_owner: &str) -> Value {
    json!({
        "repository": { "name": repository, "owner": { "login": "Owner" } },
        "number": number,
        "title": format!("{repository} PR"),
        "body": "PR description",
        "headRefName": format!("topic/{number}"),
        "baseRefName": "main",
        "url": format!("https://github.com/Owner/{repository}/pull/{number}"),
        "isDraft": true,
        "merged": false,
        "headRepositoryOwner": { "login": head_owner },
    })
}
