# Configuration options

`jx` has two configuration surfaces:

- `jx` TOML config for clone layout and workflow behavior.
- jj config for command aliases and terminal styling.

## `jx` config files

Config files compose in this order:

1. `~/.config/jx/*.toml`, lexically sorted
2. workspace-root `.jx/config.toml`

Later scalar values override earlier ones. Lists such as reviewers are normalized
and deduplicated where the workflow expects sets.

## Clone and workspace layout

This section is the configuration reference. See the [code layout guide](code-layout.md)
for how layout discovery and project keys apply across commands.

`jx clone` normalizes shorthand inputs to a source, owner, and repo, then places
the checkout at `root/path`. `jx work` uses the same identity to place managed
workspaces at `root/workspace_dir/path/name`, keeping primary checkouts visible
while parallel work stays under the hidden workspace directory. Task workspaces
created with `jx work add --task-id` prefix that workspace name with the task id
and store the task id in workspace-local metadata for `jx stack publish`. `jx sync` can use
the same layout in reverse to initialize a local jj/Git repository and infer
private GitHub repository creation when a layout path has no repo or remotes.
Without config, `owner/repo` uses the built-in GitHub source and clones to
`~/src/github.com/owner/repo` with an SSH URL. When the current directory is a
configured layout prefix that fixes the missing source and owner, `jx clone repo`
uses that prefix to infer the full slug and clone into the matching child path.

```toml
[layout]
default_source = "github"
default_root = "~/src"
workspace_dir = ".work"

[layout.default]
path = "{host}/{owner}/{repo}"

[[layout.sources]]
name = "github"
provider = "github"
host = "github.com"
clone_url = "ssh"

[[layout.rules]]
source = "github"
owner = "example-org"
root = "~/work"
path = "{repo}"

[[layout.rules]]
source = "github"
owner = "example-user"
root = "~/projects"
path = "{repo}"
```

Supported clone URL formats are `ssh` and `https`. Explicit clone URLs keep their
input URL for the clone transport while still using normalized identity for
layout rule matching. Layout rules compose in config order; later matching rules
can override the root or path selected by earlier ones. Workspace names must use
letters, numbers, `_`, or `-` so they remain single safe path segments.

## Authentication

Without config, `jx` reads tokens from `JX_GITHUB_TOKEN`, `GH_TOKEN`, then
`GITHUB_TOKEN`.

Optional keychain lookup:

```toml
[auth.keychain]
service = "jx-example"
account = "example-user"
```

Environment tokens take precedence over configured keychain lookup.

## Repo policy

Repo policy matches the fixed `origin` GitHub repository. Unscoped `[repo]`
settings apply wherever that config file is loaded; `[[repo.rules]]` entries
match `owner/repo` globs:

```toml
[repo]
reviewers = ["example-reviewer", "ExampleOrg/platform"]
workspace_shared_paths = [".pi"]

[[repo.rules]]
repo = "example-owner/*"
advance_trunk = true
reviewers = ["owner-reviewer"]
workspace_shared_paths = [".local-tool-state"]
```

`advance_trunk` makes repository sync (`jx sync` or `jx sync --repo`) move the
local trunk bookmark to the newest contiguous stack commit with changes, a
non-empty description, and no conflicts before pushing tracked bookmarks, then
leaves an empty working-copy change on top when needed.

Check commands run before selected lifecycle operations when at least one
changed file matches the configured repo-relative glob patterns. Commands are
argv arrays, run from the workspace root, and must exit successfully without
changing the jj working-copy commit:

```toml
[[repo.checks]]
id = "generated-sources"
before = ["pull_request", "push", "sync"]
paths = ["schema/**", "src/generated/**"]
command = ["./scripts/check-generated"]

[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.checks]]
id = "api-contract"
before = ["pull_request"]
paths = ["api/**"]
command = ["./scripts/check-api-contract"]
```

