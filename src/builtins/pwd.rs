//! `pwd` builtin.
//!
//! GNU Bash source ownership:
//! - builtins/cd.def (`pwd_builtin`)

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};
use std::path::Path;

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

pub(crate) fn execute_with_io<'a, I, W, E>(
    args: I,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    let env_vars: HashMap<_, _> = crate::executor::env_helpers::process_env_pairs()
        .into_iter()
        .collect();
    // Standalone entry (no session): the process cwd is the only cwd.
    let cwd = env::current_dir().unwrap_or_default();
    execute_with_env_and_io(args, &env_vars, &cwd, stdout, stderr)
}

pub(crate) fn execute_with_env_and_io<'a, I, W, E>(
    args: I,
    env_vars: &HashMap<String, String>,
    cwd: &Path,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<i32>
where
    I: IntoIterator<Item = &'a str>,
    W: Write,
    E: Write,
{
    // GNU builtins/cd.def pwd_builtin: the default mode follows the
    // `physical` shell option (set -P), like cd (rubash#1039).
    let mut mode = if crate::builtins::set::shell_option_enabled(env_vars, "physical") {
        Mode::Physical
    } else {
        Mode::Logical
    };

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
                    // rubash#705 R7.4: GNU's get_name_for_error shape —
                    // `<script>: line N:` in script mode, `bash: line N:`
                    // under -c, bare `bash:` with neither. The old
                    // hardcoded `rubash: pwd:` prefix was the
                    // mixed-branding holdout on this path.
                    let prefix = match (
                        env_vars.get("__RUBASH_SCRIPT_NAME"),
                        env_vars.get("__RUBASH_CURRENT_LINE"),
                    ) {
                        (Some(script), Some(line)) => format!("{script}: line {line}: "),
                        _ => "bash: ".to_string(),
                    };
                    writeln!(stderr, "{prefix}pwd: -{other}: invalid option")?;
                    writeln!(stderr, "pwd: usage: pwd [-LP]")?;
                    return Ok(EX_USAGE);
                }
            }
        }
    }

    let Some(directory) = current_directory(mode, env_vars, cwd)? else {
        return Ok(EXECUTION_FAILURE);
    };

    writeln!(stdout, "{directory}")?;
    Ok(EXECUTION_SUCCESS)
}

fn current_directory(
    mode: Mode,
    env_vars: &HashMap<String, String>,
    cwd: &Path,
) -> io::Result<Option<String>> {
    if mode == Mode::Physical {
        if let Some(physical) = env_vars.get("__RUBASH_PHYSICAL_PWD") {
            return Ok(Some(physical.clone()));
        }
    }
    // G-02: the physical truth is the session cwd field, not the
    // process cwd.
    let physical = cwd.to_path_buf();

    if mode == Mode::Logical {
        if let Some(logical) = logical_pwd_if_current(&physical, env_vars) {
            return Ok(Some(logical));
        }
    }

    // GNU pwd.def (physical mode) prints get_working_directory — one
    // canonical shell-absolute form: the POSIX /<drive>/... spelling
    // (rubash#537), the same form cd's bindpwd and the startup PWD use.
    let display = crate::executor::path::shell_pwd_display_path(&shell_display_path(&physical));
    // GNU canonicalizes the logical spelling rather than echoing raw
    // getcwd, so an exactly-two-slashes $PWD prefix (POSIX `//`, the UNC
    // root on Windows) survives even in physical mode while getcwd
    // collapses it (rubash#1415). Restore it when $PWD names this very
    // directory.
    if mode == Mode::Physical && logical_pwd_if_current(&physical, env_vars).is_some() {
        if let Some(pwd) = env_vars.get("PWD") {
            if pwd.starts_with("//")
                && !pwd.starts_with("///")
                && display.starts_with('/')
                && !display.starts_with("//")
            {
                return Ok(Some(format!("/{display}")));
            }
        }
    }
    Ok(Some(display))
}

