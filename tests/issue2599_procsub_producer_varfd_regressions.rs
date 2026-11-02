//! rubash#2599 (split from #2522 / PR #2569) — inside the respawned
//! procsub producer (`rubash -c`, the port of subst.c:6362
//! process_substitute's forked child), an external command whose
//! redirect list first rebinds fd N with a plain entry (`9>o1.txt`)
//! and then dups it via `{w}>&N` had the `{w}` descriptor delivered
//! from the producer's AMBIENT fd N binding (inherited from the outer
//! shell, or set by an earlier `exec N>...` inside the producer)
//! instead of the same-list rebind: the child's writes landed in the
//! outer shell's fd-9 file, the `9>` target stayed empty, and no
//! diagnostic spoke.
//!
//! GNU owners: do_redirections is ONE ordered walk over ONE
//! descriptor space (redir.c:237) — the REDIR_VARASSIGN F_DUPFD copy
//! (redir.c:940 area) resolves its source at its list position, where
//! the earlier plain entries already hold their endpoints; the
//! producer is a forked child (subst.c:6362) whose in-body commands
//! run the same walk.
//!
//! Root cause (this fix): the delivery walk resolved the dup
//! correctly — walk_opened carried o1's description and
//! deliver_fd_var_dup stamped/pinned it at the var's slot — but the
//! fork-side pin pass (pin_ambient_fds_in_child) broke the pin list's
//! own precedence. The merged pin list is ambient-first, command-pins
//! appended, and later pins win a shared slot by dup2 order
//! (rubash#1686). In the producer's sparse fd space the walk opens
//! `9>o1.txt` at OS fd 10 — exactly the {var} slot — so the command
//! pin for the slot was source==target, and the pin pass "applied"
//! it during the staging phase by merely clearing CLOEXEC. The
//! ambient pin for the same slot (the applier's copy of the ambient
//! fd 9) then dup2'd over it in phase two. Respawned shells are the
//! exposed face because their fd space is sparse enough for the
//! walk's open to land on the var's slot; a script shell's busier fd
//! space masks it. The pin pass now stages source==target pins like
//! every other pin, so list order decides the slot uniformly.
//!
//! The first two pins moved verbatim from PR #2569 (they were the
//! #2522 wave-3 residuals pc1b/pc1); the third is the captain's p4
//! isolation shape. Expectations byte-verified against GNU Bash
//! 5.3.20 (script-file probes; captain's matrix in
//! rubash-wave-20261102/captain-2569/probes/).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2599-procsub-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture");
}

/// Run `driver.sh` (with `@RUBASH@` replaced by this build's binary path)
/// as the outer shell's main script, stdin null.
fn run_driver(dir: &Path, driver: &str) -> (String, String, Option<i32>) {
    write(dir, "driver.sh", &driver.replace("@RUBASH@", RUBASH));
    let output = Command::new(RUBASH)
        .arg("driver.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash driver");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Wave-3 headline (pc1b): a procsub body whose external child carries a
/// close-bearing `{v}>&-` list. GNU: the reader's reports reach the
/// procsub pipe (`w10=OK`, `got4=[]`), the `{w}>&9` descriptor writes W10
/// into o1.txt, and the only diagnostic is the reader's own fd-4 EBADF
/// (fd 4 is not inherited into the procsub body). Pre-fix: the re-print
/// split `{w}>&9` and `{v}>&-`, so fd 1 was closed by the misread `>& -`,
/// fd 10 never existed, and stdout printed only `rc=0 o1=[]`.
#[test]
fn procsub_body_varfd_close_list_delivered() {
    let dir = scratch("headline");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nline=$(cat <&4 2>&1)\necho \"got4=[$line]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\ncat < <(\"@RUBASH@\" reader.sh 9>o1.txt {w}>&9 {v}>&-)\necho \"rc=$? o1=[$(cat o1.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w10=OK\ngot4=[]\nrc=0 o1=[W10]\n");
    assert_eq!(
        stderr, "reader.sh: line 2: 4: Bad file descriptor\n",
        "the reader's own fd-4 diagnostic is the only stderr"
    );
}

/// The consumer-side fd juggling variant (wave-3 pc1): the procsub pipe
/// opened at fd 4 first, then dup'd to stdin — the reader's completion
/// must still order before the parent continues.
#[test]
fn procsub_body_varfd_with_pipe_at_fd4() {
    let dir = scratch("fd4");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\nline=$(cat <&4 2>&1)\necho \"got4=[$line]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seedout.txt\nv=9\ncat 4< <(echo PSIN; \"@RUBASH@\" reader.sh 9>o1.txt {w}>&9 {v}>&-) <&4\necho \"rc=$? o1=[$(cat o1.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "PSIN\nw10=OK\ngot4=[]\nrc=0 o1=[W10]\n");
    assert_eq!(
        stderr, "reader.sh: line 2: 4: Bad file descriptor\n",
        "the reader's own fd-4 diagnostic is the only stderr"
    );
}

/// Captain's p4 isolation shape: the ambient fd-9 binding is created
/// INSIDE the producer (a body-level `exec 9>dummy.txt` ahead of the
/// command), not inherited from the outer shell. The command's own
/// list still rebinds 9 first (`9>o1.txt`), so GNU's ordered walk
/// (redir.c:237) lands the `{w}` copy on o1.txt; dummy.txt and the
/// outer seed file both stay empty. Pre-fix the write landed in
/// dummy.txt — the producer's ambient binding won the var's slot.
#[test]
fn procsub_body_varfd_dup_after_producer_exec_rebind() {
    let dir = scratch("producer-exec");
    write(
        &dir,
        "reader.sh",
        "if echo W10 >&10; then echo \"w10=OK\"; else echo \"w10=FAIL\"; fi\necho \"got4=[]\"\n",
    );
    let (stdout, stderr, code) = run_driver(
        &dir,
        "exec 9>seed.txt\ncat < <(exec 9>dummy.txt; \"@RUBASH@\" reader.sh 9>o1.txt {w}>&9)\necho \"rc=$? o1=[$(cat o1.txt)] seed=[$(cat seed.txt)] dummy=[$(cat dummy.txt)]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "w10=OK\ngot4=[]\nrc=0 o1=[W10] seed=[] dummy=[]\n");
    assert_eq!(stderr, "");
}
