//! rubash#3792 — `cmd <&N` where N is a write-only fd must dup the
//! descriptor the way GNU's dup2 does: the child inherits the write-only
//! fd, its own reads fail EBADF, and the CHILD reports the error with its
//! own diagnostic and exit status. Rubash instead failed the redirect
//! itself (`{fd}: Bad file descriptor`, child never runs), and its
//! emulated `cat` read the underlying file's content as if the fd were
//! readable (the #1963 alias re-open served a dup).
//!
//! GNU owner: redir.c:1193 `dup2 (redir_fd, redirector)` in
//! do_redirection_internal — no access-mode check; the dup succeeds for
//! any open fd and the access-mode failure surfaces later, in the
//! child's own read (EBADF). The redirector gate is only
//! redir.c:1151 (`redir_fd != redirector`).
//!
//! Pipeline members are the same dup in a forked element
//! (execute_cmd.c:2620 execute_pipeline: do_piping first, then the
//! element's own do_redirections), so a member's `<&N` supersedes the
//! pipe — the pipeline pins below cover the cat/head/grep stage arms,
//! the stage-0 position, and a readable-dup control.
//!
//! Expectations verified against GNU bash 5.2.21 on this Linux host
//! (script files, LC_ALL=C) and the issue's GNU probes; the seed is
//! R131-C. The alias spelling stays a fresh open: `cat < /dev/fd/3`
//! on a write-only fd 3 still reads the file (redir.c:725 redir_open,
//! rubash#1963) — that control guards the fix against overreach.
//!
//! Out of scope, not #3792 (noted for the captain): `cmd 4<&3` (a
//! non-stdin dup of a write-only fd — the numbered-dup delivery in
//! external_inner.rs is inside PR #3538's hunks), in-list
//! `cmd 3>f1 <&3` (the source is bound by the same list, not the
//! ambient table), `cp /dev/stdin`, and the move form `cat <&3-`.
//! #3770 (`read` builtin on write-only fd 0) is a different trigger
//! and code path (read_builtin.rs), also out of scope.

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash3792-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run_script(tag: &str, body: &str) -> (String, String, Option<i32>) {
    let dir = tempdir(tag);
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------
// Consumer flavor gate for the pipeline pins (issue #3792, REPORT
// §10.4).
//
// The pipeline pins below pin TWO contracts at once:
//   1. the shell-side contract — rubash delivers the write-only dup
//      onto the stage's fd 0, superseding the pipe, so the pipe
//      payload is never served; and
//   2. the consumer-side contract — a GNU coreutils consumer whose
//      stdin is that write-only fd fails its own read, reports its
//      own EBADF diagnostic, and exits nonzero (cat/wc/tr).
//
// Contract 2 is a property of the CONSUMER binary, not of rubash.
// On the CI census runner, `cat`/`wc`/`tr` resolve to uutils
// coreutils, which silently exits 0 on a write-only regular-file
// stdin instead of failing the read; under GNU bash in the same
// environment the same divergence appears, so the consumer half of
// these pins cannot discriminate there (full evidence chain in
// REPORT §10.4). Where the consumer is not GNU coreutils, the four
// affected pins therefore assert only contract 1 (the payload must
// not be served) and say so on stderr; they do NOT encode the
// non-GNU rc/counts as expected behavior. Wherever GNU coreutils
// is present — developer hosts, the WSL oracle environment — the
// pins run at full strength with both contracts asserted.
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsumerFlavor {
    Gnu,
    Uutils,
    Unrecognized,
}

/// Probe `<name> --version` once per consumer and classify it by
/// the version text ("GNU coreutils" vs "uutils"). The probe
/// resolves through this test process's PATH, which is the PATH
/// the rubash child inherits in `run_script`, so the
/// classification describes the binary the pipeline stage will
/// actually spawn.
fn consumer_flavor(name: &str) -> ConsumerFlavor {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, ConsumerFlavor>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(flavor) = cache.lock().expect("consumer flavor cache").get(name) {
        return *flavor;
    }
    let flavor = match Command::new(name).arg("--version").output() {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            if text.contains("GNU coreutils") {
                ConsumerFlavor::Gnu
            } else if text.contains("uutils") {
                ConsumerFlavor::Uutils
            } else {
                ConsumerFlavor::Unrecognized
            }
        }
        Err(_) => ConsumerFlavor::Unrecognized,
    };
    cache
        .lock()
        .expect("consumer flavor cache")
        .insert(name.to_owned(), flavor);
    flavor
}

