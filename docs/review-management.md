# Review management

`jx review` is a personal GitHub review inbox. It combines current PR snapshots
with repository policy and local visibility decisions to focus on work that needs
your attention.

## Visibility and local state

Dismissal changes the local inbox, not GitHub reviews. A dismissed PR can return
when it needs attention again. Some rows are hidden automatically based on PR
state; an explicit undismissal can bring them back.

The shared local pull-request store keeps snapshots, history, and visibility
actions. Cached views use that stored state and may be stale. The live inbox
refreshes it from GitHub.

Use `jx review --help` and its subcommand help for filtering, dismissal, and
history. Press `?` in the dashboard for effective keybindings. Repository policy
and manual actions are configured through the usual
[configuration layers](configuration.md).

## Source

- [Inbox and dismissal behavior](../src/commands/review.rs)
- [Pull-request storage](../src/repository/pull_request_store.rs)
- [Repository review policy](../src/repository/config/repo_policy.rs)
