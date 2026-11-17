# ts-conformance

Runs zelda against TypeScript's own test suite (~12,400 tests in
`tests/cases/{compiler,conformance}`) and compares its diagnostics with tsc's.

## How it works

**The oracle is tsc's reference baselines, not a live tsc.** For every test,
the TypeScript repo checks in the exact errors tsc reports
(`<test>.errors.txt`), generated with that test's own `// @strict`,
`// @target`, ... directives. We pin the submodule to a TypeScript release
(currently **v7.0.2**) and read the TS 7 baselines from
`tsc/testdata/baselines/reference/submodule/`. That makes runs fast (~10s for
the whole suite), hermetic (no Node, no network), and reproducible.

For each test:

1. **Split** the test file into units on `// @filename:` and collect its
   `// @option: value` settings, replicating the harness exactly (baseline
   line numbers depend on it).
2. **Pick a configuration.** Tests with varying options
   (`// @strict: true, false`) have one baseline per combination, like
   `foo(strict=true).errors.txt`. zelda has no options and behaves like tsc's
   defaults, so we compare against the closest one: strict on, newest target.
   A config only counts if tsc actually ran it (some baseline exists for it);
   the TS 7 harness silently skips configs that use removed options.
3. **Skip** tests that need things zelda doesn't model, with a reason:
   JavaScript files, custom `@lib`, non-strict mode, module resolution
   settings, `node_modules`, symlinks. `--all` runs them anyway.
4. **Check** the units in a temp dir, in a separate worker process. A panic,
   stack overflow, or hang is reported as that test's outcome instead of
   taking down the run.
5. **Compare** diagnostics by `(file, line, code)`. Columns and message
   wording are not compared.

Outcomes: `pass`, `fail` (diagnostics differ), `parse error` (oxc rejected a
file), `crash`, `timeout`, `skipped`.

## Regression ratchet

[`passing.txt`](passing.txt) lists every test that currently passes. A run
fails if any of them stops passing. When your change makes more tests pass, lock
them in:

```sh
cargo conformance --update
```

and commit the updated `passing.txt`. CI runs with `--locked`, which also
fails if tests pass that aren't listed, so the file always reflects reality.

## Usage

```sh
./scripts/setup-ts-tests.sh                   # once: fetch the test suite (~90 MB)

cargo conformance                             # run everything
cargo conformance enum                        # tests whose path contains "enum" (case-insensitive)
cargo conformance 'conformance/types/union/*' # glob
cargo conformance compiler/foo.ts -v          # show the diagnostic diff
cargo conformance --update                    # rewrite passing.txt
cargo conformance --json results.json         # every result, for scripting
cargo conformance --skipped                   # list skipped tests and why
```

When 20 or fewer tests run, the diff for each failing test is printed
automatically:

```
── compiler/aliasAssignments.ts [fail]
   baseline: aliasAssignments
   missing  aliasAssignments_1.ts(3,1): error TS2322: Type 'number' is not assignable to ...
   extra    aliasAssignments_1.ts(2,9): error TS2304: Cannot find name 'moduleA'.
```

The summary ends with error-code rankings: the codes zelda most often misses
or over-reports, and how many tests each one is the *only* remaining problem
in. That column is a good guide to what to work on next.

## Updating TypeScript

```sh
git -C _submodules/typescript fetch --depth 1 origin tag v7.x.y
git -C _submodules/typescript checkout v7.x.y
cargo conformance --update
git add _submodules/typescript ts-conformance/passing.txt
```
