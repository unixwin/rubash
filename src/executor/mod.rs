//! Executor Module - Bash Command Executor
//!
//! Executes parsed AST commands.

use crate::parser::{Ast, CommandNode};
use std::process::{Command, Stdio};
use std::fs::File;
use std::env;

/// Exit code storage
#[derive(Debug, Clone)]
pub struct ExitCode(pub i32);

impl Default for ExitCode {
    fn default() -> Self {
        Self(0)
    }
}

/// Execution error
#[derive(Debug)]
pub enum ExecuteError {
    CommandNotFound(String),
    IoError(std::io::Error),
    ExitCode(i32),
}

impl std::fmt::Display for ExecuteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecuteError::CommandNotFound(cmd) => write!(f, "command not found: {}", cmd),
            ExecuteError::IoError(e) => write!(f, "IO error: {}", e),
            ExecuteError::ExitCode(code) => write!(f, "exit code: {}", code),
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
pub struct Executor {
    exit_code: i32,
    env_vars: std::collections::HashMap<String, String>,
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
        for (i, cmd) in ast.commands.iter().enumerate() {
            if i > 0 {
                // Pipeline - pass output to next command
            }
            self.execute_command(cmd)?;
        }
        Ok(())
    }

    /// Execute a single command
    pub fn execute_command(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        // Handle builtins first
        if let Some(word) = cmd.words.first() {
            match word.as_str() {
                "exit" => {
                    let code = cmd.words.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    self.exit_code = code;
                    return Err(ExecuteError::ExitCode(code));
                }
                "echo" => {
                    let args: Vec<&str> = cmd.words.iter().skip(1).map(|s| s.as_str()).collect();
                    println!("{}", args.join(" "));
                    return Ok(());
                }
                "pwd" => {
                    if let Ok(cwd) = env::current_dir() {
                        println!("{}", cwd.display());
                    }
                    return Ok(());
                }
                "cd" => {
                    let dir = cmd.words.get(1).map(|s| s.as_str()).unwrap_or("~");
                    let dir = if dir == "~" {
                        env::var("HOME").unwrap_or_else(|_| ".".to_string())
                    } else {
                        dir.to_string()
                    };
                    env::set_current_dir(&dir)?;
                    return Ok(());
                }
                "export" => {
                    if let Some(var) = cmd.words.get(1) {
                        if let Some(pos) = var.find('=') {
                            let name = &var[..pos];
                            let value = &var[pos+1..];
                            self.env_vars.insert(name.to_string(), value.to_string());
                            env::set_var(name, value);
                        }
                    }
                    return Ok(());
                }
                "true" => {
                    self.exit_code = 0;
                    return Ok(());
                }
                "false" => {
                    self.exit_code = 1;
                    return Ok(());
                }
                _ => {}
            }
        }

        // External command execution
        self.execute_external(cmd)
    }

    fn execute_external(&mut self, cmd: &CommandNode) -> Result<(), ExecuteError> {
        if cmd.words.is_empty() {
            return Ok(());
        }

        let mut process = Command::new(&cmd.words[0]);
        process.args(&cmd.words[1..]);

        // If there's an assignment, execute with modified environment
        for (var_name, var_value) in &cmd.assignments {
            process.env(var_name, var_value);
        }

        // Handle redirections
        if let Some(ref redirect) = cmd.redirect_out {
            let file = File::create(&redirect.target)?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_in {
            let file = File::open(&redirect.target)?;
            process.stdin(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.append {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&redirect.target)?;
            process.stdout(Stdio::from(file));
        }

        if let Some(ref redirect) = cmd.redirect_err {
            let file = File::create(&redirect.target)?;
            process.stderr(Stdio::from(file));
        }

        let status = process.status()?;
        self.exit_code = status.code().unwrap_or(0);

        if !status.success() {
            return Err(ExecuteError::ExitCode(self.exit_code));
        }

        Ok(())
    }

    /// Get the last exit code
    pub fn last_exit_code(&self) -> i32 {
        self.exit_code
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
}