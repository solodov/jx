# Agent guidance

## Source organization

Follow `docs/development.md` for source organization rules. Keep the module facades in `src/*.rs` as stable public/reexport surfaces, and put implementation details in focused submodules under `src/<area>/`.

When adding or moving code:

- Prefer a focused existing submodule over growing a facade file.
- Preserve existing public crate paths through facade reexports unless an API cleanup is explicit.
- Keep unit tests under the module they exercise in `src/<area>/tests/`; use root `tests/` only for black-box integration coverage.
- Update `docs/development.md` when organization rules change.
