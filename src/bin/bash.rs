//! Thin Windows-compatible bash entry point for Niubash installations.
//!
//! AI invoker entry — forwards to the host shell (niu/winuxsh).
//!
//! The shim intentionally forwards the command line and standard handles
//! without interpreting shell syntax. It lets Unix scripts resolve
//! `/usr/bin/bash` to the installed Niubash executable. The pre-rename
//! `winuxsh.exe` / `WINUXSH_SHELL` names stay in the fallback chain so
//! layouts built by older installers keep launching.

use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    let Some(shell) = locate_shell() else {
        let _ = writeln!(
            io::stderr(),
            "bash shim: cannot locate niu.exe; set NIU_SHELL to its path",
        );
        std::process::exit(127);
    };

    // GNU shell.c:733-737 keys the POSIX-mode contract off argv[0]: a shell
    // invoked as `sh' (or `ash') starts in POSIX mode, and a leading `-'
    // marks a login shell. One shim binary serves every installed name
    // (bash.exe, sh.exe, ...), so Windows argv[0] cannot carry the contract
    // through Command::new — the child would see the resolved host path and
    // `sh -c 'set -o | grep -w posix'` reported off with `$0` = niu. Forward
    // the invocation name through NIU_INVOKED_AS instead; the host applies
    // the same shell.c rules (posix on for sh/ash, login marker) AND binds
    // `$0` to the face: GNU set_shell_name (shell.c:1778-1806) derives
    // dollar_vars[0] from the same argv[0], so `sh -c 'echo $0'` prints `sh`
    // and `bash -c 'echo $0'` prints `bash` (rubash#590; #504 forwarded
    // only the sh/ash faces). Replacing — never scrubbing — the variable
    // keeps the isolation #504 introduced: it describes how one process was
    // invoked, and a stale value inherited from an outer `sh' run must not
    // switch a nested `sh -c 'bash -c ...'` into POSIX mode (bash.c: the
    // child shell keys off its own argv[0], which this forward re-creates
    // faithfully).
    let mut command = Command::new(&shell);
    match forwarded_invocation_name(env::args_os().next().as_deref()) {
        Some(invoked_as) => {
            command.env("NIU_INVOKED_AS", invoked_as);
        }
        None => {
            command.env_remove("NIU_INVOKED_AS");
        }
    }
    let status = command
        .args(env::args_os().skip(1))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            let _ = writeln!(
                io::stderr(),
                "bash shim: failed to start {}: {error}",
                shell.display()
            );
            std::process::exit(126);
        }
    }
}

fn locate_shell() -> Option<PathBuf> {
    for variable in ["NIU_SHELL", "WINUXSH_SHELL"] {
        if let Some(path) = env::var_os(variable).map(PathBuf::from) {
            if is_executable_file(&path) && !same_as_current_exe(&path) {
                return Some(path);
            }
        }
    }

    if let Ok(current) = env::current_exe() {
        let mut directory = current.parent().map(Path::to_path_buf);
        while let Some(path) = directory {
            for name in ["niu.exe", "winuxsh.exe"] {
                let candidate = path.join(name);
                if is_executable_file(&candidate) && !same_as_current_exe(&candidate) {
                    return Some(candidate);
                }
            }
            directory = path.parent().map(Path::to_path_buf);
        }
    }

    for variable in ["WINUXSH", "SHELL"] {
        if let Some(path) = env::var_os(variable).map(PathBuf::from) {
            if is_executable_file(&path) && !same_as_current_exe(&path) {
                return Some(path);
            }
        }
    }

    find_on_path("niu.exe")
        .or_else(|| find_on_path("winuxsh.exe"))
        .or_else(|| find_on_path("winuxsh"))
}

