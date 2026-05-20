# Configuration options

`jx` has two configuration surfaces:

- `jx` TOML config for clone layout and workflow behavior.
- jj config for command aliases and terminal styling.

## `jx` config files

Config files compose in this order:

1. `~/.config/jx/*.toml`, lexically sorted
2. workspace-root `.jx.toml`

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
and store the task id in workspace-local metadata for `jx pr`. `jx sync` uses
the same layout in reverse to infer private GitHub repository creation when a
new local repo has no remotes. Without config, `owner/repo` uses the built-in
GitHub source and clones to `~/src/github.com/owner/repo` with an SSH URL.

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

`advance_trunk` makes `jx sync` move the local trunk bookmark to the newest
contiguous stack commit with both changes and a non-empty description before
pushing tracked bookmarks, then leaves an empty working-copy change on top when
needed.

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

Event handlers run configured PR automation while `jx pr` prepares, creates, or
updates a pull request. Handlers can update the selected commit title, add
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
`jx pr --no-event-handlers` to disable all configured handlers for one run.
`prepend_task_id` rewrites the selected commit title before PR planning, using
`TASK-ID: title` and normalizing common existing task prefixes.

Reviewers may be GitHub users or teams written as `org/team`.

Path reviewer rules add reviewers when changed-file globs match. Each repo
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

## Shell integration

`jx shell init bash` prints optional shell integration for `eval`. The generated
navigation function resolves current-repository jj workspace names and trunk
aliases first, then global `jx work` locations, then optionally falls back to
zoxide when `zoxide = "auto"` and the `zoxide` binary is installed.

```toml
[shell]
navigation = "u"
zoxide = "auto"
```

Set `zoxide = "never"` to omit zoxide fallback. Omit `navigation` or set it to
an empty string to skip generating a navigation function.

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
