//! The model-facing contract for local revision-scoped commands.

use clap::{Arg, ArgAction, Command};

/// Defines inputs, examples, results, and recovery guidance for each change command.
pub(in crate::commands) fn change_command() -> Command {
    let revision = || {
        Arg::new("revision")
            .value_name("REVISION")
            .required(true)
            .help("One JJ change ID, bookmark, commit hash, or revset; quote expressions")
    };
    Command::new("change")
        .about("Read, patch, and validate JJ changes without switching the working copy")
        .long_about(GROUP_ABOUT)
        .after_long_help(details(GROUP_HELP))
        .subcommand_required(true)
        .arg(Arg::new("json").long("json").global(true).action(ArgAction::SetTrue)
            .help("Write one JSON result or operation-error object to stdout"))
        .subcommand(Command::new("new")
            .about("Create a described change without switching @")
            .long_about(NEW_ABOUT)
            .after_long_help(details(NEW_HELP))
            .arg(Arg::new("from").long("from").value_name("PARENT_REVISION").required(true)
                .help("One parent revision; the new change inherits its recorded file tree"))
            .arg(Arg::new("message").short('m').long("message").value_name("DESCRIPTION").required(true)
                .help("Non-empty commit description; quote spaces and newlines")))
        .subcommand(Command::new("read")
            .about("Read a regular UTF-8 file from a pinned revision")
            .long_about(READ_ABOUT)
            .after_long_help(details(READ_HELP))
            .arg(revision())
            .arg(Arg::new("path").value_name("PATH").required(true)
                .help("Literal repository-root-relative file path, not a fileset")))
        .subcommand(Command::new("search")
            .about("Search a pinned revision using JJ string-pattern syntax")
            .long_about(SEARCH_ABOUT)
            .after_long_help(details(SEARCH_HELP))
            .arg(revision())
            .arg(Arg::new("pattern").short('p').long("pattern").value_name("PATTERN").required(true)
                .help("Single-line pattern: regex by default, or use an explicit kind:pattern")))
        .subcommand(Command::new("apply")
            .about("Apply an atomic file batch, rejecting a stale change")
            .long_about(APPLY_ABOUT)
            .after_long_help(details(APPLY_HELP))
            .arg(revision())
            .arg(Arg::new("expect").long("expect").value_name("COMMIT_HASH").required(true)
                .help("Exact full snapshot.commit_id from a previous read or mutation, not a change ID"))
            .arg(Arg::new("stdin").long("stdin").required(true).action(ArgAction::SetTrue)
                .help("Read one JSON batch from stdin; close stdin after the object")))
        .subcommand(Command::new("run")
            .about("Run a command in an isolated revision checkout without saving file changes")
            .long_about(RUN_ABOUT)
            .after_long_help(details(RUN_HELP))
            .arg(revision())
            .arg(Arg::new("command").value_name("COMMAND").required(true).num_args(1..).trailing_var_arg(true)
                .help("Program and arguments after --; put jx options before --")))
}

fn details(command_help: &str) -> String {
    format!("{command_help}\n\n{SHARED_CONTRACT}")
}

const GROUP_ABOUT: &str = "Read, patch, and validate recorded JJ revisions without switching @ or snapshotting working files. Run from inside a JJ workspace; no origin remote or GitHub credentials are required. Search and run also require an installed jj with those native commands.";

const GROUP_HELP: &str = r#"Example workflow:
  Assumes README.md exists. Replace <...> placeholders using preceding JSON
  responses; do not pass the placeholders literally. The stdin block is input to
  apply, not a shell command.

  jx change new --from @ --message 'Agent edit' --json
  jx change read <CHANGE_ID> README.md --json
  jx change apply <CHANGE_ID> --expect <READ_COMMIT_HASH> --stdin --json
  stdin:
  {"operations":[{"op":"write","path":"agent-note.txt","content":"hello\n"}]}
  jx change run <APPLIED_COMMIT_HASH> --json -- sh -c 'test -s agent-note.txt'

  Take CHANGE_ID from new's snapshot.change_id, READ_COMMIT_HASH from read's
  snapshot.commit_id, and APPLIED_COMMIT_HASH from apply's snapshot.commit_id.
  An apply result with conflicts needs attention even if its exit status is zero.
  Use the latest returned commit hash for every subsequent edit or validation.

