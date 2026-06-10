//! bashrs - A Rust implementation of GNU Bash
//!
//! This crate provides a complete implementation of a POSIX-compatible shell.

pub mod lexer;
pub mod parser;
pub mod executor;

// Re-export commonly used types
pub use lexer::{Token, TokenKind};
pub use parser::{Ast, CommandNode, Redirect};
pub use executor::{Executor, ExecuteError};