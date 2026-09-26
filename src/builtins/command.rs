//! `command` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/command.def (`command_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Complete(i32),
    Execute {
        words: Vec<String>,
        use_standard_path: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DescribeMode {
    Reusable,
    Verbose,
}

/// Execute `command` with arguments after the command name.
pub fn execute(args: &[String], prefix: &str) -> io::Result<CommandAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        prefix,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    prefix: &str,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<CommandAction>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let mut use_standard_path = false;
    let mut describe_mode = None;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'p' => use_standard_path = true,
                'v' => describe_mode = Some(DescribeMode::Reusable),
                'V' => describe_mode = Some(DescribeMode::Verbose),
                other => {
                    writeln!(stderr, "{prefix}command: -{}: invalid option", other)?;
                    writeln!(stderr, "command: usage: command [-pVv] command [arg ...]")?;
                    return Ok(CommandAction::Complete(EX_USAGE));
                }
            }
        }

        index += 1;
    }

    let operands = &args[index..];
    if operands.is_empty() {
        return Ok(CommandAction::Complete(EXECUTION_SUCCESS));
    }

    if let Some(mode) = describe_mode {
        let mut any_found = false;
        for name in operands {
            if describe_command(name, mode, use_standard_path, stdout)? {
                any_found = true;
            } else if mode == DescribeMode::Verbose {
                writeln!(stderr, "{prefix}command: {}: not found", name)?;
            }
        }

        return Ok(CommandAction::Complete(if any_found {
            EXECUTION_SUCCESS
        } else {
            EXECUTION_FAILURE
        }));
    }

    Ok(CommandAction::Execute {
        words: operands.iter().map(|word| (*word).to_string()).collect(),
        use_standard_path,
    })
}

fn describe_command<W>(
    name: &str,
    mode: DescribeMode,
    use_standard_path: bool,
    stdout: &mut W,
) -> io::Result<bool>
where
    W: Write,
{
    if is_shell_builtin(name) {
        match mode {
            DescribeMode::Reusable => writeln!(stdout, "{name}")?,
            DescribeMode::Verbose => writeln!(stdout, "{name} is a shell builtin")?,
        }
        return Ok(true);
    }

    let Some(path) = find_in_path(name, use_standard_path) else {
        return Ok(false);
    };

    match mode {
        DescribeMode::Reusable => writeln!(stdout, "{}", path.display())?,
        DescribeMode::Verbose => writeln!(stdout, "{name} is {}", path.display())?,
    }

    Ok(true)
}

fn is_shell_builtin(name: &str) -> bool {
    matches!(
        name,
        "." | ":"
            | "["
            | "alias"
            | "bg"
            | "bind"
            | "break"
            | "builtin"
            | "caller"
            | "cd"
            | "command"
            | "compgen"
            | "complete"
            | "compopt"
            | "continue"
            | "declare"
            | "dirs"
            | "disown"
            | "echo"
            | "enable"
            | "env"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "false"
            | "fc"
            | "fg"
            | "getopts"
            | "hash"
            | "help"
            | "history"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "logout"
            | "mapfile"
            | "popd"
            | "printf"
            | "pushd"
            | "pwd"
            | "read"
            | "readarray"
            | "readonly"
            | "return"
            | "set"
            | "setopt"
            | "shift"
            | "shopt"
            | "source"
            | "suspend"
            | "test"
            | "times"
            | "trap"
            | "type"
            | "typeset"
            | "true"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "unsetopt"
            | "wait"
    ) || (cfg!(windows) && name == "sudo")
}

fn find_in_path(name: &str, use_standard_path: bool) -> Option<PathBuf> {
    // GNU builtins/type.def:371-379 describe_command(): `command -p`
    // (CDESC_STDPATH) walks conf_standard_path(); every other mode calls
    // find_user_command (findcmd.c:247) — the same FS_EXEC_PREFERRED +
    // file_to_lose_on walk the executor uses to run commands. Delegating
    // to crate::executor::path keeps describe and execute on one lookup.
    let mut env_vars: HashMap<String, String> = env::vars().collect();
    if use_standard_path {
        env_vars.insert(
            "PATH".to_string(),
            crate::executor::path::standard_path(&env_vars),
        );
    }
    crate::executor::path::find_user_command(name, &env_vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (CommandAction, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action =
            execute_with_io(args.iter().copied(), "rubash: ", &mut stdout, &mut stderr).unwrap();

        (
            action,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn no_operands_succeeds() {
        assert_eq!(run(&[]).0, CommandAction::Complete(EXECUTION_SUCCESS));
    }

    #[test]
    fn reusable_description_reports_builtin_name() {
        let (action, stdout, stderr) = run(&["-v", "echo"]);

        assert_eq!(action, CommandAction::Complete(EXECUTION_SUCCESS));
        assert_eq!(stdout, "echo\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn reusable_description_reports_extended_builtin_names() {
        let (action, stdout, stderr) = run(&["-v", "read", "mapfile", "declare", "alias"]);

        assert_eq!(action, CommandAction::Complete(EXECUTION_SUCCESS));
        assert_eq!(stdout, "read\nmapfile\ndeclare\nalias\n");
        assert!(stderr.is_empty());
    }

    #[test]
    fn execute_action_preserves_operands() {
        assert_eq!(
            run(&["--", "echo", "hello"]).0,
            CommandAction::Execute {
                words: vec!["echo".to_string(), "hello".to_string()],
                use_standard_path: false,
            }
        );
    }
}