Use each subcommand's --help for its input schema, output example, and plain-text
behavior. Use --help rather than -h to include this full contract."#;

const NEW_ABOUT: &str = "Create one described child of the selected parent. It initially has the parent's recorded file tree and no file delta. The parent may be immutable. The new change is not checked out, and unsnapshotted working files are not captured. The description must contain non-whitespace text.";

const NEW_HELP: &str = r#"Example:
  jx change new --from @ --message 'Agent edit' --json

JSON result (identifier placeholders are illustrative):
{"snapshot":{"change_id":"<CHANGE_ID>","commit_id":"<COMMIT_HASH>","operation_id":"<OPERATION_ID>"},"changed":true,"conflicted_changes":[]}

changed is true for a newly created change. conflicted_changes contains its change
ID if it inherited conflicts. Use snapshot.change_id as the target of later apply
requests. Without --json, print the change ID and commit hash, followed by a conflict
summary if needed. After an uncertain failure, inspect existing changes before
retrying new: blindly retrying can create a second change."#;

const READ_ABOUT: &str = "Read one existing, unconflicted, regular UTF-8 file from a recorded revision. Paths are literal repository-root-relative names, not filesets. This does not read or snapshot working files. Pass a full commit hash for a repeatable read; a change ID resolves its live version at the start of the command.";

const READ_HELP: &str = r#"Example:
  jx change read <CHANGE_ID> README.md --json

JSON result:
{"snapshot":{"change_id":"<CHANGE_ID>","commit_id":"<COMMIT_HASH>","operation_id":"<OPERATION_ID>"},"path":"README.md","content":"before\n"}

content is the complete file text, with newlines escaped by JSON. Use this response's
snapshot.commit_id as apply's --expect value. Re-read after stale_change rather
than replacing the version guard on a patch based on old text. Missing files,
directories, symlinks, conflicted file values, and invalid UTF-8 are refused.
Without --json, stdout is exactly the file content, with no added newline."#;

const SEARCH_ABOUT: &str = "Search all files in one recorded revision through jj file search. Patterns match one line at a time. This does not search or snapshot working files. Pass a full commit hash to search the same version as an earlier read.";

const SEARCH_HELP: &str = r#"Pattern syntax:
  regex:EXPR       Regular-expression search; also the default without a kind.
  substring:TEXT   Literal substring anywhere in a line.
  exact:TEXT       Match a whole line exactly.
  glob:PATTERN     Match a whole line with shell wildcards; use * around a substring.
  Add -i to any kind for case-insensitive matching: regex-i:, substring-i:,
  exact-i:, glob-i:. Quote patterns to avoid shell expansion. Use an explicit kind
  when the pattern itself contains a colon. Patterns do not match across lines.

Examples:
  jx change search <COMMIT_HASH> --pattern 'substring:before' --json
  jx change search <COMMIT_HASH> --pattern 'regex:validate(_input)?' --json
  jx change search <COMMIT_HASH> --pattern 'glob:*TODO*' --json
  jx change search <COMMIT_HASH> --pattern 'substring-i:BEFORE' --json

JSON result:
{"snapshot":{"change_id":"<CHANGE_ID>","commit_id":"<COMMIT_HASH>","operation_id":"<OPERATION_ID>"},"output":"README.md:1:before\n"}

output is JJ-formatted text with paths and 1-based line numbers, not an array of
structured matches. Display details follow JJ; do not split filenames on colons.
No matches is success with output="". Invalid patterns or native JJ failures use
an error object. Without --json, stdout is the same search text."#;

