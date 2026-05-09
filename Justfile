set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

lint *paths:
    cargo fmt --all
    cargo clippy --all-targets --all-features -- -D warnings

build *paths:
    cargo build --all-targets

install:
    cargo install --path . --locked

test *paths:
    cargo test --all-targets
