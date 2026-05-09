use std::process::Command;

#[test]
fn binary_rejects_unknown_command() {
    // Verifies: Binary rejects unknown command.
    let output = Command::new(env!("CARGO_BIN_EXE_jx"))
        .arg("unknown")
        .output()
        .expect("run jx binary");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .expect("stderr is utf-8")
        .contains("unrecognized subcommand"));
}
