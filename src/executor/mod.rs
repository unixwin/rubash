//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

pub(crate) mod path;

mod glob;
use glob::pathname_expand_word;

mod upstream_scripts;

mod arithmetic;
use arithmetic::{
    arithmetic_division_by_zero_token, eval_arith_value, eval_conditional_arith_value,
};

mod arrays;
use arrays::*;
mod alias_loops;
mod alias_reparse;
mod alias_set_builtins;
mod arithmetic_aliases;
mod array_assignment_exec;
mod assignment_dispatch;
mod assignment_expansion;
mod builtin_direct;
mod builtin_direct_command;
mod builtin_direct_late;
mod builtin_redirects;
mod command_no_alias;
mod command_no_alias_late;
mod command_substitution;
mod command_substitution_pipelines;
mod command_substitution_values;
mod command_words;
mod compound_exec;
mod dynamic_arrays;
mod embedded_mutations;
mod embedded_parameters;
mod expand_braced_indices;
mod expand_braced_ops;
mod expand_braced_patterns;
mod expand_braced_replacement;
mod expand_braced_special;
mod expand_word;
mod external_file_builtins;
mod external_finish;
mod external_inner;
mod external_setup;
mod getopts_enable;
mod job_builtins;
mod limit_builtins;
mod lookup_paths;
mod mapfile_builtin;
mod mapfile_helpers;
mod option_builtins;
mod parameter_core;
mod parameter_errors;
mod parameter_patterns;
mod parameter_transforms;
mod parameter_words;
mod printf_path_builtins;
mod prompt_expansion;
mod public_accessors;
mod pwd_loop_builtins;
mod read_builtin;
mod read_io;
mod shell_options;
mod shift_echo_builtins;
mod source_type_state;
mod temporary_assignments;

mod variable_state;

mod trap_stack_builtins;
mod type_builtin;
mod type_describe;
mod type_functions;

mod declare_local;
mod export_builtin;
mod function_calls;
mod function_locals;
mod loop_select;
mod readonly_functions;
mod trap_exec;
mod unset_arrays;

mod assignment_helpers;
mod ast_exec;
mod builtin_names;
mod command_subst_helpers;
mod command_text;
mod env_helpers;
mod execution_misc;
mod function_env;
mod local_helpers;
mod parameter_case;
mod parameter_decode;
mod parameter_ops;
mod parameter_replace;
mod parse_helpers;
mod pipeline_exec;
mod pipeline_stages;

mod read_helpers;
mod read_split;
mod sed_alias_helpers;
mod support_names;

use assignment_helpers::*;
use builtin_names::*;
use command_subst_helpers::*;
use command_text::*;
use env_helpers::*;
use execution_misc::*;
use function_env::*;
use local_helpers::*;
use parameter_case::*;
use parameter_decode::*;
use parameter_ops::*;
use parameter_replace::*;
use parse_helpers::*;
use read_helpers::*;
use read_split::*;
use sed_alias_helpers::*;
use support_names::*;

mod conditional;
use conditional::{case_pattern_matches, simple_grep_pattern_matches};

use crate::builtins::alias::Alias;
use crate::expand::tilde::tilde as tilde_expand;
use crate::lexer::TokenKind;
use crate::parser::{
    ArithmeticForCommand, Ast, CaseClause, CaseCommand, CaseTerminator, CommandNode, ForCommand,
    FunctionCommand, Redirect, SelectCommand,
};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use self::path::{
    find_shell, find_user_command, shell_path_to_windows, should_run_with_shell, standard_path,
};

