use super::*;
use serde_json::json;

#[test]
fn replacements_use_original_offsets() {
    let edits = vec![
        FileReplacement {
            old_text: "alpha".into(),
            new_text: "a much longer prefix".into(),
        },
        FileReplacement {
            old_text: "gamma".into(),
            new_text: "g".into(),
        },
    ];
    assert_eq!(
        replace_exact("alpha beta gamma\n", &edits).unwrap(),
        "a much longer prefix beta g\n"
    );
}

#[test]
fn replacements_reject_missing_duplicate_and_overlapping_matches() {
    for edits in [
        json!([{ "old_text": "missing", "new_text": "x" }]),
        json!([{ "old_text": "a", "new_text": "x" }]),
        json!([{ "old_text": "abc", "new_text": "x" }, { "old_text": "bc", "new_text": "y" }]),
    ] {
        let edits = serde_json::from_value::<Vec<FileReplacement>>(edits).unwrap();
        assert!(replace_exact("abc abcde", &edits).is_err());
    }
}

#[test]
fn overlapping_occurrences_and_replacements_are_rejected() {
    let repeated = vec![FileReplacement {
        old_text: "aa".into(),
        new_text: "x".into(),
    }];
    assert!(replace_exact("aaa", &repeated).is_err());
    let repeated_unicode = vec![FileReplacement {
        old_text: "éé".into(),
        new_text: "x".into(),
    }];
    assert!(replace_exact("ééé", &repeated_unicode).is_err());
    let overlapping = vec![
        FileReplacement {
            old_text: "abc".into(),
            new_text: "x".into(),
        },
        FileReplacement {
            old_text: "bc".into(),
            new_text: "y".into(),
        },
    ];
    assert!(replace_exact("abcdef", &overlapping).is_err());
}

#[test]
fn empty_matches_and_empty_batches_are_rejected() {
    for operations in [
        json!([]),
        json!([{ "op": "edit", "path": "file", "edits": [] }]),
        json!([{ "op": "edit", "path": "file", "edits": [{ "old_text": "", "new_text": "x" }] }]),
    ] {
        let batch: ChangeBatch =
            serde_json::from_value(json!({ "operations": operations })).unwrap();
        assert!(batch.validate().is_err());
    }
}

#[test]
fn rejects_path_escapes_and_repository_metadata() {
    for path in [
        "",
        "../outside",
        "/outside",
        "a/../outside",
        "./file",
        ".git/config",
        "nested/.jj/repo",
        "a\\b",
        "a//b",
    ] {
        assert!(validate_path(path).is_err(), "accepted {path:?}");
    }
    validate_path("src/example.rs").unwrap();
}

#[test]
fn misspelled_request_fields_are_not_silently_ignored() {
    assert!(serde_json::from_value::<ChangeBatch>(json!({ "operations": [{ "op": "write", "path": "file", "content": "x", "contents": "y" }] })).is_err());
}
