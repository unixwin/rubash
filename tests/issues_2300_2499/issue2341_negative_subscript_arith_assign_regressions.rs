//! rubash#2341 — an out-of-range negative subscript in an arithmetic
//! assignment must diagnose at the bind. GNU's read of the lvalue
//! reports the bare name (arrayfunc.c get_array_value INDEX_ERROR ->
//! err_badarraysub(var->name)) and the bind then reports the full
//! lvalue token as written (assign_array_element_internal ->
//! err_badarraysub(name), arrayfunc.c:425-435) and stores nothing.
//! Plain `=` performs no read, so it emits only the bind line.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2341-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("t.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("t.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn plain_assign_reports_bind_line_only() {
    let script = "a=(1 2 3); (( a[-4] = 9 )); echo \"rc=$? arr=[${a[@]}]\"\n";
    let (stdout, stderr, code) = rubash_file("eq", script);
    assert_eq!(stdout, "rc=0 arr=[1 2 3]\n");
    assert_eq!(stderr, "t.sh: line 1: a[-4]: bad array subscript\n");
    assert_eq!(code, Some(0));
}

#[test]
fn compound_assign_reports_read_line_then_bind_line() {
    let script = "\
a=(1 2 3); (( a[-4] += 9 )); echo \"rc=$? arr=[${a[@]}]\"
a=(1 2 3); (( a[-4]++ )); echo \"rc=$? arr=[${a[@]}]\"
a=(1 2 3); (( a[0-4] = 9 )); echo \"rc=$? arr=[${a[@]}]\"
a=(1 2 3); i=-4; (( a[i] += 9 )); echo \"rc=$? arr=[${a[@]}]\"
a=(1 2 3); let \"a[-4] = 9\"; echo \"rc=$? arr=[${a[@]}]\"
";
    let (stdout, stderr, code) = rubash_file("compound", script);
    assert_eq!(
        stdout,
        "rc=0 arr=[1 2 3]\n\
         rc=1 arr=[1 2 3]\n\
         rc=0 arr=[1 2 3]\n\
         rc=0 arr=[1 2 3]\n\
         rc=0 arr=[1 2 3]\n"
    );
    assert_eq!(
        stderr,
        "t.sh: line 1: a: bad array subscript\n\
         t.sh: line 1: a[-4]: bad array subscript\n\
         t.sh: line 2: a: bad array subscript\n\
         t.sh: line 2: a[-4]: bad array subscript\n\
         t.sh: line 3: a[0-4]: bad array subscript\n\
         t.sh: line 4: a: bad array subscript\n\
         t.sh: line 4: a[i]: bad array subscript\n\
         t.sh: line 5: a[-4]: bad array subscript\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn unset_array_negative_assign_reports_and_creates_nothing() {
    let script = "unset b; (( b[-1] = 9 )); echo \"rc=$?\"; declare -p b; echo \"drc=$?\"\n";
    let (stdout, stderr, code) = rubash_file("unset", script);
    assert_eq!(stdout, "rc=0\ndrc=1\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: b[-1]: bad array subscript\n\
         t.sh: line 1: declare: b: not found\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn in_range_negative_subscript_assign_is_silent() {
    let script = "\
a=(1 2 3); (( a[-3] = 9 )); echo \"rc=$? arr=[${a[@]}]\"
a=(1 2 3); (( a[-1] += 9 )); echo \"rc=$? arr=[${a[@]}]\"
";
    let (stdout, stderr, code) = rubash_file("inrange", script);
    assert_eq!(stdout, "rc=0 arr=[9 2 3]\nrc=0 arr=[1 2 12]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
