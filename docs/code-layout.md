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
  is provided.
- `jx work` lists, completes, resolves, adds, and removes locations in the
  configured layout. `jx work add` creates managed workspaces under the hidden
  workspace tree, and `jx work remove` refuses paths outside that managed tree.
- `jx remote-status` uses the current repository by default, can target one
  primary repository key, and can scan all configured primary repositories.
  `--repo` remains a glob filter for global scans.
- `jx fetch` uses the current repository by default, can target one primary
  repository key, and can scan every safe primary repository with `--all`.
- `jx sync` uses the current repository by default and can target one primary
  repository key. When run from an uninitialized or no-remote layout path, it can
  initialize the jj repo or infer the GitHub repository to create from the path.
- `jx shell init bash` exposes layout keys to shell completion. Navigation
  completion includes primary repositories and managed workspaces; project
  argument completion includes only primary repositories.

## Current repository versus layout repository

Commands with no project argument operate on the current working directory and
walk up to the enclosing `.jj` workspace. Project arguments resolve through the
global layout index first and then run the same command as if the process had
started in that repository's primary checkout.

This split lets `jx` stay predictable inside a workspace while still supporting
fast cross-repository workflows from anywhere with access to the configured
layout roots.
