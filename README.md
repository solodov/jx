# jx

`jx` is a layout-aware companion for
[Jujutsu](https://jj-vcs.github.io/jj/latest/) and GitHub. It keeps `jj` as the
source of truth for local work and adds repository discovery, managed workspaces,
pull-request stacks, and a personal review inbox.

Use it inside one checkout, or configure a layout to work across repositories
from anywhere. General commit editing stays in `jj`; `jx` coordinates that work
with GitHub.

## Command groups

- **Local work**: `jx log`, `jx status`, and `jx diff` inspect the current checkout.
- **Repositories**: `jx clone` and `jx work` manage checkouts and parallel workspaces.
- **Remote state**: `jx remote-status`, `jx fetch`, `jx push`, and `jx sync` maintain repositories.
- **Pull requests**: `jx stack` manages publishing and stack relationships.
- **Reviews**: `jx review` manages your review inbox.
- **GitHub navigation**: `jx open` opens repository and pull-request pages.
- **Forks**: `jx fork` maintains fork relationships.
- **Shell integration**: `jx shell` provides completion and layout-aware navigation.

Run `jx --help` or `jx <command> --help` for usage. In interactive dashboards,
press `?` for keybindings.

## Guides

- [Configuration](docs/configuration.md): file structure, layering, and matching.
- [Code layout](docs/code-layout.md): repository identity and managed workspaces.
- [Stack management](docs/stack-management.md): how jj stacks relate to GitHub PRs.
- [Review management](docs/review-management.md): the inbox and local dismissal model.
- [Development](docs/development.md): building, testing, and source organization.

## Scope

`jx` uses the fixed `origin` remote for GitHub publishing and pushes PR heads to
the same repository, not a fork. Fork maintenance is a separate workflow. Use
`jj` directly when you need more general control.
