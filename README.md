# jx

`jx` is a layout-aware companion for
[Jujutsu](https://jj-vcs.github.io/jj/latest/) and GitHub. It keeps `jj` as the
source of truth for commits, workspaces, and bookmarks, then adds the workflow
context that usually lives in shell habits: where repositories belong, how
parallel workspaces are named, which projects can be updated together, and how a
local task turns into a pull request or pull-request stack.

The core idea is that your code is not just one current checkout. Once `jx` knows
your layout, it can treat your primary repositories and managed workspaces as one
indexed working set: clone into predictable paths, jump to projects by key, check
remote state across repos, safely fetch or sync eligible checkouts, and keep task
identity attached from workspace creation through PR publishing. That makes it a
fit for day-to-day work across many repositories, not only for one-off commands
inside the directory you already happen to be in.

## What it helps with

- **Layout-aware project work**: clone GitHub repositories into configured roots,
  manage hidden parallel workspaces, and target primary checkouts from anywhere.
- **Safe multi-repository maintenance**: inspect remote state, fetch eligible
  repositories, and sync writable repositories without hand-rolling shell loops.
- **Task-aware workspaces**: create workspaces that carry a task id into local
  metadata so PR bookmarks and publishing can use the same task context later.
- **Stacked pull request management**: publish, display, move, refresh, and sync
  PR stacks from local jj ancestry while keeping GitHub bases and descriptions
  aligned.
- **Jujutsu-first daily flow**: read the current stack, inspect status, diff with
  configured renderers, fetch/rebase around `origin`, and push tracked bookmark
  state without replacing `jj`.
- **GitHub publishing**: reuse or create same-repository bookmarks, derive PR
  title/body from jj descriptions, apply labels, and suggest reviewers from
  repo/file ownership rules.

Run `jx --help` or `jx <command> --help` for exact flags and eligibility rules.
The README is a tour; detailed layout behavior lives in
[code layout](docs/code-layout.md), stack workflows are in
[stack management](docs/stack-management.md), and full configuration is in
[configuration options](docs/configuration.md).

## Layout changes the workflow

Without layout configuration, `jx` is still useful inside one jj checkout. With a
layout, it becomes a workspace-level tool:

- each repository has a stable identity and primary checkout path;
- managed workspaces live under a hidden sibling tree instead of cluttering the
  primary project list;
- project keys can be completed and used from outside the checkout;
- all-repository commands can scan only configured primary checkouts;
- conservative gates keep read-only, pull-needed, or misconfigured repositories
  visible without treating them like successful mutations.

This is what makes commands such as global remote status, all-repository fetch,
and conservative all-repository sync practical: `jx` knows which directories are
projects, which are workspaces, and which ones are safe candidates.

A small layout config turns repository names into destinations and shell targets:

```toml
[layout]
default_root = "~/src"
workspace_dir = ".work"

[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"
```

With that in place, `jx` can clone and find work by identity instead of by
remembered paths:

```sh
jx clone example-owner/api        # -> ~/work/api
jx work add fix-auth --task-id ABC-123
jx work                           # lists primary and managed workspaces
jx work root api@ABC-123-fix-auth # prints the managed workspace path
jx remote-status --all            # checks every primary checkout
jx fetch --all                    # fetches safe primary checkouts
jx sync --all                     # syncs eligible writable primary checkouts
```

Shell integration makes those layout keys interactive. Add the output of
`jx shell init bash` to your Bash startup file to get completion for project
arguments and, when configured, a navigation helper. Jumping between primary
repositories and managed workspaces becomes a completion-driven workflow rather
than a path-memory exercise.

## Task-aware work

For work repositories, task ids are part of the workflow rather than just branch
text. A task workspace can be named with the task id for completion and visual
scanning, while `jx` stores the task id as workspace-local metadata:

```sh
jx work add fix-auth --task-id ABC-123
jx stack publish
```

Later, PR publishing can use that metadata when planning generated bookmark
names, so the operator does not have to repeat the task id on every command. Use
command help for exact task-id flags and validation. The metadata format and
workspace naming behavior are documented in [code layout](docs/code-layout.md).

## Stack-aware PRs

`jx` treats stacked pull requests as a local workflow first. It records stack
relationships in `.jx/stack.toml`, derives parent/child ordering from jj commits
and bookmarks, and syncs GitHub PR bases and generated stack context from that
local state. That keeps stack edits, PR publishing, interactive opening, and
repository sync on one model instead of separate shell steps.

Use `jx stack` for stack display, movement, and PR publishing, and `jx sync` for
pushing synchronized bookmark state. See
[stack management](docs/stack-management.md) for examples and operational notes.

## Workflow at a glance

Inspect local work:

```sh
jx
jx status
jx diff
```

Manage repositories and workspaces in the configured layout:

```sh
jx clone example-owner/example-repo
jx work
```

Understand and update remote state:

```sh
jx remote-status
jx fetch
jx sync
```

Publish and manage PR stacks:

```sh
jx push
jx stack
```

For better ergonomics, expose the same workflows through `jj` aliases in
`~/.config/jj/config.toml` so `jj` remains the single entry point:

```toml
[aliases]
st = ["util", "exec", "--", "jx", "status"]
dx = ["util", "exec", "--", "jx", "diff"]
stack = ["util", "exec", "--", "jx", "stack"]
sync = ["util", "exec", "--", "jx", "sync"]
push = ["util", "exec", "--", "jx", "push"]
```

Choose alias names that fit your existing jj config; the important part is that
`jj util exec -- jx ...` keeps the workflow reachable from the jj command
surface.

## Scope

`jx` is intentionally narrow:

- `jj` commits and bookmarks are the local workflow model.
- GitHub publishing uses the fixed `origin` remote.
- PR heads are pushed to the same GitHub repository, not a fork.
- Configurable remotes and configurable bookmark roots are out of scope.

Those constraints keep the tool predictable: if a workflow needs broad `jj`
control, use `jj` directly.

## Configuration

Configuration is optional. Without config, `jx` uses `origin`, clones GitHub
shorthands under `~/src`, reads tokens from `JX_GITHUB_TOKEN`, `GH_TOKEN`, then
`GITHUB_TOKEN`, and applies no default reviewers. Layout configuration is the
piece that turns `jx` from a per-repository helper into a multi-repository
workspace tool.

Terminal links use OSC8 hyperlinks and the jj `link` color label. `jx`
underlines links by default; override `colors.link` in `~/.config/jj/config.toml`
if you prefer a different visual style.

Config files are TOML and compose in this order:

1. `~/.config/jx/*.toml`, lexically sorted
2. workspace-root `.jx/config.toml`

Supported config covers clone/workspace layout, repo policy, lifecycle checks,
reviewers, file-based reviewer rules, named diff renderers, and optional
keychain token lookup:

```toml
[layout]
default_root = "~/src"

[[layout.rules]]
source = "github"
owner = "example-owner"
root = "~/work"
path = "{repo}"

[repo]
reviewers = ["example-reviewer", "ExampleOrg/platform"]
workspace_shared_paths = [".pi"]

[[repo.checks]]
id = "generated-sources"
before = ["pull_request", "push", "sync"]
paths = ["schema/**", "src/generated/**"]
command = ["./scripts/check-generated"]

[[repo.rules]]
repo = "example-owner/*"
advance_trunk = true
workspace_shared_paths = [".local-tool-state"]

[[repo.rules.reviewer_rules]]
paths = ["foo/bar/**", "bar/bux/*.py"]
reviewers = ["work-reviewer", "ExampleOrg/frontend"]

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

[auth.keychain]
service = "jx-example"
account = "example-user"
```

Notes:

- Layout rules place clones and managed workspaces by normalized source, owner,
  and repo identity.
- Repo rules match the fixed `origin` GitHub repo with `owner/repo` globs.
- Repo checks run check-only commands before selected lifecycle operations when
  changed files match configured globs.
- `workspace_shared_paths` symlink existing local-only paths such as `.pi` from
  the primary checkout into managed `jx work add` workspaces; missing sources
  are skipped, and configured paths must be untracked at the exact selected
  checkout path.
- Reviewers may be GitHub users or teams written as `org/team`.
- Reviewer rules match changed-file globs within their repo policy.
- Diff tools are selected by name; `external` tools compare jj's generated
  left/right trees, while `pipe` tools consume a `jj diff` stream on stdin.
- Extra renderer arguments after `jx diff -- ...` are appended to the configured
  renderer arguments.
- Scalar settings such as `diff.default_tool` and keychain lookup use the last
  configured value.

## Development

Use the thin `just` wrappers from the repository root:

```sh
just lint
just build
just test
just install
```

See [development guide](docs/development.md) for source organization and test layout conventions.
