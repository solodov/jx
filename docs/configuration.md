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

`jx clone` normalizes shorthand inputs to a source, owner, and repo, then places
the checkout at `root/path`. `jx work` uses the same identity to place managed
workspaces at `root/workspace_dir/path/name`, keeping primary checkouts visible
while parallel work stays under the hidden workspace directory. `jx sync` uses
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

[[repo.rules]]
repo = "example-owner/*"
advance_trunk = true
reviewers = ["owner-reviewer"]
```

`advance_trunk` makes `jx sync` move the local trunk bookmark to the newest
contiguous stack commit with both changes and a non-empty description before
pushing tracked bookmarks, then leaves an empty working-copy change on top when
needed.

Reviewers may be GitHub users or teams written as `org/team`.

Path reviewer rules add reviewers when changed-file globs match. Each repo
policy can contain multiple path rules:

```toml
[[repo.reviewer_rules]]
paths = ["docs/**"]
reviewers = ["ExampleOrg/docs"]

[[repo.rules]]
repo = "example-owner/example-repo"

[[repo.rules.reviewer_rules]]
paths = ["foo/bar/**", "bar/bux/*.py"]
reviewers = ["work-reviewer", "ExampleOrg/frontend"]
```

## Shell integration

`jx shell init bash` prints optional shell integration for `eval`. The generated
navigation function resolves `jx work` locations first, then optionally falls
back to zoxide when `zoxide = "auto"` and the `zoxide` binary is installed.

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
