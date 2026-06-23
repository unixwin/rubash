//! `command` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/command.def (`command_builtin`)

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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
pub fn execute(args: &[String]) -> io::Result<CommandAction> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(args.iter().map(String::as_str), &mut stdout, &mut stderr)
}

fn execute_with_io<'a, I, W, E>(
    args: I,
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
                    writeln!(stderr, "rubash: command: -{}: invalid option", other)?;
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
                writeln!(stderr, "rubash: command: {}: not found", name)?;
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
            | "wait"
    )
}

fn find_in_path(name: &str, use_standard_path: bool) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    let path_value = if use_standard_path {
        default_standard_path().to_string()
    } else {
        env::var("PATH").unwrap_or_default()
    };

    for dir in env::split_paths(&path_value) {
        let path = dir.join(name);
        if path.is_file() {
            return Some(path);
        }

        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let path = dir.join(format!("{name}.{ext}"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }

    None
}

fn default_standard_path() -> &'static str {
    if cfg!(windows) {
        r"C:\Windows\System32;C:\Windows"
    } else {
        "/usr/local/bin:/usr/bin:/bin"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> (CommandAction, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let action = execute_with_io(args.iter().copied(), &mut stdout, &mut stderr).unwrap();

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
