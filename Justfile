# git-sprout — common dev tasks. `just <target>` or `just` for the menu.

default:
    @just --list

# Build the release binaries.
build:
    cargo build --release -p git-sprout

# Run the binary after a fresh build.
run *args: build
    ./target/release/git-sprout {{args}}

# Run all rust tests, parallel via nextest if available.
test:
    @cargo nextest run --workspace 2>/dev/null || cargo test --workspace

# Format + clippy, across the tool and the differential harness.
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --manifest-path tests/differential/Cargo.toml --all -- --check
    cargo clippy --manifest-path tests/differential/Cargo.toml --all-targets -- -D warnings

# Run the differential suite (with no SPROUT_BIN it compares real git against real git).
differential *args:
    cargo test --release --manifest-path tests/differential/Cargo.toml -- --nocapture {{args}}

# Run the differential suite against the built binary.
differential-tool *args: build
    SPROUT_BIN="$PWD/target/release/git-sprout" \
        cargo test --release --manifest-path tests/differential/Cargo.toml -- --nocapture {{args}}

# Run the Linux kernel parity fixture (needs `just fixtures` first).
kernel *args:
    cargo test --release --manifest-path tests/differential/Cargo.toml --test kernel -- --ignored --nocapture {{args}}

# Fetch the fixture repositories the differential suite needs (kernel shallow clone).
fixtures:
    ./tests/fixtures/fetch.sh

# Run the benchmark harness; writes bench/results/bench.json.
bench *args:
    ./bench/run.sh {{args}}

# Refresh the site's figures from bench/results/bench.json.
site-numbers:
    ./bench/render-site-numbers.py

# Check the site fits its byte budget.
site-check:
    ./docs/check-budget.sh

# Publish docs/ to the sprout.alltuner.com Garage bucket (tailnet only).
deploy:
    ./docs/deploy.sh

# Clean build outputs.
clean:
    cargo clean
