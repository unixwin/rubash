//! Issues rubash#3315 + rubash#3318 — path-spelling faces of the exec
//! builtin family.
//!
//! #3315: a plain external command invoked by a slash path must keep the
//! typed word as the child's argv[0] — GNU shell_execve (execute_cmd.c)
//! hands execve the command word verbatim, so `/bin/cat /nonexistent`
//! prints `/bin/cat: ...`. Rubash's in-process cat emulation hardcoded
//! `cat:` for its operand diagnostics; the prefix is now the typed word
//! (the same argv[0] the spawn funnel's `arg0` already carries).
//!
//! #3318: `$BASH` must be the INVOCATION path — GNU variables.c:776
//! get_bash_name keeps an absolute argv[0] verbatim (symlinks kept),
//! PWD-joins a `./` spelling, and PATH-walks a bare name to the
//! as-written join. Rubash seeded it from std::env::current_exe, which
//! canonicalizes symlinks, so a `/tmp/linkbash` invocation reported the
//! resolved binary path.

use std::process::Command;

fn rubash() -> &'static str {
    env!("CARGO_BIN_EXE_rubash")
}

/// The exact render main.rs applies to the $BASH seed: separators fold to
/// `/` and the result passes through the shared display exit
/// (`shell_pwd_display_path`), which on Windows also folds a drive
/// designator to the persona form (`D:/...` -> `/d/...`). Pin expectations
/// are spelled natively and normalized through the same transform.
fn display(path: &str) -> String {
    rubash::executor::shell_pwd_display_path(&path.replace('\\', "/"))
}

/// #3315: the typed slash path is the emulated cat's diagnostic argv[0].
#[test]
fn emulated_cat_diagnostic_uses_typed_path() {
    let output = Command::new(rubash())
        .arg("-c")
        .arg("/bin/cat /nonexistent-xyz")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("/bin/cat: /nonexistent-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}

/// #3315 guard: the bare-name spelling still prints `cat:`.
#[test]
fn emulated_cat_diagnostic_uses_bare_name() {
    let output = Command::new(rubash())
        .arg("-c")
        .arg("cat /nonexistent-xyz")
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("cat: /nonexistent-xyz: No such file or directory"),
        "stderr={stderr:?}"
    );
}

/// #3318: invoking the binary through a symlink keeps the LINK as $BASH;
/// a `./` spelling PWD-joins; a PATH-found bare name reports the join.
#[test]
fn bash_var_keeps_invocation_path() {
    let dir = std::env::temp_dir().join("rubash3318");
    std::fs::create_dir_all(&dir).unwrap();
    // Windows spawns need the .exe extension to resolve the image; the
    // bare name stays on unix.
    let link_name = if cfg!(windows) {
        "linkbash3318.exe"
    } else {
        "linkbash3318"
    };
    let link = dir.join(link_name);
    let _ = std::fs::remove_file(&link);
    #[cfg(unix)]
    std::os::unix::fs::symlink(rubash(), &link).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(rubash(), &link).unwrap();

    // Absolute symlink invocation.
    let output = Command::new(&link)
        .arg("-c")
        .arg("echo $BASH")
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        display(&link.to_string_lossy()),
        "absolute symlink: stdout={stdout:?}"
    );

    // `./`-spelling invocation: $PWD + /linkbash3318. Unix-only at the
    // process level: a Windows child always sees the RESOLVED program
    // path as argv[0] (the typed-word arg0 at path.rs:2062 is
    // `#[cfg(unix)]` — std::process::Command has no argv[0] override on
    // Windows), so the `./` fast path is pinned at lib level below.
    #[cfg(unix)]
    {
        let output = Command::new(format!("./{link_name}"))
            .arg("-c")
            .arg("echo $BASH")
            .current_dir(&dir)
            .output()
            .expect("run rubash");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            display(&format!("{}/{link_name}", dir.to_string_lossy())),
            "./-spelling: stdout={stdout:?}"
        );
    }

    // The `./` fast path itself is platform-neutral (variables.c:790-799
    // joins PWD literally, dropping the `.`): assert the lib-level join
    // on every target.
    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    env.insert("PWD".to_string(), cwd.clone());
    assert_eq!(
        rubash::executor::bash_invocation_name("./linkbash3318", &env).as_deref(),
        Some(format!("{cwd}/linkbash3318").as_str()),
        "./ fast-path join"
    );

    // PATH-found bare name: the join as written.
    #[cfg(unix)]
    let output = Command::new("env")
        .arg(format!(
            "PATH={}:{}",
            dir.to_string_lossy(),
            "/usr/bin:/bin"
        ))
        .arg(&link_name)
        .arg("-c")
        .arg("echo $BASH")
        .output()
        .expect("run rubash");
    // Same delivery constraint as the `./` case: CreateProcess searches
    // the PARENT's PATH for a bare image name, so the PATH walk is done
    // by a rubash spawn that inherits the staged PATH.
    #[cfg(windows)]
    let output = Command::new(rubash())
        .env("PATH", format!("{}:/usr/bin:/bin", dir.to_string_lossy()))
        .arg("-c")
        .arg(format!("{link_name} -c 'echo $BASH'"))
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        display(&format!("{}/{link_name}", dir.to_string_lossy())),
        "PATH-found: stdout={stdout:?}"
    );

    // Forward-slash drive spelling: `C:/...` argv[0] is absolute too
    // (general.h:269 __CYGWIN__ ABSPATH), kept verbatim and rendered
    // through the same display exit.
    #[cfg(windows)]
    {
        let output = Command::new(link.to_string_lossy().replace('\\', "/"))
            .arg("-c")
            .arg("echo $BASH")
            .output()
            .expect("run rubash");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            display(&link.to_string_lossy()),
            "forward-slash drive: stdout={stdout:?}"
        );
    }
}

/// #3318 on the Windows spellings: the absolute test is the Cygwin-port
/// ABSPATH (general.h:269) — a drive-letter designator (`C:` anything,
/// drive-relative `C:name` included) or a leading separator of either
/// spelling (general.h:279 ISDIRSEP) — so `C:\...`, `C:/...`, `\dir\...`,
/// `\\?\...` and `\\server\share\...` all keep verbatim like POSIX `/...`
/// and never reach the PATH walk or the textual $PWD join. These are
/// lib-level assertions because most of the spellings cannot be spawned
/// as a real argv[0] in a test process.
#[cfg(windows)]
#[test]
fn bash_invocation_name_windows_absolute_spellings() {
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    for spelling in [
        "C:\\Tools\\bash.exe",
        "C:/Tools/bash.exe",
        "C:bash.exe",
        "\\Tools\\bash.exe",
        "\\\\?\\C:\\Tools\\bash.exe",
        "\\\\server\\share\\bash.exe",
    ] {
        assert_eq!(
            rubash::executor::bash_invocation_name(spelling, &env).as_deref(),
            Some(spelling),
            "spelling={spelling:?}"
        );
    }
}

/// POSIX keeps general.h:266 `x[0] == '/'`: the drive/UNC spellings are
/// NOT absolute there — `C:\foo` is a bare name that PATH-walks and
/// misses (the caller then falls back), never a verbatim keep.
#[cfg(unix)]
#[test]
fn bash_invocation_name_posix_absolute_only() {
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    assert_eq!(
        rubash::executor::bash_invocation_name("C:\\Tools\\bash", &env),
        None
    );
    assert_eq!(
        rubash::executor::bash_invocation_name("\\\\server\\share\\bash", &env),
        None
    );
}
