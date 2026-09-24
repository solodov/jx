# Code layout

A layout lets `jx` find repositories and managed workspaces without depending on
the current directory. Each repository has a normalized identity: source, host,
owner, and repository name.

## Checkouts and workspaces

The layout maps each identity to a primary checkout and a family of parallel jj
workspaces:

```text
primary checkout:   <root>/<path>
managed workspace:  <root>/<workspace_dir>/<path>/<workspace-name>
```

Primary checkouts are the targets for cross-repository maintenance. Managed
workspaces provide separate working copies of the same repository. They can carry
task and project context independently of their directory names.

## Discovery

`jx` discovers locations under configured layout roots and assigns keys that
identify them without full paths. Managed workspace keys include an `@workspace`
suffix. Commands run against the current checkout unless given another target;
layout-aware commands can resolve those targets from elsewhere.

See [configuration](configuration.md) for placement rules and matching. Use
`jx clone --help`, `jx work --help`, and `jx shell --help` for command usage.

## Source

- [Layout resolution and discovery](../src/repository/config/layout.rs)
- [Workspace metadata](../src/repository/workspace_metadata.rs)
- [Workspace commands](../src/commands/work.rs)
