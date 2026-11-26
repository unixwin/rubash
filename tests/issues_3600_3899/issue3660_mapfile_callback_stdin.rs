//! rubash#3660 regressions: `mapfile -C callback` ran the callback
//! with the shell's ambient stdin instead of the stream mapfile
//! itself was reading. GNU runs the callback in-process on the
//! builtin's own input descriptor (builtins/mapfile.def:108-131
//! run_callback -> evalstring), syncing the fd offset to the
//! consumed boundary before each invocation (mapfile.def:195-197
//! zsyncfd; lib/sh/zread.c:294), so a callback `read` consumes the
//! very next record of mapfile's stream, interleaved with the
//! driver's records. The seed is `seed_r123b.sh` from the issue
//! (diff-hunt r123 shape mf57, two stable passes); the remaining
//! checks extend the same sharing to the other input faces (plain
//! file, exec-bound fd, -u fd, heredoc, inherited pipe, non-newline
//! delimiter) and to the fd-alias mixup the seed's control case
//! exposed (a `< /dev/fd/N` target must resolve through the fd
//! table, not be opened as a filesystem path).
//!
//! Expectations verified byte-for-byte against GNU bash 5.3.20.

use std::io::Write;
use std::process::{Command, Stdio};

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue3660-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("input.txt"), "F1\nF2\nF3\nF4\nF5\n").expect("write input.txt");
    dir
}

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("file");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("dashc");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash -c");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_file_stdin(script: &str, stdin_text: &str) -> (String, String, Option<i32>) {
    let dir = scratch("pipe");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash file");
    child
        .stdin
        .as_mut()
        .expect("child stdin")
        .write_all(stdin_text.as_bytes())
        .expect("write child stdin");
    let output = child.wait_with_output().expect("wait rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---- the issue seed (r123 mf57), run as -c units like the seed ----

#[test]
fn seed_callback_reads_next_line_3() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; echo \"cbread=$x\"; }; mapfile -t -C cb -c 1 a < <(printf \"L1\\nL2\\nL3\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L2\ncbread=\ndeclare -a a=([0]=\"L1\" [1]=\"L3\")\n"
    );
}

#[test]
fn seed_callback_reads_next_line_5() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; echo \"cbread=$x\"; }; mapfile -t -C cb -c 1 a < <(printf \"L1\\nL2\\nL3\\nL4\\nL5\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L2\ncbread=L4\ncbread=\ndeclare -a a=([0]=\"L1\" [1]=\"L3\" [2]=\"L5\")\n"
    );
}

#[test]
fn seed_callback_quantum_two() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; echo \"cbread=$x\"; }; mapfile -t -C cb -c 2 a < <(printf \"L1\\nL2\\nL3\\nL4\\nL5\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L3\ncbread=\ndeclare -a a=([0]=\"L1\" [1]=\"L2\" [2]=\"L4\" [3]=\"L5\")\n"
    );
}

#[test]
fn seed_callback_reads_two_lines() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; read -r y; echo \"cbread=$x/$y\"; }; mapfile -t -C cb -c 1 a < <(printf \"L1\\nL2\\nL3\\nL4\\nL5\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L2/L3\ncbread=L5/\ndeclare -a a=([0]=\"L1\" [1]=\"L4\")\n"
    );
}

