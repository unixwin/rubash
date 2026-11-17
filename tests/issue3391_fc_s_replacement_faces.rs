//! rubash#3391 — `fc -s` two faces (PR #3390 Owed; review-3390 §5).
//!
//! Face 1 — `strsub` empty-result quirk: when a `pat=` substitution eats
//! the WHOLE command, GNU returns the ORIGINAL string, not "".
//! GNU anchor: `stringlib.c:136-171` strsub — `temp` stays NULL when the
//! pattern consumes every character and the replacement is empty, so
//! strsub returns `savestring(string)`. `fc -s true=` on `true` therefore
//! executes `true` (stderr `true`, history keeps `true`); a partial eat
//! (`echo xtruex` -> `echo xx`) works the same on both sides.
//!
//! Face 2 — `fc -s` appends instead of replacing in script mode: GNU's
//! `fc_replhist` (`builtins/fc.def:766-780`) deletes the last history
//! entry UNCONDITIONALLY for a non-empty command (early return only on an
//! empty command) and then `maybe_add_history`s the new command. Rubash
//! gated the pop on `shell.last_line_added`, which the script driver
//! resets to false after each line (`src/script_driver.rs:2301`,
//! history-off branch), so in script mode the pop never fired and the
//! substituted command was APPENDED (`echo xtruex` + `echo xx`).
//! The same unconditional delete is what GNU does interactively when
//! HISTIGNORE filters the `fc` line itself: the previous real command is
//! deleted and the new one takes its place (PTY probe vs GNU 5.3.20,
//! clean HISTFILE, `HISTIGNORE="fc *"`).
//!
//! NOTE on the two face-2 cells: the script-mode cell
//! (`fc_s_script_mode_replaces_last_entry`) passes on the current base
//! too — the #3390 merge's script_driver.rs change made the script
//! driver record the `fc` line itself, so the pop gate fires there now
//! and the old script-mode divergence is closed. The cell stays as a
//! regression guard for the script shape. The fail-first proof for face 2
//! is the interactive cell
//! (`fc_s_histignore_filtered_line_still_replaces_last_entry`): with
//! HISTIGNORE filtering the `fc` line, base appends (3 entries) while
//! GNU and head replace (2 entries).
//!
//! Expectations below were verified byte-for-byte against GNU bash 5.3.20
//! (script files, `~/workspace/rubash-audit/build-gnu/bash-5.3/bash`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3391-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_rubash(script: &str, dir: &PathBuf) -> (String, String, i32) {
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn fc_s_empty_replacement_of_whole_command_returns_original() {
    // F3: `fc -s true=` on `true` executes `true`, not "".
    let dir = tempdir("whole-eaten");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'true'\nfc -s true=; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "fc rc=0\n    1  true\n", "stderr: {stderr}");
    assert_eq!(stderr, "true\n");
}

#[test]
fn fc_s_script_mode_replaces_last_entry() {
    // F4: in script mode the substituted command REPLACES the last entry;
    // history shows a single `echo xx`, not `echo xtruex` + `echo xx`.
    let dir = tempdir("script-replace");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'echo xtruex'\nfc -s true=; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "xx\nfc rc=0\n    1  echo xx\n", "stderr: {stderr}");
    assert_eq!(stderr, "echo xx\n");
}

#[test]
fn fc_s_empty_replacement_keeps_prior_entries() {
    // F6: after the quirk, `true` replaces the last entry only; the
    // earlier `echo seed-line` entry survives with its number.
    let dir = tempdir("prior-entries");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'echo seed-line'\nhistory -s 'true'\nfc -s true=; echo \"fc rc=$?\"\nfc -l\n",
        &dir,
    );
    assert_eq!(
        stdout, "fc rc=0\n1\t echo seed-line\n2\t true\n",
        "stderr: {stderr}"
    );
    assert_eq!(stderr, "true\n");
}

#[test]
fn fc_s_nonempty_replacement_of_whole_command() {
    // Control: a non-empty replacement that eats the whole command works
    // (`true` -> `false`, rc follows the re-executed command).
    let dir = tempdir("nonempty-rep");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'true'\nfc -s true=false; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "fc rc=1\n    1  false\n", "stderr: {stderr}");
    assert_eq!(stderr, "false\n");
}

#[test]
fn fc_e_dash_shares_reexec_replacement() {
    // `fc -e -` is the same execute form as `fc -s` (fc.def:243-246),
    // so it replaces the last entry too.
    let dir = tempdir("e-dash");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'echo xtruex'\nfc -e - true=; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "xx\nfc rc=0\n    1  echo xx\n", "stderr: {stderr}");
    assert_eq!(stderr, "echo xx\n");
}

#[test]
fn fc_s_chained_subs_empty_result_returns_original() {
    // Chained subs apply the quirk per strsub call (fc_dosubs, fc.def):
    // `truex` -x-> `true` -true=-> `true` (whole string eaten, original
    // returned), not "".
    let dir = tempdir("chained");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s 'truex'\nfc -s x= true=; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "fc rc=0\n    1  true\n", "stderr: {stderr}");
    assert_eq!(stderr, "true\n");
}

fn run_rubash_interactive(input: &str, dir: &PathBuf) -> (String, String, i32) {
    // Interactive rubash with piped stdin (no PTY needed): the HISTIGNORE
    // filter applies at read time, so the `fc` line itself is never
    // recorded — the shape in which face 2 diverges on base.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--norc", "-i"])
        .env("HISTIGNORE", "fc *")
        .env("HISTFILE", dir.join("hist"))
        .env("HOME", dir)
        .env("TERM", "dumb")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(input.as_bytes())
                .expect("write stdin");
            child.wait_with_output()
        })
        .expect("run rubash -i");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn fc_s_histignore_filtered_line_still_replaces_last_entry() {
    // Face 2, fail-first shape: interactively, HISTIGNORE="fc *" filters
    // the `fc` line itself out of the history, so on base the
    // `last_line_added` pop gate never fires and the substituted command
    // is APPENDED (history shows `echo xtruex` + `echo xx`). GNU's
    // fc_replhist deletes the last entry unconditionally (fc.def:766-780),
    // so the substituted command REPLACES it. The script-mode cell above
    // cannot express this shape (the script driver records the `fc` line
    // itself, so the gate fires there on base too); this interactive cell
    // is the fail-first proof for face 2. Verified against GNU 5.3.20:
    // base lists 3 entries, GNU and head list 2.
    let dir = tempdir("histignore-interactive");
    let (stdout, stderr, _) =
        run_rubash_interactive("history -s 'echo xtruex'\nfc -s true=\nhistory\n", &dir);
    assert!(
        stdout.contains("    1  echo xx\n    2  history\n"),
        "history block must show the replacement only; stdout: {stdout}"
    );
    assert!(stderr.contains("echo xx\n"), "stderr: {stderr}");
}

#[test]
fn fc_s_empty_command_stays_a_noop() {
    // Control (F5): an empty substituted command echoes one blank line
    // and pops/records nothing — fc_replhist's early return (fc.def:770).
    let dir = tempdir("empty-cmd");
    let (stdout, stderr, _) = run_rubash(
        "history -c\nhistory -s ''\nfc -s; echo \"fc rc=$?\"\nhistory\n",
        &dir,
    );
    assert_eq!(stdout, "fc rc=0\n    1  \n", "stderr: {stderr}");
    assert_eq!(stderr, "\n");
}
