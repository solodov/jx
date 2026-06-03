# Agent guidance

## Source organization

Follow `docs/development.md` for source organization rules. Keep the module facades in `src/*.rs` as stable public/reexport surfaces, and put implementation details in focused submodules under `src/<area>/`.

When adding or moving code:

- Prefer a focused existing submodule over growing a facade file.
- Preserve existing public crate paths through facade reexports unless an API cleanup is explicit.
- Keep unit tests under the module they exercise in `src/<area>/tests/`; use root `tests/` only for black-box integration coverage.
- Update `docs/development.md` when organization rules change.

## Perf analysis

When investigating `jx` latency, prefer the repo script over ad hoc log parsing:

- `python3 scripts/analyze-perf-log.py --command sync`
- `python3 scripts/analyze-perf-log.py --command stack --latest 3`
- Default log path: `~/.local/state/jx/jx-perf.log`

Read perf spans effect-first:

- Start from the latest matching `command.run`.
- Inspect nested command spans such as `sync.current_repository` or `stack.publish`.
- Compare slow steps before proposing optimizations.
- Distinguish human or interactive waits from machine time.
- Treat Git transport spans such as `git_push_refs` separately from GitHub API spans.
