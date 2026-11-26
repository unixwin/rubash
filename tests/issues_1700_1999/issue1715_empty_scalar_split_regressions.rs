//! rubash#1715 regressions: word splitting an empty scalar under a
//! non-whitespace IFS manufactured one empty field; GNU yields zero
//! fields (subst.c:12377 word_split of an empty expansion result is an
//! empty list, whatever IFS holds).
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1715-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1715_empty_scalar_for_loop_zero_iterations() {
    let (stdout, _, code) =
        rubash_file("IFS=':'\nv=''\nc=0; for x in $v; do c=$((c+1)); done\necho \"n=$c\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=0\n");
}

#[test]
fn issue1715_unset_scalar_for_loop_zero_iterations() {
    let (stdout, _, code) =
        rubash_file("IFS=':'\nc=0; for x in $never_set_1715; do c=$((c+1)); done\necho \"n=$c\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=0\n");
}

#[test]
fn issue1715_empty_scalar_set_positional_zero() {
    let (stdout, _, code) = rubash_file("IFS=':'\nv=''\nset -- $v\necho \"n=$#\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=0\n");
}

#[test]
fn issue1715_empty_scalar_argument_consumption_zero() {
    let (stdout, _, code) = rubash_file("IFS=','\nv=''\nf() { echo \"argc=$#\"; }\nf $v\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "argc=0\n");
}

#[test]
fn issue1715_empty_scalar_slash_ifs_zero() {
    let (stdout, _, code) =
        rubash_file("IFS='/'\nv=''\nc=0; for x in $v; do c=$((c+1)); done\necho \"n=$c\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=0\n");
}

#[test]
fn issue1715_delimiter_only_scalar_still_one_empty_field() {
    // Control: the fix must not over-drop. A delimiter-only value still
    // splits into exactly one (empty) field under GNU.
    let (stdout, _, code) = rubash_file(
        "IFS=':'\nv=':'\nc=0; for x in $v; do c=$((c+1)); printf '<%s>' \"$x\"; done\necho \" n=$c\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "<> n=1\n");
}
