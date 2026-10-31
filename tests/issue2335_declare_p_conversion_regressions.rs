//! rubash#2335 — `declare -p` is a display-only query: a type flag riding
//! along on the command (`-A` on an indexed array, `-a` on an associative
//! array) must not run the conversion validation. GNU declare_builtin
//! takes the pflag branch (builtins/declare.def:389) before all
//! making-variables machinery, so the variable prints as-is with rc 0.
//!
//! Expectations verified against GNU bash 5.3.20 with script files.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2335-{tag}-{}", std::process::id()));
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
fn declare_p_with_mismatched_array_flag_prints_variable() {
    let script = "\
a=(p q); declare -p a; declare -p -A a; echo rc:$?
declare -A b=([x]=1); declare -p -a b; echo rc:$?
";
    let (stdout, stderr, code) = rubash_file("mismatch", script);
    assert_eq!(
        stdout,
        "declare -a a=([0]=\"p\" [1]=\"q\")\n\
         declare -a a=([0]=\"p\" [1]=\"q\")\n\
         rc:0\n\
         declare -A b=([x]=\"1\" )\n\
         rc:0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn declare_p_with_scalar_type_flags_prints_variable() {
    let script =
        "s=hi; declare -p -a s; declare -p -A s; declare -p -i s; declare -p -r s; echo rc:$?\n";
    let (stdout, stderr, code) = rubash_file("scalar", script);
    assert_eq!(
        stdout,
        "declare -- s=\"hi\"\n\
         declare -- s=\"hi\"\n\
         declare -- s=\"hi\"\n\
         declare -- s=\"hi\"\n\
         rc:0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn declare_without_p_still_rejects_array_conversion() {
    // The conversion machinery itself is untouched: without -p the same
    // flag mismatch remains an error and the variable keeps its value.
    let script = "a=(p q); declare -A a; echo rc:$?; declare -p a\n";
    let (stdout, stderr, code) = rubash_file("convert", script);
    assert_eq!(stdout, "rc:1\ndeclare -a a=([0]=\"p\" [1]=\"q\")\n");
    assert_eq!(
        stderr,
        "t.sh: line 1: declare: a: cannot convert indexed to associative array\n"
    );
    assert_eq!(code, Some(0));
}
