//! `pwd` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/cd.def (`pwd_builtin`)

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Logical,
    Physical,
}

/// Execute `pwd` with arguments after the command name.
pub fn execute(args: &[String]) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args.iter().map(String::as_str), &mut stdout, &mut stderr)
}

fn execute_with_io<'a, I, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let mut mode = Mode::Logical;

    for arg in args {
        if arg == "--" {
            break;
        }

        if !arg.starts_with('-') || arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'L' => mode = Mode::Logical,
                'P' => mode = Mode::Physical,
                other => {
                    writeln!(stderr, "rubash: pwd: -{}: invalid option", other)?;
                    writeln!(stderr, "pwd: usage: pwd [-LP]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
    }

    let Some(directory) = current_directory(mode)? else {
        return Ok(EXECUTION_FAILURE);
    };

    writeln!(stdout, "{}", directory.display())?;
    Ok(EXECUTION_SUCCESS)
}

fn current_directory(mode: Mode) -> io::Result<Option<PathBuf>> {
    let physical = env::current_dir()?;

    if mode == Mode::Logical {
        if let Some(logical) = logical_pwd_if_current(&physical) {
            return Ok(Some(logical));
        }
    }

    Ok(Some(physical))
}

fn logical_pwd_if_current(physical: &Path) -> Option<PathBuf> {
    let logical = env::var_os("PWD").map(PathBuf::from)?;

    if !logical.is_absolute() {
        return None;
    }

    let logical_physical = logical.canonicalize().ok()?;
    let current_physical = physical.canonicalize().ok()?;

    if logical_physical == current_physical {
        Some(logical)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_io(args.iter().copied(), &mut stdout, &mut stderr).unwrap();

        (
            status,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn accepts_logical_and_physical_options() {
        assert_eq!(run(&["-L"]).0, EXECUTION_SUCCESS);
        assert_eq!(run(&["-P"]).0, EXECUTION_SUCCESS);
        assert_eq!(run(&["-LP"]).0, EXECUTION_SUCCESS);
    }

    #[test]
    fn rejects_invalid_options() {
        let (status, stdout, stderr) = run(&["-x"]);

        assert_eq!(status, EX_USAGE);
        assert!(stdout.is_empty());
        assert!(stderr.contains("invalid option"));
    }
}
