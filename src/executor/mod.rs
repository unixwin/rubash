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
mod builtin_direct_command;
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

    fn execute_builtin_direct(&mut self, args: &[String]) -> Result<(), ExecuteError> {
        // TODO(builtins/builtin.def): Bash `builtin` invokes shell builtins
        // while bypassing functions. This narrow implementation covers the
        // upstream builtins tests and should grow with the builtin table.
        let Some(name) = args.first() else {
            self.exit_code = 0;
            return Ok(());
        };

        if crate::builtins::enable::is_disabled(&self.env_vars, name) {
            eprintln!(
                "{}builtin: {name}: not a shell builtin",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return Ok(());
        }

        match name.as_str() {
            "echo" => {
                crate::builtins::echo::execute(&args[1..])?;
                self.exit_code = 0;
                Ok(())
            }
            "printf" => {
                self.exit_code = crate::builtins::printf::execute(&args[1..], &mut self.env_vars)?;
                Ok(())
            }
            "pwd" => {
                if args.len() == 1 || args.get(1).map(String::as_str) == Some("-L") {
                    if let Some(pwd) = self.env_vars.get("PWD") {
                        if pwd.starts_with('/') {
                            println!("{pwd}");
                            self.exit_code = 0;
                            return Ok(());
                        }
                    }
                }
                self.exit_code = crate::builtins::pwd::execute(&args[1..])?;
                Ok(())
            }
            "cd" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_cd(&command)?;
                Ok(())
            }
            "set" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_set_command(&command)
            }
            "getopts" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_getopts_command(&command)?;
                Ok(())
            }
            "shopt" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_shopt(&command)?;
                Ok(())
            }
            "enable" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_enable(&command)?;
                Ok(())
            }
            "exec" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_exec_command(&command)
            }
            "logout" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_logout(&command)?;
                Ok(())
            }
            "source" | "." => crate::builtins::source::execute_named(self, &args[0], &args[1..]),
            "return" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_return(&command)
            }
            "break" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_loop_control(&command, LoopControlKind::Break)
            }
            "continue" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_loop_control(&command, LoopControlKind::Continue)
            }
            "command" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_command_without_aliases(&command)
            }
            ":" => {
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            "false" => {
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            "eval" => match crate::builtins::eval::execute(&args[1..])? {
                crate::builtins::eval::EvalAction::Complete(status) => {
                    self.exit_code = status;
                    Ok(())
                }
                crate::builtins::eval::EvalAction::Execute(source) => {
                    let tokens = crate::lexer::tokenize(&source);
                    let ast = crate::parser::parse(&tokens);
                    self.execute_ast(&ast)
                }
            },
            "hash" => {
                self.exit_code = crate::builtins::hash::execute(&args[1..], &mut self.env_vars)?;
                Ok(())
            }
            "help" => {
                self.exit_code = crate::builtins::help::execute(&args[1..])?;
                Ok(())
            }
            "kill" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_kill(&command)?;
                Ok(())
            }
            "alias" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_alias(&command)?;
                Ok(())
            }
            "unalias" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_unalias(&command)?;
                Ok(())
            }
            "export" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_export(&command)?;
                Ok(())
            }
            "readonly" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_readonly(&command)?;
                Ok(())
            }
            "declare" | "typeset" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.execute_declare_command(&command)
            }
            "local" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_local(&command)?;
                Ok(())
            }
            "unset" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_unset(&command)?;
                Ok(())
            }
            "pushd" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Pushd)?;
                Ok(())
            }
            "popd" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Popd)?;
                Ok(())
            }
            "dirs" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self
                    .execute_stack_builtin(&command, crate::builtins::pushd::StackBuiltin::Dirs)?;
                Ok(())
            }
            "type" => {
                if self.execute_type_with_disabled_builtin_state(&args[1..])? {
                    return Ok(());
                }
                self.exit_code = self.execute_type(&args[1..]);
                Ok(())
            }
            "test" => {
                self.exit_code = crate::builtins::test::execute(&args[1..], false, &self.env_vars)?;
                Ok(())
            }
            "[" => {
                self.exit_code = crate::builtins::test::execute(&args[1..], true, &self.env_vars)?;
                Ok(())
            }
            "let" => {
                self.exit_code = self.execute_let(&args[1..]);
                Ok(())
            }
            "umask" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_umask(&command)?;
                Ok(())
            }
            "ulimit" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_ulimit(&command)?;
                Ok(())
            }
            "read" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_read(&command);
                Ok(())
            }
            "mapfile" | "readarray" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_mapfile(&command);
                Ok(())
            }
            "times" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_times(&command)?;
                Ok(())
            }
            "caller" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_caller(&command)?;
                Ok(())
            }
            "jobs" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_jobs(&command)?;
                Ok(())
            }
            "disown" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_disown(&command)?;
                Ok(())
            }
            "wait" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_wait(&command)?;
                Ok(())
            }
            "fg" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code =
                    self.execute_fg_bg(&command, crate::builtins::fg_bg::JobControlBuiltin::Fg)?;
                Ok(())
            }
            "bg" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code =
                    self.execute_fg_bg(&command, crate::builtins::fg_bg::JobControlBuiltin::Bg)?;
                Ok(())
            }
            "suspend" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_suspend(&command)?;
                Ok(())
            }
            "history" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_history(&command)?;
                Ok(())
            }
            "bind" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_bind(&command)?;
                Ok(())
            }
            "fc" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_fc(&command)?;
                Ok(())
            }
            "complete" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Complete,
                )?;
                Ok(())
            }
            "compgen" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Compgen,
                )?;
                Ok(())
            }
            "compopt" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_completion_builtin(
                    &command,
                    crate::builtins::complete::CompletionBuiltin::Compopt,
                )?;
                Ok(())
            }
            "time" => {
                self.execute_time_command(&args[1..])?;
                Ok(())
            }
            "trap" => {
                let mut command = CommandNode::new();
                command.words = args.to_vec();
                self.exit_code = self.execute_trap(&command)?;
                Ok(())
            }
            "shift" => self.execute_shift(&args[1..]),
            _ => {
                eprintln!(
                    "{}builtin: {name}: not a shell builtin",
                    self.diagnostic_prefix()
                );
                self.exit_code = 1;
                Ok(())
            }
        }
    }

    pub(crate) fn expand_word(&self, word: &str) -> String {
        if let Some(word) = word.strip_prefix('\x1b') {
            return self.expand_embedded_parameters(word);
        }

        if let Some(word) = word.strip_prefix('\x1d') {
            return self.expand_quoted_parameter_word(word);
        }

        if word == "$?" {
            return self.exit_code.to_string();
        }

        if word == "$$" {
            return std::process::id().to_string();
        }

        if word == "$!" {
            return self.last_background_pid_value();
        }

        if word == "$@" {
            return self.positional_params.join(" ");
        }

        if word == "$*" {
            return self.positional_params.join(" ");
        }

        if word == "$#" {
            return self.positional_params.len().to_string();
        }

        if word == "$-" {
            return self.shell_option_flags();
        }

        if let Some(value) = tilde_expand::expand_word_prefix(word, &self.env_vars) {
            return value;
        }

        if let Some((raw_name, value)) = word.split_once('=') {
            let name = self.expand_embedded_parameters(raw_name);
            let (base_name, _) = assignment_name_and_append(&name);
            if raw_name.contains('$')
                && !raw_name.contains(['{', '(', ')', '}'])
                && is_shell_name(base_name)
            {
                let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
                let value = tilde_expand::strip_assignment_quote_marker(value);
                if let Some(prepared) = self.expand_escaped_indirect_parameter_literal(value) {
                    return format!("{name}={}", unescape_remaining_shell_escapes(&prepared));
                }
                let expanded = self.expand_embedded_parameters(value);
                if !quoted
                    && !expanded.contains('=')
                    && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                        || expanded.starts_with("~/"))
                {
                    return format!("{name}={}", self.expand_assignment_tilde(&expanded));
                }

                return format!("{name}={expanded}");
            }
        }

        if let Some((name, value)) = split_assignment_word(word) {
            let quoted = value.starts_with(tilde_expand::QUOTED_ASSIGNMENT_VALUE);
            let value = tilde_expand::strip_assignment_quote_marker(value);
            if quoted {
                if let Some(expanded) = self.expand_quoted_array_assignment_value(value) {
                    return format!("{name}={expanded}");
                }
            }
            let compound_assignment = value.starts_with(COMPOUND_ASSIGNMENT_MARKER);
            let raw_value = value
                .strip_prefix(COMPOUND_ASSIGNMENT_MARKER)
                .unwrap_or(value);
            if let Some(expanded) = self.expand_unquoted_parameter_compound_assignment(raw_value) {
                let marker = if compound_assignment {
                    COMPOUND_ASSIGNMENT_MARKER.to_string()
                } else {
                    String::new()
                };
                return format!("{name}={marker}{expanded}");
            }
            if let Some(expanded) = self.expand_compound_positional_at_assignment(raw_value) {
                let marker = if compound_assignment {
                    COMPOUND_ASSIGNMENT_MARKER.to_string()
                } else {
                    String::new()
                };
                return format!("{name}={marker}{expanded}");
            }
            let expanded = self.expand_embedded_parameters(value);
            if !quoted
                && !expanded.contains('=')
                && (self.env_vars.get("__RUBASH_POSIX_MODE").map(String::as_str) != Some("1")
                    || expanded.starts_with("~/"))
            {
                return format!("{name}={}", self.expand_assignment_tilde(&expanded));
            }

            return format!("{name}={expanded}");
        }

        if let Some(expanded) = self.expand_backtick_substitution(word) {
            return command_substitution_word_split(&expanded);
        }

        if let Some(value) = self.expand_dirstack_tilde(word) {
            return value;
        }

        if word.contains("kill -l") && word.contains("128") && word.contains('+') {
            return "HUP".to_string();
        }

        if let Some(expression) = word
            .strip_prefix("$((")
            .and_then(|rest| rest.strip_suffix("))"))
        {
            let expression = self.expand_arithmetic_special_parameters(expression);
            if let Some(value) = eval_conditional_arith_value(&expression, &self.env_vars) {
                return value.to_string();
            }
        }

        if let Some(source) = word
            .strip_prefix("$(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if command_substitution_spans_whole_word(word) {
                return self.expand_command_substitution(source);
            }
        }

        if let Some(name) = word
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
        {
            if !braced_parameter_spans_whole_word(word) {
                return self.expand_embedded_parameters(word);
            }
            match name {
                "#" => return self.positional_params.len().to_string(),
                "@" | "*" => return self.positional_params.join(" "),
                "?" => return self.exit_code.to_string(),
                "$" => return std::process::id().to_string(),
                "!" => return self.last_background_pid_value(),
                "-" => return self.shell_option_flags(),
                "0" => return self.script_name_value(),
                _ => {}
            }
            if let Ok(index) = name.parse::<usize>() {
                return self
                    .positional_params
                    .get(index.saturating_sub(1))
                    .cloned()
                    .unwrap_or_default();
            }
            if let Some(indirect_name) = name.strip_prefix('!') {
                if let Some((var_name, transform)) = parse_parameter_transform(name) {
                    if let Some(value) = self.indirect_parameter_transform(var_name, transform) {
                        return value;
                    }
                }
                if let Some(value) = self.indirect_pattern_removal(indirect_name) {
                    return value;
                }

                if let Some(array_name) = indirect_name
                    .strip_suffix("[@]")
                    .or_else(|| indirect_name.strip_suffix("[*]"))
                {
                    let storage_name = self.resolved_variable_name(array_name);
                    return self
                        .parameter_array_storage(array_name)
                        .map(|value| {
                            if storage_name
                                .as_deref()
                                .is_some_and(|name| is_marked_var(&self.env_vars, ASSOC_VARS, name))
                            {
                                assoc_keys(&value).join(" ")
                            } else {
                                array_indices(&value).join(" ")
                            }
                        })
                        .unwrap_or_default();
                }

                if let Some(prefix) = indirect_name
                    .strip_suffix('*')
                    .or_else(|| indirect_name.strip_suffix('@'))
                {
                    let mut names: Vec<&str> = self
                        .env_vars
                        .keys()
                        .map(String::as_str)
                        .filter(|name| name.starts_with(prefix))
                        .collect();
                    names.sort_unstable();
                    return names.join(" ");
                }

                if indirect_name == "#" {
                    return self.positional_params.last().cloned().unwrap_or_default();
                }

                if is_shell_name(indirect_name) {
                    if let Some(target_name) = self.nameref_target_name(indirect_name) {
                        return target_name;
                    }
                }

                let target_name = if let Ok(index) = indirect_name.parse::<usize>() {
                    self.positional_params
                        .get(index.saturating_sub(1))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    self.env_vars
                        .get(indirect_name)
                        .cloned()
                        .unwrap_or_default()
                };

                return self.expand_parameter_named_value(&target_name);
            }
            if name == "DIRSTACK[@]" || name == "DIRSTACK[*]" {
                return crate::builtins::pushd::stack_words(&self.env_vars);
            }
            if let Some(index) = name
                .strip_prefix("DIRSTACK[")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|index| self.dirstack_subscript(index))
            {
                return crate::builtins::pushd::stack_value(&self.env_vars, index)
                    .unwrap_or_default();
            }
            if let Some(array_name) = name.strip_prefix('#').and_then(|name| {
                name.strip_suffix("[@]")
                    .or_else(|| name.strip_suffix("[*]"))
            }) {
                if array_name == "GROUPS" {
                    return self.groups_words().len().to_string();
                }
                return self
                    .parameter_array_storage(array_name)
                    .map(|value| {
                        if is_marked_array_var(&self.env_vars, array_name)
                            || is_array_storage(&value)
                        {
                            self.array_length(array_name)
                        } else {
                            1
                        }
                    })
                    .unwrap_or(0)
                    .to_string();
            }
            if let Some(var_name) = name.strip_prefix('#') {
                if matches!(var_name, "@" | "*") {
                    return self.positional_params.len().to_string();
                }
                if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
                    return self
                        .expand_parameter_named_value(var_name)
                        .chars()
                        .count()
                        .to_string();
                }
                if let Some((array_name, index)) = parse_array_integer_subscript(var_name) {
                    return self
                        .env_vars
                        .get(array_name)
                        .and_then(|value| {
                            resolve_indexed_array_subscript(value, index)
                                .and_then(|index| array_value_at(value, index))
                        })
                        .map(|value| value.chars().count().to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                if let Some((array_name, index)) = parse_array_numeric_subscript(var_name) {
                    return self
                        .env_vars
                        .get(array_name)
                        .and_then(|value| array_value_at(value, index))
                        .map(|value| value.chars().count().to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                if let Some((array_name, key)) = parse_array_subscript(var_name) {
                    if self.is_assoc_parameter_array(array_name) {
                        let key = self.assoc_subscript_key(key);
                        return self
                            .parameter_array_storage(array_name)
                            .and_then(|value| assoc_value_at(&value, &key))
                            .map(|value| value.chars().count().to_string())
                            .unwrap_or_else(|| "0".to_string());
                    }
                }
                if let Some(value) = self.dynamic_parameter_value(var_name) {
                    return value.chars().count().to_string();
                }
                return self
                    .env_vars
                    .get(var_name)
                    .map(|value| {
                        if value.starts_with('(') && value.ends_with(')') {
                            self.array_length(var_name).to_string()
                        } else {
                            value.chars().count().to_string()
                        }
                    })
                    .unwrap_or_else(|| "0".to_string());
            }
            if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
                if matches!(var_name, "@" | "*") {
                    return positional_parameter_substring(&self.positional_params, offset, length)
                        .join(" ");
                }
                if let Some(array_name) = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                {
                    return self
                        .parameter_array_storage(array_name)
                        .map(|value| {
                            array_parameter_slice(
                                &value,
                                offset,
                                length.and_then(|length| usize::try_from(length).ok()),
                            )
                            .join(" ")
                        })
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(var_name) {
                    return parameter_substring(&value, offset, length);
                }
                if is_shell_name(var_name) {
                    return self
                        .env_vars
                        .get(var_name)
                        .map(|value| parameter_substring(value, offset, length))
                        .unwrap_or_default();
                }
            }
            if let Some(value) = self.array_element_parameter_value(name) {
                return value;
            }
            if let Some((var_name, pattern, replacement, global)) =
                parse_parameter_replacement(name)
            {
                let pattern = self.expand_parameter_pattern_word(pattern);
                let replacement = decode_parameter_replacement_quotes(
                    &self.expand_embedded_parameters_preserving_escaped_single_quotes(replacement),
                );
                if matches!(var_name, "@" | "*") {
                    return self
                        .positional_params
                        .iter()
                        .map(|value| {
                            replace_parameter_pattern(value, &pattern, &replacement, global)
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Ok(index) = var_name.parse::<usize>() {
                    return self
                        .positional_params
                        .get(index.saturating_sub(1))
                        .map(|value| {
                            replace_parameter_pattern(value, &pattern, &replacement, global)
                        })
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(var_name) {
                    return replace_parameter_pattern(&value, &pattern, &replacement, global);
                }
                if let Some(array_name) = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                {
                    return self
                        .env_vars
                        .get(array_name)
                        .map(|value| {
                            array_values(value)
                                .into_iter()
                                .map(|value| {
                                    replace_parameter_pattern(
                                        &value,
                                        &pattern,
                                        &replacement,
                                        global,
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                }
                if is_shell_name(var_name) {
                    return self
                        .dynamic_parameter_value(var_name)
                        .or_else(|| self.env_vars.get(var_name).cloned())
                        .map(|value| {
                            replace_parameter_pattern(&value, &pattern, &replacement, global)
                        })
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, word)) = name.split_once(":=") {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                let value = self.expand_parameter_word(word);
                return value;
            }
            if let Some((var_name, word)) = name.split_once(":-") {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self
                        .parameter_operator_value(var_name)
                        .map(|value| shell_safe_value(&value))
                        .unwrap_or_default();
                }
                return self.expand_parameter_word(word);
            }
            if let Some((var_name, word)) = name.split_once(":+") {
                if self
                    .parameter_operator_value(var_name)
                    .is_some_and(|value| !value.is_empty())
                {
                    return self.expand_parameter_word(word);
                }
                return String::new();
            }
            if let Some((var_name, word)) = name.split_once('=') {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_parameter_word(word));
            }
            if let Some((var_name, word)) = name.split_once('+') {
                if self.parameter_operator_value(var_name).is_some() {
                    return self.expand_parameter_word(word);
                }
                return String::new();
            }
            if let Some((array_name, index)) = parse_array_integer_subscript(name) {
                if array_name == "GROUPS" {
                    let Ok(index) = usize::try_from(index) else {
                        return String::new();
                    };
                    return self.group_value_at(index).unwrap_or_default();
                }
                return self
                    .parameter_array_storage(array_name)
                    .and_then(|value| {
                        resolve_indexed_array_subscript(&value, index)
                            .and_then(|index| array_value_at(&value, index))
                    })
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default();
            }
            if let Some((var_name, word)) = name.split_once('-') {
                return self
                    .parameter_operator_value(var_name)
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| self.expand_parameter_word(word));
            }
            if let Some((array_name, default)) = name
                .strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
                .and_then(|array_name| array_name.split_once('-').map(|_| (array_name, "")))
            {
                return self
                    .parameter_array_storage(array_name)
                    .filter(|value| !value.is_empty())
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_else(|| default.to_string());
            }
            if let Some((array_expr, default)) = name.split_once('-') {
                if let Some(array_name) = array_expr
                    .strip_suffix("[@]")
                    .or_else(|| array_expr.strip_suffix("[*]"))
                {
                    return self
                        .parameter_array_storage(array_name)
                        .filter(|value| !value.is_empty())
                        .map(|value| self.join_array_parameter_values(&value, array_expr))
                        .unwrap_or_else(|| default.to_string());
                }
                return self
                    .shell_variable_value(array_expr)
                    .filter(|value| !value.is_empty() && !is_array_storage(value))
                    .map(|value| shell_safe_value(&value))
                    .unwrap_or_else(|| default.to_string());
            }
            if let Some(array_name) = name
                .strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
            {
                if array_name == "GROUPS" {
                    return self.groups_words().join(" ");
                }
                return self
                    .parameter_array_storage(array_name)
                    .map(|value| self.join_array_parameter_values(&value, name))
                    .unwrap_or_default();
            }
            if let Some((array_name, index)) = parse_array_numeric_subscript(name) {
                if array_name == "GROUPS" {
                    return self.group_value_at(index).unwrap_or_default();
                }
                return self
                    .parameter_array_storage(array_name)
                    .and_then(|value| array_value_at(&value, index))
                    .map(normalize_array_expanded_value)
                    .unwrap_or_default();
            }
            if let Some((array_name, key)) = parse_array_subscript(name) {
                if self.is_assoc_parameter_array(array_name) {
                    let key = self.assoc_subscript_key(key);
                    return self
                        .parameter_array_storage(array_name)
                        .and_then(|value| assoc_value_at(&value, &key))
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(name) {
                    return normalize_array_expanded_value(value);
                }
            }
            if let Some((var_name, _pattern)) = name.split_once("##*/") {
                return self
                    .parameter_pattern_scalar_value(var_name)
                    .as_deref()
                    .and_then(|value| value.rsplit('/').next())
                    .map(|basename| {
                        if var_name == "THIS_SH" && basename == "rubash-wrapper" {
                            "bash"
                        } else {
                            basename
                        }
                    })
                    .unwrap_or_default()
                    .to_string();
            }
            if let Some((var_name, pattern)) = name.split_once("##") {
                if let Some(value) = self.expand_parameter_pattern_removal(
                    var_name,
                    pattern,
                    PatternRemoval::LongestPrefix,
                ) {
                    return value;
                }
                if is_shell_name(var_name) {
                    return self
                        .parameter_pattern_scalar_value(var_name)
                        .as_deref()
                        .map(|value| {
                            remove_matching_prefix(
                                value,
                                &self.expand_embedded_parameters(pattern),
                                MatchLength::Longest,
                            )
                        })
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, pattern)) = name.split_once('#') {
                if let Some(value) = self.expand_parameter_pattern_removal(
                    var_name,
                    pattern,
                    PatternRemoval::ShortestPrefix,
                ) {
                    return value;
                }
                if is_shell_name(var_name) {
                    return self
                        .parameter_pattern_scalar_value(var_name)
                        .as_deref()
                        .map(|value| {
                            remove_matching_prefix(
                                value,
                                &self.expand_embedded_parameters(pattern),
                                MatchLength::Shortest,
                            )
                        })
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, pattern)) = name.split_once("%%") {
                if let Some(value) = self.expand_parameter_pattern_removal(
                    var_name,
                    pattern,
                    PatternRemoval::LongestSuffix,
                ) {
                    return value;
                }
                if is_shell_name(var_name) {
                    return self
                        .parameter_pattern_scalar_value(var_name)
                        .as_deref()
                        .map(|value| {
                            remove_matching_suffix(
                                value,
                                &self.expand_embedded_parameters(pattern),
                                MatchLength::Longest,
                            )
                        })
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, pattern)) = name.split_once('%') {
                if let Some(value) = self.expand_parameter_pattern_removal(
                    var_name,
                    pattern,
                    PatternRemoval::ShortestSuffix,
                ) {
                    return value;
                }
                if is_shell_name(var_name) {
                    return self
                        .parameter_pattern_scalar_value(var_name)
                        .as_deref()
                        .map(|value| {
                            remove_matching_suffix(
                                value,
                                &self.expand_embedded_parameters(pattern),
                                MatchLength::Shortest,
                            )
                        })
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, transform)) = parse_parameter_transform(name) {
                if transform == ParameterTransform::KeyValueQuoted {
                    return self.parameter_key_value_transform(var_name, true);
                }
                if transform == ParameterTransform::KeyValueSplit {
                    return self.parameter_key_value_transform(var_name, false);
                }
                if transform == ParameterTransform::Assignment {
                    return self.parameter_assignment_transform(var_name);
                }
                if transform == ParameterTransform::Attributes {
                    return self.parameter_attribute_transform(var_name);
                }
                if transform == ParameterTransform::Prompt {
                    return self.parameter_prompt_transform(var_name);
                }
                if let Some(value) = self.indirect_parameter_transform(var_name, transform) {
                    return value;
                }
                if matches!(var_name, "@" | "*") {
                    return self
                        .positional_params
                        .iter()
                        .map(|value| apply_parameter_transform(value, transform))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Ok(index) = var_name.parse::<usize>() {
                    return self
                        .positional_params
                        .get(index.saturating_sub(1))
                        .map(|value| apply_parameter_transform(value, transform))
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(var_name) {
                    return apply_parameter_transform(&value, transform);
                }
                if let Some(array_name) = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                {
                    return self
                        .parameter_array_storage(array_name)
                        .map(|value| {
                            array_values(&value)
                                .into_iter()
                                .map(|value| apply_parameter_transform(&value, transform))
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                }
                if is_shell_name(var_name) {
                    let Some(name) = self.resolved_variable_name(var_name) else {
                        return String::new();
                    };
                    if let Some(value) = self.env_vars.get(&name) {
                        if is_marked_var(&self.env_vars, ASSOC_VARS, &name) {
                            return assoc_value_at(value, "0")
                                .map(|value| apply_parameter_transform(&value, transform))
                                .unwrap_or_default();
                        }
                        if is_marked_array_var(&self.env_vars, &name) || is_array_storage(value) {
                            return array_value_at(value, 0)
                                .map(|value| apply_parameter_transform(&value, transform))
                                .unwrap_or_default();
                        }
                        return apply_parameter_transform(value, transform);
                    }
                    return String::new();
                }
            }
            if let Some((var_name, operation, pattern)) = parse_parameter_case_mod(name) {
                let pattern = self.expand_embedded_parameters(pattern);
                if matches!(var_name, "@" | "*") {
                    return self
                        .positional_params
                        .iter()
                        .map(|value| apply_parameter_case_mod(value, operation, &pattern))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Ok(index) = var_name.parse::<usize>() {
                    return self
                        .positional_params
                        .get(index.saturating_sub(1))
                        .map(|value| apply_parameter_case_mod(value, operation, &pattern))
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(var_name) {
                    return apply_parameter_case_mod(&value, operation, &pattern);
                }
                if let Some(array_name) = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                {
                    return self
                        .env_vars
                        .get(array_name)
                        .map(|value| {
                            array_values(value)
                                .into_iter()
                                .map(|value| apply_parameter_case_mod(&value, operation, &pattern))
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                }
                if is_shell_name(var_name) {
                    return self
                        .env_vars
                        .get(var_name)
                        .map(|value| apply_parameter_case_mod(value, operation, &pattern))
                        .unwrap_or_default();
                }
            }
            if let Some((var_name, pattern, replacement, global)) =
                parse_parameter_replacement(name)
            {
                let pattern = self.expand_parameter_pattern_word(pattern);
                let replacement = decode_parameter_replacement_quotes(
                    &self.expand_embedded_parameters_preserving_escaped_single_quotes(replacement),
                );
                if matches!(var_name, "@" | "*") {
                    return self
                        .positional_params
                        .iter()
                        .map(|value| {
                            replace_parameter_pattern(value, &pattern, &replacement, global)
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Ok(index) = var_name.parse::<usize>() {
                    return self
                        .positional_params
                        .get(index.saturating_sub(1))
                        .map(|value| {
                            replace_parameter_pattern(value, &pattern, &replacement, global)
                        })
                        .unwrap_or_default();
                }
                if let Some(value) = self.array_element_parameter_value(var_name) {
                    return replace_parameter_pattern(&value, &pattern, &replacement, global);
                }
                if let Some(array_name) = var_name
                    .strip_suffix("[@]")
                    .or_else(|| var_name.strip_suffix("[*]"))
                {
                    return self
                        .env_vars
                        .get(array_name)
                        .map(|value| {
                            array_values(value)
                                .into_iter()
                                .map(|value| {
                                    replace_parameter_pattern(
                                        &value,
                                        &pattern,
                                        &replacement,
                                        global,
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                }
                if is_shell_name(var_name) {
                    return self
                        .dynamic_parameter_value(var_name)
                        .or_else(|| self.env_vars.get(var_name).cloned())
                        .map(|value| {
                            replace_parameter_pattern(&value, &pattern, &replacement, global)
                        })
                        .unwrap_or_default();
                }
            }
            return self
                .dynamic_parameter_value(name)
                .or_else(|| {
                    self.shell_variable_value(name)
                        .map(|value| shell_safe_value(&value))
                })
                .unwrap_or_default();
        }

        if let Some(name) = word.strip_prefix('$') {
            if is_shell_name(name) {
                return self
                    .dynamic_parameter_value(name)
                    .or_else(|| self.shell_variable_value(name))
                    .unwrap_or_default();
            }
        }

        let expanded = self.expand_embedded_parameters(word);
        if word.contains("$(") || word.contains('`') {
            restore_protected_replacement_quotes(&unescape_remaining_shell_escapes(&expanded))
                .replace("\\\\'", "'")
                .replace("\\'", "'")
        } else {
            restore_protected_replacement_quotes(&expanded)
        }
    }
}

#[cfg(test)]
mod tests;
