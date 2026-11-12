use super::super::change_error_value;
use crate::jj::JjError;

#[test]
fn shared_help_covers_every_operation_error_code() {
    let help = super::change_command().render_long_help().to_string();
    let errors = [
        JjError::StaleChange {
            expected: "old".into(),
            actual: "new".into(),
        },
        JjError::MutationBusy {
            path: "repo.lock".into(),
        },
        JjError::ConcurrentOperations,
        JjError::DivergentChange {
            change_id: "change".into(),
        },
        JjError::AbandonedChange {
            change_id: "change".into(),
        },
        JjError::CheckedOutChanges {
            workspaces: vec!["default".into()],
        },
        JjError::Interrupted,
        JjError::ChangeRequest {
            message: "invalid".into(),
        },
        JjError::ChangeCommand {
            message: "failed".into(),
        },
        JjError::Backend {
            message: "failed".into(),
        },
    ];
    for error in errors {
        let value = change_error_value(&error);
        let code = value["code"].as_str().unwrap();
        assert!(
            help.contains(&format!("{code}:")),
            "undocumented error code {code}"
        );
    }
}