#[test]
fn seed_callback_reads_separate_fd_control() {
    let (stdout, _, code) = rubash_c(
        "exec 3< <(printf \"r1\\nr2\\nr3\"); cb(){ read -r -u 3 x; echo \"cbread=$x\"; }; mapfile -t -C cb -c 1 a < <(printf \"a\\nb\\nc\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=r1\ncbread=r2\ncbread=r3\ndeclare -a a=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

#[test]
fn seed_callback_no_read_control() {
    let (stdout, _, code) = rubash_c(
        "cb(){ echo \"cb:$1\"; }; mapfile -t -C cb -c 1 a < <(printf \"L1\\nL2\\nL3\\n\"); declare -p a",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cb:0\ncb:1\ncb:2\ndeclare -a a=([0]=\"L1\" [1]=\"L2\" [2]=\"L3\")\n"
    );
}

// ---- the same sharing on the other input faces (script files) ----

#[test]
fn script_file_procsub_interleaves() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 a < <(printf \"L1\\nL2\\nL3\\n\")\ndeclare -p a\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L2\ncbread=\ndeclare -a a=([0]=\"L1\" [1]=\"L3\")\n"
    );
}

#[test]
fn plain_file_redirect_interleaves() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 a < input.txt\ndeclare -p a\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=F2\ncbread=F4\ncbread=\ndeclare -a a=([0]=\"F1\" [1]=\"F3\" [2]=\"F5\")\n"
    );
}

#[test]
fn exec_bound_fd0_interleaves() {
    let (stdout, _, code) = rubash_file(
        "exec 0< input.txt\ncb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 b\ndeclare -p b\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=F2\ncbread=F4\ncbread=\ndeclare -a b=([0]=\"F1\" [1]=\"F3\" [2]=\"F5\")\n"
    );
}

#[test]
fn numbered_fd_callback_reads_same_fd() {
    let (stdout, _, code) = rubash_file(
        "exec 3< input.txt\ncb(){ read -r -u 3 x; echo \"cbread=$x\"; }\nmapfile -t -u 3 -C cb -c 1 d\ndeclare -p d\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=F2\ncbread=F4\ncbread=\ndeclare -a d=([0]=\"F1\" [1]=\"F3\" [2]=\"F5\")\n"
    );
}

#[test]
fn callback_plain_read_ignores_numbered_fd() {
    // GNU: with `-u 3`, a callback `read` without -u reads fd 0 (here
    // /dev/null), never the mapfile descriptor; the array still gets
    // every record of fd 3.
    let (stdout, _, code) = rubash_file(
        "exec 3< input.txt\ncb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -u 3 -C cb -c 1 c\ndeclare -p c\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=\ncbread=\ncbread=\ncbread=\ncbread=\ndeclare -a c=([0]=\"F1\" [1]=\"F2\" [2]=\"F3\" [3]=\"F4\" [4]=\"F5\")\n"
    );
}

#[test]
fn heredoc_interleaves() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 e <<EOF\nH1\nH2\nH3\nEOF\ndeclare -p e\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=H2\ncbread=\ndeclare -a e=([0]=\"H1\" [1]=\"H3\")\n"
    );
}

#[test]
fn heredoc_interleaves_over_ambient_fd0_file() {
    // The heredoc is the mapfile command's fd-0 binding for its
    // duration; an ambient `exec 0<file` binding must not govern the
    // callback's reads instead.
    let (stdout, _, code) = rubash_file(
        "exec 0< input.txt\ncb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 e <<EOF\nH1\nH2\nH3\nEOF\ndeclare -p e\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=H2\ncbread=\ndeclare -a e=([0]=\"H1\" [1]=\"H3\")\n"
    );
}

#[test]
fn callback_read_u0_with_file_redirect() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r -u 0 x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 f < input.txt\ndeclare -p f\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=F2\ncbread=F4\ncbread=\ndeclare -a f=([0]=\"F1\" [1]=\"F3\" [2]=\"F5\")\n"
    );
}

#[test]
fn callback_read_u0_over_heredoc() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r -u 0 x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 c <<EOF\nN1\nN2\nN3\nEOF\ndeclare -p c\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=N2\ncbread=\ndeclare -a c=([0]=\"N1\" [1]=\"N3\")\n"
    );
}

#[test]
fn non_newline_delimiter_callback_reads_raw_stream() {
    // GNU: the driver's records split on -d '|', but the callback's
    // `read` consumes the raw stream with read's own newline
    // semantics.
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -d '|' -C cb -c 1 g < <(printf 'A|B|C|D|E|')\ndeclare -p g\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "cbread=B|C|D|E|\ndeclare -a g=([0]=\"A\")\n");
}

