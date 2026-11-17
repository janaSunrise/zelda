//! Conformance testing for zelda against TypeScript's own test suite.
//!
//! The oracle is tsc's *reference baselines*: for every test in
//! `tests/cases/{compiler,conformance}`, the TypeScript repo checks in the exact
//! diagnostics tsc produces (`<test>.errors.txt`). We run zelda on the same
//! input and compare diagnostics by `(file, line, code)`. tsc itself is never
//! executed, so runs are fast, hermetic, and reproducible.
//!
//! Pipeline for one test:
//!
//! 1. [`source`]: split the test file into units (`// @filename:`) and
//!    collect its `// @option: value` settings, exactly as tsc's harness does.
//! 2. [`tsc`]: pick the baseline configuration to compare against and read
//!    the expected diagnostics.
//! 3. [`support`]: skip tests that rely on features zelda deliberately does
//!    not model (JS files, custom libs, non-strict mode, ...).
//! 4. [`runner`]: write the units to a temp dir and check them in a separate
//!    worker process ([`zelda`]), so crashes, stack overflows, and hangs are
//!    contained and reported per test.
//! 5. [`compare`]: diff expected vs actual diagnostics.
//! 6. [`expectations`]: compare against the committed list of passing tests
//!    to detect regressions.

pub mod analysis;
pub mod compare;
pub mod diagnostic;
pub mod expectations;
pub mod report;
pub mod runner;
pub mod source;
pub mod support;
pub mod tsc;
pub mod types;
pub mod zelda;
