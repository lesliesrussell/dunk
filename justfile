# Commands run from this file's directory. Arguments are passed as shell positional parameters.
set positional-arguments
set shell := ["sh", "-eu", "-c"]

# Show available developer and maintenance commands.
default:
    @just --list

# Check local development and runtime prerequisites without network access.
doctor:
    #!/bin/sh
    set -eu
    missing=0
    for tool in cargo rustc python3 brew; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            printf 'Missing %s. See README.md prerequisite instructions.\n' "$tool" >&2
            missing=1
        fi
    done
    if command -v cargo >/dev/null 2>&1; then
        for component in fmt clippy; do
            if ! cargo "$component" --version >/dev/null 2>&1; then
                printf 'Missing Rust component: run rustup component add rustfmt clippy.\n' >&2
                missing=1
            fi
        done
    fi
    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import pty, fcntl, termios' || missing=1
    fi
    test "$missing" -eq 0
    printf 'Development and Homebrew prerequisites are available.\n'

# Show toolchain versions and package metadata (offline; no environment dump).
versions:
    just --version
    rustc --version
    cargo --version
    python3 --version
    cargo metadata --locked --offline --no-deps --format-version 1

# Bootstrap a checkout: check prerequisites and fetch locked Cargo dependencies.
setup: doctor deps

# Fetch development/build dependencies without modifying Cargo.lock (network possible).
deps:
    cargo fetch --locked

# Run the TUI in debug mode; pass application arguments after the recipe name.
run *args:
    cargo run --locked -- "$@"

# Load the real Homebrew catalog without opening the UI (may access Brew metadata).
brew-check:
    cargo run --locked -- --check

# Format Rust sources.
fmt:
    just --fmt
    cargo fmt --all

# Check Rust formatting without changing files.
fmt-check:
    just --fmt --check
    cargo fmt --all -- --check

# Run Clippy for all targets, treating warnings as errors.
lint:
    cargo clippy --locked --all-targets -- -D warnings

# Type-check all Rust targets without producing executables.
typecheck:
    cargo check --locked --all-targets

# Run Rust unit tests; forward filters and test-runner arguments to Cargo.
test *args:
    cargo test --locked "$@"

# Run fast pre-commit checks; excludes PTY and real Homebrew diagnostics.
check: fmt-check lint typecheck test

# Build the debug executable; forward additional Cargo build arguments.
build *args:
    cargo build --locked "$@"

# Build the optimized executable; forward additional Cargo build arguments.
release *args:
    cargo build --locked --release "$@"

# Run the Unix PTY end-to-end test against fake Brew, never the real installation.
e2e:
    cargo build --locked --target-dir target
    python3 tests/smoke.py

# Run the non-interactive validation suite, including the fake-Brew PTY test.
ci: check e2e

# Generate Rust API documentation locally without opening a browser.
docs:
    cargo doc --locked --no-deps

# Install dunk through Cargo; optional root is Cargo's installation prefix.
install root="":
    #!/bin/sh
    set -eu
    if [ -n "$1" ]; then
        exec cargo install --locked --path . --bin dunk --root "$1"
    fi
    exec cargo install --locked --path . --bin dunk

# Remove only Cargo-tracked dunk; use the same optional root as install.
uninstall root="":
    #!/bin/sh
    set -eu
    if [ -n "$1" ]; then
        exec cargo uninstall --package dunk --bin dunk --root "$1"
    fi
    exec cargo uninstall --package dunk --bin dunk

# Remove this checkout's target/ build artifacts only; preserve Cargo's shared cache.
clean:
    cargo clean --target-dir target