#[test]
fn dup_write_only_fd_cat_empty() {
    let (stdout, stderr, code) = run_script("cat-empty", "exec 3>f1; cat <&3; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat:") && stderr.contains("Bad file descriptor"),
        "expected the child's own cat EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_content() {
    let (stdout, stderr, code) = run_script(
        "cat-content",
        "printf 'hello\\n' > f1; exec 3>>f1; cat <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected the child's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_dash() {
    let (stdout, stderr, code) = run_script("cat-dash", "exec 3>f1; cat - <&3; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("cat: -: Bad file descriptor"),
        "expected `cat: -: Bad file descriptor`, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_numbered() {
    let (stdout, stderr, code) = run_script("cat-n", "exec 3>f1; cat -n <&3; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected the child's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_slash_path() {
    let (stdout, stderr, code) =
        run_script("cat-slash", "exec 3>f1; /usr/bin/cat <&3; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected the child's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_command_cat() {
    let (stdout, stderr, code) = run_script(
        "cat-command",
        "exec 3>f1; command cat <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("Bad file descriptor"),
        "expected the child's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_in_pipeline() {
    let (stdout, stderr, code) = run_script(
        "cat-pipe",
        "exec 3>f1; printf 'x\\n' | cat <&3; echo \"rc=$?\"\n",
    );
    let flavor = consumer_flavor("cat");
    if flavor == ConsumerFlavor::Gnu {
        assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
        assert!(
            stderr.contains("cat:") && stderr.contains("Bad file descriptor"),
            "expected the child's own cat EBADF diagnostic, got: {stderr}"
        );
    } else {
        // Non-GNU consumer (REPORT §10.4): its silent rc=0 on a
        // write-only stdin is the consumer's divergence, not the
        // shell's — assert only the delivery contract.
        eprintln!(
            "note: `cat` is {flavor:?}, not GNU coreutils; skipping the consumer-behavior \
             assertions (rc=1 and cat's own EBADF diagnostic) and asserting only the \
             shell-side delivery contract (REPORT §10.4)"
        );
        assert!(
            !stdout.contains("x\n"),
            "the write-only dup must supersede the pipe: payload must not be served, \
             got stdout: {stdout}"
        );
    }
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_head() {
    let (stdout, stderr, code) = run_script("head", "exec 3>f1; head -c 5 <&3; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("head:") && stderr.contains("Bad file descriptor"),
        "expected head's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_dd() {
    let (stdout, stderr, code) = run_script(
        "dd",
        "exec 3>f1; dd bs=1 count=2 <&3 2>/dev/null; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "dd's own stderr is redirected away: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_grep() {
    let (stdout, stderr, code) = run_script(
        "grep",
        "printf 'a\\nb\\n' > f1; exec 3>>f1; grep -c . <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "0\nrc=2\n", "stderr: {stderr}");
    assert!(
        stderr.contains("grep:"),
        "expected grep's own diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_head_in_pipeline() {
    // Same mechanism, another inline stage arm: the member's own
    // `<&3` dup supersedes the pipe for head too (GNU: the element's
    // do_redirections runs after do_piping, execute_cmd.c:2620).
    let (stdout, stderr, code) = run_script(
        "head-pipe",
        "exec 3>f1; printf 'x\\n' | head -c 5 <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("head:") && stderr.contains("Bad file descriptor"),
        "expected head's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_grep_in_pipeline() {
    let (stdout, stderr, code) = run_script(
        "grep-pipe",
        "printf 'a\\nb\\n' > f1; exec 3>>f1; printf 'x\\n' | grep -c . <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "0\nrc=2\n", "stderr: {stderr}");
    assert!(
        stderr.contains("grep:") && stderr.contains("Bad file descriptor"),
        "expected grep's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_cat_in_pipeline_stage0() {
    // The dup winner on the FIRST stage routes the same way: cat
    // reports EBADF, the downstream stage sees an empty pipe.
    let (stdout, stderr, code) = run_script(
        "cat-pipe-stage0",
        "exec 3>f1; cat <&3 | wc -c; echo \"rc=$?\"\n",
    );
    // The stdout assertion is the shell-side delivery contract in
    // every flavor: the dup supersedes cat's stdin, cat emits
    // nothing, and the downstream `wc -c` counts an empty pipe.
    assert_eq!(stdout, "0\nrc=0\n", "stderr: {stderr}");
    let flavor = consumer_flavor("cat");
    if flavor == ConsumerFlavor::Gnu {
        assert!(
            stderr.contains("cat:") && stderr.contains("Bad file descriptor"),
            "expected cat's own EBADF diagnostic, got: {stderr}"
        );
    } else {
        eprintln!(
            "note: `cat` is {flavor:?}, not GNU coreutils; skipping the consumer-behavior \
             assertion (cat's own EBADF diagnostic) — the stdout assertion above already \
             pins the shell-side delivery contract (REPORT §10.4)"
        );
    }
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_head_line_in_pipeline() {
    // Line-mode head: the inline Head slot arm ACCEPTS this shape
    // (only byte mode declines), so this pin separates the
    // stage-dispatch route from the arm's own serve. GNU bash
    // 5.3.20: `head: error reading 'standard input': Bad file
    // descriptor`, rc=1.
    let (stdout, stderr, code) = run_script(
        "head-line-pipe",
        "exec 3>f1; printf 'x\\n' | head -1 <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert!(
        stderr.contains("head:") && stderr.contains("Bad file descriptor"),
        "expected head's own EBADF diagnostic, got: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_wc_in_pipeline() {
    // GNU bash 5.3.20: wc prints the zero count, reports
    // `wc: 'standard input': Bad file descriptor`, rc=1.
    let (stdout, stderr, code) = run_script(
        "wc-pipe",
        "exec 3>f1; printf 'x\\n' | wc -c <&3; echo \"rc=$?\"\n",
    );
    let flavor = consumer_flavor("wc");
    if flavor == ConsumerFlavor::Gnu {
        assert_eq!(stdout, "0\nrc=1\n", "stderr: {stderr}");
        assert!(
            stderr.contains("wc:") && stderr.contains("Bad file descriptor"),
            "expected wc's own EBADF diagnostic, got: {stderr}"
        );
    } else {
        // Non-GNU consumer (REPORT §10.4): assert only the
        // delivery contract — wc counted zero bytes, i.e. the
        // payload ("x\n", 2 bytes) was not served. The zero count
        // is the shell-side invariant (GNU wc prints it too, before
        // erroring); the rc is the consumer's and is deliberately
        // not asserted here.
        eprintln!(
            "note: `wc` is {flavor:?}, not GNU coreutils; skipping the consumer-behavior \
             assertions (rc=1 and wc's own EBADF diagnostic) and asserting only the \
             shell-side delivery contract (REPORT §10.4)"
        );
        assert!(
            stdout.starts_with("0\n"),
            "wc must count 0 bytes when the write-only dup wins fd 0 (payload not \
             served), got stdout: {stdout}"
        );
    }
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_tr_in_pipeline() {
    // GNU bash 5.3.20: `tr: read error: Bad file descriptor`, rc=1.
    let (stdout, stderr, code) = run_script(
        "tr-pipe",
        "exec 3>f1; printf 'x\\n' | tr a b <&3; echo \"rc=$?\"\n",
    );
    let flavor = consumer_flavor("tr");
    if flavor == ConsumerFlavor::Gnu {
        assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
        assert!(
            stderr.contains("tr:") && stderr.contains("Bad file descriptor"),
            "expected tr's own EBADF diagnostic, got: {stderr}"
        );
    } else {
        // Non-GNU consumer (REPORT §10.4): its silent rc=0 on a
        // write-only stdin is the consumer's divergence, not the
        // shell's — assert only the delivery contract.
        eprintln!(
            "note: `tr` is {flavor:?}, not GNU coreutils; skipping the consumer-behavior \
             assertions (rc=1 and tr's own EBADF diagnostic) and asserting only the \
             shell-side delivery contract (REPORT §10.4)"
        );
        assert!(
            !stdout.contains("x\n"),
            "the write-only dup must supersede the pipe: payload must not be served, \
             got stdout: {stdout}"
        );
    }
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_dd_in_pipeline() {
    // dd has no inline slot arm: a pipeline dd stage always reaches
    // the external stage spawner, so this pin isolates the spawner's
    // own fd-0 adjudication for the ambient write-only dup.
    let (stdout, stderr, code) = run_script(
        "dd-pipe",
        "exec 3>f1; printf 'x\\n' | dd bs=1 count=2 <&3 2>/dev/null; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "dd's own stderr is redirected away: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn dup_readable_fd_cat_in_pipeline_control() {
    // Control: a dup of a READABLE fd in a pipeline still serves the
    // descriptor's stream (the write-only route must not overreach).
    let (stdout, stderr, code) = run_script(
        "cat-pipe-readable",
        "printf 'data\\n' > f1; exec 3<f1; printf 'x\\n' | cat <&3; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "data\nrc=0\n", "stderr: {stderr}");
    assert_eq!(stderr, "", "no diagnostic for a readable dup: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn dup_write_only_fd_controls() {
    let (stdout, _stderr, code) = run_script(
        "controls",
        "printf 'data\\n' > f1; exec 3<f1; cat <&3; echo \"ro-rc=$?\"\n\
         printf 'data\\n' > f1; exec 3<>f1; cat <&3; echo \"rw-rc=$?\"\n\
         exec 3>f1; read -u 3 x; echo \"readu-rc=$?\"\n\
         printf 'data\\n' > f1; exec 3<f1; echo hi >&3; echo \"wro-rc=$?\"\n\
         exec 3>f1; echo hi >&3; echo \"wwo-rc=$?\"; cat f1\n\
         printf 'hello\\n' > f1; exec 3>f1; cat f1 <&3; echo \"fileop-rc=$?\"\n\
         printf 'hello\\n' > f1; exec 3>f1; cat < /dev/fd/3; echo \"alias-rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "data\n\
         ro-rc=0\n\
         data\n\
         rw-rc=0\n\
         readu-rc=1\n\
         wro-rc=1\n\
         wwo-rc=0\n\
         hi\n\
         fileop-rc=0\n\
         alias-rc=0\n"
    );
    assert_eq!(code, Some(0));
}
