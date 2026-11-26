//! rubash#2557 — a compound `{ }` pipeline element OVER-folds its
//! DISCARD status under `-c`: rubash gives 1, GNU gives 2.
//!
//! GNU law: a pipeline element runs in a forked child; the child's
//! DISCARD is preserved by execute_in_subshell's catch
//! (execute_cmd.c:1821-1825 — a non-EXIT result keeps
//! last_command_exit_value, the pre-set EX_BADUSAGE 2 from
//! subst.c:8164), and the parent reads the child's exit status — the
//! parent's parse_and_execute catch (builtins/evalstring.c:423-450)
//! never sees the jump, so it cannot fold it. The compound-stage
//! harvest deliberately keeps the raw status; the fold leaked INSIDE
//! the element clone's own execution, through the inherited
//! parse_and_execute frame the clone constructor renumbers.
//!
//! Expectations verified against GNU bash 5.3.20 (script files and
//! `-c` strings); the file-mode pins run as script files, the `-c`
//! pins as `-c` strings.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2557-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> String {
    let script = dir.join(name);
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(name)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_c(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const SHAPE: &str = "echo ${never_set_xyz[@]:=dv}";

#[test]
fn issue2557_compound_element_keeps_status_under_c() {
    let stdout = run_c(&format!(
        "{{ {SHAPE}; }} | cat\necho \"ps=${{PIPESTATUS[0]}}\""
    ));
    assert_eq!(stdout, "ps=2\n");
}

#[test]
fn issue2557_compound_element_keeps_status_from_file() {
    let dir = tempdir("file");
    let stdout = run_script(
        &dir,
        "probe.sh",
        &format!("{{ {SHAPE}; }} | cat\necho \"ps=${{PIPESTATUS[0]}}\"\n"),
    );
    assert_eq!(stdout, "ps=2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2557_compound_element_inside_eval_keeps_status() {
    // The eval frame belongs to the parent; the compound element is
    // still a forked child whose status crosses as a wait status.
    let dir = tempdir("eval");
    let stdout = run_script(
        &dir,
        "probe.sh",
        &format!("eval '{{ {SHAPE}; }} | cat; echo \"ps=${{PIPESTATUS[0]}}\"'\n"),
    );
    assert_eq!(stdout, "ps=2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2557_compound_element_last_stage_keeps_status() {
    // `echo seed |` feeds the last stage so no stage ever reads the
    // test process's own stdin.
    let stdout = run_c(&format!("echo seed | {{ {SHAPE}; }}\necho \"rc=$?\""));
    assert_eq!(stdout, "rc=2\n");
}
