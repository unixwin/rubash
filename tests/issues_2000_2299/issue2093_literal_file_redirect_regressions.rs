//! Issue rubash#2093: the /bin and /usr/bin redirect faces for
//! echo/printf/pwd (registry `SlashKind::RedirectToBuiltin`) answered
//! BUILTIN semantics unconditionally — no check on the literal file the
//! spelling names. GNU findcmd.c:383-385 execve's a slash-bearing word
//! verbatim, so when a real executable file exists at that path the
//! FILE must answer (its own --help/--version banner, no xpg_echo
//! sensitivity, the physical-pwd default); only a missing file keeps
//! the emulated redirect (the Windows no-/bin default,
//! docs/builtins.md §4.3).
//!
//! Fix: an existence gate (path.rs `literal_executable_file` — the
//! literal probe of find_user_command's slash branch, WITHOUT the
//! Windows basename-PATH fallbacks) applied at both claim faces:
//! external_inner::handle_external_shortcuts (echo) and
//! external_file_builtins::handle_external_file_builtins (pwd/printf).
//! The pre-existing gating asymmetry is preserved: printf/pwd ride the
//! external_file_builtins host flag, the echo face does not — the gate
//! alone decides on the echo face, so a gate-open /bin/echo runs the
//! file even with the layer off.
//!
//! Windows pins drive the gate through a configured
//! `__RUBASH_SHELL_ROOT` (path.rs `configured_shell_root`): an empty
//! root keeps the redirect (gate closed — the documented Windows
//! default), a root carrying `bin/echo.bat` & friends opens it (the
//! PATHEXT probe in `executable_candidate` finds the batch file from
//! the extensionless spelling; the spawn runs it through the command
//! processor). Output is captured by file redirection, statuses by the
//! run's final status — the same harness issue2066 uses.
//!
//! Unix pins compare rubash's answer with a DIRECT spawn of the same
//! literal file (std::process::Command) — the file is its own oracle,
//! so each pin fails exactly while a face still answers with builtin
//! semantics.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rubash::api::{RunOptions, Session};

/// Command resolution keeps a process-global cache with a single
/// fingerprint slot (executor/path.rs, rubash#159), and its `hashed`
/// mirror is shared by every executor state in the process. Sessions
/// with different `__RUBASH_SHELL_ROOT`/PATH fingerprints thrash it,
/// so these pins serialize (the same pattern as issue2066's SERIAL)
/// and every session starts with `hash -r`.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2093-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn opts() -> RunOptions {
    RunOptions::default()
}

/// The empty-bin PATH shared by every session here: the faces under
/// test answer (or decline to a literal path) before any PATH walk, so
/// an empty PATH keeps the pins hermetic against host toolsets.
fn empty_bin(root: &Path) -> PathBuf {
    let bin = root.join("empty-bin");
    std::fs::create_dir_all(&bin).expect("empty bin");
    bin
}

/// Shell-side spelling of a host path (forward slashes; the script
/// always single-quotes it, so spaces survive too).
fn shell_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Run one command line, capturing its stdout into `out` and
/// returning its exit status.
fn capture_out(session: &mut Session, line: &str, out: &Path) -> i32 {
    let script = format!("{line} > '{}'", shell_path(out));
    let outcome = session.run(&script, &opts()).expect("run");
    outcome.status
}

fn read_trim(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[cfg(windows)]
fn session_with_root(root: &Path) -> Session {
    let mut session = Session::builder()
        .env("PATH", shell_path(&empty_bin(root)))
        .env("__RUBASH_SHELL_ROOT", root.display().to_string())
        .working_directory(root.to_path_buf())
        .build()
        .expect("session builds");
    let _ = session.run("hash -r", &opts());
    session
}

// ------------------------------------------------------------------
// Windows: the gate driven through __RUBASH_SHELL_ROOT.
// ------------------------------------------------------------------

#[cfg(windows)]
fn write_probe_bat(root: &Path, dir: &str, name: &str, marker: &str) {
    let dir_path = root.join(dir);
    std::fs::create_dir_all(&dir_path).expect("probe dir");
    std::fs::write(
        dir_path.join(format!("{name}.bat")),
        format!("@echo {marker}\r\n"),
    )
    .expect("write probe");
}

#[cfg(windows)]
fn closed_root(name: &str) -> PathBuf {
    let root = temp_dir(name);
    empty_bin(&root);
    root
}

#[cfg(windows)]
fn open_root(name: &str) -> PathBuf {
    let root = temp_dir(name);
    empty_bin(&root);
    write_probe_bat(&root, "bin", "echo", "PROBE-2093-ECHO-BIN");
    write_probe_bat(&root, "usr/bin", "echo", "PROBE-2093-ECHO-USRBIN");
    write_probe_bat(&root, "bin", "printf", "PROBE-2093-PRINTF");
    write_probe_bat(&root, "bin", "pwd", "PROBE-2093-PWD");
    root
}

#[cfg(windows)]
#[test]
fn issue2093_gate_closed_keeps_builtin_answers() {
    let _serial = serial();
    let root = closed_root("closed");
    let mut session = session_with_root(&root);

    // Family 1 (echo): the builtin echo answer for --version is the
    // literal word, rc 0.
    let out = root.join("echo-version.out");
    let rc = capture_out(&mut session, "/bin/echo --version", &out);
    assert_eq!(rc, 0, "builtin echo accepts --version as a literal word");
    assert_eq!(read_trim(&out), "--version");

    // Family 1 (printf/pwd): the builtin usage error, rc 2.
    let out = root.join("printf-version.out");
    let rc = capture_out(&mut session, "/bin/printf --version 2>/dev/null", &out);
    assert_eq!(rc, 2, "builtin printf refuses --version with usage rc 2");
    assert_eq!(read_trim(&out), "", "no version banner on the builtin face");

    let out = root.join("pwd-version.out");
    let rc = capture_out(&mut session, "/bin/pwd --version 2>/dev/null", &out);
    assert_eq!(rc, 2, "builtin pwd refuses --version with usage rc 2");

    // Family 2 (xpg_echo): with no literal file the emulated echo is
    // still the echo builtin, so it honors the shell option.
    let _ = session.run("shopt -s xpg_echo", &opts());
    let out = root.join("xpg.out");
    let rc = capture_out(&mut session, "/bin/echo 'a\\tb'", &out);
    let _ = session.run("shopt -u xpg_echo", &opts());
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "a\tb", "xpg_echo interprets the escape");
}

