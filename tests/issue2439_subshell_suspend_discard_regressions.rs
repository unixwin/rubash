//! Issue 2439: `suspend foo` inside a `( )` subshell under `-c`
//! reported status 2 for the region and then poisoned the whole
//! shell's exit code to 1.
//!
//! GNU builtins/suspend.def suspend_builtin hands a non-option
//! operand to no_args (builtins/common.c): the diagnostic,
//! set_exit_status (EX_BADUSAGE 2), and a jump_to_top_level
//! (DISCARD) that abandons the rest of the current list. The throw
//! dies at the forked region's own top level (execute_cmd.c
//! execute_in_subshell), and the status the parent sees depends on
//! the framing the region's execution hangs from: a `-c` process —
//! run_one_command's parse_and_execute framing (shell.c:1477) —
//! folds it to EXECUTION_FAILURE 1, exactly like the
//! command-substitution body frame (builtins/evalstring.c:423-426,
//! rubash#1541), while a script-file/stdin reader framing keeps the
//! pre-set 2. rubash folded only the comsub frame, so under `-c`
//! the region kept 2 and the still-armed pending reached
//! execute_ast's depth-0 tail conversion, which converted the
//! entire `-c` run to ExitCode(1) after the parent list had already
//! continued.
//!
//! Oracle: GNU bash 5.3.20 — `bash -c '(suspend foo); echo after
//! rc=$?'` prints `after rc=1` and exits 0; the identical line in a
//! script file prints `after rc=2` and exits 0.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn rubash_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2439-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run(tag: &str, args: &[&str], stdin_text: Option<&str>) -> Run {
    let dir = scratch(tag);
    let mut cmd = Command::new(rubash_bin());
    cmd.args(args)
        .env("HOME", &dir)
        .env_remove("BASH_ENV")
        .env_remove("POSIXLY_CORRECT");
    let run = match stdin_text {
        Some(text) => {
            let mut child = cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn shell");
            child
                .stdin
                .as_mut()
                .expect("stdin pipe")
                .write_all(text.as_bytes())
                .expect("write stdin");
            let output = child.wait_with_output().expect("wait for shell");
            Run {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                code: output.status.code(),
            }
        }
        None => {
            let output = cmd
                .stdin(Stdio::null())
                .output()
                .expect("run shell");
            Run {
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                code: output.status.code(),
            }
        }
    };
    let _ = fs::remove_dir_all(&dir);
    run
}

#[test]
fn issue2439_dash_c_subshell_suspend_folds_to_one_and_parent_continues() {
    let r = run("c", &["-c", "(suspend foo); echo after rc=$?"], None);
    assert_eq!(r.stdout, "after rc=1\n");
    assert!(r.stderr.contains("suspend: too many arguments"));
    assert_eq!(r.code, Some(0));

    // The DISCARD abandons the rest of the REGION's list, not the
    // parent's: INNER never prints, the parent echo does.
    let r = run(
        "c-inner",
        &["-c", "(suspend foo; echo INNER); echo after rc=$?"],
        None,
    );
    assert_eq!(r.stdout, "after rc=1\n");
    assert_eq!(r.code, Some(0));

    // Function nesting does not change the framing.
    let r = run(
        "c-fn",
        &["-c", "f(){ (suspend foo); echo fnrc=$?; }; f"],
        None,
    );
    assert_eq!(r.stdout, "fnrc=1\n");
    assert_eq!(r.code, Some(0));
}

#[test]
fn issue2439_reader_framings_keep_status_two() {
    // Script-file framing: the region keeps the pre-set EX_BADUSAGE.
    let dir = scratch("script");
    let script = dir.join("t.sh");
    fs::write(&script, "(suspend foo); echo after rc=$?\n").expect("write script");
    let r = run("script-run", &[script.to_str().unwrap()], None);
    assert_eq!(r.stdout, "after rc=2\n");
    assert_eq!(r.code, Some(0));
    let _ = fs::remove_dir_all(&dir);

    // Stdin (script) framing: same.
    let r = run("stdin", &[], Some("(suspend foo); echo after rc=$?\n"));
    assert_eq!(r.stdout, "after rc=2\n");
    assert_eq!(r.code, Some(0));
}

#[test]
fn issue2439_controls() {
    // A bad OPTION is a plain EX_USAGE return, not the DISCARD
    // class: status 2 and the parent list continues normally.
    let r = run("opt", &["-c", "(suspend -x); echo after rc=$?"], None);
    assert_eq!(r.stdout, "after rc=2\n");
    assert!(r.stderr.contains("suspend: -x: invalid option"));
    assert!(r.stderr.contains("suspend: usage: suspend [-f]"));
    assert_eq!(r.code, Some(0));

    // Top-level `-c`: the DISCARD abandons the whole string and the
    // shell exits 1 (the pre-existing rubash#705 shape).
    let r = run("toplevel", &["-c", "suspend foo; echo after"], None);
    assert_eq!(r.stdout, "");
    assert!(r.stderr.contains("suspend: too many arguments"));
    assert_eq!(r.code, Some(1));
}
