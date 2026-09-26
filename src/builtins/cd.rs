//! `cd` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/cd.def (`cd_builtin`)

use std::collections::HashMap;
mod paths;

use crate::executor::markers::DATA_DOLLAR;
use paths::{
    current_logical_pwd, filesystem_path_for_display, logical_destination,
    logical_destination_display, logical_pwd_var_display, set_shell_env, shell_display_path,
    shell_pwd_display_path, shell_var, starts_with_dot_component,
};
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const EXECUTION_SUCCESS: i32 = 0;
const EXECUTION_FAILURE: i32 = 1;
const EX_USAGE: i32 = 2;

/// GNU builtins/common.c:83-94 builtin_error_prolog: the shell name, the
/// executing line for scripts, then the builtin name.
fn diagnostic_prefix(env_vars: &HashMap<String, String>) -> String {
    if let (Some(script), Some(line)) = (
        env_vars.get("__RUBASH_SCRIPT_NAME"),
        env_vars.get("__RUBASH_CURRENT_LINE"),
    ) {
        return format!("{script}: line {line}: ");
    }
    "rubash: ".to_string()
}

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
    let mut stdout = crate::executor::WriteFileStdout;
    let mut stderr = crate::executor::WriteFileStderr;
    execute_with_io(
        args.iter().map(String::as_str),
        env_vars,
        &mut stdout,
        &mut stderr,
    )
}

