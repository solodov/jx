# jj test layout

This directory keeps jj-boundary unit tests split by behavior area and source-module ownership.

- `mod.rs` declares the feature modules and owns shared workspace fixtures, commit builders, and bookmark helpers reused across jj test files.
- Feature files such as `workspace.rs`, `facts.rs`, `push.rs`, and `rebase.rs` own tests for the matching jj boundary behavior.
- Add new tests to the narrowest matching feature file, usually the file that matches the source module under `src/jj/`.
- Keep feature-specific helpers in the feature file that uses them; promote helpers to `mod.rs` only when multiple feature files need them.
- Split a feature file further only when it becomes hard to scan or has a clear internal boundary.

Use root-level `tests/` only for black-box integration coverage of public APIs or the compiled binary surface.
