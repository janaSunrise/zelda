# TSC Compatibility Test Suite

A test harness for comparing Zelda's type checking output against TypeScript's official `tsc` compiler.
Measures both correctness (error code matching) and performance (speedup ratio).

## Quick Start

From the zelda root directory.

```bash
# 1. Build zelda in release mode
cargo build --release

# 2. Install TypeScript
cd tsc-tests && npm install

# 3. Run the test suite
cargo run --release -- -z ../target/release/zelda
```

## Usage

```
tsc-tests [OPTIONS]

Options:
  -f, --fixtures <PATH>     Directory containing test fixtures [default: fixtures]
  -z, --zelda <PATH>        Path to zelda binary [default: ../target/release/zelda]
  -o, --output <FORMAT>     Output format: table, json, compact [default: table]
  -i, --iterations <N>      Benchmark iterations per file [default: 3]
  -s, --summary-only        Only show summary (no individual results)
  -r, --run <FILES>         Run specific fixture file(s) instead of whole directory
  -h, --help                Print help
```

## Examples

```bash
# Run all tests with table output
cargo run --release

# Run a specific fixture
cargo run --release -- -r fixtures/basics/variables.ts

# JSON output (for CI integration)
cargo run --release -- --output json

# Compact one-line summary
cargo run --release -- --output compact

# More benchmark iterations for stable timing
cargo run --release -- --iterations 10
```

## Output Formats

### JSON

```json
{
  "summary": {
    "total_files": 2,
    "passed": 2,
    "failed": 0,
    "matched_errors": 3,
    "speedup": 607.0
  },
  "results": [...]
}
```

## Interpreting Results

| Column | Meaning |
|--------|---------|
| **Matched** | Error codes found by both zelda and tsc |
| **Missing** | Error codes tsc found but zelda didn't (features to implement) |
| **Extra** | Error codes zelda found but tsc didn't (stricter or different codes) |
| **Speedup** | How many times faster zelda is than tsc |

A test **passes** when `Missing = 0` and `Extra = 0`.

## Adding New Fixtures

Create a `.ts` file in the appropriate `fixtures/` subdirectory:

```typescript
// fixtures/basics/example.ts

// Should pass - correct types
const x: number = 42;

// Should error: TS2322
const bad: number = "wrong";
```

## CI Integration

```yaml
- name: Run TSC compatibility tests
  run: |
    cd tsc-tests
    cargo run --release -- -z ../target/release/zelda --output compact
```

Exit code is 1 if any tests fail.
