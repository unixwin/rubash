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
/// Functions whose stored body words were never alias-expanded at read
/// time: the body was parsed from an executor-level (non-streamed) input
/// — `.`-sourced file, eval'd text — where expand_aliases runs lazily at
/// execution (parse.y alias_expand_token happens in the reader under
/// GNU). At call time the ambient `__RUBASH_ALIAS_STREAMED` marker usually
/// reflects the CALLER's stream, so execute_function lifts it for these
/// bodies (modernish: `not` inside readlink.mm/myfn bodies).
pub const UNSTREAMED_FUNCTION_BODIES: &str = "__RUBASH_UNSTREAMED_FUNCTION_BODIES";
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
pub const FD_PIPE_PREFIX: &str = "__RUBASH_FD_PIPE_";
/// rubash#1902: the fd-alias facts the test/[[ evaluator consumes. A
/// File-bound fd carries its target path in __RUBASH_FD_TARGET_<fd>
/// (every file predicate answers from the referent, the procfs-symlink
/// semantics); every other open fd carries its device class plus mode
/// bits in __RUBASH_FD_ACCESS_<fd> (class char first: `p` fifo, `c`
/// console, `d` directory, `n` null-style char device, `f` generic file;
/// then the granted r/w/x letters). An empty value is the closed-entry
/// tombstone: /proc/self/fd/N is gone and every predicate fails.
pub const FD_TARGET_PREFIX: &str = "__RUBASH_FD_TARGET_";
pub const FD_ACCESS_PREFIX: &str = "__RUBASH_FD_ACCESS_";
/// rubash#1902 (stage face): "1" while a pipeline stage's fd 0 is the
/// staged FUNCTION_STDIN mirror standing in for the inter-stage pipe
/// (GNU execute_cmd.c:2620 execute_pipeline + do_piping,
/// execute_cmd.c:6373: the element's fd 0 IS the pipe). The stage
/// funnels clear the fd-table slot for the mirror's duration, so the
/// entries-driven fd-fact sync has no row to classify; the stage entry
/// installs this fact beside the mirror (and restores it with the
/// mirror), and sync_fd_alias_facts turns it into the fd-0 access mark
/// only when no fd-table entry produced one. It rides env_vars like
/// the marks themselves so stage clones (command_substitution_executor
/// fork copies) inherit it verbatim.
pub const FD0_STAGE_PIPE: &str = "__RUBASH_FD0_STAGE_PIPE";
/// rubash#2042: marks a file-backed open descriptor for the
/// `test -r /proc/<pid>/fd/N` face. GNU answers -r TRUE for every
/// live fd magic link regardless of the fd's open mode (measured on
/// real procfs, bash 5.3.20, uid 1: `lr-x` regular read-only,
/// `l-wx` write-only append, `lr-x` /dev/null all answer -r yes —
/// the link is readable iff the descriptor exists). The pipe marker
/// above deliberately does not cover regular files (rubash#1467
/// keeps them unmarked so the pipe verdicts stay exact), so the
/// file-backed class carries its own mark, rewritten fresh each
/// call like the pipes.
pub const FD_FILE_PREFIX: &str = "__RUBASH_FD_FILE_";
pub const FD_STDOUT_TARGET: &str = "__RUBASH_FD_STDOUT";
pub const FD_STDERR_TARGET: &str = "__RUBASH_FD_STDERR";
pub const FD_COPROC_STDIN_TARGET_PREFIX: &str = "__RUBASH_COPROC_STDIN:";
pub const FD_PROCESS_STDIN_TARGET: &str = "__RUBASH_FD_PROCESS_STDIN";
pub const INHERIT_PROCESS_STDIN: &str = "__RUBASH_INHERIT_PROCESS_STDIN";
pub const LOCAL_EXPORT_ENV: &str = "__RUBASH_LOCAL_EXPORT_ENV";
pub const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
/// GNU att_invisible (variables.c): variables bound invisible at startup
/// (COMP_WORDBREAKS, bashline.c) exist and answer `declare -p NAME`, but
/// print_var_list — the `set` listing — skips them (rubash#1325).
pub const INVISIBLE_VARS: &str = "__RUBASH_INVISIBLE_VARS";
// GNU builtins/set.def:928 + variables.c:3839 unbind_variable: `unset -v`
// on a dynamic variable (BASHPID, RANDOM, BASH_SUBSHELL, ...) removes it
// like an ordinary one — the name stays unbound and does not
// re-materialize. Rubash materializes dynamic values on demand, so the
// unbind is recorded here instead of deleting an env slot.
pub const UNSET_DYNAMIC_VARS: &str = "__RUBASH_UNSET_DYNAMIC_VARS";
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