const APPLY_ABOUT: &str = "Apply an ordered JSON file batch to one live, mutable, unconflicted change. The selected version must equal the full commit hash supplied as --expect. Working files are never snapshotted or checked out. A content-changing batch is refused if its target is a registered working-copy commit or an ancestor of one: move affected workspaces off the stack first. This avoids native stale-copy recovery overwriting later disk edits. Descendants are rebased normally, and any conflicts in the resulting subtree are reported.";

const APPLY_HELP: &str = r#"Example:
  jx change apply <CHANGE_ID> --expect <COMMIT_HASH> --stdin --json

Stdin example (one JSON object, then EOF):
{"operations":[{"op":"edit","path":"README.md","edits":[{"old_text":"before","new_text":"after"}]},{"op":"write","path":"agent-note.txt","content":"hello\n"},{"op":"move","from":"old.txt","to":"renamed.txt"},{"op":"delete","path":"obsolete.txt"}]}

Batch schema:
  operations must be a non-empty array. Unknown fields are rejected.
  edit:   {"op":"edit","path":"PATH","edits":[{"old_text":"OLD","new_text":"NEW"}]}
          edits must be a non-empty array, and old_text must not be empty.
          Each old_text must occur exactly once, including overlapping occurrences.
          All edits in one operation match its original text, not each other's
          results; their ranges must not overlap. new_text may be empty.
  write:  {"op":"write","path":"PATH","content":"UTF-8 text"}
          Create or replace a regular file. New files are non-executable; existing
          executable modes are preserved. Parent directories are created in the tree.
  move:   {"op":"move","from":"SOURCE","to":"DESTINATION"}
          Move a regular file, preserving content and executable mode. The destination
          must not exist; source and destination must differ.
  delete: {"op":"delete","path":"PATH"}
          Remove an existing regular file, not a directory or symlink.

Operations run in array order and see previous operations' results. Read/edit text
must be UTF-8; write content is a JSON string. Paths follow the shared input
conventions below. Ignore and auto-track settings do not filter explicit
operations. Validation and batch-operation failures publish nothing: the complete
batch is applied as one revision rewrite, or none of its file edits are published.
For an interrupted or failed publication, inspect state before retrying.

JSON result:
{"snapshot":{"change_id":"<CHANGE_ID>","commit_id":"<NEW_COMMIT_HASH>","operation_id":"<OPERATION_ID>"},"changed":true,"conflicted_changes":[]}

changed=false means the final tree is unchanged; no rewrite was published. Such a
no-op is allowed on an occupied stack. conflicted_changes lists change IDs for all
conflicted revisions in the resulting subtree, including pre-existing descendant
conflicts. A non-empty list is a successful mutation, not a rollback. Stop and inspect
those changes before publishing code. Reuse the new snapshot.commit_id for later work.
Without --json, print the change ID and commit hash, an unchanged marker when needed,
and a conflict summary when present.

Recovery:
  stale_change: re-read the live change and rebuild the batch from fresh text.
  checked_out_changes: inspect error.workspaces; move those workspaces off the stack
  deliberately, then re-read. Do not discard or forget working copies automatically.
  Divergent/abandoned changes, immutable targets, and targets with conflicts are not
  editable through this command. Use JJ to reconcile or select another live target."#;

const RUN_ABOUT: &str = "Run a program against an isolated, fully materialized revision checkout through jj run --ignore-changes. The command starts at the revision root; it does not inherit the caller's subdirectory. Use snapshot.commit_id to validate the exact edited version. Put jx options before --, then supply the program and arguments. No shell is added implicitly, no program stdin is supplied, and file changes are never saved into the revision. This is not a sandbox for network access or other external effects.";

const RUN_HELP: &str = r#"Examples:
  jx change run <COMMIT_HASH> --json -- printf 'ok\n'
  jx change run <COMMIT_HASH> --json -- sh -c 'test -s agent-note.txt'

