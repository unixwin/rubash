//! Issue rubash#797 (eco-harvest verdict, run 39078222286): after sourcing a
//! script that embeds bash-preexec (asset: justjake/Dotfiles
//! `zsh/rc.d/28_wezterm.zsh`, whose `__wezterm_install_bash_prexec` inlines
//! rcaloras/bash-preexec), the harness saw the interactive session stop
//! executing input (`hang:enter1:ECO>`).
//!
//! Triage against the verdict machinery (eco-test.py: the ConPTY leg drives
//! niu.exe, whose engine is the pinned rubash) and a full differential
//! against WSL GNU bash 5.3.0:
//!
//! - The DEBUG-trap ASSEMBLY path is at parity: the multi-line
//!   `PROMPT_COMMAND` install string
//!   `$'trap_string="$(trap -p DEBUG)"\ntrap - DEBUG\ninstall'` runs all
//!   three lines at the next prompt render, `trap -p DEBUG` readback has
//!   GNU's `trap -- '...' DEBUG` single-quoted form (bash-preexec's
//!   `sed "s/[^']*'\(.*\)'[^']*/\1/"` prior-trap extraction depends on it),
//!   `trap - DEBUG` clears, the re-installed trap fires before each typed
//!   command and at each prompt render, and the trap body's command
//!   substitutions do NOT re-enter the trap (verified byte-identical
//!   transcripts, asset-level and minimal-form, before this file's pins).
//! - The one REAL divergence on that surface: GNU brackets every
//!   PROMPT_COMMAND run with a `$_` save/restore — parse.y:3017
//!   `execute_variable_command` does `last_lastarg = save_lastarg()` before
//!   the PC's parse_and_execute and :3022-3023 `bind_lastarg(last_lastarg)`
//!   on every normal return (save_parser_state itself carries no lastarg —
//!   parse.y:7248/:7250 cover only the exit value and PIPESTATUS). The
//!   engine left `$_` on the PC's OWN last command word, so the DEBUG trap
//!   installed by bash-preexec handed `bp_install`/`bp_precmd_invoke` —
//!   prompt-machinery internals — to `preexec "$_"` (wezterm's
//!   `__wezterm_user_vars_preexec` then reports the wrong WEZTERM_PROG).
//!   The exit-jump path skips the restore (the longjmp runs past :3022),
//!   the same skip rule the $?/PIPESTATUS restore already follows.
//! - The verdict's HANG itself is NOT an engine control-flow divergence:
//!   the asset's precmd/preexec design spawns ~10-18 Windows processes per
//!   prompt render + per command (`id`, `hostname`, `base64`, `sed`,
//!   `history | sed` …); under eco-test's `--workers 8` on a 2-core
//!   windows-latest runner the enter1 phase (one prompt render + one
//!   preexec + one command) exceeds the 6s ENTER_TIMEOUT while alive.
//!   Same class as the #451 reclassifications (upstream-heavy-external):
//!   niu.exe on rubash master runs the full asset healthy (enter phases
//!   ~0.3s each locally), and the piped-`-i` transcripts below are
//!   byte-identical with GNU 5.3.0.
//!
//! The minimal form here keeps the load-bearing constructs of the asset's
//! embedded bash-preexec (the `$'...\n...'` install string appended onto
//! PROMPT_COMMAND, the `trap -p DEBUG` capture, `trap - DEBUG`, the
//! re-trap with `"$_"`, precmd/precmd dispatch) — not the file itself.
//!
//! The source-scoping pins also cover the IGNORE edge the CR review caught
//! on the first cut: `trap '' DEBUG` (IGNORE, trap.c:900 sets SIG_TRAPPED)
//! is distinguished from `trap - DEBUG` by TRAP TABLE MEMBERSHIP
//! (`__RUBASH_TRAPS`), never by action text — an entry ignore is not
//! snapshotted (TRAP_STRING is NULL for it, trap.h:57-59) and a file's
//! ignore blocks the entry action's return (trap_if_untrapped).
//!
//! GNU anchors (bash.git @b4608166, Bash-5.3 patch 15): parse.y:3011-3025
//! execute_variable_command; eval.c:318-330; execute_cmd.c:4177-4193
//! save_lastarg/bind_lastarg (bind_lastarg(NULL) binds the null string).
#![cfg(windows)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// The minimal bash-preexec form: what the asset's install path does,
/// compressed to the constructs the pins depend on.
const BP_MIN: &str = r#"bp_preexec_invoke() {
  echo "BP-EXEC:$1"
}
bp_precmd_invoke() {
  echo "BP-PRECMD"
}
bp_install() {
  trap 'bp_preexec_invoke "$_"' DEBUG
  PROMPT_COMMAND='bp_precmd_invoke'
}
PROMPT_COMMAND=${PROMPT_COMMAND}$'bp_trap_string="$(trap -p DEBUG)"\ntrap - DEBUG\nbp_install'
"#;

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-it797-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("bp_min.sh"), BP_MIN).expect("write fixture");
    dir
}

