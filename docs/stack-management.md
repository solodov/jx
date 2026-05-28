# Stack management

`jx` treats a pull-request stack as jj-first local state that can be projected to
GitHub. The commits, bookmarks, and ancestry in jj define the stack shape;
`.jx/stack.toml` keeps durable PR metadata so the shape remains understandable
after a parent bookmark disappears or a parent PR merges.

That local model drives the user-facing workflows:

- `jx pr` publishes or updates a PR, records it in stack metadata, and syncs the
  generated stack context for the affected stack component.
- `jx stack` shows stored stack state, opens stored PRs interactively, refreshes
  metadata from local bookmarks and GitHub, or moves the current stack.
- `jx sync -s` pushes the current stack component and updates PR descriptions
  from the same stack metadata.

Use `jj` for general commit editing. Use `jx stack` when changing stack graph
shape, because a raw `jj rebase` can move commits without updating
`.jx/stack.toml` or GitHub PR bases.

## Mental model

A stack node is a local bookmark that represents a PR head. Its parent is the PR
base branch when that base branch is another known PR head; otherwise the node is
a stack root based on trunk or another non-stack branch.

`jx` renders stack rows in merge order:

```text
✓ #10     Prepare API shape
◯ #12     Add service layer
└─ ◉ #13     Wire UI to service
```

Symbols are stable across CLI output and generated PR descriptions:

- `✓` merged
- `◉` current selection
- `◌` draft
- `◯` open

The stored metadata is a cache and coordination layer, not a replacement for jj
or GitHub. `jx` repairs it from local ancestry when publishing, refreshing,
syncing, or moving stacks.

## Publish a stack

Create commits and bookmarks with your normal jj flow, then publish each PR with
`jx pr`. When a published PR belongs to a stack, `jx` updates every known PR in
that connected component so GitHub descriptions show the same stack tree.

```sh
jx pr                 # publish or update the current change
jx stack              # show the locally stored stack
jx sync -s            # push/sync the current stack component later
```

Publishing a child PR records its base relationship. Publishing or updating any
PR in the component refreshes generated stack context for the component, including
root PRs whose body may need stale stack context removed.

## Refresh stack metadata

Use refresh when local bookmarks and GitHub PRs already exist but `.jx/stack.toml`
is missing, stale, or incomplete.

```sh
jx stack refresh
```

Refresh reads local PR bookmark heads, finds open GitHub PRs authored by you,
refreshes stored PR numbers, applies local jj ancestry, writes `.jx/stack.toml`,
and syncs affected PR bases/descriptions. It does not push branches, create PRs,
close PRs, or delete PRs.

## Move a stack

Stack moves default to the current jj change and its descendants. They update the
local stack graph, then sync affected old and new stack components to GitHub.

```sh
jx stack --trunk                 # move the current stack onto trunk
jx stack --onto topic/root       # move it onto another stack branch
jx stack --onto abc1234          # move it onto a commit/change prefix
jx stack --onto topic/root --no-sync
```

`--onto` and `--trunk` are mutually exclusive. `--no-sync` is the escape hatch
for local-only repair: it updates jj and local stack metadata but skips pushing
branches and skips GitHub PR base/description updates.

Target resolution favors jj-style identifiers before fuzzy branch names:

1. commit id prefix
2. change id prefix
3. local bookmark fragment, ranked as exact, prefix, then contains

Ambiguous bookmark fragments fail instead of guessing.

## Open a stack PR

Use the interactive stack selector when you want to open a known PR by its stack
position instead of remembering the PR number.

```sh
jx stack -i
jx stack -i --print
```

The selector uses the stored stack snapshot, so merged or missing local ancestors
can still appear when they explain the current stack shape. Only rows with a
known PR URL or number are selectable.

## What sync updates

Stack sync updates GitHub from local stack state:

- pushes selected stack bookmarks when the command includes a push phase;
- updates PR base branches to match local parent relationships;
- updates generated stack context in PR descriptions;
- preserves labels, reviewers, and other PR fields outside the specific publish
  operation that requested them.

This keeps GitHub review state aligned with the jj stack while leaving normal jj
history operations under jj's control.