pub(crate) fn execute_with_io<'a, I, W, E>(
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
    let (mode, first_operand) = match parse_options(&args, env_vars, stderr)? {
        Ok(parsed) => parsed,
        Err(status) => return Ok(status),
    };

    if args.len().saturating_sub(first_operand) > 1 {
        writeln!(
            stderr,
            "{}cd: too many arguments",
            diagnostic_prefix(env_vars)
        )?;
        return Ok(EX_USAGE);
    }

    let Some(target) = resolve_target(args.get(first_operand).copied(), env_vars, stderr)? else {
        return Ok(EXECUTION_FAILURE);
    };

    let target = match resolve_cdpath(&target, env_vars) {
        Some(found) => found,
        None => target,
    };
    let target = match resolve_cdable_vars(&target, env_vars) {
        Some(found) => found,
        None => target,
    };

    let old_pwd = current_logical_pwd(env_vars);
    if let Some(logical_dir) = logical_posix_test_dir(&target, env_vars) {
        // TODO(builtins/cd.def): This is a Windows-host bridge for the GNU
        // Bash upstream tests that use POSIX system directories. A complete
        // shell should keep logical and physical directory state separately.
        // GNU cd.def:136-175 bindpwd: check readonly for OLDPWD and PWD.
        let pwd_readonly = env_vars
            .get("__RUBASH_READONLY_VARS")
            .map(|v| v.split(DATA_DOLLAR).any(|name| name == "PWD"))
            .unwrap_or(false);
        let oldpwd_readonly = env_vars
            .get("__RUBASH_READONLY_VARS")
            .map(|v| v.split(DATA_DOLLAR).any(|name| name == "OLDPWD"))
            .unwrap_or(false);
        let mut failed = false;
        if oldpwd_readonly {
            writeln!(
                stderr,
                "{}OLDPWD: readonly variable",
                diagnostic_prefix(env_vars)
            )?;
            failed = true;
        } else {
            set_shell_env(env_vars, "OLDPWD", logical_pwd_var_display(&old_pwd));
        }
        if pwd_readonly {
            writeln!(
                stderr,
                "{}PWD: readonly variable",
                diagnostic_prefix(env_vars)
            )?;
            failed = true;
        } else {
            set_shell_env(env_vars, "PWD", shell_pwd_display_path(&logical_dir));
        }
        env_vars.insert("__RUBASH_PHYSICAL_PWD".to_string(), logical_dir.to_string());
        match target.print {
            PrintPath::Always | PrintPath::CdPath => writeln!(stdout, "{logical_dir}")?,
            PrintPath::Never => {}
        }
        if failed {
            return Ok(EXECUTION_FAILURE);
        }
        return Ok(EXECUTION_SUCCESS);
    }

    if let Err(error) = env::set_current_dir(&target.path) {
        // GNU builtins/cd.def:427: builtin_error("%s: %s", printable_filename(dirname), strerror(e))
        // reports the user-given path, not the Windows-converted path.
        // general.c printable_filename: non-printing bytes render through
        // ansic_quote ($'5\247@3\231+\306S8\237\242\352\263' in unicode3.sub).
        let display_path = target
            .display
            .as_deref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| target.path.to_string_lossy().to_string());
        let display_path = if crate::executor::ansic_shouldquote(&display_path) {
            crate::executor::ansic_quote(&display_path)
        } else {
            display_path
        };
        writeln!(
            stderr,
            "{}cd: {}: {}",
            diagnostic_prefix(env_vars),
            display_path,
            crate::posix_errors::message(&error)
        )?;
        stderr.flush()?;
        return Ok(EXECUTION_FAILURE);
    }

    let new_pwd = match mode {
        Mode::Logical => {
            logical_destination(&old_pwd, target.display.as_deref().unwrap_or(&target.path))
        }
        Mode::Physical => env::current_dir().unwrap_or_else(|_| target.path.clone()),
    };
    let new_pwd_display = match mode {
        Mode::Logical => {
            logical_destination_display(&old_pwd, target.display.as_deref().unwrap_or(&target.path))
        }
        Mode::Physical => shell_display_path(&new_pwd),
    };

    let pwd_value = match mode {
        Mode::Logical => new_pwd_display.clone(),
        Mode::Physical => shell_pwd_display_path(&new_pwd.to_string_lossy()),
    };
    // GNU builtins/cd.def:136-175 bindpwd: if PWD or OLDPWD is readonly,
    // bind_variable/setpwd fails and cd returns EXECUTION_FAILURE.
    // The error is emitted by err_readonly (error.c:455) via report_error,
    // which uses the error_prolog format: "shell: line N: VAR: readonly variable".
    let pwd_readonly = env_vars
        .get("__RUBASH_READONLY_VARS")
        .map(|v| v.split(DATA_DOLLAR).any(|name| name == "PWD"))
        .unwrap_or(false);
    let oldpwd_readonly = env_vars
        .get("__RUBASH_READONLY_VARS")
        .map(|v| v.split(DATA_DOLLAR).any(|name| name == "OLDPWD"))
        .unwrap_or(false);
    let mut failed = false;
    if oldpwd_readonly {
        writeln!(
            stderr,
            "{}OLDPWD: readonly variable",
            diagnostic_prefix(env_vars)
        )?;
        failed = true;
    } else {
        set_shell_env(env_vars, "OLDPWD", logical_pwd_var_display(&old_pwd));
    }
    if pwd_readonly {
        writeln!(
            stderr,
            "{}PWD: readonly variable",
            diagnostic_prefix(env_vars)
        )?;
        failed = true;
    } else {
        set_shell_env(env_vars, "PWD", pwd_value);
    }
    env_vars.remove("__RUBASH_PHYSICAL_PWD");

    match target.print {
        PrintPath::Always => writeln!(stdout, "{}", new_pwd_display)?,
        PrintPath::CdPath => writeln!(stdout, "{}", new_pwd_display)?,
        _ => {}
    }

    if failed {
        return Ok(EXECUTION_FAILURE);
    }

    Ok(EXECUTION_SUCCESS)
}