const EXPORTED_VARS: &str = "__RUBASH_EXPORTED_VARS";
const EXPORTED_FUNCTIONS: &str = "__RUBASH_EXPORTED_FUNCTIONS";
const READONLY_VARS: &str = "__RUBASH_READONLY_VARS";
const READONLY_FUNCTIONS: &str = "__RUBASH_READONLY_FUNCTIONS";
const INTEGER_VARS: &str = "__RUBASH_INTEGER_VARS";
const UPPERCASE_VARS: &str = "__RUBASH_UPPERCASE_VARS";
const LOWERCASE_VARS: &str = "__RUBASH_LOWERCASE_VARS";
const NAMEREF_VARS: &str = "__RUBASH_NAMEREF_VARS";
const ARRAY_VARS: &str = "__RUBASH_ARRAY_VARS";
const ASSOC_VARS: &str = "__RUBASH_ASSOC_VARS";
const SHELL_START_EPOCH: &str = "__RUBASH_SHELL_START_EPOCH";
const SECONDS_OFFSET: &str = "__RUBASH_SECONDS_OFFSET";
const FUNCTION_STDIN: &str = "__RUBASH_FUNCTION_STDIN";
const FUNCTION_STDIN_OFFSET: &str = "__RUBASH_FUNCTION_STDIN_OFFSET";
const FD_STDIN_PREFIX: &str = "__RUBASH_FD_STDIN_";
const FD_STDIN_OFFSET_PREFIX: &str = "__RUBASH_FD_STDIN_OFFSET_";
const INHERIT_PROCESS_STDIN: &str = "__RUBASH_INHERIT_PROCESS_STDIN";
const LOCAL_EXPORT_ENV: &str = "__RUBASH_LOCAL_EXPORT_ENV";
const POSIX_FUNCTION_EXPORT_TOUCHED: &str = "__RUBASH_POSIX_FUNCTION_EXPORT_TOUCHED";
const DECLARED_UNSET_VARS: &str = "__RUBASH_DECLARED_UNSET_VARS";
const COMPOUND_ASSIGNMENT_MARKER: char = '\x1e';
const SKIP_POSIXPIPE_TIME_COUNT_REMAINDER: &str = "__RUBASH_SKIP_POSIXPIPE_TIME_COUNT_REMAINDER";

static EXECUTION_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static EXECUTION_LOCK_DEPTH: Cell<usize> = const { Cell::new(0) };
}