/// The invocation name worth forwarding to the host shell, if any. Returns
/// the raw `argv[0]` basename with `.exe` stripped (leading `-' preserved,
/// mirroring GNU shell.c's set_shell_name) when the stem names a shell face
/// the host must reproduce: `sh'/`ash' (the POSIX contract, rubash#504) and
/// `bash'/`rbash' (rubash#590 — GNU set_shell_name binds dollar_vars[0] to
/// the same argv[0], so `$0` must read the invocation face, and
/// shell.c:1258-1299 shell_is_restricted keys `rbash' off it). Any other
/// name forwards nothing, and the caller scrubs any inherited
/// NIU_INVOKED_AS instead.
fn forwarded_invocation_name(argv0: Option<&std::ffi::OsStr>) -> Option<String> {
    let argv0 = argv0?.to_str()?;
    let basename = argv0.rsplit(['/', '\\']).next()?;
    // Windows spellings can carry ".EXE"; strip the suffix case-insensitively.
    let stem = match basename.len().checked_sub(4) {
        Some(cut) if basename[cut..].eq_ignore_ascii_case(".exe") => &basename[..cut],
        _ => basename,
    };
    let stripped = stem.strip_prefix('-').unwrap_or(stem);
    if matches!(
        stripped.to_ascii_lowercase().as_str(),
        "sh" | "ash" | "bash" | "rbash"
    ) {
        Some(stem.to_string())
    } else {
        None
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| is_executable_file(candidate) && !same_as_current_exe(candidate))
    })
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn same_as_current_exe(path: &Path) -> bool {
    let Ok(current) = env::current_exe() else {
        return false;
    };
    let Ok(candidate) = path.canonicalize() else {
        return false;
    };
    current
        .canonicalize()
        .ok()
        .is_some_and(|current| current == candidate)
}

#[cfg(test)]
mod tests {
    use super::forwarded_invocation_name;

    #[test]
    fn sh_and_ash_names_forward_the_invocation_name() {
        assert_eq!(
            forwarded_invocation_name(Some("sh".as_ref())),
            Some("sh".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("ash".as_ref())),
            Some("ash".to_string())
        );
        // Windows spellings: the installed shims are sh.exe under both
        // winuxcmd\bin and winuxcmd\usr\bin.
        assert_eq!(
            forwarded_invocation_name(Some("C:/x/winuxcmd/bin/sh.exe".as_ref())),
            Some("sh".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("\\usr\\bin\\ASH.EXE".as_ref())),
            Some("ASH".to_string())
        );
    }

    #[test]
    fn login_marker_is_preserved_in_the_forwarded_name() {
        // GNU shell.c set_shell_name: a leading `-' marks a login shell and
        // must survive the forward so the host can honor it.
        assert_eq!(
            forwarded_invocation_name(Some("-sh".as_ref())),
            Some("-sh".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("-SH.exe".as_ref())),
            Some("-SH".to_string())
        );
    }

    #[test]
    fn other_names_forward_nothing() {
        // Non-shell names stay scrubbed: they carry no GNU contract the host
        // could reproduce (the shim's own dev name included).
        assert_eq!(forwarded_invocation_name(Some("shd".as_ref())), None);
        assert_eq!(forwarded_invocation_name(Some("niu".as_ref())), None);
        assert_eq!(forwarded_invocation_name(Some("bashsh".as_ref())), None);
        assert_eq!(forwarded_invocation_name(None), None);
    }

    #[test]
    fn bash_and_rbash_faces_forward_the_invocation_name() {
        // rubash#590: `$0` must read the invocation face — GNU set_shell_name
        // (shell.c:1797-1799) binds dollar_vars[0] to argv[0], which this
        // forward erases, so the bash/rbash faces travel in NIU_INVOKED_AS
        // the same way sh/ash already do.
        assert_eq!(
            forwarded_invocation_name(Some("bash".as_ref())),
            Some("bash".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("C:/x/winuxcmd/usr/bin/bash.exe".as_ref())),
            Some("bash".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("rbash".as_ref())),
            Some("rbash".to_string())
        );
        // GNU shell.c:1258-1299 (shell_is_restricted / maybe_make_restricted)
        // keys the restricted shell off the same basename, and shell.c:1787
        // keeps a leading `-' (login marker) on the name.
        assert_eq!(
            forwarded_invocation_name(Some("-rbash.exe".as_ref())),
            Some("-rbash".to_string())
        );
        assert_eq!(
            forwarded_invocation_name(Some("\\usr\\bin\\BASH.EXE".as_ref())),
            Some("BASH".to_string())
        );
    }
}