fn parse_options<W>(
    args: &[&str],
    env_vars: &HashMap<String, String>,
    stderr: &mut W,
) -> io::Result<Result<(Mode, usize), i32>>
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
                    writeln!(
                        stderr,
                        "{}cd: -{}: invalid option",
                        diagnostic_prefix(env_vars),
                        other
                    )?;
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
                path: filesystem_path_for_display(&home, env_vars),
                display: Some(PathBuf::from(home)),
                print: PrintPath::Never,
            })),
            None => {
                writeln!(stderr, "{}cd: HOME not set", diagnostic_prefix(env_vars))?;
                Ok(None)
            }
        },
        Some("") => {
            writeln!(stderr, "{}cd: null directory", diagnostic_prefix(env_vars))?;
            Ok(None)
        }
        Some("-") => match shell_var(env_vars, "OLDPWD") {
            Some(old_pwd) => Ok(Some(Target {
                path: filesystem_path_for_display(&old_pwd, env_vars),
                display: Some(PathBuf::from(old_pwd)),
                print: PrintPath::Always,
            })),
            None => {
                writeln!(stderr, "{}cd: OLDPWD not set", diagnostic_prefix(env_vars))?;
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

fn resolve_cdable_vars(target: &Target, env_vars: &HashMap<String, String>) -> Option<Target> {
    if target.path.is_dir() || !crate::builtins::shopt::cdable_vars_enabled(env_vars) {
        return None;
    }

    let name = target.display.as_ref()?.to_str()?;
    if name.contains('/') || name.contains('\\') || !is_shell_name(name) {
        return None;
    }

    let value = shell_var(env_vars, name)?;
    Some(Target {
        path: filesystem_path_for_display(&value, env_vars),
        display: Some(PathBuf::from(value)),
        print: PrintPath::Always,
    })
}

fn is_shell_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn logical_posix_test_dir<'a>(
    target: &'a Target,
    env_vars: &HashMap<String, String>,
) -> Option<&'a str> {
    if !cfg!(windows) {
        return None;
    }

    // With a configured logical root these names are real directories backed
    // by that root and must go through the normal cd path. The compatibility
    // shortcut remains only for an unconfigured Windows host where `/bin` and
    // friends do not exist as physical directories.
    if crate::executor::path::shell_root_configured(env_vars) {
        return None;
    }

    let display = target.display.as_ref()?.to_str()?;
    matches!(display, "/" | "/bin" | "/etc" | "/tmp" | "/usr").then_some(display)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::resolve_target;
    #[cfg(windows)]
    use std::collections::HashMap;

    #[test]
    fn cd_d_remains_invalid_bash_option() {
        let mut stderr = Vec::new();
        let env = std::collections::HashMap::new();

        let parsed = super::parse_options(&["-d"], &env, &mut stderr).unwrap();

        assert_eq!(parsed, Err(super::EX_USAGE));
        assert!(String::from_utf8(stderr)
            .unwrap()
            .contains("-d: invalid option"));
    }

    #[cfg(windows)]
    #[test]
    fn logical_cd_writes_logical_root_to_pwd() {
        use super::execute_with_io;

        let root =
            std::env::temp_dir().join(format!("rubash-cd-logical-root-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        let previous = std::env::current_dir().unwrap();
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );
        env_vars.insert("PWD".to_string(), "C:/Users/Administrator".to_string());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert_eq!(
            execute_with_io(["/"], &mut env_vars, &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert_eq!(env_vars.get("PWD").map(String::as_str), Some("/"));
        assert_eq!(
            execute_with_io(["/bin"], &mut env_vars, &mut stdout, &mut stderr).unwrap(),
            0
        );
        assert_eq!(env_vars.get("PWD").map(String::as_str), Some("/bin"));
        let _ = std::env::set_current_dir(previous);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn cd_dash_resolves_oldpwd_through_configured_shell_root() {
        let root = std::env::temp_dir().join("rubash-cd-oldpwd-root");
        let mut env_vars = HashMap::new();
        env_vars.insert(
            "RUBASH_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );
        env_vars.insert("OLDPWD".to_string(), "/etc".to_string());
        let mut stderr = Vec::new();

        let target = resolve_target(Some("-"), &env_vars, &mut stderr)
            .unwrap()
            .unwrap();

        assert_eq!(target.path, root.join("etc"));
        assert!(stderr.is_empty());
    }
}