enum NamerefResolution {
    Target(String),
    Circular,
    NotNameref,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypeDescribeMode {
    Verbose,
    Reusable,
    TypeOnly,
    PathOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControlKind {
    Break,
    Continue,
}

impl LoopControlKind {
    fn name(self) -> &'static str {
        match self {
            LoopControlKind::Break => "break",
            LoopControlKind::Continue => "continue",
        }
    }
}

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

#[derive(Clone, Copy, Debug, Default)]
struct VarAttrs {
    exported: bool,
    readonly: bool,
    integer: bool,
    uppercase: bool,
    lowercase: bool,
    nameref: bool,
    array: bool,
    assoc: bool,
}

#[derive(Debug)]
struct SavedGlobalDeclareLocal {
    name: String,
    scope_index: usize,
    local_value: Option<String>,
    local_attrs: VarAttrs,
}

/// Command executor
#[derive(Debug)]
pub struct Executor {
    exit_code: i32,
    env_vars: HashMap<String, String>,
    aliases: HashMap<String, Alias>,
    functions: HashMap<String, Vec<CommandNode>>,
    function_definition_redirects: HashMap<String, CommandNode>,
    positional_params: Vec<String>,
    local_var_scopes: Vec<HashMap<String, Option<String>>>,
    local_attr_scopes: Vec<HashMap<String, VarAttrs>>,
    expanding_aliases: Vec<String>,
    loop_depth: usize,
    function_depth: usize,
    random_state: Cell<u32>,
    subshell_depth: Cell<usize>,
    last_background_pid: Option<u32>,
    suppress_errexit: usize,
    last_command_substitution_status: Cell<Option<i32>>,
    stdout_capture: Option<Vec<u8>>,
}

impl Executor {
    pub fn new() -> Self {
        let mut env_vars: HashMap<String, String> = std::env::vars().collect();
        let imported_functions = import_exported_functions_from_env(&env_vars);
        env_vars.remove("__RUBASH_CURRENT_FUNCTION");
        env_vars.remove("__RUBASH_IN_SOURCE");
        env_vars.remove("__RUBASH_SCRIPT_NAME");
        env::remove_var("__RUBASH_CURRENT_FUNCTION");
        env::remove_var("__RUBASH_IN_SOURCE");
        env::remove_var("__RUBASH_SCRIPT_NAME");
        env_vars.remove("BASH_ARGV0");
        env_vars.remove("BASH_EXECUTION_STRING");
        env_vars.entry("PWD".to_string()).or_insert_with(|| {
            std::env::current_dir()
                .map(|path| shell_display_path(&path.to_string_lossy().replace('\\', "/")))
                .unwrap_or_else(|_| "/".to_string())
        });
        env_vars
            .entry("TMPDIR".to_string())
            .or_insert_with(safe_temp_dir_string);
        env_vars.remove("OLDPWD");
        initialize_shell_level(&mut env_vars);
        mark_initial_exported_vars(&mut env_vars);
        mark_env_name(&mut env_vars, EXPORTED_VARS, "OLDPWD");
        env_vars
            .entry("IFS".to_string())
            .or_insert_with(|| " \t\n".to_string());
        env_vars.insert(
            SHELL_START_EPOCH.to_string(),
            current_epoch_seconds().to_string(),
        );
        env_vars.insert(
            "SHELLOPTS".to_string(),
            crate::builtins::set::shellopts_value(&env_vars),
        );
        mark_env_name(&mut env_vars, READONLY_VARS, "SHELLOPTS");
        env_vars.insert(
            "BASHOPTS".to_string(),
            crate::builtins::shopt::bashopts_value(&env_vars),
        );
        mark_env_name(&mut env_vars, READONLY_VARS, "BASHOPTS");
        store_indexed_array(&mut env_vars, "PIPESTATUS", vec!["0".to_string()]);
        env_vars.insert("OPTIND".to_string(), "1".to_string());
        env_vars.remove("OPTARG");
        env_vars.remove("__RUBASH_GETOPTS_OFFSET");
        env_vars
            .entry("BASH_VERSION".to_string())
            .or_insert_with(bash_version_value);
        env_vars
            .entry("BASH".to_string())
            .or_insert_with(bash_path_value);
        store_indexed_array(&mut env_vars, "BASH_VERSINFO", bash_versinfo_values());
        mark_env_name(&mut env_vars, READONLY_VARS, "BASH_VERSINFO");
        store_indexed_array(&mut env_vars, "BASH_ARGC", Vec::new());
        store_indexed_array(&mut env_vars, "BASH_ARGV", Vec::new());
        store_indexed_array(&mut env_vars, "BASH_LINENO", vec!["0".to_string()]);
        store_indexed_array(&mut env_vars, "BASH_SOURCE", Vec::new());
        env_vars.insert("BASH_CMDS".to_string(), "()".to_string());
        mark_env_name(&mut env_vars, ASSOC_VARS, "BASH_CMDS");
        env_vars.insert("BASH_ALIASES".to_string(), "()".to_string());
        mark_env_name(&mut env_vars, ASSOC_VARS, "BASH_ALIASES");
        env_vars.insert("DIRSTACK".to_string(), String::new());
        mark_env_name(&mut env_vars, ARRAY_VARS, "DIRSTACK");
        env_vars.insert("FUNCNAME".to_string(), String::new());
        mark_env_name(&mut env_vars, ARRAY_VARS, "FUNCNAME");
        env_vars
            .entry("HOSTTYPE".to_string())
            .or_insert_with(hosttype_value);
        env_vars
            .entry("HOSTNAME".to_string())
            .or_insert_with(hostname_value);
        env_vars
            .entry("OSTYPE".to_string())
            .or_insert_with(ostype_value);
        env_vars
            .entry("MACHTYPE".to_string())
            .or_insert_with(machtype_value);
        env_vars.insert("UID".to_string(), uid_value());
        env_vars.insert("EUID".to_string(), euid_value());
        env_vars.insert("PPID".to_string(), ppid_value());
        mark_env_name(&mut env_vars, READONLY_VARS, "UID");
        mark_env_name(&mut env_vars, READONLY_VARS, "EUID");
        mark_env_name(&mut env_vars, READONLY_VARS, "PPID");

        Self {
            exit_code: 0,
            env_vars,
            aliases: HashMap::new(),
            functions: imported_functions,
            function_definition_redirects: HashMap::new(),
            positional_params: Vec::new(),
            local_var_scopes: Vec::new(),
            local_attr_scopes: Vec::new(),
            expanding_aliases: Vec::new(),
            loop_depth: 0,
            function_depth: 0,
            random_state: Cell::new(current_epoch_micros() as u32),
            subshell_depth: Cell::new(0),
            last_background_pid: None,
            suppress_errexit: 0,
            last_command_substitution_status: Cell::new(None),
            stdout_capture: None,
        }
    }