#[cfg(windows)]
#[test]
fn issue2093_gate_open_file_answers_all_faces() {
    let _serial = serial();
    let root = open_root("open");
    let mut session = session_with_root(&root);

    // /bin and /usr/bin echoes.
    let out = root.join("echo-bin.out");
    let rc = capture_out(&mut session, "/bin/echo --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(
        read_trim(&out),
        "PROBE-2093-ECHO-BIN",
        "the literal file ran"
    );

    let out = root.join("echo-usrbin.out");
    let rc = capture_out(&mut session, "/usr/bin/echo --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-ECHO-USRBIN");

    // printf face: the file answers rc 0 where the builtin gave rc 2.
    let out = root.join("printf.out");
    let rc = capture_out(&mut session, "/bin/printf --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-PRINTF");

    // pwd face.
    let out = root.join("pwd.out");
    let rc = capture_out(&mut session, "/bin/pwd --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-PWD");

    // Family 2 (xpg_echo) with the gate open: the file answers and
    // IGNORES the shell option — the builtin TAB rendering (family 2's
    // pre-fix answer) never appears.
    let _ = session.run("shopt -s xpg_echo", &opts());
    let out = root.join("xpg-open.out");
    let rc = capture_out(&mut session, "/bin/echo 'a\\tb'", &out);
    let _ = session.run("shopt -u xpg_echo", &opts());
    assert_eq!(rc, 0);
    assert_eq!(
        read_trim(&out),
        "PROBE-2093-ECHO-BIN",
        "the file, not the option-honoring builtin, answered"
    );

    // Command-substitution face: the fork runs the same dispatch, so
    // the file answers there too.
    let out = root.join("comsub.out");
    let script = format!(
        "x=$(/bin/echo --version); printf '%s\\n' \"$x\" > '{}'",
        shell_path(&out)
    );
    let outcome = session.run(&script, &opts()).expect("comsub run");
    assert_eq!(outcome.status, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-ECHO-BIN");
}

#[cfg(windows)]
#[test]
fn issue2093_flag_matrix_layer_off_leaves_the_gate_in_charge() {
    let _serial = serial();
    // Layer OFF + gate OPEN: printf/pwd already fell to the real spawn
    // with the flag off (pre-existing), so the file answers. The echo
    // face is NOT flag-gated — the gate alone decides, and with the
    // gate open the file answers there too.
    let root = open_root("flag-off-open");
    let mut session = session_with_root(&root);
    session
        .executor_mut()
        .set_external_file_builtins_enabled(false);

    let out = root.join("printf-off.out");
    let rc = capture_out(&mut session, "/bin/printf --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-PRINTF");

    let out = root.join("pwd-off.out");
    let rc = capture_out(&mut session, "/bin/pwd --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "PROBE-2093-PWD");

    let out = root.join("echo-off.out");
    let rc = capture_out(&mut session, "/bin/echo --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(
        read_trim(&out),
        "PROBE-2093-ECHO-BIN",
        "the echo face follows the gate, not the host flag"
    );

    // Layer OFF + gate CLOSED: the echo face keeps the emulated
    // builtin answer (it never consults the flag on either setting).
    let root = closed_root("flag-off-closed");
    let mut session = session_with_root(&root);
    session
        .executor_mut()
        .set_external_file_builtins_enabled(false);
    let out = root.join("echo-off-closed.out");
    let rc = capture_out(&mut session, "/bin/echo --version", &out);
    assert_eq!(rc, 0);
    assert_eq!(read_trim(&out), "--version");
}

// ------------------------------------------------------------------
// Unix: the literal system file is its own oracle.
// ------------------------------------------------------------------

#[cfg(unix)]
fn direct_spawn(path: &str, args: &[&str]) -> Option<(String, i32)> {
    let output = std::process::Command::new(path).args(args).output().ok()?;
    Some((
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status.code().unwrap_or(-1),
    ))
}

/// A unix session with a hermetic PATH: the pins resolve slash-bearing
/// words literally, so a PATH walk never matters for them.
#[cfg(unix)]
fn unix_session(dir: &Path) -> Session {
    let mut session = Session::builder()
        .env("PATH", shell_path(&empty_bin(dir)))
        .working_directory(dir.to_path_buf())
        .build()
        .expect("session builds");
    let _ = session.run("hash -r", &opts());
    session
}

#[cfg(unix)]
#[test]
fn issue2093_echo_version_matches_the_literal_file() {
    if !Path::new("/bin/echo").is_file() {
        return; // no literal file on this host: the redirect must stand anyway
    }
    let _serial = serial();
    let dir = temp_dir("ux-echo-version");
    let mut session = unix_session(&dir);
    let direct = direct_spawn("/bin/echo", &["--version"]).expect("spawn /bin/echo");

    let out = dir.join("echo-version.out");
    let rc = capture_out(&mut session, "/bin/echo --version", &out);
    // Pre-fix the face answered the builtin echo (literal "--version",
    // rc 0); post-fix the file answers itself.
    assert_eq!(rc, direct.1, "exit status must match the literal file");
    assert_eq!(
        read_trim(&out),
        direct.0.trim(),
        "stdout must be the literal file's own answer"
    );
    assert_ne!(
        direct.0.trim(),
        "--version",
        "host sanity: this pin needs a file whose --version differs from the word"
    );
}

#[cfg(unix)]
#[test]
fn issue2093_xpg_echo_does_not_reach_the_literal_file() {
    if !Path::new("/bin/echo").is_file() {
        return;
    }
    let _serial = serial();
    let dir = temp_dir("ux-xpg");
    let mut session = unix_session(&dir);
    // The file does not read shell options: the escape stays literal.
    let direct = direct_spawn("/bin/echo", &["a\\tb"]).expect("spawn /bin/echo");

    let _ = session.run("shopt -s xpg_echo", &opts());
    let out = dir.join("xpg.out");
    let rc = capture_out(&mut session, "/bin/echo 'a\\tb'", &out);
    let _ = session.run("shopt -u xpg_echo", &opts());
    assert_eq!(rc, direct.1);
    assert_eq!(
        read_trim(&out),
        direct.0.trim(),
        "xpg_echo must not leak into the literal file's answer"
    );
}

#[cfg(unix)]
#[test]
fn issue2093_printf_version_matches_the_literal_file() {
    if !Path::new("/bin/printf").is_file() {
        return;
    }
    let _serial = serial();
    let dir = temp_dir("ux-printf");
    let mut session = unix_session(&dir);
    let direct = direct_spawn("/bin/printf", &["--version"]).expect("spawn /bin/printf");

    let out = dir.join("printf-version.out");
    let rc = capture_out(&mut session, "/bin/printf --version", &out);
    // Pre-fix the face answered the builtin printf usage error (rc 2).
    assert_eq!(rc, direct.1);
    assert_eq!(read_trim(&out), direct.0.trim());
}

#[cfg(unix)]
#[test]
fn issue2093_pwd_runs_the_literal_file_physical_default() {
    if !Path::new("/bin/pwd").is_file() {
        return;
    }
    let _serial = serial();
    let dir = temp_dir("ux-pwd");
    // Directories come from the test, not from the shell: the session
    // PATH stays hermetic and `cd` (a builtin) walks the link.
    std::fs::create_dir_all(dir.join("sub/real")).expect("real dir");
    #[cfg(unix)]
    std::os::unix::fs::symlink("real", dir.join("sub/link")).expect("symlink");
    let mut session = unix_session(&dir);
    // Through a symlinked tail: the logical $PWD keeps the link path
    // while the literal pwd file answers getcwd (physical) — the exact
    // divergence family 3 of the issue.
    let logical = dir.join("pwd-logical.out");
    let physical = dir.join("pwd-physical.out");
    let script = format!(
        "cd sub/link; printf '%s\\n' \"$PWD\" > '{}'; /bin/pwd > '{}'",
        shell_path(&logical),
        shell_path(&physical)
    );
    let outcome = session.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);
    let logical = read_trim(&logical);
    let physical = read_trim(&physical);
    let real = std::fs::canonicalize(dir.join("sub/real")).expect("canonicalize");
    assert!(
        logical.ends_with("sub/link"),
        "sanity: the logical PWD keeps the link path ({logical})"
    );
    assert_eq!(
        physical,
        real.display().to_string(),
        "the literal pwd file answers the physical directory"
    );
}
