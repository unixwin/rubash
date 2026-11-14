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

/// Outcome of a `pwd` execution: the status, plus whether the
/// physical face failed to resolve the cwd. The session executor
/// needs the second fact: in GNU, a failed physical resolution
/// (`sh_physpath (tcwd, 0)` returning 0) takes the
/// `tcwd && directory == 0` arm of builtins/cd.def pwd_builtin,
/// which calls resetpwd (cd.def:183-191) — freeing
/// the_current_working_directory and leaving it NULL. The logical
/// cache is poisoned, not merely stale, and the next logical face
/// must re-derive instead of printing the stale value (rubash#3349).
/// The standalone entry has no session cache to poison, so it only
/// reads `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PwdOutcome {
    pub status: i32,
    pub physical_resolution_failed: bool,
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
    // A one-shot process is a fresh shell, so its logical cwd is the
    // set_pwd adoption of the inherited $PWD (variables.c:907), or the
    // process cwd itself when the inherited value does not name it.
    let cwd = env::current_dir().unwrap_or_default();
    let logical = adopted_logical_pwd(&env_vars, &cwd).unwrap_or_else(|| shell_display_path(&cwd));
    Ok(execute_with_env_and_io(args, &env_vars, &cwd, Some(&logical), stdout, stderr)?.status)
}

pub(crate) fn execute_with_env_and_io<'a, I, W, E>(
    args: I,
    env_vars: &HashMap<String, String>,
    cwd: &Path,
    logical_cwd: Option<&str>,
    stdout: &mut W,
    stderr: &mut E,
) -> io::Result<PwdOutcome>
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
                    // `<script>: line N:` in script mode, the argv[0]
                    // name with its line under -c, the bare argv[0]-
                    // derived name with neither (error.c:90-120 via
                    // builtins/common.c:83-96). The fallback derives
                    // from the single argv[0] carrier chain
                    // (builtins::prefix, rubash#2791); the old
                    // hard-coded `"bash: "` arm discarded a present
                    // script/line arm and named every renamed shell
                    // "bash".
                    let prefix = crate::builtins::prefix::diagnostic_prefix(&|key: &str| {
                        env_vars.get(key).cloned()
                    });
                    writeln!(stderr, "{prefix}pwd: -{other}: invalid option")?;
                    writeln!(stderr, "pwd: usage: pwd [-LP]")?;
                    return Ok(PwdOutcome {
                        status: EX_USAGE,
                        physical_resolution_failed: false,
                    });
                }
            }
        }
    }

    // rubash#3268 (e): the session cwd is the unknown birth state
    // (empty field: born in a deleted directory, the birth getcwd
    // failed, no cd has succeeded since). GNU's pwd has no cached
    // string to print there either, so it calls
    // get_working_directory("pwd"), which prints this line and fails
    // (builtins/cd.def pwd_builtin -> builtins/common.c:592) —
    // nothing on stdout, status 1. (Master's #3298 guard, kept ahead
    // of the #2796-#2799 current_directory port: the ported function
    // prints the same line itself for a deleted-but-named cwd, so
    // this branch must not also print.)
    if cwd.as_os_str().is_empty() {
        writeln!(
            stderr,
            "pwd: error retrieving current directory: \
             getcwd: cannot access parent directories: No such file or directory"
        )?;
        return Ok(PwdOutcome {
            status: EXECUTION_FAILURE,
            physical_resolution_failed: false,
        });
    }

    let (directory, physical_resolution_failed) =
        current_directory(mode, env_vars, cwd, logical_cwd, stderr)?;
    let Some(directory) = directory else {
        return Ok(PwdOutcome {
            status: EXECUTION_FAILURE,
            physical_resolution_failed,
        });
    };

    writeln!(stdout, "{directory}")?;
    Ok(PwdOutcome {
        status: EXECUTION_SUCCESS,
        physical_resolution_failed,
    })
}

