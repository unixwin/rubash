//! rubash#1713 regressions: `read -a` with a set-but-empty IFS split
//! the line on whitespace as if IFS held its default value. GNU
//! read.def:969 hands the line to list_string (subst.c:3119) with the
//! ambient IFS; an empty IFS disables field splitting entirely, so
//! the whole line is a single field (backslash processing still
//! applies without -r), and an empty line yields no field at all.
//!
//! Expectations verified against GNU bash 5.3.20 (self-compiled oracle).

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1713-{}-{}",
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
fn issue1713_empty_ifs_read_array_whole_line_one_field() {
    let (stdout, _, code) = rubash_file(
        "printf 'a b c d\\n' > in\nIFS='' read -a arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[a b c d]\n");
}

#[test]
fn issue1713_empty_ifs_read_array_global_ifs() {
    let (stdout, _, code) = rubash_file(
        "printf 'a b c d\\n' > in\nIFS=''\nread -a arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[a b c d]\n");
}

#[test]
fn issue1713_empty_ifs_read_array_blanks_only_line() {
    let (stdout, _, code) = rubash_file(
        "printf '   \\n' > in\nIFS='' read -a arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[   ]\n");
}

#[test]
fn issue1713_empty_ifs_read_array_backslash_still_processed() {
    let (stdout, _, code) = rubash_file(
        "printf 'a\\\\ b c\\n' > in\nIFS='' read -a arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[a b c]\n");
}

#[test]
fn issue1713_empty_ifs_read_array_raw_keeps_line_verbatim() {
    let (stdout, _, code) = rubash_file(
        "printf 'a\\\\ b c\\n' > in\nIFS='' read -ra arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n[a\\ b c]\n");
}

#[test]
fn issue1713_empty_ifs_read_array_empty_line_no_fields() {
    let (stdout, _, code) =
        rubash_file("printf '\\n' > in\nIFS='' read -a arr < in\necho \"rc=$? n=${#arr[@]}\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0 n=0\n");
}

#[test]
fn issue1713_unset_ifs_read_array_still_whitespace_splits() {
    // Control: an UNSET IFS keeps the default whitespace split.
    let (stdout, _, code) = rubash_file(
        "printf 'a b c d\\n' > in\nunset IFS\nread -a arr < in\necho \"n=${#arr[@]}\"\nprintf '[%s]' \"${arr[@]}\"; echo\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=4\n[a][b][c][d]\n");
}
