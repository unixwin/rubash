//! niubash#660 — `set +o history` must stop the session's history stream:
//! commands typed after it neither enter the list nor reach $HISTFILE at
//! exit, and a startup-file opt-out must survive the interactive history
//! initialization. A later `set -o history` re-arms recording and resumes
//! the session from $HISTFILE (builtins/set.def:456-472 bash_set_history).
//!
//! Expectations verified against GNU bash 5.3.20:
//! - rcfile `set +o history` + interactive session: no $HISTFILE file is
//!   created at all (bashhist.c:642 records only under remember_on_history;
//!   shell.c:1008-1009 gates maybe_save_shell_history on it).
//! - interactive init must not re-arm the option (GNU shell.c:806-811, the
//!   post-startup load site, never touches it; the default is applied once
//!   at init_interactive, shell.c:1835-1837, before the startup files).
//! - `set -o history` re-arm reloads $HISTFILE when
//!   history_lines_this_session == 0 (set.def:459-461), with NO
//!   already-loaded guard: after an rcfile opt-out the prompt re-arm lists
//!   the seeded file twice (startup load + re-arm load).
//! - the load itself is skipped when startup lines exist (shell.c:810-811).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash660-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// The bounded wait itself is the shared helper in tests/common
/// (rubash#2197): this binary's private copy carried the pre-#2056
/// orphan-pipe hang shape and was deleted in favor of it.

struct Session {
    stdout: String,
    stderr: String,
    home: PathBuf,
}

/// Drive one piped-`-i` interactive session with its own scratch HOME and
/// optional rcfile (the repository's pipe-driven interactive harness).
/// The rcfile text, when given, is written next to the scratch HOME and
/// handed to the shell via `--rcfile`.
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
    let outcome = wait_bounded(&mut child, Duration::from_secs(30));
    assert!(
        !outcome.timed_out,
        "child {} did not exit within the bound; partial stderr: {}",
        outcome.pid,
        String::from_utf8_lossy(&outcome.stderr)
    );
    let (out, err) = (outcome.stdout, outcome.stderr);
    Session {
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        home,
    }
}

fn history_file(home: &Path) -> Option<String> {
    std::fs::read_to_string(home.join(".bash_history")).ok()
}

/// The issue's seed, on the engine's own interactive path: commands typed
/// after `set +o history` are neither listed nor saved; `set -o history`
/// re-arms recording and the exit save carries only the recorded lines.
#[test]
fn set_off_stops_recording_and_exit_save() {
    let session = run_interactive(
        "off-core",
        "echo before-off\nset +o history\necho histoff_state_$?\nhistoff_marker\nset -o history\necho resumed-line\nexit\n",
        None,
        Some("seed-a\n"),
    );
    let saved = history_file(&session.home).unwrap_or_default();
    assert!(
        !saved.contains("histoff_state_") && !saved.contains("histoff_marker"),
        "lines typed while history is off must not reach $HISTFILE; file:\n{saved}"
    );
    assert!(
        saved.contains("before-off") && saved.contains("seed-a"),
        "lines recorded while history was on stay in the file; file:\n{saved}"
    );
    assert!(
        saved.contains("resumed-line"),
        "recording resumes after `set -o history`; file:\n{saved}"
    );
}

/// The rcfile opt-out must survive the interactive history
/// initialization: GNU creates no history file at all (probe: `bash
/// --rcfile rc -i` with `set +o history` in rc, two commands on stdin —
/// no .bash_history appears). rubash used to re-arm the option at the
/// post-startup load site, so the session recorded and saved anyway.
#[test]
fn rcfile_opt_out_is_honored() {
    let session = run_interactive(
        "rc-off",
        "echo cmd-a\necho cmd-b\nexit\n",
        Some("set +o history\n"),
        None,
    );
    assert!(
        history_file(&session.home).is_none(),
        "an opted-out session must not create $HISTFILE; stdout:\n{}\nstderr:\n{}\nfile: {:?}",
        session.stdout,
        session.stderr,
        history_file(&session.home)
    );
    assert!(
        !session.stderr.contains("rc.rc"),
        "sanity: the rcfile must have been read; stderr:\n{}",
        session.stderr
    );
}

