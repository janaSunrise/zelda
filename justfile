# Zelda - Fast TypeScript Type Checker
# Run `just` to see available commands

default:
    @just --list

build:
    cargo build --release

build-debug:
    cargo build

clean:
    cargo clean

# Run unit tests
unit *ARGS:
    cargo test --lib {{ARGS}}

# Run TSC compatibility tests (builds zelda first)
test *ARGS: build
    cd tsc-tests && cargo run --release -- {{ARGS}}

# Run TSC tests without rebuilding zelda (faster iteration)
test-quick *ARGS:
    cd tsc-tests && cargo run --release -- {{ARGS}}

# Run a specific fixture file
test-file FILE: build
    cd tsc-tests && cargo run --release -- -r {{FILE}}

# Run TSC tests with compact summary only
test-summary: build
    cd tsc-tests && cargo run --release -- --summary-only --output compact

# Run all tests (unit + TSC compatibility)
test-all: unit test

# Check code compiles without building
check:
    cargo check

# Run clippy lints
lint:
    cargo clippy -- -D warnings

# Format code
fmt:
    cargo fmt

# Install TypeScript for TSC tests (run once)
setup:
    cd tsc-tests && npm install

# Run zelda on a file (debug build)
run FILE:
    cargo run -- {{FILE}}

# Run zelda on a file (release build)
run-release FILE: build
    ./target/release/zelda {{FILE}}

# Run zelda with JSON output
run-json FILE: build
    ./target/release/zelda --output json {{FILE}}
