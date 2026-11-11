//! rubash#2629 — a pipeline stage's redirect list must be walked
//! ONCE, like GNU's single do_redirections pass per element
//! (redir.c:237, reached from execute_cmd.c:2620 execute_pipeline).
//! Two leaks broke that: (1) the parent-side helper brackets (the
//! reject gate, the output-redirect preflight) run the full applier
//! on the PARENT executor, and the deferred arithmetic writes its
//! target expansions queue used to commit into the parent shell —
//! the element's evaluation landed in the wrong process; (2) the
//! stream-routing walk re-ran the applier after the stage ran, and
//! because the stage body's execute_command entries clear the
//! redirect-target memo, a function stage's `{var}` target word
//! with a side effect — `$((i+=1))` in the filename — evaluated a
//! second time, creating a second file. GNU evaluates it exactly
//! once, inside the element.
//!
//! Expectations verified against GNU bash 5.3.20 with identical
//! script files (fresh process, stdin null): the parent's `i` stays
//! 0 (the element is a forked child) and exactly `1.log` exists.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2629-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &PathBuf, text: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, text).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue2629_builtin_left_stage_expands_once() {
    let dir = tempdir("bl");
    let (stdout, stderr, code) = run_script(
        &dir,
        "i=0\necho hello {v}>$((i+=1)).log | cat >/dev/null\necho \"i=$i\"\nls *.log\n",
    );
    assert_eq!(stdout, "i=0\n1.log\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2629_builtin_right_stage_expands_once() {
    let dir = tempdir("br");
    let (stdout, stderr, code) = run_script(
        &dir,
        "i=0\nprintf x | true {v}>$((i+=1)).log\necho \"i=$i\"\nls *.log\n",
    );
    assert_eq!(stdout, "i=0\n1.log\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2629_external_stage_expands_once() {
    let dir = tempdir("ext");
    let (stdout, stderr, code) = run_script(
        &dir,
        "i=0\nprintf x | cat {v}>$((i+=1)).log >/dev/null\necho \"i=$i\"\nls *.log\n",
    );
    assert_eq!(stdout, "i=0\n1.log\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2629_function_stage_expands_once() {
    let dir = tempdir("fn");
    let (stdout, stderr, code) = run_script(
        &dir,
        "i=0\nfn() { echo in-fn; }\nfn {v}>$((i+=1)).log | cat >/dev/null\necho \"i=$i\"\nls *.log\n",
    );
    assert_eq!(stdout, "i=0\n1.log\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2629_plain_command_controls_expand_once() {
    // Controls (not stages): a plain external command evaluates the
    // target exactly once (one file, whichever side of the fork the
    // evaluation lands on — the plain funnel's parent-side binding
    // is a separate, older architecture question, not this issue's
    // stage-helper-walk scope); a plain builtin evaluates it once
    // in the shell itself (i becomes 1, v persists).
    let dir = tempdir("ctl");
    let (stdout, stderr, code) = run_script(
        &dir,
        "i=0\ncat /dev/null {v}>$((i+=1)).log >/dev/null\nls *.log\nrm -f *.log\ni=0\necho hello {v}>$((i+=1)).log >/dev/null\necho \"blt i=$i v=$v\"\nls *.log\n",
    );
    assert_eq!(stdout, "1.log\nblt i=1 v=10\n1.log\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