/// builtins/set.def:459-461 bash_set_history FLAG_ON arm: `set -o history`
/// at the prompt reloads $HISTFILE when nothing has been recorded this
/// session, and GNU has NO already-loaded guard — after an rcfile opt-out
/// (startup load already ran, lines_this_session == 0) the re-arm reads the
/// file a second time, so the listing shows the seeded lines twice
/// (verified 5.3.20: `1 seeded-1 / 2 seeded-2 / 3 seeded-1 / 4 seeded-2 /
/// 5 history`). The exit save appends only the session's own lines.
#[test]
fn rearm_reloads_histfile_after_rcfile_opt_out() {
    let session = run_interactive(
        "rearm",
        "set -o history\nhistory\nexit\n",
        Some("set +o history\n"),
        Some("seeded-1\nseeded-2\n"),
    );
    for expected in [
        "1  seeded-1",
        "2  seeded-2",
        "3  seeded-1",
        "4  seeded-2",
        "5  history",
    ] {
        assert!(
            session.stdout.contains(expected),
            "re-arm must reload $HISTFILE onto the list (GNU lists the seeded file twice); expected {expected:?} in:\n{}",
            session.stdout
        );
    }
    let saved = history_file(&session.home).unwrap_or_default();
    // GNU probe: the exit save appends only the session lines after the
    // seeded file — the re-arm duplicate lives in the in-memory list only.
    assert_eq!(
        saved, "seeded-1\nseeded-2\nhistory\nexit\n",
        "exit save appends the session lines after the seeds; file:\n{saved}"
    );
}

/// GNU probe (5.3.20, non-interactive script): a first `set -o history`
/// reads $HISTFILE into the list (bash_set_history → load_history), the
/// `history` line itself is entry 3, and the exit save appends the session
/// lines after the seeded ones.
#[test]
fn script_first_set_o_history_loads_histfile() {
    let dir = tempdir("script-load");
    let script = dir.join("p660.sh");
    std::fs::write(
        &script,
        "HISTFILE=\"$PWD/h660.txt\"\nprintf \"l1\\nl2\\n\" > \"$HISTFILE\"\nset -o history\nhistory\necho after\n",
    )
    .expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&script).current_dir(&dir).env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("HOME", dir.join("home"));
    std::fs::create_dir_all(dir.join("home")).expect("create home");
    let out = cmd.output().expect("run rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains("1  l1") && stdout.contains("2  l2") && stdout.contains("3  history"),
        "`set -o history` must load $HISTFILE (GNU 5.3.20 lists l1/l2/history); got:\n{stdout}"
    );
    let saved = std::fs::read_to_string(dir.join("h660.txt")).unwrap_or_default();
    assert_eq!(
        saved, "l1\nl2\nhistory\necho after\n",
        "exit save appends the session lines; file:\n{saved}"
    );
}

/// shell.c:810-811: when the startup (or earlier) lines exist, a later
/// `set -o history` must NOT reload — GNU probe p660_gate2: after
/// `echo mid-line` was recorded, the re-applied option lists the session
/// lines once, with no second copy of the file.
#[test]
fn script_reload_is_gated_on_empty_session_lines() {
    let dir = tempdir("script-gate");
    let script = dir.join("p660gate.sh");
    std::fs::write(
        &script,
        "HISTFILE=\"$PWD/g2.txt\"\nprintf \"l1\\nl2\\n\" > \"$HISTFILE\"\nset -o history\necho mid-line\nset +o history\nset -o history\nhistory\n",
    )
    .expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&script).current_dir(&dir).env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
    cmd.env("HOME", dir.join("home"));
    std::fs::create_dir_all(dir.join("home")).expect("create home");
    let out = cmd.output().expect("run rubash");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    for line in [
        "1  l1",
        "2  l2",
        "3  echo mid-line",
        "4  set +o history",
        "5  history",
    ] {
        assert!(
            stdout.contains(line),
            "GNU 5.3.20 lists {line:?} exactly once (no reload after session lines); got:\n{stdout}"
        );
    }
    assert!(
        !stdout.contains("6  "),
        "no sixth entry — the gated reload must not double the file; got:\n{stdout}"
    );
}
