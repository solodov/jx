set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

lint *paths:
    cargo fmt --all
    cargo clippy --all-targets --all-features -- -D warnings

build *paths:
    cargo build --all-targets

[script]
install:
    cargo build --release --locked

    dest_root="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}}"
    dest_dir="$dest_root/bin"
    dest="$dest_dir/jx"
    mkdir -p "$dest_dir"

    if [[ -f "$dest" ]] && cmp -s target/release/jx "$dest"; then
        echo "jx is already up to date at $dest"
    else
        install -m 755 target/release/jx "$dest"
        echo "Installed jx to $dest"
    fi

test *paths:
    cargo test --all-targets
