//! rubash#1174 — :? on ${arr[@]} errors even when array is set and
//! non-empty.
//!
//! GNU subst.c parameter_brace_expand: `${a[@]:?}` / `${a[*]:?}` on a
//! set, non-empty array expands normally; the error fires only for a
//! genuinely empty expansion (`${e[@]:?def}` on an empty array reports
//! `e[@]: def` and aborts the non-interactive shell), and the
//! one-empty-element forms distinguish `?` from `:?` exactly as
//! scalars do.
//!
//! Expectations verified against GNU bash 5.3.20 with SCRIPT FILES
//! (the fatal-expansion exit status is 1 in file mode).

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1174-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const MAIN_FORMS: &str = "a=(alpha beta); echo \"star:[${a[*]:?}]\"; echo rc=$?\n\
declare -A m=([k]=v); echo \"assoc:[${m[@]:?}]\"; echo rc=$?\n\
e=(); echo \"empty:[${e[@]:?def}]\"; echo rc=$?\n";

#[test]
fn colon_question_accepts_nonempty_arrays() {
    let (stdout, stderr, code) = rubash_file("main", MAIN_FORMS);
    // The two non-empty forms print and succeed; the empty form then
    // aborts the shell with the :? diagnostic.
    assert_eq!(stdout, "star:[alpha beta]\nrc=0\nassoc:[v]\nrc=0\n");
    assert!(stderr.contains("e[@]: def"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}

#[test]
fn colon_question_empty_element_forms() {
    let (stdout, stderr, code) = rubash_file(
        "elem",
        "a=(\"\")\n\
         echo \"one-empty-star:[${a[*]:?e1}]\"; echo rc=$?\n\
         echo \"one-empty-nocolon:[${a[*]?e2}]\"; echo rc=$?\n\
         b=(\"\" \"\")\n\
         echo \"two-empty-star:[${b[*]:?e3}]\"; echo rc=$?\n\
         unset u\n\
         echo \"unset:[${u[@]:?e4}]\"; echo rc=$?\n",
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("a[*]: e1"), "stderr: {stderr}");
    assert_eq!(code, Some(1));
}
