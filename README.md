# Zelda

A fast TypeScript type checker written in Rust.

Built on [oxc](https://oxc.rs).

## Getting Started

```sh
cargo build --release
./target/release/zelda check path/to/file.ts
./target/release/zelda check --format json path/to/file.ts
```

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml);
rustup installs it automatically.

## Testing

```sh
cargo test --workspace                       # unit tests
cargo clippy --workspace --all-targets       # lints (CI uses -D warnings)
```

### Conformance

zelda is tested against TypeScript's own test suite: ~12,400 tests from
`tests/cases/{compiler,conformance}` in the TypeScript repo, compared against
the diagnostics tsc reports for each (its checked-in reference baselines, so
tsc itself never needs to run).

```sh
./scripts/setup-ts-tests.sh       # once: sparse checkout of the test suite (~90 MB)
cargo conformance                 # run it all (~10s)
cargo conformance enum            # tests whose path contains "enum"
cargo conformance compiler/foo.ts # one test, with a diff of the diagnostics
cargo conformance --update        # lock in newly passing tests
```

[`ts-conformance/passing.txt`](ts-conformance/passing.txt) lists the tests
that currently pass; a run fails if any of them regress. See
[`ts-conformance/README.md`](ts-conformance/README.md) for how tests are
selected and compared.

## Layout

| Path | |
|------|-|
| `src/binder/` | Builds the symbol table from the AST |
| `src/checker/` | Type inference, assignability, control-flow narrowing |
| `src/types/` | Type representation and interning arena |
| `src/project/` | Multi-file checking and module resolution |
| `ts-conformance/` | Conformance test runner |
| `docs/` | Notes on how the type system works |
