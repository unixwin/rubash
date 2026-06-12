//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

mod path;

use crate::parser::{Ast, CommandNode};
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::process::{Command, Stdio};

use self::path::{find_shell, find_user_command, should_run_with_shell};

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
            ExecuteError::UnknownBuiltin(name) => write!(f, "rubash: {}: builtin command not found", name),
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
    aliases: HashMap<String, String>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            exit_code: 0,
            env_vars: std::env::vars().collect(),
            aliases: HashMap::new(),
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
        if cmd.words.is_empty() {
            for (name, value) in &cmd.assignments {
                self.env_vars.insert(name.clone(), value.clone());
                env::set_var(name, value);
            }
            self.exit_code = 0;
            return Ok(());
        }

        let mut variable_expanded = cmd.clone();
        variable_expanded.words = cmd
            .words
            .iter()
            .map(|word| self.expand_word(word))
            .collect();

        let expanded;
        let cmd = if let Some(word) = variable_expanded.words.first() {
            if let Some(value) = self.aliases.get(word) {
                let mut words: Vec<String> = value
                    .split_whitespace()
                    .map(str::to_string)
                    .collect();
                words.extend(variable_expanded.words.iter().skip(1).cloned());
                expanded = CommandNode {
                    words,
                    ..variable_expanded.clone()
                };
                &expanded
            } else {
                &variable_expanded
            }
        } else {
            &variable_expanded
        };

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
                    self.exit_code = crate::builtins::cd::execute(&cmd.words[1..], &mut self.env_vars)?;
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
                ":" => { self.exit_code = crate::builtins::colon::colon(); Ok(()) }
                "true" => { self.exit_code = crate::builtins::colon::true_builtin(); Ok(()) }
                "false" => { self.exit_code = crate::builtins::colon::false_builtin(); Ok(()) }
                "env" => { self.do_env(); Ok(()) }
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
                    self.exit_code = crate::builtins::set::unset(&cmd.words[1..], &mut self.env_vars)?;
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

        if let Some(name) = word.strip_prefix("${").and_then(|rest| rest.strip_suffix('}')) {
            return self.env_vars.get(name).cloned().unwrap_or_default();
        }

        if let Some(name) = word.strip_prefix('$') {
            return self.env_vars.get(name).cloned().unwrap_or_default();
        }

        word.to_string()
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

        if let Some(ref redirect) = cmd.redirect_in {
            let file = File::open(&redirect.target)?;
            process.stdin(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_out {
            let file = File::create(&redirect.target)?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.append {
            let file = OpenOptions::new().create(true).append(true).open(&redirect.target)?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_err {
            let file = File::create(&redirect.target)?;
            process.stderr(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_err_append {
            let file = OpenOptions::new().create(true).append(true).open(&redirect.target)?;
            process.stderr(Stdio::from(file));
        }

        match process.status() {
            Ok(status) => {
                self.exit_code = status.code().unwrap_or(1);
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
