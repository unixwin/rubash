//! rubash#1204 regressions: `select ... done < file` / `done <&N` ignored
//! the compound's fd-0 input binding and read the process's inherited
//! stdin instead (instant EOF, status 1). GNU do_redirections binds fd 0
//! before the select loop runs and select_query's read_builtin consumes
//! that descriptor (read.def:294); the loop now serves record-wise reads
//! off the fd table whenever fd 0 is bound away from the inherited stdin.
//!
//! Expectations verified byte-for-byte against WSL GNU Bash 5.3.20.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1204-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    std::fs::write(dir.join("in.txt"), "2\n").expect("write in.txt");
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
fn select_reads_fd0_file_redirect() {
    let (stdout, _stderr, code) = rubash_file(
        "select c in aa bb cc; do echo \"c=[$c] REPLY=[$REPLY]\"; break; done < in.txt\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[bb] REPLY=[2]\nrc=0\n");
}

#[test]
fn select_reads_fd0_dup_redirect() {
    let (stdout, _stderr, code) = rubash_file(
        "exec 3< in.txt\nselect c in aa bb cc; do echo \"c=[$c]\"; break; done <&3\necho \"rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "c=[bb]\nrc=0\n");
}

#[test]
fn select_keeps_eof_failure_without_input() {
    // Closed fd 0 still reports EOF -> status 1 like GNU.
    let (stdout, _stderr, code) =
        rubash_file("select c in aa bb; do echo \"c=[$c]\"; break; done <&-\necho \"rc=$?\"\n");
    assert_eq!(code, Some(0));
    // GNU select_query writes the EOF newline to stdout, then the read
    // failure sets status 1.
    assert_eq!(stdout, "\nrc=1\n");
}
