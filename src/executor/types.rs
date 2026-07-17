//! Executor types and constants.
//!
//! This module contains type definitions and constants used by the executor.

/// Execution error
#[derive(Debug)]
pub enum ExecuteError {
    CommandNotFound(String),
    IoError(std::io::Error),
    ExitCode(i32),
    Break(usize),
    Continue(usize),
    Return(i32),
    UnknownBuiltin(String),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::CommandNotFound(cmd) => write!(f, "rubash: {}: command not found", cmd),
            ExecuteError::IoError(e) => write!(f, "rubash: {}", e),
            ExecuteError::ExitCode(code) => write!(f, "exit code: {}", code),
            ExecuteError::Break(level) => write!(f, "break {}", level),
            ExecuteError::Continue(level) => write!(f, "continue {}", level),
            ExecuteError::Return(status) => write!(f, "return {}", status),
            ExecuteError::UnknownBuiltin(name) => {
                write!(f, "rubash: {}: builtin command not found", name)
            }
        }
    }
}

impl std::error::Error for ExecuteError {}

impl From<std::io::Error> for ExecuteError {
    fn from(e: std::io::Error) -> Self {
        ExecuteError::IoError(e)
    }
}

/// Constants for environment variable names used by the executor.
pub const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub const EXPORTED_FUNCTIONS: &str = "__RUBASH_EXPORTED_FUNCTIONS";
pub const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
pub const READONLY_FUNCTIONS: &str = "__RUBASH_READONLY_FUNCTIONS";
pub const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
pub const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
pub const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
pub const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub const SHELL_START_EPOCH: &str = "__RUBASH_SHELL_START_EPOCH";
pub const SECONDS_OFFSET: &str = "__RUBASH_SECONDS_OFFSET";
pub const FUNCTION_STDIN: &str = "__RUBASH_FUNCTION_STDIN";
pub const FUNCTION_STDIN_OFFSET: &str = "__RUBASH_FUNCTION_STDIN_OFFSET";
pub const FD_STDIN_PREFIX: &str = "__RUBASH_FD_STDIN_";
pub const FD_STDIN_OFFSET_PREFIX: &str = "__RUBASH_FD_STDIN_OFFSET_";
pub const FD_CLOSED_PREFIX: &str = "__RUBASH_FD_CLOSED_";
pub const FD_STDOUT_TARGET: &str = "__RUBASH_FD_STDOUT";
pub const FD_STDERR_TARGET: &str = "__RUBASH_FD_STDERR";
pub const FD_PROCESS_STDIN_TARGET: &str = "__RUBASH_FD_PROCESS_STDIN";
pub const INHERIT_PROCESS_STDIN: &str = "__RUBASH_INHERIT_PROCESS_STDIN";
pub const LOCAL_EXPORT_ENV: &str = "__RUBASH_LOCAL_EXPORT_ENV";
pub const POSIX_FUNCTION_EXPORT_TOUCHED: &str = "__RUBASH_POSIX_FUNCTION_EXPORT_TOUCHED";
pub const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
pub const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';
pub const ARRAY_FIELD_SPLIT_MARKER: char = '\x1c';
pub const SKIP_POSIXPIPE_TIME_COUNT_REMAINDER: &str = "__RUBASH_SKIP_POSIXPIPE_TIME_COUNT_REMAINDER";
