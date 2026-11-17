//! Zelda: a fast TypeScript type checker built on [oxc](https://oxc.rs).
//!
//! The pipeline is parse (oxc) → bind ([`binder`]) → check ([`checker`]),
//! coordinated across files by [`project::Project`].

pub mod binder;
pub mod checker;
pub mod errors;
pub mod lib_dts;
pub mod project;
pub mod resolver;
pub mod symbols;
pub mod tsconfig;
pub mod types;

pub use project::{Project, ProjectError};
