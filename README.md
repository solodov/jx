# jx

`jx` stands for **Jj eXtended**: a small, opinionated companion for everyday
[Jujutsu](https://jj-vcs.github.io/jj/latest/) workflows. It keeps `jj` as the
source of truth, but smooths over the repeated glue work
around reading the current stack, updating from `origin`, pushing bookmarks, and
publishing GitHub pull requests.

The motivation is simple: common jj workflows are powerful, but the happy path
can still feel like stitching together local revsets, Git transport, GitHub
state, code layout, and repository conventions. `jx` makes that path feel like
one workflow without becoming a replacement for `jj`.

`jx` also models a local code layout: it can place primary repository clones,
manage hidden parallel workspaces, target configured projects from outside their
checkout, and run safe maintenance across every known primary checkout.

## What it helps with

- **See the right local context**: workspace-scoped log, current status, and
  focused diffs that can skip tests or use a configured renderer.
- **Understand and navigate remote state**: compare local `origin` trunk state
  with GitHub and open repositories or PR lists from layout keys.
- **Update safely**: fetch from `origin`, then rebase or repair local jj work
  around the updated trunk; with a configured layout, fetch or sync all eligible
  primary checkouts.
- **Clone, bootstrap, and branch out in a consistent layout**: expand
  repository shorthands into configured local roots, place parallel workspaces
  under the hidden workspace layout, and infer new GitHub repositories from that
  layout.
- **Publish with fewer steps**: reuse or create bookmarks, push selected work or
  tracked bookmark state, and create/update same-repository GitHub PRs.
- **Keep review setup repeatable**: derive PR title/body from the jj description
  and suggest configured reviewers from file ownership rules.

Run `jx --help` or `jx <command> --help` for the exact command and flag details.
The README is intentionally a tour, not the full manual.

## Workflow at a glance

Inspect local work:

```sh
jx
jx status
jx diff
```

Clone a repository or manage parallel workspaces in the configured layout:

```sh
jx clone example-owner/example-repo
jx work
```

Create or update a pull request:

```sh
jj edit <change-or-bookmark>
jx pr
```

Fetch latest changes and update existing PR heads:

```sh
jx sync
```

`jx sync` fetches from `origin`, rebases or repairs local jj work onto the
updated trunk, then pushes tracked `origin` bookmark updates, including
deletions. It stops before pushing if the fetch/rebase step creates conflicts.
With a configured layout, `jx fetch --all` and `jx sync --all` apply the same
idea across safe primary checkouts; see [code layout](docs/code-layout.md) for
the eligibility rules.

Push the current commit after more changes:

```sh
jx push
```

Check whether GitHub has moved ahead of local `origin` state:

```sh
jx remote-status
```

For better ergonomics, expose the same workflows through `jj` aliases in
`~/.config/jj/config.toml` so `jj` remains the single entry point:

```toml
[aliases]
st = ["util", "exec", "--", "jx", "status"]
dx = ["util", "exec", "--", "jx", "diff"]
pr = ["util", "exec", "--", "jx", "pr"]
sync = ["util", "exec", "--", "jx", "sync"]
push = ["util", "exec", "--", "jx", "push"]
```

Choose alias names that fit your existing jj config; the important part is that
`jj util exec -- jx ...` keeps the workflow reachable from the jj command
surface.

## Assumptions

`jx` is intentionally narrow:

- `jj` commits and bookmarks are the local workflow model.
- GitHub publishing uses the fixed `origin` remote.
- PR heads are pushed to the same GitHub repository, not a fork.
- Hooks, configurable remotes, and configurable bookmark roots are out of scope.

Those constraints keep the tool predictable: if a workflow needs broad `jj`
control, use `jj` directly.

## Configuration

Configuration is optional. Without config, `jx` uses `origin`, clones GitHub
shorthands under `~/src`, reads tokens from `JX_GITHUB_TOKEN`, `GH_TOKEN`, then
`GITHUB_TOKEN`, and applies no default reviewers. See
[configuration options](docs/configuration.md) for the full configuration
reference and [code layout](docs/code-layout.md) for how configured projects are
found and shared across commands.

Terminal links use OSC8 hyperlinks and the jj `link` color label. `jx`
underlines links by default; override `colors.link` in `~/.config/jj/config.toml`
if you prefer a different visual style.

Config files are TOML and compose in this order:

1. `~/.config/jx/*.toml`, lexically sorted
2. workspace-root `.jx.toml`

Supported config covers clone/workspace layout, repo policy, reviewers,
file-based reviewer rules, named diff renderers, and optional keychain token
lookup:

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

[[repo.rules]]
repo = "example-owner/*"
advance_trunk = true

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
