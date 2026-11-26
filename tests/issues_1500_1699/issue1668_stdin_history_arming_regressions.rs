//! rubash#1668 — the reverse residual of #1629: in non-interactive
//! stdin mode, after `set -o history` turns recording on mid-stream,
//! GNU records the lines read afterwards (bashhist.c
//! maybe_add_history under remember_on_history; builtins/set.def
//! bash_set_history loads $HISTFILE at the switch). rubash never
//! installed a session list on this path — the list sprang into
//! existence only when the `history` builtin first ran — so every
//! line between the switch and the first builtin was dropped, and
//! the load-on-set of HISTFILE was skipped the same way.
//!
//! Expectations verified against GNU bash 5.3.20 (piped stdin).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn spawn_with(input: &str, args: &[&str], envs: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    stdin.write_all(input.as_bytes()).expect("write stdin");
    drop(stdin);
    let output = child.wait_with_output().expect("wait rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_stdin(input: &str) -> String {
    spawn_with(input, &[], &[])
}

#[test]
fn lines_after_set_o_history_are_recorded() {
    // The switch line itself is read while the option is still off;
    // the lines after it enter the list. Before the fix the two
    // recorded lines were missing and the listing held only the
    // `history` line itself.
    let stdout = run_stdin("set -o history\necho a\nhistory\n");
    assert_eq!(stdout, "a\n    1  echo a\n    2  history\n");
}

#[test]
fn lines_before_arming_are_not_recorded() {
    let stdout = run_stdin("echo x\nset -o history\necho a\nhistory\n");
    assert_eq!(stdout, "x\na\n    1  echo a\n    2  history\n");
}

#[test]
fn startup_o_history_records_from_the_first_line() {
    let stdout = spawn_with("echo a\nhistory\n", &["-o", "history"], &[]);
    assert_eq!(stdout, "a\n    1  echo a\n    2  history\n");
}

#[test]
fn disarm_line_is_recorded_but_later_lines_are_not() {
    // The `set +o history` line is read while the option is still on,
    // so it is recorded; lines read after it are not — but `history`
    // still runs and lists.
    let stdout = run_stdin("set -o history\necho a\nset +o history\necho b\nhistory\n");
    assert_eq!(stdout, "a\nb\n    1  echo a\n    2  set +o history\n");
}

#[test]
fn unterminated_final_line_is_recorded() {
    // The EOF tail group is read (and recorded) before it executes,
    // exactly like a newline-terminated group.
    let stdout = run_stdin("set -o history\necho a\nhistory");
    assert_eq!(stdout, "a\n    1  echo a\n    2  history\n");
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1668-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn set_o_history_loads_histfile_and_exit_appends_new_lines() {
    // builtins/set.def bash_set_history -> bashhist.c load_history:
    // the switch reads $HISTFILE into the list; at exit only the
    // lines recorded this session are appended (exit_shell /
    // maybe_append_history), the loaded lines are not rewritten.
    let dir = tempdir("loadsave");
    let histfile = dir.join("hist");
    std::fs::write(&histfile, "old-entry\n").expect("seed HISTFILE");
    let histfile_arg = histfile.to_string_lossy().into_owned();
    let stdout = spawn_with(
        "set -o history\necho a\nhistory\n",
        &[],
        &[("HISTFILE", &histfile_arg)],
    );
    assert_eq!(
        stdout,
        "a\n    1  old-entry\n    2  echo a\n    3  history\n"
    );
    let saved = std::fs::read_to_string(&histfile).expect("read HISTFILE back");
    assert_eq!(saved, "old-entry\necho a\nhistory\n");
    let _ = std::fs::remove_dir_all(&dir);
}