/// Resolve the directory `pwd` prints. The second return component
/// reports whether the physical face failed to resolve the cwd (the
/// `sh_physpath` failure of builtins/cd.def pwd_builtin): the caller
/// (the session executor) must then poison the logical cache, per
/// resetpwd (cd.def:183-191) — see rubash#3349.
fn current_directory<E>(
    mode: Mode,
    env_vars: &HashMap<String, String>,
    cwd: &Path,
    logical_cwd: Option<&str>,
    stderr: &mut E,
) -> io::Result<(Option<String>, bool)>
where
    E: Write,
{
    if mode == Mode::Physical {
        if let Some(physical) = env_vars.get("__RUBASH_PHYSICAL_PWD") {
            return Ok((Some(physical.clone()), false));
        }
        // GNU builtins/cd.def pwd_builtin: physical mode runs
        // sh_physpath on the_current_working_directory; when that
        // fails it resets through get_working_directory
        // (builtins/common.c:592), whose getcwd() call fails for a
        // deleted cwd and prints the diagnostic with the caller name
        // "pwd" — pwd then returns EXECUTION_FAILURE and prints
        // nothing (rubash#2798). The session cwd field is this port's
        // getcwd truth (G-02), so its resolvability is the gate: a
        // deleted cwd no longer canonicalizes. The failure ALSO takes
        // the `tcwd && directory == 0` arm (cd.def:495), which calls
        // resetpwd (cd.def:183-191): the cached logical value is freed
        // and left NULL — report it (the `true` below) so the session
        // poisons its logical cache instead of printing the stale
        // value on the next logical face (rubash#3349).
        if let Err(error) = cwd.canonicalize() {
            writeln!(
                stderr,
                "pwd: error retrieving current directory: getcwd: \
                 cannot access parent directories: {}",
                crate::posix_errors::message(&error)
            )?;
            return Ok((None, true));
        }
    }
    // G-02: the physical truth is the session cwd field, not the
    // process cwd.
    let physical = cwd.to_path_buf();
    // rubash#3268 (e): the empty field is the unknown birth state —
    // there is no directory to display (GNU fails in
    // get_working_directory; see the caller's diagnostic). Returning
    // the empty string here would print a blank line with status 0,
    // the same shape of lie as the "/" fabrication this state
    // replaced.
    if physical.as_os_str().is_empty() {
        return Ok((None, false));
    }

    if mode == Mode::Logical {
        // GNU cd.def pwd_builtin prints the_current_working_directory
        // — the shell's own logical cwd — never the $PWD variable: a
        // bare assignment to $PWD desynchronizes the variable and the
        // logical value stands (rubash#2797). The stored value prints
        // verbatim; it was canonicalized when cd (or startup adoption)
        // established it, and the one uncanonicalized form GNU stores
        // (the canon-failed `cd .` make_absolute form) prints verbatim
        // too.
        if let Some(tcwd) = logical_cwd {
            if logical_value_names_cwd(tcwd, &physical, env_vars) {
                return Ok((Some(tcwd.replace('\\', "/")), false));
            }
        }
        // Windows bridge state: cd's logical bridge branch (and
        // popd's bridged inner cd) moved ONLY the logical state —
        // the bridged POSIX name has no physical directory — and
        // marked the state with __RUBASH_PHYSICAL_PWD. A bridged
        // value can never pass the same-directory judgment above
        // (the physical field deliberately stayed put), so without
        // this arm the face falls through to the physical display
        // and the bridged name is lost — after `cd /`, `pwd` printed
        // the birth directory instead of `/` (the redir goldens
        // slice). Under the marker the cd family itself treats a
        // '/'-spelled $PWD as the logical truth
        // (paths::validated_logical_pwd, the same condition), and
        // the pre-port session fast path echoed it under the same
        // marker; print it the same way.
        if cfg!(windows) && env_vars.contains_key("__RUBASH_PHYSICAL_PWD") {
            if let Some(pwd) = env_vars.get("PWD") {
                if pwd.starts_with('/') {
                    return Ok((Some(pwd.replace('\\', "/")), false));
                }
            }
        }
        // No stored logical value (or a mover outside the cd family
        // invalidated it): derive it from $PWD as before, but print
        // the CANONICAL spelling — GNU's logical value was
        // canonicalized by sh_canonpath when it was established, so a
        // valid-but-non-canonical $PWD spelling (`/.`, trailing `/`,
        // `/../` components) never reaches the output (rubash#2797).
        if let Some(logical) = logical_pwd_if_current(&physical, env_vars) {
            return Ok((Some(canonical_logical_spelling(&logical)), false));
        }
        // rubash#3349: the poisoned state — logical_cwd is None
        // because a failed physical resolution went through resetpwd
        // (builtins/cd.def:183-191), which freed
        // the_current_working_directory and left it NULL. There is no
        // cached value to print, so this face re-derives through the
        // getcwd gate, exactly like get_working_directory("pwd")
        // (builtins/common.c:592): a deleted cwd cannot be re-derived,
        // so the same diagnostic prints and pwd fails instead of
        // printing the stale session cwd field string below. The gate
        // fires only in the poisoned state: with a stored logical
        // value the arm above already printed it (the #2798 control),
        // and with a resolvable cwd the fall-through below prints the
        // re-derived physical display.
        if logical_cwd.is_none() {
            if let Err(error) = cwd.canonicalize() {
                writeln!(
                    stderr,
                    "pwd: error retrieving current directory: getcwd: \
                     cannot access parent directories: {}",
                    crate::posix_errors::message(&error)
                )?;
                return Ok((None, false));
            }
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
                return Ok((Some(format!("/{display}")), false));
            }
        }
    }
    Ok((Some(display), false))
}

