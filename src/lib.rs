//! bash-rs - A Rust implementation of GNU Bash
//!
//! This crate provides a complete implementation of a POSIX-compatible shell.

// Legacy cleanup pending: these warnings exist on master and are tracked
// for removal. Suppress them at crate level so downstream CI stays clean
// while the fixes are rolled out incrementally.
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(unused_mut)]
#![allow(dead_code)]
#![allow(unreachable_patterns)]
#![allow(clashing_extern_declarations)]

pub mod builtins;
pub mod executor;
pub mod expand;
pub mod history;
pub mod history_expand;
pub mod invocation;
pub mod jobs;
pub mod lexer;
pub mod locale;
pub mod parser;
pub mod posix_errors;
pub mod shell;

// Re-export commonly used types
#[cfg(windows)]
pub use executor::{ElevationOutput, ElevationRequest, SudoMode};
pub use executor::{ExecuteError, Executor};
pub use lexer::{Token, TokenKind};
pub use parser::{Ast, CommandNode, Redirect};