Supported `before` values are `pull_request`, `push`, and `sync`. A failing
command prints its captured output and aborts the operation. If a command exits
successfully but modifies tracked working-copy content, `jx` aborts and leaves
the changes visible for review or revert.

Lifecycle hooks run configured mutating commands at selected repository workflow
points. Commands are argv arrays. `workspace.delete.before` hooks run after delete
confirmation but before the workspace is moved or removed, always with the
workspace being deleted as the current directory. Successful hooks are reported
in command output as `Event[hook-id]: ran ...`, including the command argv. Each hook start,
success, and error is appended to `~/.local/state/jx/jx-hooks.log` as JSONL; set
`JX_HOOK_LOG=/path/to/log` to override the path or `off` to disable this log. A
failing hook prints captured output, aborts deletion, and leaves the workspace
intact:

```toml
[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.hooks]]
id = "stop-build-server"
on = "workspace.delete.before"
command = ["build-tool", "shutdown"]

[[repo.rules.hooks]]
id = "clear-build-cache"
on = "workspace.delete.before"
command = ["build-tool", "clean", "--all"]
```

Matching rule hooks compose after base hooks. A matching rule can replace a
previous hook with the same `id`, or disable it with `id = "..."` and
`enabled = false`.

`workspace_shared_paths` lists repo-relative local-only paths that managed
`jx work add` workspaces should symlink from the primary checkout after jj
creates the workspace. This is intended for ignored checkout state such as `.pi`.
Paths compose through base repo policy and matching repo rules, normalize in
config order, and dedupe exact duplicates. Empty, absolute, escaping, and
parent/child-overlapping paths are rejected. Missing sources in the primary
checkout are skipped. Existing sources must be untracked in the selected checkout
at the exact configured path; tracked parent directories are allowed for nested
paths. If post-create setup fails, `jx` reports the failure without rolling back
the created jj workspace, and shell integration does not enter it.

Stack status and review views can classify repository-specific approval gate
checks separately from test health, highlight stale review wait time, omit noisy
checks, stack-status labels, or reviewer identities, report label-driven
auto-merge state, hide pre-merge-only labels after merge, and rewrite title
prefixes before display ellipsizing. Review views can also omit review-only
labels without affecting stack status, and ignore command-style author comments
that should not resurface dismissed reviews.
Matching review-gate checks are removed from the `Chk` aggregate and drive the
review state instead:
all configured gate globs must have passing matching checks for the PR to render
approved unless GitHub still reports a protected review requirement, while
missing, pending, unknown, or failing gate checks render as waiting review.
Ignored checks are removed without affecting check or review state. Remaining
checks still decide whether `Chk` is passing, pending, or failing. Review-wait
thresholds accept `m`, `h`, or `d` suffixes; fresh waits
render subdued, overdue waits render red, drafts stay subdued, and merged PRs
stay green. Check ignore and reviewer ignore entries are Rust regexes; review-gate
and label entries are globs. `auto_merge_labels` and `hidden_labels` use the
same glob syntax plus snapshot-backed `when` conditions such as `ALWAYS`,
`NOT_DRAFT`, `MERGED`, and `TARGETS_DEFAULT_BRANCH`. Configured auto-merge
labels are hidden from label chips; matching non-draft open PRs show `◎` when
armed, and otherwise-ready matching PRs show an orange `◆` to indicate that
auto-merge is not armed. Existing `ignored_labels` entries are unconditional hides, and
`ignored_labels_when_merged` entries are merged-only hides. Review rules support
the same `hidden_labels` shape for review-only omissions. Conditions
in one rule are ANDed; repeated rules for the same label are ORed. Supported
conditions are `ALWAYS`, `DRAFT`, `NOT_DRAFT`, `OPEN`, `CLOSED`, `MERGED`,
`NOT_MERGED`, `TARGETS_DEFAULT_BRANCH`, and `TARGETS_NON_DEFAULT_BRANCH`.
`ignored_author_response_comments` entries are multiline Rust regexes matched
against PR-author comment bodies before dismissal resurfacing. Local review
visibility state lives in the shared pull-request store; `review-dismissals.toml`
is no longer read. See [review management](review-management.md) for dismissal,
audit-log, and store behavior. Title rewrites use Rust regex capture
replacements:

