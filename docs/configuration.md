# Configuration

`jx` uses optional TOML files for layout, repository policy, and user preferences.
Command usage lives in `jx <command> --help`; dashboard keybindings live in `?`.
This guide covers configuration structure and composition, not every option.

## Files and scope

Files load in this order:

1. `~/.config/jx/*.toml`, in lexical filename order.
2. Workspace-root `.jx/config.toml`, when the command loads workspace config.

Use global files for personal defaults and cross-repository rules. Use the
workspace file for project policy. Layout-wide discovery uses global config;
repository operations can then load the selected workspace's config.

Dashboard keybindings are user-global and cannot be set in workspace config.
Jujutsu aliases and terminal styling belong in jj's own configuration, not these
files.

## File shape

A file can contain any subset of the supported sections. `[section]` declares a
TOML table; `[[section.rules]]` appends a rule. Nested tables under an array entry
belong to the most recently declared entry.

```toml
[layout]
default_root = "~/src"

[[layout.rules]]
source = "github"
owner = "example-org"
root = "~/work"
path = "{repo}"

[repo]
reviewers = ["example-reviewer"]

[[repo.rules]]
repo = "example-org/*"

[repo.rules.review]
hide_pending_checks = true

[ui]
default_command = ["status"]
```

The main sections are `layout`, `repo`, `ui`, `shell`, `diff`, and `auth`.
Unsupported keys and invalid values are reported as configuration errors.

## Merging

Composition is field-specific, not a generic TOML deep merge:

- Later scalar values replace earlier ones; omitted values keep their inherited
  setting. Command argv arrays are also replaced, not concatenated.
- Rule lists accumulate in file order. All matching rules apply, not just the
  first or most specific match.
- Set-like lists, such as reviewers and shared workspace paths, accumulate and
  deduplicate. Ordered transformations, such as title rewrites, compose in order.
- Named layout sources and diff tools replace earlier definitions with the same
  name.
- Checks, hooks, handlers, and manual actions with the same `id` replace the whole
  earlier definition. Hooks, handlers, and actions also support
  `enabled = false` to remove an inherited entry.

Do not assume an empty list clears an accumulated setting. Consult the owning
configuration type below for field-specific behavior.

## Matching

**Layout rules** match a source and the supplied owner/repository names exactly.
At least one of owner or repository is required. Later matching rules override
only the placement fields they specify. Layout maps repository identity to
checkout paths; see [code layout](code-layout.md) for the model.

**Repository policy** matches the fixed `origin` repository's `owner/repo` slug
with globs. All `[repo]` defaults are merged first, then matching `[[repo.rules]]`
apply in file order. A global matching rule can therefore override a local base
value; use a matching local rule to override it.

**Manual PR actions** match the selected PR's repository, not the caller's.
`review_actions` and `stack_status_actions` are independent sets. Each resolves
global defaults, matching global rules, local defaults, then matching local rules,
retaining file order within each phase.

## Dashboard preferences

Review and stack-status dashboards share configurable keys under global
`[ui.dashboard.keys]`. Each operation replaces its inherited bindings;
unspecified operations remain inherited and `[]` unbinds one. Press `?` in the
dashboard to see the effective bindings.

## Source reference

Use the owning types and parsers for supported fields, defaults, and validation:

| Area | Source |
| --- | --- |
| File discovery and composition | [config.rs](../src/repository/config.rs) |
| TOML sections and parsing | [parse.rs](../src/repository/config/parse.rs) |
| Layout | [layout.rs](../src/repository/config/layout.rs) |
| Repository policy | [repo_policy.rs](../src/repository/config/repo_policy.rs) |
| Manual actions | [actions.rs](../src/repository/config/actions.rs), [parser](../src/repository/config/actions/parse.rs) |
| UI and keybindings | [ui.rs](../src/repository/config/ui.rs), [dashboard_keys.rs](../src/repository/config/dashboard_keys.rs) |
| Shell and diff tools | [shell.rs](../src/repository/config/shell.rs), [diff.rs](../src/repository/config/diff.rs) |
| Authentication | [auth.rs](../src/repository/auth.rs) |
