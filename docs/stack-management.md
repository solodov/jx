# Stack management

`jx` coordinates jj commit ancestry with GitHub pull-request relationships.
Commits and bookmarks define local stack shape; GitHub owns PR review and merge
state. `.jx/stack.toml` retains the relationships needed when parent bookmarks
disappear or parent PRs merge.

## Working model

A stack node represents a PR head. Its parent is another known PR head or the
branch it is based on. Stack-aware publishing and moves keep GitHub bases and
generated stack context aligned with the local graph, while preserving the
operator-written PR description.

Use `jj` for general commit editing and `jx stack` for coordinated stack changes.
A raw rebase can change local ancestry without updating PR relationships.

Stored stack display is local. Status reads GitHub into the local cache; refresh
repairs stack metadata and updates affected PR bases and descriptions. Publishing
and sync can push bookmarks. Consult command help for the exact effects of the
operation you intend to run.

Use `jx stack --help` and `jx sync --help` for workflows and options. Interactive
status dashboards show their keybindings with `?`.

## Source

- [Stack model and planning](../src/domain/pull_request_stack.rs)
- [Stack commands](../src/commands/stack.rs)
- [Publishing coordination](../src/commands/pull_request_manager.rs)
