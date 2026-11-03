//! rubash#2682 (deep-audit A1-F09): a heredoc attached to a command in
//! an if/elif/while/until CONDITION had its gathered body dumped on the
//! compound body's first command instead of the condition's redirect,
//! so the condition command read its "body" from the process stdin at
//! execution time — a hang on a live stdin, an empty read on /dev/null.
//! GNU gathers heredoc bodies at the header line's newline into the
//! redirect nodes the single parse already built (parse.y:3648-3654
//! gather_here_documents; make_cmd.c make_here_document).
use std::{
    env,
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run_script(source: &str) -> (String, String, i32) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(source)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn issue2682_if_condition_heredoc_body_is_delivered() {
    let (stdout, stderr, code) =
        run_script("if cat <<END; then\nhello\nEND\necho in-then\nfi\necho after");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello\nin-then\nafter\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2682_while_condition_heredoc_body_is_delivered() {
    let (stdout, stderr, code) =
        run_script("while cat <<END; do\nhello\nEND\necho iter\nbreak\ndone\necho after");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello\niter\nafter\n");
}

#[test]
fn issue2682_until_condition_heredoc_body_is_delivered() {
    // The condition's exit status must be decided by the delivered
    // body: grep finds `hello' only if the heredoc reached it, so the
    // until body runs zero times. With the body lost (the pre-fix
    // behavior) grep read the process stdin instead, failed, and the
    // body ran.
    let (stdout, stderr, code) = run_script(
        "until grep -q hello <<END; do\nhello\nEND\necho in-body\nbreak\ndone\necho after",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "after\n");

    let (stdout, stderr, code) = run_script(
        "until grep -q zzz <<END; do\nhello\nEND\necho in-body\nbreak\ndone\necho after",
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "in-body\nafter\n");
}

#[test]
fn issue2682_elif_condition_heredoc_body_is_delivered() {
    let (stdout, stderr, code) =
        run_script("if false; then\necho a\nelif cat <<END; then\nhello\nEND\necho elif-body\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello\nelif-body\n");
}

#[test]
fn issue2682_negated_and_chained_and_piped_conditions() {
    let (stdout, stderr, code) =
        run_script("if ! cat <<END; then\nhello\nEND\necho in-then\nelse\necho in-else\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "hello\nin-else\n");

    let (stdout, stderr, code) =
        run_script("if cat <<A && cat <<B; then\nbodyA\nA\nbodyB\nB\necho chained\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "bodyA\nbodyB\nchained\n");

    let (stdout, stderr, code) =
        run_script("if cat <<END | grep -c hell; then\nhello\nEND\necho piped\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "1\npiped\n");
}

#[test]
fn issue2682_condition_and_body_heredocs_on_the_same_if() {
    let (stdout, stderr, code) =
        run_script("if cat <<C; then\ncondbody\nC\ncat <<B\nbodyline\nB\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "condbody\nbodyline\n");
}

#[test]
fn issue2682_same_line_outer_and_condition_heredocs_pair_in_stream_order() {
    let (stdout, stderr, code) =
        run_script("cat <<X; if cat <<Y; then\nbodyX\nX\nbodyY\nY\necho in-then\nfi");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "bodyX\nbodyY\nin-then\n");

    let (stdout, stderr, code) =
        run_script("cat <<X; while cat <<Y; do\nbodyX\nX\nbodyY\nY\necho in-loop\nbreak\ndone");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "bodyX\nbodyY\nin-loop\n");
}

#[test]
fn issue2682_if_condition_heredoc_never_reads_the_process_stdin() {
    // The hang face, bounded: stdin is a pipe whose write end stays
    // OPEN but never delivers a byte. Before the fix the condition's
    // `cat` blocked on that read forever.
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("if cat <<END; then\nhello\nEND\necho in-then\nfi\necho after")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rubash -c");
    let _held_open_stdin = child.stdin.take().expect("stdin pipe");
    let mut stdout = child.stdout.take().expect("stdout pipe");
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait().expect("poll child") {
            Some(status) => break Some(status),
            None => {
                assert!(
                    Instant::now() < deadline,
                    "rubash blocked reading the process stdin for a \
                     condition heredoc (issue2682 hang)"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let status = status.expect("child status");
    let mut text = String::new();
    stdout.read_to_string(&mut text).expect("read child stdout");
    assert!(status.success(), "status: {status:?}");
    assert_eq!(text, "hello\nin-then\nafter\n");
}
