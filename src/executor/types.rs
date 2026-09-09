//! Executor types and constants.
//!
//! This module is the single source of truth for the executor's shared
//! constants (environment-variable markers, fd-table key prefixes, etc.).
//! `mod.rs` re-exports them with `pub(crate) use types::*;` so that every
//! other executor submodule resolves these names to this module rather than
//! to duplicated local definitions.

/// Constants for environment variable names used by the executor.
pub const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
pub const EXPORTED_FUNCTIONS: &str = "__RUBASH_EXPORTED_FUNCTIONS";
pub const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
pub const READONLY_FUNCTIONS: &str = "__RUBASH_READONLY_FUNCTIONS";
pub const FUNC_TRACE_FUNCTIONS: &str = "__RUBASH_FUNC_TRACE_FUNCTIONS";
pub const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
pub const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
pub const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
pub const CAPCASE_VARS: &str = "__RUBASH_CAPCASE_VARS";
pub const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
pub const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
pub const SHELL_START_EPOCH: &str = "__RUBASH_SHELL_START_EPOCH";
pub const SECONDS_OFFSET: &str = "__RUBASH_SECONDS_OFFSET";
pub const FUNCTION_STDIN: &str = "__RUBASH_FUNCTION_STDIN";
pub const FUNCTION_STDIN_OFFSET: &str = "__RUBASH_FUNCTION_STDIN_OFFSET";
pub const FD_STDIN_PREFIX: &str = "__RUBASH_FD_STDIN_";
pub const FD_STDIN_OFFSET_PREFIX: &str = "__RUBASH_FD_STDIN_OFFSET_";
pub const FD_DYNAMIC_INPUT_PREFIX: &str = "__RUBASH_FD_DYNAMIC_INPUT_";
pub const FD_OUTPUT_PREFIX: &str = "__RUBASH_FD_OUTPUT_";
pub const FD_OUTPUT_PROCESS_SUBSTITUTION_PREFIX: &str = "__RUBASH_FD_OUTPUT_PROCESS_SUBSTITUTION_";
pub const FD_CLOSED_PREFIX: &str = "__RUBASH_FD_CLOSED_";
pub const FD_STDOUT_TARGET: &str = "__RUBASH_FD_STDOUT";
pub const FD_STDERR_TARGET: &str = "__RUBASH_FD_STDERR";
pub const FD_COPROC_STDIN_TARGET_PREFIX: &str = "__RUBASH_COPROC_STDIN:";
pub const FD_PROCESS_STDIN_TARGET: &str = "__RUBASH_FD_PROCESS_STDIN";
pub const INHERIT_PROCESS_STDIN: &str = "__RUBASH_INHERIT_PROCESS_STDIN";
pub const LOCAL_EXPORT_ENV: &str = "__RUBASH_LOCAL_EXPORT_ENV";
pub const POSIX_FUNCTION_EXPORT_TOUCHED: &str = "__RUBASH_POSIX_FUNCTION_EXPORT_TOUCHED";
pub const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
pub const COMPOUND_ASSIGNMENT_MARKER: &str = "__RUBASH_CA1__";
pub const ARRAY_FIELD_SPLIT_MARKER: char = '';
pub const SKIP_POSIXPIPE_TIME_COUNT_REMAINDER: &str =
    "__RUBASH_SKIP_POSIXPIPE_TIME_COUNT_REMAINDER";
