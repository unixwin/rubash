//! bash-rs - A Rust implementation of GNU Bash
//!
//! This crate provides a complete implementation of a POSIX-compatible shell.

pub mod builtins;
pub mod executor;
pub mod expand;
pub mod lexer;
pub mod parser;

// Re-export commonly used types
pub use executor::{ExecuteError, Executor};
pub use lexer::{Token, TokenKind};
pub use parser::{Ast, CommandNode, Redirect};
