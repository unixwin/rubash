//! Issue rubash#1318: a failing standalone assignment (readonly target)
//! inside a `{ }` group or a function body must abandon the rest of the
//! reader's current list (GNU subst.c bash_variable_assignment_error ->
//! exp_jump_to_top_level(DISCARD)); the next line still runs.
//!
//! Expected outputs verified against GNU bash 5.3.20.

use std::io::Write;
use std::process::Command;

fn run_script(source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-1318-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("case.sh");
    let mut file = std::fs::File::create(&path).expect("write script");
    file.write_all(source.as_bytes()).expect("write script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn readonly_assignment_in_group_abandons_line() {
    let (stdout, stderr, code) =
        run_script("readonly z\necho H1; { z=1; }; echo \"rc=$?\"; echo AFTER\n");
    assert_eq!(stdout, "H1\n");
    assert!(stderr.contains("z: readonly variable"), "{stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn readonly_assignment_in_function_abandons_line() {
    let (stdout, stderr, code) =
        run_script("readonly z\necho H1; f() { z=1; }; f; echo \"rc=$?\"; echo AFTER\n");
    assert_eq!(stdout, "H1\n");
    assert!(stderr.contains("z: readonly variable"), "{stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn readonly_assignment_in_if_condition_abandons_line() {
    let (stdout, _, code) =
        run_script("readonly z\necho H1; if z=1; then echo A; else echo B; fi; echo AFTER\n");
    assert_eq!(stdout, "H1\n");
    assert_eq!(code, Some(1));
}

#[test]
fn readonly_assignment_next_line_still_runs() {
    let (stdout, _, code) = run_script("readonly z\n{ z=1; echo SKIP; }\necho \"rc=$? AFTER\"\n");
    assert_eq!(stdout, "rc=1 AFTER\n");
    assert_eq!(code, Some(0));
}

#[test]
fn readonly_assignment_in_subshell_is_contained() {
    let (stdout, _, code) = run_script("readonly z\necho H1; ( z=1 ); echo \"sub rc=$? AFTER\"\n");
    assert_eq!(stdout, "H1\nsub rc=1 AFTER\n");
    assert_eq!(code, Some(0));
}