```toml
[[repo.rules]]
repo = "example-owner/example-repo"

[repo.rules.stack_status]
ignored_checks = ["^ci/noisy-check$", "^generated-advisory/.*"]
ignored_labels = ["generated-*"]
ignored_labels_when_merged = ["auto-merge", "run-ci"]
auto_merge_labels = ["auto-merge"]
hidden_labels = [
  { label = "run-ci", when = ["NOT_DRAFT", "TARGETS_DEFAULT_BRANCH"] },
]
ignored_reviewers = ["^automation-bot$", "-bot$"]
review_gate_checks = ["approval gate"]
review_wait_threshold = "4h"

[repo.rules.review]
ignored_author_response_comments = ["^/automation merge\\s*$"]
ignored_labels = ["team-review"]
hidden_labels = [
  { label = "review-only-noise", when = ["ALWAYS"] },
]

[[repo.rules.stack_status.title_rewrites]]
pattern = "^\\[([A-Z]+-[0-9]+)\\] (.+)$"
replace = "$1: $2"
```

Event handlers run configured PR automation while `jx stack publish` prepares,
creates, or updates pull requests. Handlers can update the selected commit title, add
labels, or ask the command layer to open the PR in an operator browser. `when`
uses a small GitHub-search-like AND query with `has:task`, `is:draft`,
`is:ready`, `has:reviewers`, `label:name`, and `-term` negation:

```toml
[[repo.event_handlers]]
id = "prepend-task-id-to-commit-title"
on = "pull_request.prepare"
when = "has:task"
run = "prepend_task_id"

[[repo.event_handlers]]
id = "label-draft-prs"
on = "pull_request.created"
when = "is:draft -label:bar"
run = "add_labels"
labels = ["bar"]

[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.event_handlers]]
id = "open-unreviewed-prs"
on = "pull_request.created"
when = "-has:reviewers -is:draft"
run = "open_pull_request"
```

Matching rule handlers compose after base handlers. A matching rule can disable a
previous handler with `id = "..."` and `enabled = false`. Use
`jx stack publish --no-event-handlers` to disable all configured handlers for one run.
Default output reports handlers that changed PR or commit state, plus browser
open attempts; no-op matches are kept quiet. Prepare effects appear in the PR
preview, and create/update effects appear after publishing. `prepend_task_id`
rewrites the selected commit title before PR planning, using `TASK-ID: title`
and normalizing common existing task prefixes.

Work item handlers can run generic commands when `jx stack status` observes a
PR with `fixes_work_ids` transition to merged. Commands run from the repository
root, and each start, success, or error is appended to the central
`~/.local/state/jx/jx-work-item-handlers.log` JSONL log. Set
`JX_WORK_ITEM_HANDLER_LOG=/path/to/log` to override the path or `off` to disable
this log. The command is configured as an argument array, not a shell string, and supports
placeholders such as `{work_id}`, `{repo}`, `{pr_number}`, `{pr_url}`, `{title}`,
and `{branch}`:

```toml
[repo.rules.work_items]
apply_on_stack_status = true

[[repo.rules.work_item_handlers]]
id = "resolve-ticket"
on = "work_item.fixed"
command = ["ticket", "resolve", "{work_id}"]
```

Reviewers may be GitHub users or teams written as `org/team`. Repo-level
reviewer lists power shell completion for `jx stack publish --reviewer` only;
completion is advisory, so syntactically valid reviewers that are not configured
can still be typed explicitly.

Path reviewer rules add reviewers when changed-file globs match. These are the
configured reviewers that appear in the publish selection prompt. Each repo
policy can contain multiple path rules:

