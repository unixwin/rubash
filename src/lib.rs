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

pub mod api;
pub mod build_info;
pub mod builtins;
pub mod console_readline;
pub(crate) mod debugger;
pub mod executor;
pub mod expand;
#[cfg(any(windows, unix))]
pub(crate) mod fd;
pub mod history;
pub mod history_expand;
pub mod invocation;
pub mod jobs;
pub mod lexer;
pub mod locale;
pub mod parser;
pub mod posix_errors;
pub mod proc_vfs;
pub mod readline_editor;
pub mod script_driver;
pub mod shell;
pub mod special_paths;
pub(crate) mod stack_growth;
pub mod token_str;
pub mod usage;
pub(crate) mod winuxcmd_fifo;

// The DAP frontend entry points (1.5.0 debugger design section
// 13; PLAN row 10): the one part of the debugger the binaries and
// embedding hosts dispatch to. The session core stays crate-
// private behind this facade until the API finalization gives it
// a curated public surface.
pub use debugger::{
    run_dap_server, run_dap_stdio, run_debug_cli, run_debug_cli_with_stream, DapLaunchPreset,
    DebugCliPreset,
};

// Re-export commonly used types
#[cfg(windows)]
pub use executor::{ElevationOutput, ElevationRequest, SudoMode};
pub use executor::{ExecuteError, Executor};
pub use lexer::{Token, TokenKind};
pub use locale::decode_to_visible_text;
pub use parser::{Ast, CommandNode, Redirect};