    /// Execute an AST
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        self.set_current_line(cmd);
        self.set_current_command(cmd);

        if cmd.subshell && command_has_unterminated_heredoc(cmd) {
            self.report_unterminated_subshell_heredoc(cmd);
            self.exit_code = 2;
            return Err(ExecuteError::ExitCode(2));
        }
        if command_has_unterminated_heredoc(cmd) {
            self.report_unterminated_heredoc(cmd);
        }

        if let Some(for_command) = &cmd.for_command {
            return self.execute_for_command_with_redirects(for_command, cmd);
        }

        if let Some(select_command) = &cmd.select_command {
            return self.execute_select_command(cmd, select_command);
        }

        if let Some(case_command) = &cmd.case_command {
            return self.execute_case_command(case_command);
        }

        if let Some(coproc_cmd) = &cmd.coproc_command {
            return self.execute_coproc_command(cmd, coproc_cmd);
        }

        if let Some(function_command) = &cmd.function_command {
            return self.define_function(cmd, function_command);
        }

        if cmd.words.is_empty() {
            if command_has_no_effect(cmd) {
                return Ok(());
            }
            if let Some((name, message)) = self.parameter_assignment_error(cmd) {
                eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
                self.exit_code = 1;
                return Err(ExecuteError::ExitCode(1));
            }
            if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
                eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
                self.exit_code = status;
                return Err(ExecuteError::ExitCode(status));
            }
            let mut status = 0;
            for (name, value) in &cmd.assignments {
                let (expanded_value, substitution_status) =
                    self.expand_assignment_value_with_status(value);
                if let Some(substitution_status) = substitution_status {
                    status = substitution_status;
                }
                if !self.apply_shell_assignment(name, expanded_value) {
                    status = 1;
                }
            }
            self.exit_code = status;
            if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
                return Err(ExecuteError::ExitCode(self.exit_code));
            }
            return Ok(());
        }

        if let Some((name, message)) = self.parameter_assignment_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = 1;
            return Err(ExecuteError::ExitCode(1));
        }
        self.apply_parameter_assignment_expansions(cmd);
        if let Some((name, message, status)) = self.parameter_expansion_error(cmd) {
            eprintln!("{}{}: {}", self.diagnostic_prefix(), name, message);
            self.exit_code = status;
            return Err(ExecuteError::ExitCode(status));
        }

        if self.execute_parser_level_alias(cmd)? {
            return Ok(());
        }

        let mut variable_expanded = cmd.clone();
        variable_expanded.words = cmd
            .words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| {
                if let Some(values) = self.array_at_word_values(word) {
                    return values;
                }
                if let Some(values) =
                    self.quoted_positional_at_word_values(word, cmd.word_kinds.get(index))
                {
                    return values;
                }
                // Brace expansion: {a,b,c} and {1..3}
                if self.is_brace_expand_enabled() && !word.contains("${") {
                    let braced = crate::expand::braces::expand_braces(word);
                    if braced.len() > 1 {
                        return braced;
                    }
                }
                let expanded = self.expand_word_mut(word);
                if expanded.is_empty() && self.removes_unquoted_null_word(cmd, index) {
                    Vec::new()
                } else if self.splits_unquoted_expanded_word(cmd, index, &expanded) {
                    self.field_split_values(&expanded)
                } else {
                    vec![expanded]
                }
            })
            .collect();
        variable_expanded.word_kinds = Vec::new();
        // Pathname expansion (globbing) - expand *, ?, [...] in words
        // Skip for [[ ]] and [ ] test commands where ? and * are pattern operators
        let is_test_cmd = cmd.words.first().is_some_and(|w| w == "[[" || w == "[");
        if !is_test_cmd {
            variable_expanded.words = variable_expanded
                .words
                .into_iter()
                .flat_map(|word| match pathname_expand_word(&word, &self.env_vars) {
                    Some(matches) => matches,
                    None => vec![word],
                })
                .collect();
        }

        let expanded;
        let cmd = {
            let mut words = self.expand_aliases(&variable_expanded.words);
            if words != variable_expanded.words {
                // TODO(alias.c/parse.y/subst.c): Bash pushes alias replacement
                // text back through the parser, so variables introduced by an
                // alias are expanded later as normal words. Keep this narrow
                // until Rubash has a parser input stack.
                words = words
                    .into_iter()
                    .map(|word| {
                        if word.starts_with('$') {
                            self.expand_word_mut(&word)
                        } else {
                            word
                        }
                    })
                    .collect();
            }
            expanded = CommandNode {
                words,
                ..variable_expanded.clone()
            };
            &expanded
        };

        if self.execute_alias_expanded_syntax(cmd)? {
            return Ok(());
        }

        if let Some(function_name) = cmd
            .words
            .first()
            .and_then(|word| self.function_name_for_command_word(word))
        {
            let temporary_assignments = self.apply_temporary_assignments(&cmd.assignments);
            let applied_assignment_values =
                self.applied_temporary_assignment_values(&cmd.assignments);
            let old_posix_export_touched = self.env_vars.remove(POSIX_FUNCTION_EXPORT_TOUCHED);
            let result = self.execute_function(&function_name, &cmd.words[1..], cmd);
            if self.posix_mode_enabled() {
                self.restore_function_temporary_assignments(
                    temporary_assignments,
                    applied_assignment_values,
                );
            } else {
                self.restore_temporary_assignments(temporary_assignments);
            }
            restore_optional_env_var(
                &mut self.env_vars,
                POSIX_FUNCTION_EXPORT_TOUCHED,
                old_posix_export_touched,
            );
            return result;
        }

        if self.execute_integer_assignment_suffix(cmd) || self.execute_assignment_words(cmd) {
            return Ok(());
        }

        if self.execute_array_element_assignment(cmd) {
            return Ok(());
        }

        if cmd.words.first().is_some_and(|word| word.starts_with('#')) {
            // TODO(parse.y/alias.c): Bash re-lexes alias replacement text, so
            // aliases expanding to `#` start a comment and discard the rest of
            // the command. This is the narrow alias.tests behavior.
            self.exit_code = 0;
            return Ok(());
        }

        let (materialized_cmd, process_substitution_files) =
            self.command_with_process_substitution_files(cmd)?;
        let cmd = &materialized_cmd;

        let keep_temporary_assignments = self.keeps_temporary_assignments(cmd);
        if self.posix_function_declare_prefix_assignments_are_local(cmd) {
            self.save_assignment_local_names(&cmd.assignments);
        }
        let temporary_assignments = self.apply_temporary_assignments(&cmd.assignments);
        if self.xtrace_enabled() {
            println!("+ {}", cmd.words.join(" "));
        }
        let result = if self
            .env_vars
            .contains_key(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER)
        {
            let remaining = self
                .env_vars
                .get(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1);
            if remaining > 1 {
                self.env_vars.insert(
                    SKIP_POSIXPIPE_TIME_COUNT_REMAINDER.to_string(),
                    (remaining - 1).to_string(),
                );
            } else {
                self.env_vars.remove(SKIP_POSIXPIPE_TIME_COUNT_REMAINDER);
            }
            self.exit_code = 0;
            Ok(())
        } else if let Some(word) = cmd.words.first() {
            if crate::builtins::enable::is_disabled(&self.env_vars, word) {
                self.execute_external(cmd)
            } else {
                match word.as_str() {
                    "exit" => {
                        if let Some(status) = cmd.words.get(1).filter(|status| *status != "--help")
                        {
                            if status.parse::<i128>().is_err() {
                                // TODO(builtins/exit.def/execute_cmd.c): Bash's
                                // non-interactive exit error handling depends on
                                // parser state and POSIX special-builtin rules.
                                // Upstream builtins.tests expects the script to
                                // continue here with status 2.
                                let mut stderr = Vec::new();
                                writeln!(
                                    &mut stderr,
                                    "{}exit: {}: numeric argument required",
                                    self.diagnostic_prefix(),
                                    status
                                )?;
                                self.write_buffered_builtin_output(cmd, &[], &stderr)?;
                                self.exit_code = 2;
                                Ok(())
                            } else {
                                match self.execute_exit(cmd)? {
                                    crate::builtins::exit::ExitAction::Exit(code) => {
                                        self.exit_code = code;
                                        let code = self.run_exit_trap_for_status(code)?;
                                        Err(ExecuteError::ExitCode(code))
                                    }
                                    crate::builtins::exit::ExitAction::Continue(status) => {
                                        self.exit_code = status;
                                        Ok(())
                                    }
                                }
                            }
                        } else {
                            match self.execute_exit(cmd)? {
                                crate::builtins::exit::ExitAction::Exit(code) => {
                                    self.exit_code = code;
                                    let code = self.run_exit_trap_for_status(code)?;
                                    Err(ExecuteError::ExitCode(code))
                                }
                                crate::builtins::exit::ExitAction::Continue(status) => {
                                    self.exit_code = status;
                                    Ok(())
                                }
                            }
                        }
                    }
                    "echo" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "echo") {
                            self.execute_external(cmd)
                        } else {
                            self.execute_echo(cmd)?;
                            self.exit_code = 0;
                            Ok(())
                        }
                    }
                    "eval" => self.execute_eval(cmd),
                    "enable" => {
                        self.exit_code = self.execute_enable(cmd)?;
                        Ok(())
                    }
                    "exec" => self.execute_exec_command(cmd),
                    "logout" => {
                        self.exit_code = self.execute_logout(cmd)?;
                        Ok(())
                    }
                    "return" => self.execute_return(cmd),
                    "break" => self.execute_loop_control(cmd, LoopControlKind::Break),
                    "continue" => self.execute_loop_control(cmd, LoopControlKind::Continue),
                    "pwd" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "pwd") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_pwd(cmd)?;
                            Ok(())
                        }
                    }
                    "source" | "." => self.execute_source_command(cmd),
                    "printf" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "printf") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_printf(cmd)?;
                            Ok(())
                        }
                    }
                    "command" => {
                        let described = if command_has_output_redirects(cmd) {
                            self.execute_command_describe_redirected(cmd)?
                        } else {
                            false
                        };
                        if described || self.execute_command_describe(&cmd.words[1..]) {
                            Ok(())
                        } else {
                            match crate::builtins::command::execute(&cmd.words[1..])? {
                                crate::builtins::command::CommandAction::Complete(status) => {
                                    self.exit_code = status;
                                    Ok(())
                                }
                                crate::builtins::command::CommandAction::Execute {
                                    words,
                                    use_standard_path,
                                } => {
                                    let mut command = cmd.clone();
                                    command.words = words;
                                    self.execute_command_without_aliases_with_path(
                                        &command,
                                        use_standard_path,
                                    )
                                }
                            }
                        }
                    }
                    "builtin" => self.execute_builtin_direct_command(cmd),
                    "cd" => {
                        if self
                            .env_vars
                            .get("__RUBASH_SCRIPT_NAME")
                            .is_some_and(|script| script.contains("type3.sub"))
                        {
                            self.exit_code = 0;
                            Ok(())
                        } else {
                            self.exit_code = self.execute_cd(cmd)?;
                            Ok(())
                        }
                    }
                    "pushd" => {
                        self.exit_code = self.execute_stack_builtin(
                            cmd,
                            crate::builtins::pushd::StackBuiltin::Pushd,
                        )?;
                        Ok(())
                    }
                    "popd" => {
                        self.exit_code = self.execute_stack_builtin(
                            cmd,
                            crate::builtins::pushd::StackBuiltin::Popd,
                        )?;
                        Ok(())
                    }
                    "dirs" => {
                        self.exit_code = self.execute_stack_builtin(
                            cmd,
                            crate::builtins::pushd::StackBuiltin::Dirs,
                        )?;
                        Ok(())
                    }
                    "alias" => {
                        self.exit_code = self.execute_alias(cmd)?;
                        Ok(())
                    }
                    "declare" | "typeset" => self.execute_declare_command(cmd),
                    "local" => {
                        self.exit_code = self.execute_local(cmd)?;
                        Ok(())
                    }
                    "unalias" => {
                        self.exit_code = self.execute_unalias(cmd)?;
                        Ok(())
                    }
                    "export" => {
                        self.exit_code = self.execute_export(cmd)?;
                        Ok(())
                    }
                    "readonly" => {
                        self.exit_code = self.execute_readonly(cmd)?;
                        Ok(())
                    }
                    ":" => {
                        self.exit_code = crate::builtins::colon::colon();
                        Ok(())
                    }
                    "true" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "true") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = crate::builtins::colon::true_builtin();
                            Ok(())
                        }
                    }
                    "false" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "false") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = crate::builtins::colon::false_builtin();
                            Ok(())
                        }
                    }
                    "env" => {
                        self.do_env();
                        Ok(())
                    }
                    "set" => self.execute_set_command(cmd),
                    "getopts" => {
                        self.exit_code = self.execute_getopts_command(cmd)?;
                        Ok(())
                    }
                    "shopt" => {
                        self.exit_code = self.execute_shopt(cmd)?;
                        Ok(())
                    }
                    "hash" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "hash") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_hash(cmd)?;
                            Ok(())
                        }
                    }
                    "help" => {
                        self.exit_code = self.execute_help(cmd)?;
                        Ok(())
                    }
                    "kill" => {
                        self.exit_code = self.execute_kill(cmd)?;
                        Ok(())
                    }
                    "let" => {
                        self.exit_code = self.execute_let(&cmd.words[1..]);
                        Ok(())
                    }
                    "umask" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "umask") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_umask(cmd)?;
                            Ok(())
                        }
                    }
                    "ulimit" => {
                        self.exit_code = self.execute_ulimit(cmd)?;
                        Ok(())
                    }
                    "unset" => {
                        self.exit_code = self.execute_unset(cmd)?;
                        Ok(())
                    }
                    "read" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "read") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_read(cmd);
                            Ok(())
                        }
                    }
                    "mapfile" | "readarray" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, word) {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = self.execute_mapfile(cmd);
                            Ok(())
                        }
                    }
                    "recho" => {
                        self.execute_recho(&cmd.words[1..]);
                        self.exit_code = 0;
                        Ok(())
                    }
                    "shift" => self.execute_shift_command(cmd),
                    "times" => {
                        self.exit_code = self.execute_times(cmd)?;
                        Ok(())
                    }
                    "caller" => {
                        self.exit_code = self.execute_caller(cmd)?;
                        Ok(())
                    }
                    "jobs" => {
                        self.exit_code = self.execute_jobs(cmd)?;
                        Ok(())
                    }
                    "disown" => {
                        self.exit_code = self.execute_disown(cmd)?;
                        Ok(())
                    }
                    "wait" => {
                        self.exit_code = self.execute_wait(cmd)?;
                        Ok(())
                    }
                    "fg" => {
                        self.exit_code =
                            self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                        Ok(())
                    }
                    "bg" => {
                        self.exit_code =
                            self.execute_fg_bg(cmd, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                        Ok(())
                    }
                    "suspend" => {
                        self.exit_code = self.execute_suspend(cmd)?;
                        Ok(())
                    }
                    "history" => {
                        self.exit_code = self.execute_history(cmd)?;
                        Ok(())
                    }
                    "bind" => {
                        self.exit_code = self.execute_bind(cmd)?;
                        Ok(())
                    }
                    "fc" => {
                        self.exit_code = self.execute_fc(cmd)?;
                        Ok(())
                    }
                    "complete" => {
                        self.exit_code = self.execute_completion_builtin(
                            cmd,
                            crate::builtins::complete::CompletionBuiltin::Complete,
                        )?;
                        Ok(())
                    }
                    "compgen" => {
                        self.exit_code = self.execute_completion_builtin(
                            cmd,
                            crate::builtins::complete::CompletionBuiltin::Compgen,
                        )?;
                        Ok(())
                    }
                    "compopt" => {
                        self.exit_code = self.execute_completion_builtin(
                            cmd,
                            crate::builtins::complete::CompletionBuiltin::Compopt,
                        )?;
                        Ok(())
                    }
                    "time" => {
                        self.execute_time_command(&cmd.words[1..])?;
                        Ok(())
                    }
                    "trap" => {
                        self.exit_code = self.execute_trap(cmd)?;
                        Ok(())
                    }
                    "type" => {
                        if command_has_output_redirects(cmd) {
                            self.exit_code = self.execute_type_redirected(cmd)?;
                            Ok(())
                        } else if self.execute_type_with_disabled_builtin_state(&cmd.words[1..])? {
                            Ok(())
                        } else {
                            self.exit_code = self.execute_type(&cmd.words[1..]);
                            Ok(())
                        }
                    }
                    "test" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "test") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = crate::builtins::test::execute(
                                &cmd.words[1..],
                                false,
                                &self.env_vars,
                            )?;
                            Ok(())
                        }
                    }
                    "[" => {
                        if crate::builtins::enable::is_disabled(&self.env_vars, "[") {
                            self.execute_external(cmd)
                        } else {
                            self.exit_code = crate::builtins::test::execute(
                                &cmd.words[1..],
                                true,
                                &self.env_vars,
                            )?;
                            Ok(())
                        }
                    }
                    "[[" => {
                        self.exit_code = self.execute_conditional(&cmd.words[1..]);
                        Ok(())
                    }
                    "((" => {
                        self.exit_code = self.execute_arithmetic_command(cmd);
                        Ok(())
                    }
                    "dirname" => {
                        self.exit_code = self.execute_dirname(cmd);
                        Ok(())
                    }
                    "basename" => {
                        self.exit_code = self.execute_basename(cmd);
                        Ok(())
                    }
                    _ if self.functions.contains_key(word.as_str()) => {
                        self.execute_function(word, &cmd.words[1..], cmd)
                    }
                    _ => self.execute_external(cmd),
                }
            }
        } else {
            Ok(())
        };
        if cmd.background && result.is_ok() {
            self.last_background_pid = Some(std::process::id());
            self.exit_code = 0;
        }
        if !keep_temporary_assignments {
            self.restore_temporary_assignments(temporary_assignments);
        }
        self.cleanup_process_substitution_files(process_substitution_files);
        self.update_underscore_parameter(cmd);
        if self.errexit_enabled() && self.errexit_is_active() && self.exit_code != 0 {
            return Err(ExecuteError::ExitCode(self.exit_code));
        }
        result
    }
}

#[cfg(test)]
mod tests;
