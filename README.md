# Zelda

A fast TypeScript type checker written in Rust.

Built on [oxc](https://oxc.rs).

## Getting Started

```sh
cargo build --release
./target/release/zelda path/to/file.ts
```

## Testing

Unit tests:
```sh
cargo test
```

### Conformance Tests

Conformance tests compare zelda's output against TypeScript's official `tsc` compiler. We use TypeScript's own test suite (~12,000 tests) as the oracle.

**Test Suite Structure:**

The tests come from `_submodules/typescript/tests/cases/`:
- `compiler/` - General compiler tests
- `conformance/` - Language conformance tests

Each test is a `.ts` file. We run both zelda and tsc on each file, then compare the error codes they produce.

**Setup:**
```sh
# Clone TypeScript submodule
git submodule update --init --depth 1

# Install TypeScript (required for running tsc)
npm install -g typescript
```

**Run:**
```sh
cargo build --release
cargo run -p ts-conformance
```

**Filtering:**

Use `--filter` to run a subset of tests. The pattern matches against filenames:
```sh
# Run tests with "union" in the filename
cargo run -p ts-conformance -- --filter "union*"

# Run tests starting with "generic"
cargo run -p ts-conformance -- --filter "generic*"

# Run a specific test
cargo run -p ts-conformance -- -r _submodules/typescript/tests/cases/compiler/foo.ts
```
