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

# Format + clippy.
check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Run the differential suite against real `git worktree add`.
differential *args:
    cargo test --release -p git-sprout --test differential -- --nocapture {{args}}

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
