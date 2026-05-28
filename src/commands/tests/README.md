# Command test layout

This directory keeps command-facing unit tests split by behavior area so the suite stays easy to navigate.

- `mod.rs` declares the feature modules and owns shared fixtures, fake services, and helpers used across multiple command test files.
- Feature files such as `work.rs`, `sync.rs`, and `pull_request.rs` own tests for that command or workflow area.
- Add new tests to the narrowest matching feature file.
- Keep feature-specific helpers in the feature file that uses them; promote helpers to `mod.rs` only when multiple feature files need them.
- Split a feature file further only when it becomes hard to scan or has a clear internal feature boundary.

Use root-level `tests/` only for black-box integration coverage of public APIs or the compiled binary surface.
