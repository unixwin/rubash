//! `cd` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/cd.def (`cd_builtin`)

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Logical,
    Physical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrintPath {
    Never,
    CdPath,
    Always,
}

#[derive(Debug, Clone)]
struct Target {
    path: PathBuf,
    display: Option<PathBuf>,
    print: PrintPath,
}

/// Execute `cd` with arguments after the command name.
pub fn execute(args: &[String], env_vars: &mut HashMap<String, String>) -> io::Result<i32> {
    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    execute_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

fn execute_with_io<'a, I, W, E>(
    args: I,
    env_vars: &mut HashMap<String, String>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let args: Vec<&str> = args.into_iter().collect();
    let (mode, first_operand) = match parse_options(&args, stderr)? {
        Ok(parsed) => parsed,
        Err(status) => return Ok(status),
    };

    if args.len().saturating_sub(first_operand) > 1 {
        writeln!(stderr, "rubash: cd: too many arguments")?;
        return Ok(EX_USAGE);
    }

    let Some(target) = resolve_target(args.get(first_operand).copied(), env_vars, stderr)? else {
        return Ok(EXECUTION_FAILURE);
    };

    let target = match resolve_cdpath(&target, env_vars) {
        Some(found) => found,
        None => target,
    };

    let old_pwd = current_logical_pwd(env_vars);
    if let Err(error) = env::set_current_dir(&target.path) {
        writeln!(stderr, "rubash: cd: {}: {}", target.path.display(), error)?;
        return Ok(EXECUTION_FAILURE);
    }

    let new_pwd = match mode {
        Mode::Logical => {
            logical_destination(&old_pwd, target.display.as_deref().unwrap_or(&target.path))
        }
        Mode::Physical => env::current_dir().unwrap_or_else(|_| target.path.clone()),
    };
    let new_pwd_display = shell_display_path(&new_pwd);

    set_shell_env(env_vars, "OLDPWD", old_pwd.to_string_lossy().into_owned());
    set_shell_env(env_vars, "PWD", new_pwd_display.clone());

    match target.print {
        PrintPath::Always => writeln!(stdout, "{}", new_pwd_display)?,
        PrintPath::CdPath => writeln!(stdout, "{}", new_pwd_display)?,
        _ => {}
    }

    Ok(EXECUTION_SUCCESS)
}

fn parse_options<W>(args: &[&str], stderr: &mut W) -> io::Result<Result<(Mode, usize), i32>>
where
    W: Write,
{
    let mut mode = Mode::Logical;
    let mut index = 0;

    while let Some(arg) = args.get(index) {
        if *arg == "--" {
            return Ok(Ok((mode, index + 1)));
        }

        if !arg.starts_with('-') || *arg == "-" {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'L' => mode = Mode::Logical,
                'P' => mode = Mode::Physical,
                'e' => {}
                other => {
                    writeln!(stderr, "rubash: cd: -{}: invalid option", other)?;
                    writeln!(stderr, "cd: usage: cd [-L|[-P [-e]]] [dir]")?;
                    return Ok(Err(EX_USAGE));
                }
            }
        }

        index += 1;
    }

    Ok(Ok((mode, index)))
}

fn resolve_target<W>(
    operand: Option<&str>,
    env_vars: &HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<Option<Target>>
where
    W: Write,
{
    match operand {
        None => match shell_var(env_vars, "HOME") {
            Some(home) => Ok(Some(Target {
                path: PathBuf::from(home),
                display: None,
                print: PrintPath::Never,
            })),
            None => {
                writeln!(stderr, "rubash: cd: HOME not set")?;
                Ok(None)
            }
        },
        Some("") => {
            writeln!(stderr, "rubash: cd: null directory")?;
            Ok(None)
        }
        Some("-") => match shell_var(env_vars, "OLDPWD") {
            Some(old_pwd) => Ok(Some(Target {
                path: PathBuf::from(&old_pwd),
                display: Some(PathBuf::from(old_pwd)),
                print: PrintPath::Always,
            })),
            None => {
                writeln!(stderr, "rubash: cd: OLDPWD not set")?;
                Ok(None)
            }
        },
        Some(dir) => Ok(Some(Target {
            path: filesystem_path_for_display(dir, env_vars),
            display: Some(PathBuf::from(dir)),
            print: PrintPath::Never,
        })),
    }
}

fn resolve_cdpath(target: &Target, env_vars: &HashMap<String, String>) -> Option<Target> {
    if target
        .display
        .as_ref()
        .and_then(|path| path.to_str())
        .is_some_and(|path| path.starts_with('/'))
    {
        return None;
    }

    if target.path.is_absolute() || starts_with_dot_component(&target.path) {
        return None;
    }

    let cdpath = shell_var(env_vars, "CDPATH")?;
    for unit in cdpath.split(':') {
        let base = if unit.is_empty() { "." } else { unit };
        let candidate = filesystem_path_for_display(base, env_vars).join(&target.path);
        let display = Path::new(base).join(&target.path);

        if candidate.is_dir() {
            return Some(Target {
                print: if unit.is_empty() {
                    PrintPath::Never
                } else {
                    PrintPath::CdPath
                },
                display: Some(display),
                path: candidate,
            });
        }
    }

    None
}

fn starts_with_dot_component(path: &Path) -> bool {
    matches!(
        path.components().next(),
        Some(Component::CurDir | Component::ParentDir)
    )
}

fn current_logical_pwd(env_vars: &HashMap<String, String>) -> PathBuf {
    shell_var(env_vars, "PWD")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn logical_destination(old_pwd: &Path, target: &Path) -> PathBuf {
    let combined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        old_pwd.join(target)
    };

    normalize_logical_path(&combined)
}

fn normalize_logical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        normalized
    }
}

fn shell_var(env_vars: &HashMap<String, String>, name: &str) -> Option<String> {
    env_vars
        .get(name)
        .cloned()
        .or_else(|| env::var(name).ok())
        .filter(|value| !value.is_empty())
}

fn filesystem_path_for_display(dir: &str, env_vars: &HashMap<String, String>) -> PathBuf {
    // TODO(general.c/pathnames.h): In the Windows upstream-test harness, keep
    // Bash-visible /tmp paths logical while backing them with TMPDIR under the
    // guarded work directory.
    if cfg!(windows) {
        if dir == "/tmp" {
            if let Some(tmpdir) = shell_var(env_vars, "TMPDIR") {
                return PathBuf::from(tmpdir);
            }
        }
        if let Some(rest) = dir.strip_prefix("/tmp/") {
            if let Some(tmpdir) = shell_var(env_vars, "TMPDIR") {
                return PathBuf::from(tmpdir).join(rest);
            }
        }
    }

    PathBuf::from(dir)
}

fn set_shell_env(env_vars: &mut HashMap<String, String>, name: &str, value: String) {
    env_vars.insert(name.to_string(), value.clone());
    env::set_var(name, OsString::from(value));
}

fn shell_display_path(path: &Path) -> String {
    let mut value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        if value.len() >= 3 && value.as_bytes()[1] == b':' && value.as_bytes()[2] == b'/' {
            value = value[2..].to_string();
        }
    }
    if value.is_empty() {
        "/".to_string()
    } else {
        value
    }
}
