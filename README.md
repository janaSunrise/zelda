# Zelda

A fast Typescript type checker written in Rust.

Built on [oxc](https://oxc.rs).

## Getting Started

```sh
# Build
cargo build --release

# Run on a TypeScript file
./target/release/zelda path/to/file.ts
```

## Running Tests

We use [just](https://github.com/casey/just) as a command runner. Install it first.

Then run `just` to see all available commands.

```sh
just          # List all commands
just test     # Build zelda + run TSC compatibility tests
just unit     # Run unit tests
just test-all # Run both unit and TSC tests
```

### First-Time Setup

Install TypeScript for the TSC compatibility tests:

```bash
just setup
```