pub(crate) fn logical_pwd_if_current(
    physical: &Path,
    env_vars: &HashMap<String, String>,
) -> Option<String> {
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

/// Textual canonicalization of a logical path spelling: `.`
/// components drop, `..` pops the previous component, empty
/// components collapse — the sh_canonpath result for a spelling whose
/// components name real directories (builtins/cd.def
/// change_to_directory's PATH_CHECKDOTDOT processing), WITHOUT
/// resolving symlinks. Exactly two leading slashes survive (the POSIX
/// `//` root, rubash#1415); three or more collapse to one.
pub(crate) fn canonical_logical_spelling(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    let normalized = if path.starts_with('/') {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    };
    if path.starts_with("//") && !path.starts_with("///") && !normalized.starts_with("//") {
        format!("/{normalized}")
    } else {
        normalized
    }
}

/// Whether a logical cwd VALUE names the session's current directory
/// — the `same_file(temp_string, ".")` judgment of variables.c:907
/// set_pwd, in the runtime form logical_pwd_if_current already uses:
/// canonical equality when both sides resolve. When either side
/// cannot be resolved (a deleted cwd resolves on neither side), the
/// judgment falls back to canonical-spelling equality: the stored
/// logical value and the session cwd field spell the same directory,
/// which is the only same-directory evidence a deleted cwd leaves.
pub(crate) fn logical_value_names_cwd(
    value: &str,
    cwd: &Path,
    env_vars: &HashMap<String, String>,
) -> bool {
    let value_fs = crate::executor::path::shell_path_to_windows(value, env_vars);
    if let (Ok(value_physical), Ok(current_physical)) =
        (value_fs.canonicalize(), cwd.canonicalize())
    {
        return value_physical == current_physical;
    }
    canonical_logical_spelling(value) == canonical_logical_spelling(&shell_display_path(cwd))
}

/// The startup adoption of an inherited $PWD (variables.c:907
/// set_pwd): when the imported value is absolute and names the
/// current directory, the shell's logical cwd is its sh_canonpath
/// canonicalization (spelling canonicalization, NO symlink
/// resolution). The $PWD variable itself is a separate question —
/// set_pwd leaves the imported spelling in the variable in
/// non-POSIX mode (rubash#2799). None when the value names another
/// directory (or no directory), in which case the logical cwd is
/// the physical cwd.
pub(crate) fn adopted_logical_pwd(
    env_vars: &HashMap<String, String>,
    cwd: &Path,
) -> Option<String> {
    let verbatim = logical_pwd_if_current(cwd, env_vars)?;
    Some(canonical_logical_spelling(&verbatim))
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
            None,
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
    fn logical_mode_prints_bridged_name_in_bridge_state() {
        // After cd's Windows bridge branch (`cd /` with no configured
        // shell root; popd's bridged inner cd is the twin), the
        // session's logical position is the bridged POSIX name: $PWD
        // and the stored logical cwd hold it, __RUBASH_PHYSICAL_PWD
        // marks the state, and the physical cwd field deliberately
        // stayed in the real directory. Logical pwd must print the
        // bridged name, not the physical directory — the shape the
        // redir goldens slice records (redir6.sub: `cd /`, `pwd`).
        let cwd = env::temp_dir();
        let mut env_vars = HashMap::new();
        env_vars.insert("PWD".to_string(), "/".to_string());
        env_vars.insert("__RUBASH_PHYSICAL_PWD".to_string(), "/".to_string());
        for args in [&["-L"][..], &[][..]] {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = execute_with_env_and_io(
                args.iter().copied(),
                &env_vars,
                &cwd,
                Some("/"),
                &mut stdout,
                &mut stderr,
            )
            .unwrap();
            assert_eq!(status, EXECUTION_SUCCESS);
            assert_eq!(String::from_utf8(stdout).unwrap(), "/\n");
        }

        // Negative control: the same values WITHOUT the bridge marker
        // are a desynchronized $PWD, not a bridge state — the face
        // must not print the stored spelling; it derives the physical
        // directory instead (rubash#2797).
        env_vars.remove("__RUBASH_PHYSICAL_PWD");
        let expected = crate::executor::path::shell_pwd_display_path(&shell_display_path(&cwd));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = execute_with_env_and_io(
            ["-L"].iter().copied(),
            &env_vars,
            &cwd,
            Some("/"),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(status, EXECUTION_SUCCESS);
        assert_eq!(String::from_utf8(stdout).unwrap(), format!("{expected}\n"));
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