JSON result (stderr is illustrative and may include native JJ diagnostics):
{"snapshot":{"change_id":"<CHANGE_ID>","commit_id":"<COMMIT_HASH>","operation_id":"<OPERATION_ID>"},"stdout":"ok\n","stderr":"<JJ_DIAGNOSTICS>","exit_code":0}

stdout and stderr capture program output and JJ diagnostics as text; invalid UTF-8
bytes are replaced. Do not use this command as a binary transport. A failed program
normally returns this same result shape with a nonzero exit_code, not an error
object. Failure to resolve the revision or start JJ uses an error object instead.
exit_code is the native JJ process status, also used as jx's exit status. It is not
necessarily the program's original exit status: a program exiting 23 normally makes
JJ return 1. If no numeric status is available, jx uses 1.

No repository mutation lock is held during execution. Another agent may edit the
live change meanwhile; the result still describes the pinned snapshot, not the
newer version. JJ may reuse checkout slots and build artifacts. --ignore-changes
prevents revision writes; it does not guarantee an empty, pristine build directory.
Without --json, forward captured stdout and stderr to their respective streams."#;

const SHARED_CONTRACT: &str = r#"Shared input conventions:
  Run inside a JJ workspace. Each revision must resolve to exactly one commit: use
  a JJ change ID, bookmark, full commit hash, or quoted revset such as 'trunk()'.
  @ means the recorded working-copy commit, not unsnapshotted files. Read/apply paths
  are forward-slash, repository-root-relative literals, not filesets. Absolute paths,
  empty components, '.', '..', backslashes, and .git or .jj components are refused.
  Read/edit require UTF-8 text; symlinks are not editable.

Shared JSON contract:
  --json writes one object to stdout. Parse stdout only; stderr may contain diagnostics.
  Successful results contain snapshot:
    change_id: logical change identity, retained across rewrites; divergence can make
               it resolve to multiple commits.
    commit_id: immutable full version hash. Use it to pin read/search/run and as
               apply's --expect value. Do not abbreviate it or substitute a change ID.
    operation_id: repository operation snapshot for context, not an edit guard.
  An unrelated operation does not invalidate a still-current target commit hash.
  Identifier placeholders in help examples are illustrative, not literal inputs.

Operation errors:
{"error":{"code":"stale_change","message":"...","expected":"<OLD_COMMIT_HASH>","actual":"<CURRENT_COMMIT_HASH>"}}

  Every operation error has code and message. Branch on code, not message text.
  stale_change: expected/actual are full commit hashes; re-read and rebuild the patch.
  checked_out_changes: workspaces lists affected workspace names; move them off the
    stack deliberately, preserving files, then re-read.
  divergent_change: reconcile the live versions with JJ before editing.
  abandoned_change: select a live change; an old commit hash cannot bypass this check.
  concurrent_operations: inspect and reconcile JJ operation state, preserving working
    files, then re-read. Do not automatically restore or discard operation history.
  repository_busy: lock wait timed out; wait, re-read, and retry if still appropriate.
  interrupted: inspect whether the operation completed before retrying.
  invalid_request: fix the batch, path, file type/encoding, or target constraints.
  jj_failed: inspect the native JJ message; fix the pattern, command support, or setup.
  repository_error: inspect the repository diagnostic and current state before retrying.

Exit statuses:
  0: success; new/apply may still report conflicted_changes.
  1: operation failure for new/read/search/apply.
  run: native JJ status as described above; failed execution can still return a run
       result rather than an error object.
  2: command-line usage error, normally plain stderr rather than a JSON error object.
  130: cooperative interruption; any completed JSON result may still be present.
  Startup failures can also be plain stderr. Do not assume every nonzero exit has JSON.
  Do not blindly replay new/apply after an uncertain publication or interruption.
  The repo lock coordinates participating jx clients, not bare jj commands or raw
  filesystem edits."#;

#[cfg(test)]
#[path = "../tests/change_help.rs"]
mod tests;
