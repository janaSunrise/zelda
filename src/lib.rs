//! Zelda: a fast TypeScript type checker built on [oxc](https://oxc.rs).
//!
//! zelda is a CLI. This library target exists only so the `zelda` binary and
//! the `ts-conformance` harness can share the checker; it is not a supported
//! API, and everything except the two re-exports below is crate-private.
//!
//! The pipeline is parse (oxc) → bind (`binder`) → check (`checker`),
//! coordinated across files by [`Project`].

mod binder;
mod checker;
mod errors;
mod lib_dts;
mod project;
mod resolver;
mod symbols;
mod tsconfig;
mod types;

pub use project::{Project, ProjectError};
