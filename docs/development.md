# Development guide

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

`just test` requires [cargo-nextest](https://nexte.st/). Install it before running the repository test recipe:

```sh
cargo install cargo-nextest --locked
```

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
