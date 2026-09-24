# Development guide

## Build and validation

Run `just build`, `just lint`, and `just test` from the repository root.
`just install` installs the release binary. See the [Justfile](../Justfile) for
the recipes; tests require [cargo-nextest](https://nexte.st/).

## Documentation scope

- Keep the README a short tour of command groups and links to guides.
- Keep guides focused on concepts, boundaries, and rationale. Link to the owning
  code for algorithms, defaults, formats, and edge cases.
- Keep configuration docs about file shape, scope, merging, and matching, not a
  catalog of every field or its runtime effects.
- Put actionable command usage in `--help` and effective dashboard bindings in
  `?`. Do not repeat them across guides or enumerate bindings in CLI help.
- Keep help concise: explain invocation and important effects, not internal
  scheduling, rendering, or implementation steps. Rely on generated option
  listings rather than repeating flags and defaults in prose.

## Source organization

`jx` uses facade modules at `src/*.rs` and focused implementation modules under `src/<area>/`.

- Keep facade files small. They should define public entry points, declare submodules, and reexport stable types/functions.
- Put implementation details in the focused submodule that owns the behavior.
- Preserve existing public crate paths with facade reexports unless an API cleanup is explicit.
- Prefer moving related helpers with the behavior they support instead of collecting generic helper modules.
- Keep behavior-neutral organization changes separate from API or workflow changes when possible.

Current areas:

- `commands` owns CLI parsing, command handling, production service wiring, prompts, progress, and rendering.
- `repository` owns runtime environment, local context discovery, auth token sources, and workflow configuration.
- `repository/config` owns layout, diff tool, repo policy, and TOML parsing concerns.
- `jj` owns the local Jujutsu boundary, including workspace loading, log/status/diff rendering, facts, mutations, and Git transport.
- `domain` owns deterministic workflow planning, reports, sync guards, status comparison, bookmark, push, and PR decisions.
- `github` owns GitHub API types, reviewer types, errors, and the Octocrab-backed client.

## Tests

Keep unit tests under the module they exercise:

```text
src/<area>/tests/mod.rs
```

Split a large test suite by feature only when that improves navigation:

```text
src/commands/tests/
  mod.rs
  clone.rs
  diff.rs
  push.rs
  sync.rs
  pr.rs
```

These are still crate unit tests, so they can access private helpers in their parent module. Use root-level `tests/` only for black-box integration tests that exercise public APIs or the compiled binary surface.
