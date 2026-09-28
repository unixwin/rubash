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
pub const TRACE_VARS: &str = "__RUBASH_TRACE_VARS";
pub const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
pub const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
/// Names whose associative hash table was created by a convert path
/// (GNU arrayfunc.c:111 convert_var_to_assoc -> assoc_create(0) ==
/// DEFAULT_HASH_BUCKETS 128) rather than make_new_assoc_variable
/// (variables.c:2857 ASSOC_HASH_BUCKETS 1024). The bucket count changes
/// iteration order, so it must ride with the variable like an attribute.
pub const ASSOC_128_VARS: &str = "__RUBASH_ASSOC_128_VARS";
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
pub const FD_TERMINAL_PREFIX: &str = "__RUBASH_FD_TERMINAL_";
pub const FD_STDOUT_TARGET: &str = "__RUBASH_FD_STDOUT";
pub const FD_STDERR_TARGET: &str = "__RUBASH_FD_STDERR";
pub const FD_COPROC_STDIN_TARGET_PREFIX: &str = "__RUBASH_COPROC_STDIN:";
pub const FD_PROCESS_STDIN_TARGET: &str = "__RUBASH_FD_PROCESS_STDIN";
pub const INHERIT_PROCESS_STDIN: &str = "__RUBASH_INHERIT_PROCESS_STDIN";
pub const LOCAL_EXPORT_ENV: &str = "__RUBASH_LOCAL_EXPORT_ENV";
pub const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
pub const COMPOUND_ASSIGNMENT_MARKER: &str = crate::executor::markers::COMPOUND_ASSIGNMENT_MARKER;
/// Lead-in byte inside a `( ... )` compound-assignment body marking it as a
/// whole-single-quoted declare operand whose expansion GNU defers to the
/// builtin (arrayfunc.c:557 expand_compound_array_assignment). The lexer
/// carriers in the body (\x1f, \x18, \x1a, \x14, \x17) stand for the
/// ORIGINAL syntax characters there — real `$`, `"`, backtick, `\`, `'` —
/// unlike escape-produced carriers elsewhere in an operand, which are data.
pub const DEFERRED_COMPOUND_BODY: char = crate::executor::markers::DEFERRED_COMPOUND_BODY;
/// GNU declare.def:988-1011: an operand subscript whose evaluation already
/// failed (diagnostic printed by the subscript evaluator) is carried to the
/// declare-family builtin under this sentinel so it binds the variable with
/// its attributes — `convert_var_to_array` and VSETATTR run before
/// assign_array_element — without re-evaluating (which would double-print
/// the `operand expected` diagnostic).
/// The codepoint must stay outside U+E000..=U+E1FF: E000-E100 is the
/// raw-byte marker range and E100+byte is BYTE_CHAR_BASE (conditional/
/// pattern.rs). E10A specifically collided with SQ_DOLLAR_DATA
/// (assignment_expansion.rs), which let restore_sq_content_markers decode
/// the sentinel into a literal `$` and vice versa.
pub const FAILED_SUBSCRIPT_SENTINEL: &str = crate::executor::markers::FAILED_SUBSCRIPT_SENTINEL;
pub const ARRAY_FIELD_SPLIT_MARKER: char = crate::executor::markers::ARRAY_FIELD_SPLIT_MARKER;
