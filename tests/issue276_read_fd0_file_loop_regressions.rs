//! niubash#276: EOF on a file bound to fd 0 is final — the FUNCTION_STDIN
//! mirror of the enclosing pipeline stage must not be served as the file's
//! trailing record.
//!
//! GNU builtin_read reads one descriptor (read.def:294): a `while read;
//! done < file` loop ends at the file's EOF. The read used to fall through
//! to the stage's stdin mirror after the file EOF, consuming the next
//! pipeline record inside the loop and leaving the following `read` a false
//! EOF. The script-file shape repros on master; the -c shape binds a
//! different transport, so these tests drive script files.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_script(script: &str, stdin: &[u8]) -> (String, String, Option<i32>) {
    let path = std::env::temp_dir().join(format!(
        "rubash-276-{}-{}.sh",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, script).expect("write script");
    // The scripts below create probe files with relative names (f276*.txt).
    // Run the child in its own sandbox so those never land in the repository
    // root — three such leftovers had been committed by accident.
    let sandbox = std::env::temp_dir().join(format!(
        "rubash-276-dir-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&sandbox).expect("create sandbox");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.current_dir(&sandbox)
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(&sandbox);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The issue's function shape: `f` drains a `<file` loop, the enclosing
/// pipeline stage's `read w3` must still see the pipe's record (WSL
/// 5.2.21 oracle: rc=0, w3=[m]).
#[test]
fn function_file_loop_preserves_stage_stdin() {
    let script = "\
printf 'a\\nb\\nc\\n' > f276.txt
f() { while read q; do :; done < f276.txt; }
printf 'm\\n' | { f; read w3; echo \"rc=$? w3=[$w3]\"; }
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(stdout, "rc=0 w3=[m]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// The group shape: same ownership rule without a function scope.
#[test]
fn group_file_loop_preserves_stage_stdin() {
    let script = "\
printf 'a\\nb\\nc\\n' > f276g.txt
printf 'm\\n' | { while read q; do :; done < f276g.txt; read w3; echo \"rc=$? w3=[$w3]\"; }
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(stdout, "rc=0 w3=[m]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}

/// Guard: the loop itself still consumes exactly the file's records — the
/// mirror record must not appear as a phantom loop iteration.
#[test]
fn file_loop_iterates_only_file_records() {
    let script = "\
printf 'a\\nb\\nc\\n' > f276i.txt
printf 'm\\n' | { while read q; do echo \"row[$q]\"; done < f276i.txt; }
";
    let (stdout, stderr, rc) = run_script(script, b"");
    assert_eq!(stdout, "row[a]\nrow[b]\nrow[c]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
}
