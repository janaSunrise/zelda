//! TypeScript Conformance Testing Library

pub mod analysis;
pub mod compare;
pub mod diagnostic;
pub mod report;
pub mod runner;
pub mod source;
pub mod tsc;
pub mod types;
pub mod zelda;

pub use compare::{CompareResult, Match, MatchKind};
pub use diagnostic::Diagnostic;
pub use source::TestSource;
pub use types::{Summary, TestCase, TestResult};
