//! bash-rs - A Rust implementation of GNU Bash
//!
//! This crate provides a complete implementation of a POSIX-compatible shell.

pub mod builtins;
pub mod executor;
pub mod expand;
pub mod history;
pub mod history_expand;
pub mod invocation;
pub mod jobs;
pub mod lexer;
pub mod parser;
pub mod posix_errors;
pub mod shell;

// Re-export commonly used types
#[cfg(windows)]
pub use executor::{ElevationOutput, ElevationRequest, SudoMode};
pub use executor::{ExecuteError, Executor};
pub use lexer::{Token, TokenKind};
pub use parser::{Ast, CommandNode, Redirect};