#[test]
fn inherited_pipe_stdin_interleaves() {
    let (stdout, _, code) = rubash_file_stdin(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -C cb -c 1 p\ndeclare -p p\nread -r tail; echo \"after=$tail\"\n",
        "S1\nS2\nS3\nS4\nS5\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=S2\ncbread=S4\ncbread=\ndeclare -a p=([0]=\"S1\" [1]=\"S3\" [2]=\"S5\")\nafter=\n"
    );
}

#[test]
fn external_callback_drains_stream() {
    let (stdout, _, code) = rubash_file(
        "cb(){ cat; }\nmapfile -t -C cb -c 1 b < <(printf \"M1\\nM2\\nM3\\nM4\\n\")\ndeclare -p b\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "M2\nM3\nM4\ndeclare -a b=([0]=\"M1\")\n");
}

#[test]
fn bounded_mapfile_callback_interleaves() {
    let (stdout, _, code) = rubash_file(
        "cb(){ read -r x; echo \"cbread=$x\"; }\nmapfile -t -n 2 -C cb -c 1 g < input.txt\ndeclare -p g\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=F2\ncbread=F4\ndeclare -a g=([0]=\"F1\" [1]=\"F3\")\n"
    );
}

// ---- the fd-alias mixup: a procsub redirect must not read a
// ---- different descriptor's stream that happens to share the
// ---- process (seed case 5's shape, callback and slurp faces)

#[test]
fn dual_procsub_slurp_reads_own_stream() {
    let (stdout, _, code) = rubash_file(
        "exec 3< <(printf \"r1\\nr2\\n\")\nmapfile -t a < <(printf \"a1\\na2\\n\")\ndeclare -p a\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "declare -a a=([0]=\"a1\" [1]=\"a2\")\n");
}

#[test]
fn dual_procsub_no_read_callback_reads_own_stream() {
    let (stdout, _, code) = rubash_file(
        "exec 3< <(printf \"r1\\nr2\\nr3\\n\")\ncb(){ echo \"cb:$1\"; }\nmapfile -t -C cb -c 1 b < <(printf \"a\\nb\\nc\\n\")\ndeclare -p b\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cb:0\ncb:1\ncb:2\ndeclare -a b=([0]=\"a\" [1]=\"b\" [2]=\"c\")\n"
    );
}

// ---- the staged FUNCTION_STDIN overlap (rubash#2779 × rubash#3660):
// ---- a pipeline stage's mirror is BOTH the record source (the
// ---- StagedText pull) and the callback's share — the callback `read`
// ---- carves the mirror at the published consumed boundary, its
// ---- consumption folds back into the puller instead of being
// ---- re-served as a record, and a bounded -n read still leaves the
// ---- unconsumed tail for the stage's next reader.

#[test]
fn staged_pipeline_bounded_callback_interleaves() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; echo \"cbread=$x\"; }; printf 'L1\\nL2\\nL3\\nL4\\nL5\\n' | { mapfile -t -C cb -c 2 -n 2 a; declare -p a; echo \"tail:[$(cat)]\"; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L3\ndeclare -a a=([0]=\"L1\" [1]=\"L2\")\ntail:[L4\nL5]\n"
    );
}

#[test]
fn staged_pipeline_callback_interleaves() {
    let (stdout, _, code) = rubash_c(
        "cb(){ read -r x; echo \"cbread=$x\"; }; printf 'L1\\nL2\\nL3\\nL4\\nL5\\n' | { mapfile -t -C cb -c 2 a; declare -p a; }",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "cbread=L3\ncbread=\ndeclare -a a=([0]=\"L1\" [1]=\"L2\" [2]=\"L4\" [3]=\"L5\")\n"
    );
}
