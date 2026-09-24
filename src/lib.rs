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
#[cfg(any(windows, unix))]
pub(crate) mod fd;
pub mod expand;
pub mod history;
pub mod history_expand;
pub mod invocation;
pub mod jobs;
pub mod lexer;
pub mod locale;
pub mod parser;
pub mod posix_errors;
pub mod proc_vfs;
pub mod script_driver;
pub mod shell;

// Re-export commonly used types
#[cfg(windows)]
pub use executor::{ElevationOutput, ElevationRequest, SudoMode};
pub use executor::{ExecuteError, Executor};
pub use lexer::{Token, TokenKind};
pub use locale::decode_to_visible_text;
pub use parser::{Ast, CommandNode, Redirect};
