//! Issue rubash#4158 — eval's own INPUT redirections were silently
//! dropped (the output side was fine).
//!
//! GNU runs a builtin under its own do_redirections
//! (execute_cmd.c:5606-5614 execute_builtin_or_function, redir.c
//! do_redirection_internal, RX_UNDOABLE), so eval's `< file`,
//! heredoc, here-string and numbered input redirections bind for
//! the whole parse_and_execute (builtins/evalstring.c) and the
//! reparsed body's commands read eval's descriptors. On pristine,
//! the in-process eval path installed only the output half
//! (apply_command_output_redirects + with_compound_output_redirects
//! inside execute_eval_source): the body's `cat` read an empty
//! stdin, `read` fell through to the ambient stdin (hanging on a
//! live one, EOF + rc 1 on /dev/null), heredoc bodies were lost.
//!
//! The fix binds the eval command's redirections through the
//! ordinary redirect plan (with_command_input_redirects) around
//! the Execute arm of execute_eval, so every exit path of the arm
//! runs under the binding and the restore replays at arm end.
//!
//! Verified against the cloud GNU Bash 5.3.20 oracle via script
//! files (stdout, stderr, exit status, and created-file contents
//! compared byte-for-byte). All runs use a null stdin so the
//! pristine failure mode is EOF, never a hang.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

