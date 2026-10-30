//! niubash#661 — `HISTFILE` is an ordinary (exportable) variable: every
//! history read and write after an assignment targets the live value, and
//! the engine's own default must not override the startup files' decision.
//!
//! Expectations verified against GNU bash 5.3.20:
//! - an in-pane `export HISTFILE=<path>` redirects the exit save: the
//!   redirect file receives the session's lines, the previous file stays
//!   byte-identical (bashhist.c:495 maybe_save_shell_history reads
//!   `get_string_value("HISTFILE")` at save time).
//! - an rcfile `unset HISTFILE` leaves the session with no history file:
//!   the default (variables.c:654-661 `if (remember_on_history)
//!   set_if_not("HISTFILE", bash_default_histfile())`) applies once, at
//!   shell_initialize (shell.c:580), BEFORE the startup files (shell.c:722)
//!   — the load site (bashhist.c:336) reads `get_string_value("HISTFILE")`
//!   at load time and there is no post-startup re-default. rubash used to
//!   re-default HISTFILE at the post-startup load site, resurrecting the
//!   default after an rcfile unset so the session loaded and saved the
//!   default file anyway.
//! - an rcfile HISTFILE assignment is honored for both the startup load
//!   and the exit save.
//! - the bare `history -w` form writes the live HISTFILE (history.def
//!   resolves the filename from the variable per call).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash661-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> (Vec<u8>, Vec<u8>) {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    panic!("child did not exit within {timeout:?}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        use std::io::Read;
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        use std::io::Read;
        let _ = se.read_to_end(&mut err);
    }
    (out, err)
}

struct Session {
    stdout: String,
    stderr: String,
    home: PathBuf,
    dir: PathBuf,
}

/// Drive one piped-`-i` interactive session with its own scratch HOME and
/// optional rcfile text (written next to the scratch HOME).
fn run_interactive(
    tag: &str,
    input: &str,
    rcfile_text: Option<&str>,
    seed_history: Option<&str>,
) -> Session {
    let dir = tempdir(tag);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create home");
    let rcfile = rcfile_text.map(|text| {
        let rc = dir.join("rc.rc");
        std::fs::write(&rc, text).expect("write rc");
        rc
    });
    if let Some(seed) = seed_history {
        std::fs::write(home.join(".bash_history"), seed).expect("seed history");
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-i").current_dir(&dir).env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("TERM", "xterm");
    cmd.env("HOME", &home);
    if let Some(rc) = &rcfile {
        cmd.arg("--rcfile").arg(rc);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    std::io::Write::write_all(child.stdin.as_mut().expect("stdin pipe"), input.as_bytes())
        .expect("write stdin");
    drop(child.stdin.take());
    let (out, err) = wait_bounded(&mut child, Duration::from_secs(30));
    Session {
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        home,
        dir,
    }
}

fn read_if_exists(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The issue's core shape: after `export HISTFILE=<path>` in the pane, the
/// exit save lands in the redirect file and the default file is untouched.
#[test]
fn export_histfile_redirects_the_exit_save() {
    let session = run_interactive(
        "export-redirect",
        "history | head -1\nexport HISTFILE=\"$PWD/histfile-redirect.txt\"\necho redirected-marker\nexit\n",
        None,
        Some("seed-one\nseed-two\n"),
    );
    let redirect = read_if_exists(&session.dir.join("histfile-redirect.txt")).unwrap_or_default();
    assert!(
        redirect.contains("redirected-marker"),
        "the redirect file must receive the session lines; file: {redirect:?}"
    );
    assert!(
        !redirect.contains("seed-one"),
        "the redirect file is not the old file; file: {redirect:?}"
    );
    let default_file = read_if_exists(&session.home.join(".bash_history"));
    assert_eq!(
        default_file.as_deref(),
        Some("seed-one\nseed-two\n"),
        "the pre-redirect file must stay byte-identical; file: {default_file:?}"
    );
    assert!(
        session.stdout.contains("1  seed-one"),
        "the startup load read the default file; stdout:\n{}",
        session.stdout
    );
}

/// The engine must not resurrect its default after the startup files:
/// GNU's default applies once at shell_initialize (pre-startup), so an
/// rcfile `unset HISTFILE` session creates no history file at all.
#[test]
fn rcfile_unset_histfile_is_final() {
    let session = run_interactive(
        "rc-unset",
        "echo cmd-a\necho cmd-b\nexit\n",
        Some("unset HISTFILE\n"),
        None,
    );
    assert!(
        read_if_exists(&session.home.join(".bash_history")).is_none(),
        "an rc-unset HISTFILE session must not create the default file; stdout:\n{}\nstderr:\n{}",
        session.stdout,
        session.stderr
    );
}

/// GNU probe: an rcfile HISTFILE assignment is honored for the exit save
/// (the default file never appears).
#[test]
fn rcfile_histfile_assignment_is_honored() {
    let session = run_interactive(
        "rc-custom",
        "echo cmd-a\necho cmd-b\nexit\n",
        Some("HISTFILE=\"$PWD/custom.rc1\"\n"),
        None,
    );
    let custom = read_if_exists(&session.dir.join("custom.rc1")).unwrap_or_default();
    assert!(
        custom.contains("cmd-a") && custom.contains("cmd-b"),
        "the assigned HISTFILE receives the session lines; file: {custom:?}"
    );
    assert!(
        read_if_exists(&session.home.join(".bash_history")).is_none(),
        "the default file must not appear; stdout:\n{}",
        session.stdout
    );
}

/// The bare `history -w` form writes the live HISTFILE (history.def
/// resolves the operand from the variable at call time) — GNU 5.3.20
/// probe: after a mid-script reassignment, `history -w` creates the new
/// file and the original name never appears.
#[test]
fn bare_history_w_follows_live_histfile() {
    let dir = tempdir("bare-w");
    let script = dir.join("p661.sh");
    std::fs::write(
        &script,
        "HISTFILE=\"$PWD/h661d.txt\"\nset -o history\necho one\nHISTFILE=\"$PWD/h661r.txt\"\necho two\nhistory -w\necho done\n",
    )
    .expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&script).current_dir(&dir).env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("create home");
    cmd.env("HOME", &home);
    let out = cmd.output().expect("run rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("done"),
        "script ran to completion; stdout:\n{stdout}"
    );
    assert!(
        !dir.join("h661d.txt").exists(),
        "the pre-assignment HISTFILE is never written"
    );
    let written = read_if_exists(&dir.join("h661r.txt")).unwrap_or_default();
    for expected in ["echo one", "echo two", "history -w"] {
        assert!(
            written.contains(expected),
            "the live HISTFILE receives the -w write; expected {expected:?} in:\n{written}"
        );
    }
}