```toml
[[repo.path_reviewers]]
paths = ["docs/**"]
reviewers = ["ExampleOrg/docs"]

[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.path_reviewers]]
paths = ["foo/bar/**", "bar/bux/*.py"]
reviewers = ["work-reviewer", "ExampleOrg/frontend"]
```

## Performance tracing

Stack publishing writes best-effort JSONL performance spans to
`~/.local/state/jx/jx-perf.log`, or `$XDG_STATE_HOME/jx/jx-perf.log` when
`XDG_STATE_HOME` is set. Set `JX_PERF_LOG=/path/to/jx-perf.log` to override the
path, or `JX_PERF_LOG=off` to disable tracing for one command. The log records
command phase timings such as publish planning, GitHub PR publishing, and stack
metadata refresh/sync.

## Shell integration

`jx shell init bash` prints optional shell integration for `eval`. The generated
navigation function resolves current-repository layout workspace aliases and
trunk aliases first, then global `jx work` locations, then optionally falls back
to zoxide when `zoxide = "auto"` and the `zoxide` binary is installed. Explicit
absolute and dot-relative paths are used directly. Navigation completion derives
same-repository workspace aliases from configured layout paths and discovered
`.jj` directories rather than the jj workspace registry, so a matching managed
directory can appear before it is registered as a jj workspace and an
out-of-layout jj workspace may appear only by its global key. Navigation queries
can also be unique key fragments, and slash-separated fragments can select child
directories under the matched location. When `fzf` is installed, pressing Tab for
the navigation command opens an interactive picker over navigation candidates;
typed text such as `u foo<Tab>` seeds the picker query with `foo`. Path-like
inputs such as `u ../<Tab>` keep normal directory completion.

```toml
[shell]
navigation = "u"
navigation_tab = "ut"
zoxide = "prefer"
```

Set `zoxide = "prefer"` to resolve zoxide matches before jx layout keys, while
keeping `default`, `trunk`, and `root` as jx-first aliases. Set `zoxide = "auto"`
to use zoxide only as a fallback, or `zoxide = "never"` to omit zoxide. Omit
`navigation` or set it to an empty string to skip generating a navigation
function. When `navigation_tab` is set alongside `navigation`, the generated
companion command uses the same resolution and completion; inside zellij it opens
the target in a new tab, and outside zellij it warns and enters the directory in
the current shell.

## Diff tools

Named diff tools can be selected by config or command flag:

```toml
[diff]
default_tool = "difft"

[diff.tools.difft]
mode = "external"
command = "difft"
args = ["--color=always", "--display=side-by-side"]

[diff.tools.delta]
mode = "pipe"
producer_args = ["-w", "--git"]
command = "delta"
args = []
```

`external` tools compare jj's generated left/right trees. `pipe` tools consume a
`jj diff` stream on stdin. Extra renderer arguments after `jx diff -- ...` are
appended to the configured renderer arguments.

## jj aliases

Use jj aliases if you want `jj` to remain the single command entry point:

```toml
[aliases]
st = ["util", "exec", "--", "jx", "status"]
dx = ["util", "exec", "--", "jx", "diff"]
pr = ["util", "exec", "--", "jx", "pr"]
sync = ["util", "exec", "--", "jx", "sync"]
push = ["util", "exec", "--", "jx", "push"]
```

Choose names that fit your existing jj config.

## Link styling

`jx` wraps GitHub URLs and bookmark names in OSC8 terminal hyperlinks. Linked
text uses jj's `link` color label and is underlined by default.

Override the default in `~/.config/jj/config.toml`:

```toml
[colors]
link = { underline = false }
```

Or keep the affordance but make it more visible:

```toml
[colors]
link = { underline = true, bold = true }
"bookmark link" = { underline = true, fg = "bright magenta" }
```

The `link` label stacks with existing labels such as `bookmark`, so link
styling can add an affordance without replacing normal jj colors.