fn tempdir(tag: &str) -> PathBuf {
    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rubash4158-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `script` as probe.sh in a fresh tempdir with a null stdin;
/// return (stdout, stderr, exit code, contents of out.txt if the
/// run created one, contents of other.txt if the run created one).
fn run_script(
    tag: &str,
    script: &str,
) -> (String, String, Option<i32>, Option<String>, Option<String>) {
    let dir = tempdir(tag);
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code();
    let out_txt = std::fs::read_to_string(dir.join("out.txt")).ok();
    let other_txt = std::fs::read_to_string(dir.join("other.txt")).ok();
    let _ = std::fs::remove_dir_all(&dir);
    (stdout, stderr, code, out_txt, other_txt)
}

#[test]
fn eval_cat_reads_input_file() {
    let (out, err, code, _, _) = run_script(
        "cat-file",
        "printf 'data-line\\n' > in.txt\neval 'cat' < in.txt\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "data-line\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_read_from_input_file() {
    let (out, err, code, _, _) = run_script(
        "read-file",
        "printf 'line-from-file\\nsecond\\n' > in.txt\neval 'read line' < in.txt\necho \"rc=$? got:$line\"\n",
    );
    assert_eq!(out, "rc=0 got:line-from-file\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_read_herestring() {
    let (out, err, code, _, _) = run_script(
        "herestring",
        "eval 'read x' <<< hello\necho \"rc=$? x=$x\"\n",
    );
    assert_eq!(out, "rc=0 x=hello\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_cat_heredoc_attached_to_eval() {
    let (out, err, code, _, _) = run_script(
        "heredoc",
        "eval 'cat' <<RUBASH_EOF\nheredoc-body-line\nRUBASH_EOF\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "heredoc-body-line\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_input_redirects_inside_function() {
    let (out, err, code, _, _) = run_script(
        "in-func",
        "f() { eval 'read x' <<< hello; echo \"in-func rc=$? x=$x\"; }\nf\ng() { eval 'cat' < in.txt; }\nprintf 'func-file-line\\n' > in.txt\ng\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "in-func rc=0 x=hello\nfunc-file-line\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_output_redirect_guard() {
    // Output-side control: this already worked on pristine and
    // must not move.
    let (out, err, code, out_txt, _) = run_script(
        "out-guard",
        "eval 'echo hi' > out.txt\necho \"rc=$?\"\ncat out.txt\n",
    );
    assert_eq!(out, "rc=0\nhi\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
    assert_eq!(out_txt.as_deref(), Some("hi\n"));
}

#[test]
fn eval_input_open_failure_skips_body() {
    // Guard: the dispatch pregate already refused this on
    // pristine; the binding must keep the shape (diagnostic,
    // status 1, body never runs).
    let (out, err, code, _, _) = run_script(
        "open-fail",
        "eval 'echo hi' < /nonexistent-rubash-4158/f\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "rc=1\n");
    assert_eq!(
        err,
        "probe.sh: line 1: /nonexistent-rubash-4158/f: No such file or directory\n"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn eval_body_own_redirect_wins_over_eval_input() {
    let (out, err, code, _, _) = run_script(
        "body-wins",
        "printf 'from-in\\n' > in.txt\nprintf 'from-other\\n' > other.txt\neval 'cat < other.txt' < in.txt\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "from-other\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_numbered_input_redirect() {
    let (out, err, code, _, _) = run_script(
        "numbered",
        "printf 'numbered-line\\n' > in.txt\neval 'cat <&3' 3< in.txt\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "numbered-line\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_explicit_fd0_input_redirect() {
    let (out, err, code, _, _) = run_script(
        "fd0",
        "printf 'zero-line\\n' > in.txt\neval 'read x' 0< in.txt\necho \"rc=$? x=$x\"\n",
    );
    assert_eq!(out, "rc=0 x=zero-line\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_body_commands_share_input_offset() {
    // The binding is a real descriptor: two reads in the body
    // consume successive lines, in the one-line form and in the
    // multi-line (grouped) form alike.
    let (out, err, code, _, _) = run_script(
        "share-offset",
        "printf 'first\\nsecond\\n' > two.txt\neval 'read a; read b' < two.txt\necho \"rc=$? a=$a b=$b\"\n",
    );
    assert_eq!(out, "rc=0 a=first b=second\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));

    let (out, err, code, _, _) = run_script(
        "share-offset-grouped",
        "printf 'first\\nsecond\\n' > two.txt\neval 'read a\nread b' < two.txt\necho \"rc=$? a=$a b=$b\"\n",
    );
    assert_eq!(out, "rc=0 a=first b=second\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_input_and_output_redirects_together() {
    let (out, err, code, out_txt, _) = run_script(
        "both",
        "printf 'both-line\\n' > in.txt\neval 'cat' < in.txt > out.txt\necho \"rc=$?\"\ncat out.txt\n",
    );
    assert_eq!(out, "rc=0\nboth-line\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
    assert_eq!(out_txt.as_deref(), Some("both-line\n"));
}

#[test]
fn eval_command_builtin_input_redirect() {
    let (out, err, code, _, _) = run_script(
        "command-eval",
        "command eval 'read x' <<< hi\necho \"rc=$? x=$x\"\n",
    );
    assert_eq!(out, "rc=0 x=hi\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_second_call_rereads_and_ambient_stdin_restored() {
    // Each eval call binds afresh (the file is read from the
    // start again), and the caller's stdin is untouched by the
    // binding: the trailing `read z` still sees the ambient
    // (null) stdin and hits EOF.
    let (out, err, code, _, _) = run_script(
        "reread",
        "printf 'again-line\\n' > in.txt\neval 'read x' < in.txt\neval 'read y' < in.txt\nread z\necho \"rc=$? x=$x y=$y z=[$z]\"\n",
    );
    assert_eq!(out, "rc=1 x=again-line y=again-line z=[]\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_without_redirects_keeps_enclosing_stdin() {
    // Guard: an eval carrying no redirections of its own must not
    // disturb the enclosing scope's stdin — the loop's stream is
    // shared with the eval body exactly as on pristine.
    let (out, err, code, _, _) = run_script(
        "loop-stdin",
        "printf 'l1\\nl2\\nl3\\nl4\\n' > two.txt\nwhile read l; do eval 'read x'; echo \"l=$l x=$x\"; done < two.txt\necho \"rc=$?\"\n",
    );
    assert_eq!(out, "l=l1 x=l2\nl=l3 x=l4\nrc=0\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_exec_body_redirect_does_not_escape_eval_binding() {
    // GNU: an `exec` inside the eval body performs its open (the
    // file is created) but the binding does not persist past the
    // eval — the eval command's own redirections are undoable and
    // the body's exec lands inside that bracket, so the following
    // command writes to the original stdout.
    let (out, err, code, out_txt, other_txt) = run_script(
        "exec-bracket",
        "eval 'exec > out.txt' > other.txt\necho after2\n",
    );
    assert_eq!(out, "after2\n");
    assert_eq!(err, "");
    assert_eq!(code, Some(0));
    assert_eq!(out_txt.as_deref(), Some(""));
    assert_eq!(other_txt.as_deref(), Some(""));
}
