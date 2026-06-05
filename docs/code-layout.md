# Code layout

`jx` is more than a thin wrapper around the current `jj` checkout. It also keeps
an index of configured code roots so commands can find, name, and operate on
multiple primary repository clones consistently.

The layout model is deliberately small: every repository has a normalized
identity, every identity maps to one visible primary checkout path, and managed
workspaces live under a hidden sibling tree derived from that same identity.

## Repository identity

Layout starts by normalizing repository inputs into four fields:

- `source` - a configured source name such as `github`
- `host` - the Git host such as `github.com`
- `owner` - the GitHub owner or organization
- `repo` - the repository name

Without configuration, `jx` has one built-in source:

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
```

That means `example-owner/example-repo` resolves to the identity
`github:example-owner/example-repo`, clones from
`git@github.com:example-owner/example-repo.git`, and by default lives at:

```text
~/src/github.com/example-owner/example-repo
```

Path templates may use `{source}`, `{host}`, `{owner}`, and `{repo}`. They must
render to relative paths without `.` or `..` components. Layout roots must be
absolute or start with `~/`.

## Primary checkouts and managed workspaces

For each repository identity, `jx` derives two related path families:

```text
primary checkout:     <root>/<path>
managed workspace:   <root>/<workspace_dir>/<path>/<workspace-name>
```

With the default layout, a `fix` workspace for `example-owner/example-repo` is:

```text
~/src/.work/github.com/example-owner/example-repo/fix
```

`workspace_dir` and workspace names are single path segments. Workspace names may
contain letters, numbers, `_`, and `-`.

## Layout rules

Rules override the default root and/or path for matching identities. They match a
single `source` and at least one of `owner` or `repo`. Rules compose in config
order; later matching rules override the root or path chosen by earlier matches.

```toml
[layout]
default_root = "~/src"
workspace_dir = ".work"

[layout.default]
path = "{host}/{owner}/{repo}"

[[layout.rules]]
source = "github"
owner = "example-org"
root = "~/work"
path = "{repo}"

[[layout.rules]]
source = "github"
owner = "example-org"
repo = "special-repo"
path = "special/{repo}"
```

This keeps most `example-org` repos under `~/work/<repo>`, while
`example-org/special-repo` lives under `~/work/special/special-repo`.

## Discovery and project keys

Global and project-targeted commands discover layout repositories by scanning the
configured layout roots for `.jj` workspaces. A discovered path is kept only when
it can be mapped back to either the primary checkout path or a managed workspace
path for one normalized identity.

`jx` assigns stable keys to discovered locations:

- `repo` when the repo name is unique
- `owner/repo` when multiple owners have the same repo name
- `source:owner/repo` when even `owner/repo` is ambiguous
- `repo@workspace` for managed workspaces

Primary repository commands use only keys without `@`. Managed workspace keys are
for navigation and workspace management.

## How commands use layout

- `jx clone` resolves repository shorthands through layout sources and places the
  primary checkout at the configured destination, unless an explicit destination
  is provided. From a configured layout prefix, a single repo name can infer the
  missing source and owner.
- `jx work` lists, completes, resolves, adds, and removes locations in the
  configured layout. `jx work add` creates managed workspaces under the hidden
  workspace tree, can prefix task workspaces with `--task-id`, and `jx work
  remove` refuses paths outside that managed tree.
- `jx remote-status` uses the current repository by default, can target one
  primary repository key, and can scan all configured primary repositories.
  `--repo` remains a glob filter for global scans.
- `jx open` uses the current repository by default, can target one primary
  repository key, and can use `--repo` globs to open matching GitHub repository
  pages or matching pull-request searches.
- `jx fetch` uses the current repository by default, can target one primary
  repository key, and can scan every safe primary repository with `--all`.
- `jx sync` syncs tracked bookmarks for the current repository by default and
  applies repository policy such as trunk advancement. From an uninitialized
  layout path, it can prompt to initialize the local jj/Git repository before
  continuing bootstrap. Pass a jj revision or bookmark to sync one bookmarked
  target instead, use `jx sync --repo` to force repository mode explicitly, or
  use `jx sync --all` to conservatively sync every eligible primary repository.
- `jx stack publish` uses an explicit `--task-id` when present; otherwise it can
  read the task id stored in workspace-local metadata created by
  `jx work add --task-id`.
- `jx shell init bash` exposes layout keys to shell completion. Navigation
  completion prefers current-repository layout workspace aliases, `trunk`/`root`
  aliases, and same-repository layout keys before other global work locations;
  project argument completion includes only primary repositories. The navigation
  command accepts explicit absolute and dot-relative paths, and can also resolve
  unique key fragments plus slash-separated directory fragments under the
  selected location. In zoxide-prefer mode, zoxide matches win except for the
  `default`, `trunk`, and `root` jj aliases. An optional tab companion uses the
  same resolution and opens zellij tabs when available.

## Workspace metadata

Task workspaces keep the task id visible in navigation while storing the task
association as workspace-local metadata.

```sh
jx work add fix --task-id ABC-123
```

This creates a managed workspace whose directory and jj workspace name are both:

```text
ABC-123-fix
```

It also writes:

```text
<workspace-root>/.jx/.gitignore
<workspace-root>/.jx/workspace.toml
```

The `.gitignore` file ignores the whole `.jx` metadata directory, and
`workspace.toml` contains:

```toml
task_id = "ABC-123"
```

The visible workspace name makes completion entries such as `repo@ABC-123-fix`
scannable. The metadata file remains the source of truth for `jx stack publish`,
so the workspace name is not parsed for task information.

## All-repository fetch and sync

The layout index lets `jx` run maintenance commands over primary checkouts from
any directory. These global modes use only primary repository keys; managed
workspace keys with `@` remain navigation/workspace targets and are not scanned.

`jx fetch --all` is intentionally broad but local-work safe. For each configured
primary checkout, it discovers fixed-origin repository context and fetches only
when the current workspace is a clean empty child of `origin` trunk. Repositories
that are not discoverable or are not safe for automatic fetch are skipped;
repositories that fail after selection are rendered as error rows.

`jx sync --all` is narrower because it can push. It does not initialize missing
repositories, create GitHub repositories, or prompt. A repository is eligible
only when all of these gates pass:

1. The primary checkout is discoverable and already has a GitHub `origin`.
2. The GitHub token can push to `origin`.
3. GitHub `origin` is not ahead of the local cached trunk, and the origin branch
   is not diverged.

Eligible repositories run the normal repository sync steps: fetch origin,
optionally advance the local trunk bookmark according to repo policy, then push
tracked bookmark state whose push ranges have no conflicted commits. Conflicted
bookmarks are skipped and reported separately. Local jj work does not by itself
block `sync --all`; if GitHub `origin` has not moved ahead of the cached trunk,
pushing tracked bookmark state is still safe. Repos that do not fetch, advance
trunk, or push anything are grouped as up to date. Other skips are grouped by
reason so read-only third-party checkouts, pull-needed repos, and setup issues
remain visible without being treated as failures.

## Current repository versus layout repository

Commands with no project argument operate on the current working directory and
walk up to the enclosing `.jj` workspace. Project arguments resolve through the
global layout index first and then run the same command as if the process had
started in that repository's primary checkout.

This split lets `jx` stay predictable inside a workspace while still supporting
fast cross-repository workflows from anywhere with access to the configured
layout roots.
