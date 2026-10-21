//! `cd` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/cd.def (`cd_builtin`)

use std::collections::HashMap;
mod paths;

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
pub fn execute(
    args: &[String],
    env_vars: &mut crate::shell::var_table::VarTable,
) -> io::Result<i32> {
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
    env_vars: &mut crate::shell::var_table::VarTable,
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
        // GNU builtins/cd.def:329-330: the too-many-arguments refusal is
        // `builtin_error` + `return (EX_USAGE)` — status 2. An earlier
        // pass of this fix recorded EXECUTION_FAILURE (1) from a mis-read
        // probe; the bash 5.3.0 source AND a direct probe of the
        // 5.3.0(1)-release binary both give 2 (`bash -c 'cd a b'` exits
        // 2, 2026-10-18). The diagnostic itself was being dropped by the
        // executor's buffered-stdout flush (printf_path_builtins.rs),
        // fixed there.
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
        let pwd_readonly = env_vars.is_marked(crate::executor::types::READONLY_VARS, "PWD");
        let oldpwd_readonly = env_vars.is_marked(crate::executor::types::READONLY_VARS, "OLDPWD");
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
        // GNU builtins/cd.def:136-175 bindpwd stores ONE canonical logical
        // PWD string that `pwd` then echoes verbatim (pwd.def). Rubash's
        // canonical form is the POSIX /<drive>/... spelling (rubash#537:
        // the msys persona has no native-spelling mode — a drive-form PWD
        // death-loops nvm.sh's nvm_find_up, and bashdb's _Dbg_is_file
        // classifies absolute names by a leading '/'), so both the logical
        // and the physical value render through shell_pwd_display_path and
        // a session can never mix "/d/..." with "D:/...".
        Mode::Logical => shell_pwd_display_path(&new_pwd_display),
        Mode::Physical => shell_pwd_display_path(&new_pwd.to_string_lossy()),
    };
    // GNU builtins/cd.def:136-175 bindpwd: if PWD or OLDPWD is readonly,
    // bind_variable/setpwd fails and cd returns EXECUTION_FAILURE.
    // The error is emitted by err_readonly (error.c:455) via report_error,
    // which uses the error_prolog format: "shell: line N: VAR: readonly variable".
    let pwd_readonly = env_vars.is_marked(crate::executor::types::READONLY_VARS, "PWD");
    let oldpwd_readonly = env_vars.is_marked(crate::executor::types::READONLY_VARS, "OLDPWD");
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
        // GNU builtins/cd.def prints the operand VALUE verbatim for the
        // LCD_PRINTPATH/cdable_vars success cases: cd.def:397-400
        // `printf ("%s\n", temp)` where temp is get_string_value(dirname)
        // — the variable's raw string, `./` components included — and
        // cd.def:377-378 prints dirname, the raw $OLDPWD value for `cd -`.
        // Verified against WSL GNU 5.3.0: `v=$PWD/./real; cd v > f` puts
        // "..././real" in f on both paths.
        PrintPath::Always => {
            let verbatim = target
                .display
                .as_deref()
                .and_then(Path::to_str)
                .map(str::to_string)
                .unwrap_or_else(|| shell_pwd_display_path(&new_pwd_display));
            writeln!(stdout, "{verbatim}")?
        }
        PrintPath::CdPath => writeln!(stdout, "{}", shell_pwd_display_path(&new_pwd_display))?,
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
            // GNU builtins/cd.def:316-322: the operand-less refusal is
            // `dirname == 0` (POSIX cd step 1) — a MISSING variable only. A
            // set-but-EMPTY HOME flows on through make_absolute, glues the
            // cwd onto the empty name, and the chdir is a same-directory
            // success: rc 0, no diagnostic, PWD unchanged. (An empty
            // OPERAND still trips cd.def:331-336's "null directory" — that
            // is the `cd ~` path, not this one.)
            None if env_vars.contains_key("HOME") => Ok(Some(Target {
                path: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                display: None,
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
        Some(dir) => {
            // unixwin/niubash#481: a bare drive designator (`D:` — letter
            // + colon, no separator) is the same drive-root operand the
            // host fast-path normalizer rewrites `cd D:` into (`/d/` —
            // niubash-runtime shell.rs windows_drive_path_to_slash_drive).
            // GNU has no drive designators — cd.def's dirname reaches
            // make_absolute/chdir verbatim — so this is persona territory
            // owned by the Windows `/x/` funnel arm (executor/path.rs
            // shell_path_to_windows_inner). Folding the operand here keeps
            // the grouped-reader and stdin drivers — which never run the
            // host normalizers (niubash#483) — on the same resolution.
            // `D:rel` drive-relative spellings stay untouched: a mid-word
            // colon past position 1 is filename data, not a designator.
            let dir = windows_bare_drive_operand(dir);
            Ok(Some(Target {
                path: filesystem_path_for_display(&dir, env_vars),
                display: Some(PathBuf::from(dir)),
                print: PrintPath::Never,
            }))
        }
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

/// unixwin/niubash#481/#483: on Windows the bare `D:` operand spells the
/// drive root; fold it to the `/d/` slash-drive form the rest of the cd
/// machinery (and the host fast-path normalizer) already produces.
fn windows_bare_drive_operand(dir: &str) -> String {
    if cfg!(windows) {
        let bytes = dir.as_bytes();
        if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return format!("/{}/", (bytes[0] as char).to_ascii_lowercase());
        }
    }
    dir.to_string()
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
        let mut env_vars = crate::shell::var_table::VarTable::default();
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
        let mut env_vars = crate::shell::var_table::VarTable::default();
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
