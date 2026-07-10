//! `type` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/type.def (`type_builtin`, `describe_command`)

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Short,
    Type,
    PathOnly,
}

#[derive(Debug, Clone, Copy)]
struct TypeOptions {
    all: bool,
    force_path: bool,
    output: OutputMode,
}

impl Default for TypeOptions {
    fn default() -> Self {
        Self {
            all: false,
            force_path: false,
            output: OutputMode::Short,
        }
    }
}

/// Execute `type` with arguments after the command name.
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
    let args: Vec<&str> = args.into_iter().collect();
    let mut options = TypeOptions::default();
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            index += 1;
            break;
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        let normalized = match *arg {
            "-type" | "--type" => "-t",
            "-path" | "--path" => "-p",
            "-all" | "--all" => "-a",
            other => other,
        };

        for option in normalized[1..].chars() {
            match option {
                'a' => options.all = true,
                'f' => {}
                'p' => {
                    options.output = OutputMode::PathOnly;
                    options.force_path = false;
                }
                't' => {
                    options.output = OutputMode::Type;
                    options.force_path = false;
                }
                'P' => {
                    options.output = OutputMode::PathOnly;
                    options.force_path = true;
                }
                other => {
                    writeln!(stderr, "rubash: type: -{}: invalid option", other)?;
                    writeln!(stderr, "type: usage: type [-afptP] name [name ...]")?;
                    return Ok(EX_USAGE);
                }
            }
        }

        index += 1;
    }

    let mut any_failed = false;
    for name in &args[index..] {
        if !describe_command(name, options, stdout)? {
            any_failed = true;
            if options.output == OutputMode::Short {
                writeln!(stderr, "rubash: type: {}: not found", name)?;
            }
        }
    }

    Ok(if any_failed {
        EXECUTION_FAILURE
    } else {
        EXECUTION_SUCCESS
    })
}

fn describe_command<W>(name: &str, options: TypeOptions, stdout: &mut W) -> io::Result<bool>
where
    W: Write,
{
    let mut found = false;

    if !options.force_path && is_shell_builtin(name) {
        found = true;
        match options.output {
            OutputMode::Short => writeln!(stdout, "{name} is a shell builtin")?,
            OutputMode::Type => writeln!(stdout, "builtin")?,
            OutputMode::PathOnly => {}
        }

        if !options.all {
            return Ok(true);
        }
    }

    let paths = find_all_in_path(name);
    if !paths.is_empty() {
        found = true;
    }

    for path in paths {
        match options.output {
            OutputMode::Short => writeln!(stdout, "{name} is {}", path.display())?,
            OutputMode::Type => writeln!(stdout, "file")?,
            OutputMode::PathOnly => writeln!(stdout, "{}", path.display())?,
        }

        if !options.all {
            break;
        }
    }

    Ok(found)
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
            | "true"
            | "type"
            | "typeset"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "wait"
    )
}

fn find_all_in_path(name: &str) -> Vec<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate
            .is_file()
            .then(|| candidate.to_path_buf())
            .into_iter()
            .collect();
    }

    let mut matches = Vec::new();
    let path_value = env::var("PATH").unwrap_or_default();
    for dir in env::split_paths(&path_value) {
        let path = dir.join(name);
        if path.is_file() {
            matches.push(path);
        }

        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let path = dir.join(format!("{name}.{ext}"));
                if path.is_file() {
                    matches.push(path);
                }
            }
        }
    }

    matches
}

#[cfg(test)]
#[path = "type_tests.rs"]
mod tests;
