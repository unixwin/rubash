//! rubash#1659 — mapfile reads 0 lines (or hangs) from a coproc fd.
//!
//! `mapfile -t arr <&${COPROC[0]}` must block-read the coproc
//! pipe to EOF like GNU mapfile.def's zgetline loop. Rubash's
//! mapfile dispatch only applies output redirects
//! (apply_no_output_builtin_redirects), so its input comes
//! from the string channels (stdin_string_for_command), and
//! that channel's open_fd_read_endpoint had no arm for the
//! coproc pipe read end (FdReadEndpoint::CoprocStdout) —
//! every sibling reader (read_all_bytes,
//! take_buffered_input_line, inherited_async_stdin) already
//! served it, but this helper dropped it to Bad file
//! descriptor. The string channel then failed and mapfile
//! fell through to reading inherited process stdin: a hang
//! on a held-open pipe, or 0 lines on /dev/null.
//!
//! The fix serves CoprocStdout in open_fd_read_endpoint
//! exactly like a file: DuplicateHandle shares the kernel
//! object, so a preceding `read` and a following mapfile
//! see GNU's shared-offset stream (read takes a, mapfile
//! gets b c), not a replay.
//!
//! Expectations verified against GNU bash 5.3.20. Stdin is
//! nulled so a regression blocks on nothing.

use std::process::{Command, Stdio};

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_script(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1659-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let path = dir.join("s.sh");
    std::fs::write(&path, script).expect("seed script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn mapfile_reads_coproc_fd_to_eof() {
    let script = "coproc printf \"a\\nb\\nc\\n\"\nmapfile -t arr <&${COPROC[0]}; echo \"n=${#arr[@]}\"\nprintf \"%s\\n\" \"${arr[@]}\"\nwait $COPROC_PID 2>/dev/null; echo \"wait_rc=$?\"\n";
    let (stdout, _, code) = rubash_script(script);
    assert_eq!(stdout, "n=3\na\nb\nc\nwait_rc=0\n");
    assert_eq!(code, Some(0));
}

#[test]
fn mapfile_u_flag_coproc_fd() {
    let (stdout, _, code) = rubash_c(
        "coproc printf \"a\\nb\\n\"; mapfile -u ${COPROC[0]} -t arr; echo \"n=${#arr[@]} ${arr[0]} ${arr[1]}\"; wait $COPROC_PID",
    );
    assert_eq!(stdout, "n=2 a b\n");
    assert_eq!(code, Some(0));
}

#[test]
fn mapfile_after_read_shares_offset() {
    let (stdout, _, code) = rubash_c(
        "coproc printf \"a\\nb\\nc\\n\"; read -r first <&${COPROC[0]}; mapfile -t rest <&${COPROC[0]}; echo \"first=$first n=${#rest[@]} ${rest[0]} ${rest[1]}\"; wait $COPROC_PID",
    );
    assert_eq!(stdout, "first=a n=2 b c\n");
    assert_eq!(code, Some(0));
}

#[test]
fn mapfile_slow_coproc_blocks_until_output() {
    let (stdout, _, code) = rubash_c(
        "coproc { sleep 1; printf \"x\\ny\\n\"; }; mapfile -t arr <&${COPROC[0]}; echo \"n=${#arr[@]} ${arr[0]} ${arr[1]}\"; wait $COPROC_PID",
    );
    assert_eq!(stdout, "n=2 x y\n");
    assert_eq!(code, Some(0));
}

#[test]
fn mapfile_plain_sources_unchanged() {
    let dir = std::env::temp_dir().join(format!(
        "rubash-1659p-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&dir).expect("create scratch dir");
    let file = dir.join("in.txt").display().to_string();
    std::fs::write(&file, "x\ny\n").expect("seed file");
    let (stdout, _, _) = rubash_c(&format!(
        "mapfile -t arr < {file}; echo \"n=${{#arr[@]}} ${{arr[0]}} ${{arr[1]}}\""
    ));
    assert_eq!(stdout, "n=2 x y\n");
    let (stdout, _, _) =
        rubash_c("mapfile -t arr <<EOF\np\nq\nEOF\necho \"n=${#arr[@]} ${arr[0]} ${arr[1]}\"");
    assert_eq!(stdout, "n=2 p q\n");
    let _ = std::fs::remove_dir_all(&dir);
}