struct Session {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// The piped-`-i` probe shape (the issue433/issue451 harness contract):
/// lines on stdin, stdin closed after them, GNU-styled probes.
fn run_interactive(dir: &PathBuf, lines: &str) -> Session {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-i")
        .arg("--norc")
        .current_dir(dir)
        .env_remove("PS1")
        .env_remove("PROMPT_COMMAND")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn rubash -i");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(lines.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Session {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn stdout_lines(session: &Session) -> Vec<&str> {
    session
        .stdout
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect()
}

/// The whole minimal-assembly transcript, pinned line-for-line against WSL
/// GNU bash 5.3.0 (`bash --norc -i -s` on the same driver): the install
/// string runs at the first post-source prompt render (capture -> untrap ->
/// install), the re-installed DEBUG trap fires before every typed command
/// AND before the PC dispatch at every later prompt render, and every
/// `"$_"` the trap sees is the last USER-command word — never the prompt
/// machinery's own command words (`bp_install` / `bp_precmd_invoke`).
#[test]
fn preexec_install_transcript_matches_gnu() {
    let dir = fixture_dir("full");
    let session = run_interactive(
        &dir,
        "source ./bp_min.sh\ntrap -p DEBUG\ncommand echo ECOSRC\ncommand echo ECOTWO\n",
    );
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    assert_eq!(
        stdout_lines(&session),
        vec![
            "BP-EXEC:DEBUG",
            // The fire before `trap -p DEBUG`: `$_` is still the source
            // line's last argument, NOT `bp_install` from inside the
            // install string (parse.y:3017/:3022 bracket).
            "BP-EXEC:./bp_min.sh",
            // bash-preexec's prior-trap extraction parses this form.
            "trap -- 'bp_preexec_invoke \"$_\"' DEBUG",
            "BP-EXEC:DEBUG",
            "BP-PRECMD",
            "BP-EXEC:DEBUG",
            "ECOSRC",
            "BP-EXEC:ECOSRC",
            "BP-PRECMD",
            "BP-EXEC:ECOSRC",
            "ECOTWO",
            "BP-EXEC:ECOTWO",
            "BP-PRECMD",
        ],
        "stderr: {}",
        session.stderr
    );
    // The session survived the source and kept executing (the headline:
    // `hang:enter1` had the REPL stop right here), then exited cleanly.
    assert_eq!(session.stderr.matches("bash-5.3#").count(), 5);
}

/// The one-line form of the fix pin: after the bash-preexec install
/// PROMPT_COMMAND renders, the DEBUG trap's `"$_"` must be the last typed
/// word, not the PC's last command word.
#[test]
fn debug_trap_sees_user_lastarg_not_pc_internals() {
    let dir = fixture_dir("lastarg");
    let session = run_interactive(
        &dir,
        "source ./bp_min.sh\necho USERWORD\ncommand echo AFTER\n",
    );
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    let lines = stdout_lines(&session);
    // The fire before `command echo AFTER` sees USERWORD (the last
    // argument of `echo USERWORD`), never `bp_install`/`bp_precmd_invoke`.
    assert!(
        lines.contains(&"BP-EXEC:USERWORD"),
        "the DEBUG trap must see the user's last arg: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| *l == "BP-EXEC:bp_install"),
        "PC-internal command words must not leak into $_: {lines:?}"
    );
    assert!(
        !lines.iter().any(|l| *l == "BP-EXEC:bp_precmd_invoke"),
        "PC-internal dispatch words must not leak into $_: {lines:?}"
    );
    assert!(
        lines.contains(&"AFTER"),
        "the session must keep executing: {lines:?}"
    );
}

/// bash-preexec installs a DEBUG trap whose body runs command
/// substitutions (`this_command=$(... history 1 | sed ...)`). The trap must
/// not re-enter for the commands inside its own body — GNU fires it once
/// per simple command (verified GNU 5.3.0: one marker per command, two
/// commands -> two markers).
#[test]
fn debug_trap_does_not_recurse_into_its_own_commands() {
    let dir = fixture_dir("norecurse");
    let script = dir.join("norec.sh");
    std::fs::write(
        &script,
        "trap 'echo MARK >> traplog; echo INNER $(echo x)' DEBUG\necho a\necho b\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("source ./norec.sh"))
        .current_dir(&dir)
        .env_remove("PS1")
        .output()
        .expect("run rubash -c");
    assert_eq!(output.status.code(), Some(0));
    let log = std::fs::read_to_string(dir.join("traplog")).unwrap();
    assert_eq!(
        log.lines().count(),
        2,
        "exactly one DEBUG-trap fire per simple command: {log:?}"
    );
}

/// The exit-jump path skips the `$_` restore, like it skips the $?/PIPESTATUS
/// restore (parse.y:3022 sits after the longjmp's reach): PROMPT_COMMAND
/// with `exit N` ends the session with N — pre-existing contract (#433) —
/// and must not panic on the un-restored snapshot.
#[test]
fn pc_exit_jump_skips_underscore_restore() {
    let dir = fixture_dir("exitjump");
    let session = run_interactive(
        &dir,
        "echo BEFORE\nPROMPT_COMMAND='exit 5'\necho AFTER-PC\n",
    );
    assert_eq!(session.code, Some(5), "stderr: {}", session.stderr);
    assert_eq!(session.stdout.trim_end(), "BEFORE");
}

/// An unset pre-PC `$_` comes back unset-of-value as the null string
/// (bind_lastarg(NULL) binds "", execute_cmd.c:4192-4193) — the PC path
/// must not leave the engine's `$_` binding in a half-cleared state.
#[test]
fn pc_with_unset_underscore_restores_null_binding() {
    let dir = fixture_dir("nullbind");
    let session = run_interactive(&dir, "unset _\nPROMPT_COMMAND='true'\necho END=$_\n");
    assert_eq!(session.code, Some(0), "stderr: {}", session.stderr);
    // GNU 5.3.0 piped-`-i`: END= (the null string).
    assert!(
        session.stdout.contains("END="),
        "stdout: {:?}",
        session.stdout
    );
}

// The ignore edge (CR review R1/R2/R3 on the first cut): GNU distinguishes
// `trap '' DEBUG` (IGNORE — trapped, TRAP_STRING NULL, trap.c:900 still sets
// SIG_TRAPPED) from `trap - DEBUG` (untrapped) by TRAP TABLE MEMBERSHIP, and
// source.def:211's `TRAP_STRING(DEBUG_TRAP)` snapshot skips an ignored entry
// trap whole (trap.h:57-59). The save/restore must key on membership
// (get_trap_action's Some("") = live ignore), never on action text.

/// Writes a script-mode fixture pair and runs the driver file, returning
/// (exit code, stdout).
fn run_script(
    dir: &PathBuf,
    driver: &str,
    body: &str,
    sourced: &str,
    sourced_body: &str,
) -> (Option<i32>, String) {
    std::fs::write(dir.join(sourced), sourced_body).expect("write sourced fixture");
    std::fs::write(dir.join(driver), body).expect("write driver");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(format!("./{driver}"))
        .current_dir(dir)
        .env_remove("PS1")
        .env_remove("PROMPT_COMMAND")
        .output()
        .expect("run rubash");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

fn nonempty_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect()
}

/// R1: a real entry trap plus an in-file `trap '' DEBUG` — the IGNORE is
/// SIG_TRAPPED (trap.c:900), so the entry action must NOT come back: the
/// post-source `trap -p DEBUG` reads `trap -- '' DEBUG` and no ORIG fires.
/// WSL GNU bash 5.3.0: `ORIG / in1 / in2 / after / trap -- '' DEBUG`.
#[test]
fn file_ignore_survives_source_entry_action_stays_retired() {
    let dir = fixture_dir("ig-r1");
    let (code, stdout) = run_script(
        &dir,
        "r1drv.sh",
        "trap 'echo ORIG' DEBUG\nsource ./r1.sh\necho after\ntrap -p DEBUG\n",
        "r1.sh",
        "trap '' DEBUG\necho in1\necho in2\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        nonempty_lines(&stdout),
        vec!["ORIG", "in1", "in2", "after", "trap -- '' DEBUG"],
        "the ignore must survive and the entry action must stay retired: {stdout:?}"
    );
}

/// R2: an entry IGNORE is not snapshotted or removed at all (TRAP_STRING is
/// NULL for it, trap.h:57-59) — `trap -p DEBUG` INSIDE the file still shows
/// `trap -- '' DEBUG`. WSL GNU bash 5.3.0: identical, twice.
#[test]
fn entry_ignore_stays_visible_during_source() {
    let dir = fixture_dir("ig-r2");
    let (code, stdout) = run_script(
        &dir,
        "r2drv.sh",
        "trap '' DEBUG\nsource ./r2.sh\necho after\ntrap -p DEBUG\n",
        "r2.sh",
        "trap -p DEBUG\necho in1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        nonempty_lines(&stdout),
        vec!["trap -- '' DEBUG", "in1", "after", "trap -- '' DEBUG"],
        "the entry ignore must stay live through the file: {stdout:?}"
    );
}

/// R3: entry IGNORE untrapped by the file (`trap - DEBUG`) — nothing was
/// snapshotted, so nothing comes back: no trap after the source. WSL GNU
/// bash 5.3.0: `in1 / after` and NO `trap -p` line.
#[test]
fn entry_ignore_untrapped_by_file_leaves_no_trap() {
    let dir = fixture_dir("ig-r3");
    let (code, stdout) = run_script(
        &dir,
        "r3drv.sh",
        "trap '' DEBUG\nsource ./r3.sh\necho after\ntrap -p DEBUG\n",
        "r3.sh",
        "trap - DEBUG\necho in1\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        nonempty_lines(&stdout),
        vec!["in1", "after"],
        "a file-untrapped entry ignore must leave no trap: {stdout:?}"
    );
}
