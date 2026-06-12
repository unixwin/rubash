//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

mod path;

use crate::builtins::alias::Alias;
use crate::parser::{Ast, CaseCommand, CommandNode, ForCommand};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::process::{Command, Stdio};

use self::path::{find_shell, find_user_command, shell_path_to_windows, should_run_with_shell};

/// Execution error
#[derive(Debug)]
pub enum ExecuteError {
    CommandNotFound(String),
    IoError(std::io::Error),
    ExitCode(i32),
    UnknownBuiltin(String),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::CommandNotFound(cmd) => write!(f, "rubash: {}: command not found", cmd),
            ExecuteError::IoError(e) => write!(f, "rubash: {}", e),
            ExecuteError::ExitCode(code) => write!(f, "exit code: {}", code),
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

/// Command executor
#[derive(Debug)]
pub struct Executor {
    exit_code: i32,
    env_vars: HashMap<String, String>,
    aliases: HashMap<String, Alias>,
    positional_params: Vec<String>,
    expanding_aliases: Vec<String>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            exit_code: 0,
            env_vars: std::env::vars().collect(),
            aliases: HashMap::new(),
            positional_params: Vec::new(),
            expanding_aliases: Vec::new(),
        }
    }

    /// Execute an AST
    pub fn execute_ast(&mut self, ast: &Ast) -> Result<(), ExecuteError> {
        for cmd in &ast.commands {
            self.execute_command(cmd)?;
        }
        Ok(())
    }

    /// Execute a single command
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if let Some(for_command) = &cmd.for_command {
            return self.execute_for_command(for_command);
        }

        if let Some(case_command) = &cmd.case_command {
            return self.execute_case_command(case_command);
        }

        if cmd.words.is_empty() {
            for (name, value) in &cmd.assignments {
                let expanded_value = self.expand_word(value);
                self.env_vars.insert(name.clone(), expanded_value.clone());
                env::set_var(name, expanded_value);
            }
            self.exit_code = 0;
            return Ok(());
        }

        if self.execute_parser_level_alias(cmd)? {
            return Ok(());
        }

        let mut variable_expanded = cmd.clone();
        variable_expanded.words = cmd
            .words
            .iter()
            .map(|word| self.expand_word(word))
            .collect();

        let expanded;
        let cmd = {
            let words = self.expand_aliases(&variable_expanded.words);
            expanded = CommandNode {
                words,
                ..variable_expanded.clone()
            };
            &expanded
        };

        if self.execute_assignment_words(cmd) {
            return Ok(());
        }

        if let Some(word) = cmd.words.first() {
            match word.as_str() {
                "exit" => match crate::builtins::exit::execute(&cmd.words[1..], self.exit_code)? {
                    crate::builtins::exit::ExitAction::Exit(code) => {
                        self.exit_code = code;
                        Err(ExecuteError::ExitCode(code))
                    }
                    crate::builtins::exit::ExitAction::Continue(status) => {
                        self.exit_code = status;
                        Ok(())
                    }
                },
                "echo" => {
                    crate::builtins::echo::execute(&cmd.words[1..])?;
                    self.exit_code = 0;
                    Ok(())
                }
                "eval" => match crate::builtins::eval::execute(&cmd.words[1..])? {
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
                "pwd" => {
                    self.exit_code = crate::builtins::pwd::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "source" | "." => self.execute_source(&cmd.words[1..]),
                "printf" => {
                    self.exit_code =
                        crate::builtins::printf::execute(&cmd.words[1..], &mut self.env_vars)?;
                    Ok(())
                }
                "command" => match crate::builtins::command::execute(&cmd.words[1..])? {
                    crate::builtins::command::CommandAction::Complete(status) => {
                        self.exit_code = status;
                        Ok(())
                    }
                    crate::builtins::command::CommandAction::Execute {
                        words,
                        use_standard_path: _,
                    } => {
                        let mut command = cmd.clone();
                        command.words = words;
                        self.execute_command(&command)
                    }
                },
                "cd" => {
                    self.exit_code =
                        crate::builtins::cd::execute(&cmd.words[1..], &mut self.env_vars)?;
                    Ok(())
                }
                "alias" => {
                    self.exit_code =
                        crate::builtins::alias::alias(&cmd.words[1..], &mut self.aliases)?;
                    Ok(())
                }
                "declare" => {
                    self.exit_code =
                        crate::builtins::declare::execute(&cmd.words[1..], &self.env_vars)?;
                    Ok(())
                }
                "unalias" => {
                    self.exit_code =
                        crate::builtins::alias::unalias(&cmd.words[1..], &mut self.aliases)?;
                    Ok(())
                }
                "export" => {
                    self.exit_code =
                        crate::builtins::setattr::export(&cmd.words[1..], &mut self.env_vars)?;
                    Ok(())
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
                "env" => {
                    self.do_env();
                    Ok(())
                }
                "set" => {
                    self.exit_code = crate::builtins::set::set(&cmd.words[1..], &self.env_vars)?;
                    Ok(())
                }
                "shopt" => {
                    self.exit_code = crate::builtins::shopt::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "hash" => {
                    self.exit_code = crate::builtins::hash::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "unset" => {
                    self.exit_code =
                        crate::builtins::set::unset(&cmd.words[1..], &mut self.env_vars)?;
                    Ok(())
                }
                "times" => {
                    self.exit_code = crate::builtins::times::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "type" => {
                    self.exit_code = crate::builtins::r#type::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "test" => {
                    self.exit_code =
                        crate::builtins::test::execute(&cmd.words[1..], false, &self.env_vars)?;
                    Ok(())
                }
                "[" => {
                    self.exit_code =
                        crate::builtins::test::execute(&cmd.words[1..], true, &self.env_vars)?;
                    Ok(())
                }
                _ => self.execute_external(cmd),
            }
        } else {
            Ok(())
        }
    }

    fn execute_for_command(&mut self, for_command: &ForCommand) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c): Bash `execute_for_command` applies the
        // full expansion pipeline, loop-control state, traps, and redirections.
        // This only covers `for name in words; do compound_list; done`.
        for word in &for_command.words {
            let value = self.expand_word(word);
            self.env_vars
                .insert(for_command.variable.clone(), value.clone());
            env::set_var(&for_command.variable, value);

            let body = Ast {
                commands: for_command.body.clone(),
            };
            self.execute_ast(&body)?;
        }

        self.exit_code = 0;
        Ok(())
    }

    fn execute_case_command(&mut self, case_command: &CaseCommand) -> Result<(), ExecuteError> {
        // TODO(parse.y/execute_cmd.c/pathexp.c): Bash case execution uses the
        // full pattern matcher, fall-through operators, expansion flags, and
        // compound-list control flow. This handles exact patterns and `*`.
        let word = self.expand_word(&case_command.word);
        for clause in &case_command.clauses {
            if clause
                .patterns
                .iter()
                .any(|pattern| case_pattern_matches(pattern, &word))
            {
                let body = Ast {
                    commands: clause.body.clone(),
                };
                self.execute_ast(&body)?;
                return Ok(());
            }
        }

        self.exit_code = 0;
        Ok(())
    }

    fn execute_assignment_words(&mut self, cmd: &CommandNode) -> bool {
        // TODO(variables.c/arrayfunc.c/subst.c): Bash recognizes assignment
        // words after alias expansion and routes compound array assignments
        // through `assign_array_var_from_string`. This only handles commands
        // made entirely of `name=value` words.
        if cmd.words.is_empty() || !cmd.assignments.is_empty() {
            return false;
        }

        let mut assignments = Vec::new();
        for word in &cmd.words {
            let Some((name, value)) = split_assignment_word(word) else {
                return false;
            };
            assignments.push((name.to_string(), self.expand_assignment_value(value)));
        }

        for (name, value) in assignments {
            self.env_vars.insert(name.clone(), value.clone());
            env::set_var(name, value);
        }
        self.exit_code = 0;
        true
    }

    fn expand_assignment_value(&self, value: &str) -> String {
        if let Some(array_value) = normalize_single_element_array_assignment(value) {
            return array_value;
        }

        self.expand_word(value)
    }

    fn do_env(&mut self) {
        for (key, value) in &self.env_vars {
            println!("{}={}", key, value);
        }
        self.exit_code = 0;
    }

    fn expand_word(&self, word: &str) -> String {
        if word == "$?" {
            return self.exit_code.to_string();
        }

        if let Some(name) = word
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
        {
            return self
                .env_vars
                .get(name)
                .map(|value| shell_safe_value(value))
                .unwrap_or_default();
        }

        if let Some(name) = word.strip_prefix('$') {
            if is_shell_name(name) {
                return self.env_vars.get(name).cloned().unwrap_or_default();
            }
        }

        self.expand_embedded_parameters(word)
    }

    fn expand_embedded_parameters(&self, word: &str) -> String {
        // TODO(subst.c/subst.h): This is a narrow parameter-expansion subset.
        // GNU Bash handles quoting state, operators like ${name:-word},
        // positional/special parameters, arrays, command substitution, and IFS
        // word splitting here. Keep extending this toward subst.c semantics.
        let mut output = String::new();
        let mut chars = word.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch != '$' {
                output.push(ch);
                continue;
            }

            match chars.peek().copied() {
                Some('?') => {
                    chars.next();
                    output.push_str(&self.exit_code.to_string());
                }
                Some('{') => {
                    chars.next();
                    let mut name = String::new();
                    for name_ch in chars.by_ref() {
                        if name_ch == '}' {
                            break;
                        }
                        name.push(name_ch);
                    }
                    if let Some(value) = self.env_vars.get(&name) {
                        output.push_str(&shell_safe_value(value));
                    }
                }
                Some(first) if first.is_ascii_digit() => {
                    chars.next();
                    let index = first.to_digit(10).unwrap_or(0) as usize;
                    if index > 0 {
                        output.push_str(
                            self.positional_params
                                .get(index - 1)
                                .map(String::as_str)
                                .unwrap_or(""),
                        );
                    }
                }
                Some(first) if is_shell_name_start(first) => {
                    let mut name = String::new();
                    while let Some(name_ch) = chars.peek().copied() {
                        if !is_shell_name_char(name_ch) {
                            break;
                        }
                        chars.next();
                        name.push(name_ch);
                    }
                    if let Some(value) = self.env_vars.get(&name) {
                        output.push_str(&shell_safe_value(value));
                    }
                }
                Some(other) => {
                    chars.next();
                    output.push('$');
                    output.push(other);
                }
                None => output.push('$'),
            }
        }

        output
    }

    fn expand_aliases(&self, words: &[String]) -> Vec<String> {
        let mut expanded = Vec::new();
        let mut expand_next = true;

        for word in words {
            if expand_next {
                let mut seen = Vec::new();
                let (mut alias_words, alias_expand_next) = self.expand_alias_word(word, &mut seen);
                expanded.append(&mut alias_words);
                expand_next = alias_expand_next;
            } else {
                expanded.push(word.clone());
                expand_next = false;
            }
        }

        expanded
    }

    fn execute_parser_level_alias(&mut self, cmd: &CommandNode) -> Result<bool, ExecuteError> {
        // TODO(parse.y/alias.c): GNU Bash pushes alias text back into the
        // parser input stream (`alias_expand_token` + `push_string`). This
        // reparses complex alias values at command position so aliases that
        // introduce `;`, newlines, or redirections behave closer to Bash until
        // Rubash has a real parser input stack.
        let Some(word) = cmd.words.first() else {
            return Ok(false);
        };

        if self.expanding_aliases.iter().any(|alias| alias == word) {
            return Ok(false);
        }

        let Some(alias) = self.aliases.get(word).cloned() else {
            return Ok(false);
        };

        if !needs_parser_level_alias_expansion(&alias.value) {
            return Ok(false);
        }

        let mut source = alias.value.clone();
        if !source.ends_with(' ') && !source.ends_with('\t') && !cmd.words[1..].is_empty() {
            source.push(' ');
        }
        source.push_str(&cmd.words[1..].join(" "));

        self.expanding_aliases.push(word.clone());
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.expanding_aliases.pop();
        result.map(|_| true)
    }

    fn expand_alias_word(&self, word: &str, seen: &mut Vec<String>) -> (Vec<String>, bool) {
        // TODO(alias.c/alias.h/parse.y): Bash marks AL_BEINGEXPANDED in
        // parse.y::alias_expand_token and re-reads parser input. This executor-level
        // approximation preserves AL_EXPANDNEXT and recursion suppression, but it
        // cannot make redirections or compound commands introduced by aliases parse
        // exactly like GNU Bash yet.
        if seen.iter().any(|seen_word| seen_word == word) {
            return (vec![word.to_string()], false);
        }

        let Some(alias) = self.aliases.get(word) else {
            return (vec![word.to_string()], false);
        };

        seen.push(word.to_string());
        let mut parts: Vec<String> = alias.value.split_whitespace().map(str::to_string).collect();

        if let Some(first) = parts.first().cloned() {
            let (mut first_expanded, _) = self.expand_alias_word(&first, seen);
            parts.remove(0);
            first_expanded.extend(parts);
            (first_expanded, alias.expand_next)
        } else {
            (Vec::new(), alias.expand_next)
        }
    }

    fn execute_source(&mut self, args: &[String]) -> Result<(), ExecuteError> {
        // TODO(builtins/source.def): GNU Bash `source_builtin` searches PATH,
        // handles `-p`, temporarily replaces positional parameters, and uses
        // unwind/trap machinery around `source_file`. This is the minimal
        // current-shell execution path needed by upstream alias subtests.
        let Some(filename) = args.first() else {
            eprintln!("rubash: source: filename argument required");
            self.exit_code = 2;
            return Ok(());
        };

        let source_path = shell_path_to_windows(filename, &self.env_vars);
        let source = match fs::read_to_string(&source_path) {
            Ok(source) => source,
            Err(error) => {
                eprintln!("rubash: source: {filename}: {error}");
                self.exit_code = 1;
                return Ok(());
            }
        };

        let old_positional_params = std::mem::replace(
            &mut self.positional_params,
            args.iter().skip(1).cloned().collect(),
        );
        let tokens = crate::lexer::tokenize(&source);
        let ast = crate::parser::parse(&tokens);
        let result = self.execute_ast(&ast);
        self.positional_params = old_positional_params;
        result
    }

    fn execute_external(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if cmd.words.is_empty() {
            return Ok(());
        }

        let Some(program) = find_user_command(&cmd.words[0], &self.env_vars) else {
            eprintln!("rubash: {}: command not found", cmd.words[0]);
            self.exit_code = 127;
            return Ok(());
        };

        let mut process = if should_run_with_shell(&program) {
            if let Some(shell) = find_shell(&self.env_vars) {
                let mut command = Command::new(shell);
                command.arg(&program);
                command.args(&cmd.words[1..]);
                command
            } else {
                Command::new(&program)
            }
        } else {
            let mut command = Command::new(&program);
            command.args(&cmd.words[1..]);
            command
        };

        for (var_name, var_value) in &cmd.assignments {
            process.env(var_name, var_value);
        }

        if cmd.heredoc.is_some() {
            // TODO(redir.c/parse.y): This implements the simple stdin pipe for
            // here-documents. GNU Bash stores REDIRECT nodes, tracks quoted
            // delimiters, strips tabs for <<-, and conditionally expands the
            // body before do_redirections applies it.
            process.stdin(Stdio::piped());
        } else if let Some(ref redirect) = cmd.redirect_in {
            let target = self.expand_word(&redirect.target);
            let file = File::open(shell_path_to_windows(&target, &self.env_vars))?;
            process.stdin(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_out {
            let target = self.expand_word(&redirect.target);
            let file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.append {
            let target = self.expand_word(&redirect.target);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_err {
            let target = self.expand_word(&redirect.target);
            let file = File::create(shell_path_to_windows(&target, &self.env_vars))?;
            process.stderr(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_err_append {
            let target = self.expand_word(&redirect.target);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(shell_path_to_windows(&target, &self.env_vars))?;
            process.stderr(Stdio::from(file));
        }

        match process.spawn() {
            Ok(mut child) => {
                if let Some(ref body) = cmd.heredoc {
                    if let Some(mut stdin) = child.stdin.take() {
                        stdin.write_all(body.as_bytes())?;
                    }
                }

                match child.wait() {
                    Ok(status) => {
                        self.exit_code = status.code().unwrap_or(1);
                    }
                    Err(error) => {
                        eprintln!("rubash: {}: {}", cmd.words[0], error);
                        self.exit_code = 126;
                    }
                }
            }
            Err(error) => {
                eprintln!("rubash: {}: {}", cmd.words[0], error);
                self.exit_code = 126;
            }
        }

        Ok(())
    }

    pub fn last_exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn set_env(&mut self, name: &str, value: &str) {
        self.env_vars.insert(name.to_string(), value.to_string());
        env::set_var(name, value);
    }

    pub fn get_env(&self, name: &str) -> Option<&str> {
        self.env_vars.get(name).map(|s| s.as_str())
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

fn is_shell_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    is_shell_name_start(first) && chars.all(is_shell_name_char)
}

fn is_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_shell_name_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn split_assignment_word(word: &str) -> Option<(&str, &str)> {
    let (name, value) = word.split_once('=')?;
    if is_shell_name(name) {
        Some((name, value))
    } else {
        None
    }
}

fn normalize_single_element_array_assignment(value: &str) -> Option<String> {
    let inner = value.strip_prefix('(')?.strip_suffix(')')?;
    Some(format!("({})", strip_matching_quotes(inner.trim())))
}

fn strip_matching_quotes(value: &str) -> &str {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn case_pattern_matches(pattern: &str, word: &str) -> bool {
    pattern == "*" || pattern == word
}

fn needs_parser_level_alias_expansion(value: &str) -> bool {
    value
        .chars()
        .any(|ch| matches!(ch, ';' | '\n' | '<' | '>' | '|' | '&'))
}

fn shell_safe_value(value: &str) -> String {
    // TODO(subst.c/findcmd.c): On Windows, Git Bash passes many environment
    // paths to native executables as `C:\...`. If those values are substituted
    // back into shell input for alias reparsing, backslashes are treated as
    // shell escapes. Keep absolute drive paths in `/c/...` form until Rubash
    // has a dedicated shell path type.
    if cfg!(windows) {
        let bytes = value.as_bytes();
        if bytes.len() >= 3
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
            && bytes[0].is_ascii_alphabetic()
        {
            let drive = (bytes[0] as char).to_ascii_lowercase();
            let rest = value[3..].replace('\\', "/");
            return format!("/{drive}/{rest}");
        }
    }

    value.to_string()
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    #[test]
    fn test_execute_echo() {
        let tokens = tokenize("echo hello");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        assert!(executor.execute_ast(&ast).is_ok());
    }

    #[test]
    fn test_exit_code() {
        let tokens = tokenize("exit 5");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        let result = executor.execute_ast(&ast);
        assert!(result.is_err());
        assert_eq!(executor.last_exit_code(), 5);
    }

    #[test]
    fn test_true_command() {
        let tokens = tokenize("true");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn test_colon_command() {
        let tokens = tokenize(":");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn test_false_command() {
        let tokens = tokenize("false");
        let ast = parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.last_exit_code(), 1);
    }

    #[test]
    fn test_env_var() {
        let mut executor = Executor::new();
        executor.set_env("TEST_VAR", "hello");
        assert_eq!(executor.get_env("TEST_VAR"), Some("hello"));
    }
}
