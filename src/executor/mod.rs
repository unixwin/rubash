//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

use crate::parser::{Ast, CommandNode};
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::process::{Command, Stdio};

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
}

impl Executor {
    pub fn new() -> Self {
        Self {
            exit_code: 0,
            env_vars: std::env::vars().collect(),
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
                "pwd" => {
                    self.exit_code = crate::builtins::pwd::execute(&cmd.words[1..])?;
                    Ok(())
                }
                "cd" => {
                    self.exit_code = crate::builtins::cd::execute(&cmd.words[1..], &mut self.env_vars)?;
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
                "unset" => {
                    self.exit_code = crate::builtins::set::unset(&cmd.words[1..], &mut self.env_vars)?;
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

    fn execute_external(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if cmd.words.is_empty() {
            return Ok(());
        }

        let mut process = Command::new(&cmd.words[0]);
        process.args(&cmd.words[1..]);

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

        let status = process.status()?;
        self.exit_code = status.code().unwrap_or(0);

        if !status.success() {
            return Err(ExecuteError::ExitCode(self.exit_code));
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