fn logical_pwd_if_current(physical: &Path, env_vars: &HashMap<String, String>) -> Option<String> {
    let logical = env_vars.get("PWD")?.clone();

    if !(logical.starts_with('/') || Path::new(&logical).is_absolute()) {
        return None;
    }

    let logical_physical = crate::executor::path::shell_path_to_windows(&logical, env_vars)
        .canonicalize()
        .ok()?;
    let current_physical = physical.canonicalize().ok()?;

    if logical_physical == current_physical {
        // GNU pwd.def (logical mode) echoes $PWD verbatim, and the value
        // stays in the ONE canonical POSIX /<drive>/… form the startup PWD
        // and cd's bindpwd use (rubash#537: the msys persona has no
        // native-spelling mode — a drive-form PWD death-loops nvm.sh's
        // nvm_find_up, whose ${path_%/*} walk only terminates on ""/"."
        // /a found file).
        Some(logical.replace('\\', "/"))
    } else {
        None
    }
}

fn shell_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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

    #[cfg(windows)]
    #[test]
    fn issue659_physical_mode_strips_verbatim_cwd_prefix() {
        // unixwin/niubash#659: the session cwd field can hold
        // canonicalize's extended-length form (\\?\C:\...) when a probe
        // result reaches it; `pwd -P` must still print the POSIX
        // /<drive>/... getcwd form, never `//?/...`.
        let native = env::temp_dir();
        let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", native.to_string_lossy()));
        let env_vars = HashMap::new();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_env_and_io(
            ["-P"].iter().copied(),
            &env_vars,
            &verbatim,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(status, EXECUTION_SUCCESS);
        let printed = String::from_utf8(stdout).unwrap();
        assert!(!printed.contains('?'), "{printed}");
        assert!(printed.starts_with('/'), "{printed}");
    }

    #[cfg(windows)]
    #[test]
    fn physical_mode_reports_physical_cwd() {
        // Physical mode prints get_working_directory in the one canonical
        // POSIX /<drive>/... form (GNU pwd.def, rubash#537).
        let old_pwd = env::var_os("PWD");
        env::set_var("PWD", "/usr");
        let expected = crate::executor::path::shell_pwd_display_path(&shell_display_path(
            &env::current_dir().unwrap(),
        ));

        let (_, stdout, _) = run(&["-P"]);

        assert_eq!(stdout, format!("{expected}\n"));
        match old_pwd {
            Some(value) => env::set_var("PWD", value),
            None => env::remove_var("PWD"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn logical_mode_resolves_pwd_against_executor_shell_root() {
        let root = env::temp_dir().join("rubash-pwd-logical-root");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("etc")).unwrap();

        let mut env_vars = HashMap::new();
        env_vars.insert("PWD".to_string(), "/etc".to_string());
        env_vars.insert(
            "__RUBASH_SHELL_ROOT".to_string(),
            root.to_string_lossy().to_string(),
        );

        assert_eq!(
            logical_pwd_if_current(&root.join("etc"), &env_vars),
            Some("/etc".to_string())
        );

        // /tmp is the per-user temp namespace even with a shell root set
        // (unixwin/niubash#94): with no TMPDIR it resolves to the process
        // temp dir, so a PWD of /tmp stays logical from there.
        env_vars.insert("PWD".to_string(), "/tmp".to_string());
        assert_eq!(
            logical_pwd_if_current(&env::temp_dir(), &env_vars),
            Some("/tmp".to_string())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn logical_mode_reports_posix_drive_pwd() {
        // GNU pwd.def echoes $PWD verbatim, in the ONE canonical POSIX
        // /<drive>/... spelling shared with the startup PWD and cd's
        // bindpwd value (rubash#537: no host path-style variable changes
        // this — the native D:/ spelling is retired from the engine).
        let old_pwd = env::var_os("PWD");
        let current = env::current_dir().unwrap();
        env::set_var("PWD", host_path_to_slash_drive(&current));

        let (_, stdout, _) = run(&[]);

        assert_eq!(stdout, format!("{}\n", host_path_to_slash_drive(&current)));
        match old_pwd {
            Some(value) => env::set_var("PWD", value),
            None => env::remove_var("PWD"),
        }
    }

    #[cfg(windows)]
    fn host_path_to_slash_drive(path: &Path) -> String {
        let display = shell_display_path(path);
        if display.len() >= 3 && display.as_bytes()[1] == b':' {
            let drive = (display.as_bytes()[0] as char).to_ascii_lowercase();
            format!("/{drive}/{}", &display[3..])
        } else {
            display
        }
    }
}
