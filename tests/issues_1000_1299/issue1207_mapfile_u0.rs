//! rubash#1207 regressions: `mapfile -u 0` read nothing — the `-u` fd
//! path never consulted fd 0's ordinary transports (pipeline
//! FUNCTION_STDIN mirror, `< file`, herestring). GNU mapfile.def `-u fd`
//! selects the descriptor zgetline reads and `-u 0` names fd 0 itself, so
//! it now takes the same input path as plain `mapfile`.
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1207-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    std::fs::write(dir.join("mf.txt"), "a\nb\n").expect("write mf.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
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
fn mapfile_u0_reads_pipe() {
    let (stdout, _, code) =
        rubash_file("printf 'x\\ny\\n' | { mapfile -t -u 0 arr; echo \"n=${#arr[@]}\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2\n");
}

#[test]
fn mapfile_u0_reads_file_redirect() {
    let (stdout, _, code) = rubash_file("mapfile -t -u 0 arr < mf.txt; echo \"n=${#arr[@]}\"\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=2\n");
}

#[test]
fn mapfile_plain_still_reads_pipe() {
    let (stdout, _, code) =
        rubash_file("printf 'z\\n' | { mapfile -t arr; echo \"n=${#arr[@]}\"; }\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "n=1\n");
}
