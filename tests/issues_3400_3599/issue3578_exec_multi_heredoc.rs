//! rubash#3578: a redirect-only `exec` must bind EVERY heredoc /
//! here-string entry of its redirect list at that entry's list
//! position (GNU redir.c here_document_to_fd, redir.c:423, applied in
//! do_redirections' list walk at redir.c:1065-1072 and dup2'd onto the
//! redirector at redir.c:1120; exec's redirect-only form keeps the
//! binds persistent). The old tail-only lookup bound just the last
//! `heredoc_redirects` entry, so `exec 14<<A 15<<B` left fd 14 unbound.
//!
//! Pin surface is the union of PR #3603's pins and PR #3580's wider
//! grid (three heredocs, same-fd last-wins, fd-0+numbered,
//! `{var}`+numbered, and the brace-group control), plus an
//! interleave pin for the in-walk binding position itself, plus
//! rubash#3654's reverse-order pair (unnumbered here-string first
//! with a numbered heredoc after, and its mirror); every
//! expectation byte-verified against WSL GNU bash 5.3.x script-file
//! probes. The `ulimit -n 16` preludes mirror the issue3195 pin's
//! harness so fds 13-15 stay below the soft limit.

use std::path::PathBuf;
use std::process::Command;

fn run_script(name: &str, script: &str) -> (String, String, Option<i32>, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rubash-issue3578-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        dir,
    )
}

#[test]
fn issue_script_exec_binds_every_numbered_heredoc() {
    let (stdout, stderr, code, _dir) = run_script(
        "two",
        "exec 14<<A 15<<B\n\
         line-14\n\
         A\n\
         line-15\n\
         B\n\
         cat <&14\n\
         cat <&15\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "line-14\nline-15\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_two_heredocs_bind_both_read_u() {
    // The issue's seed shape (#3580's grid): both descriptors stay
    // bound to their own bodies, read through `read -u`.
    let (stdout, stderr, code, _dir) = run_script(
        "two-read",
        "ulimit -n 16\n\
         exec 14<<A 15<<B\n\
         first\n\
         A\n\
         second\n\
         B\n\
         read -u 14 x; read -u 15 y; echo \"x=$x y=$y\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=first y=second\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_three_heredocs_bind_all() {
    let (stdout, stderr, code, _dir) = run_script(
        "three",
        "ulimit -n 16\n\
         exec 13<<A 14<<B 15<<C\n\
         one\n\
         A\n\
         two\n\
         B\n\
         three\n\
         C\n\
         read -u 13 a; read -u 14 b; read -u 15 c; echo \"a=$a b=$b c=$c\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "a=one b=two c=three\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_same_fd_heredoc_last_wins() {
    // Two entries on one descriptor: GNU's ordered walk lets the
    // later entry overwrite the earlier one.
    let (stdout, stderr, code, _dir) = run_script(
        "same-fd",
        "ulimit -n 16\n\
         exec 14<<A 14<<B\n\
         first\n\
         A\n\
         second\n\
         B\n\
         read -u 14 x; echo \"x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=second\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_binds_mixed_heredoc_here_string() {
    let (stdout, stderr, code, _dir) = run_script(
        "mixed",
        "exec 7<<Q 8<<<'w'\n\
         qbody\n\
         Q\n\
         cat <&7\n\
         cat <&8\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "qbody\nw\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_heredoc_and_numbered_here_string_bind_both() {
    let (stdout, stderr, code, _dir) = run_script(
        "mixed-read",
        "ulimit -n 16\n\
         exec 14<<A 15<<<\"strbody\"\n\
         first\n\
         A\n\
         read -u 14 x; read -u 15 y; echo \"x=$x y=$y\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=first y=strbody\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_unnumbered_heredoc_and_here_string_still_bind_fd0() {
    // Both forms keep their fd-0 semantics: the last unnumbered entry
    // wins the descriptor, exactly like GNU's sequential dup2.
    let (stdout, stderr, code, _dir) = run_script(
        "fd0",
        "exec <<SOLO\n\
         solo-body\n\
         SOLO\n\
         cat\n\
         exec 3<<<\"hs-$((1+1))\"\n\
         cat <&3\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "solo-body\nhs-2\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_stdin_and_numbered_heredoc_bind_both() {
    // An fd-0 heredoc and a numbered one in the same list: both
    // persist (the fd-0 body becomes the shell's stdin).
    let (stdout, stderr, code, _dir) = run_script(
        "fd0-plus",
        "ulimit -n 16\n\
         exec <<A 14<<B\n\
         zero\n\
         A\n\
         fourteen\n\
         B\n\
         read z; read -u 14 x; echo \"z=$z x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "z=zero x=fourteen\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_fd_var_and_numbered_heredoc_bind_both() {
    // A `{var}` heredoc and a numbered one in the same list: the var
    // allocation and the numbered bind do not crowd each other out.
    let (stdout, stderr, code, _dir) = run_script(
        "fd-var",
        "ulimit -n 16\n\
         exec {v}<<A 14<<B\n\
         varbody\n\
         A\n\
         fourteen\n\
         B\n\
         read -u \"$v\" x; read -u 14 y; echo \"x=$x y=$y\"\n\
         if [ \"$v\" -lt 16 ]; then echo below; fi\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=varbody y=fourteen\nbelow\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_heredoc_binds_persist_after() {
    // GNU applies redirect-only exec binds persistently, so a later
    // simple command still reads the descriptor.
    let (stdout, stderr, code, _dir) = run_script(
        "persist",
        "exec 9<<E\n\
         via-exec\n\
         E\n\
         echo marker\n\
         cat <&9\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "marker\nvia-exec\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_heredoc_binds_at_list_position() {
    // The bind happens at the entry's walk position, not at the tail:
    // a `3<&0` AFTER the heredoc entry dups the body GNU already bound
    // (redir.c do_redirections' ordered walk) — the shape the in-walk
    // twin pairing preserves and the old tail lookup could not.
    let (stdout, stderr, code, _dir) = run_script(
        "position",
        "exec <<A 3<&0\n\
         aaa\n\
         A\n\
         cat <&3\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "aaa\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_unnumbered_here_string_first_numbered_heredoc_binds() {
    // rubash#3654: the reverse order of the #3578 grid — an
    // unnumbered here-string FIRST, a numbered heredoc AFTER.
    // The here-string binds fd 0; the heredoc still binds fd 14
    // at its own list position (GNU redir.c:1065-1072 ordered walk).
    let (stdout, stderr, code, _dir) = run_script(
        "hs-first",
        "ulimit -n 16\n\
         exec <<<w 14<<A\n\
         word\n\
         A\n\
         read -u 14 x; echo \"x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=word\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_exec_numbered_heredoc_first_here_string_control() {
    // Reverse-order control for #3654: the same two bodies with the
    // numbered heredoc first — GNU binds fd 14 identically either way.
    let (stdout, stderr, code, _dir) = run_script(
        "hd-first",
        "ulimit -n 16\n\
         exec 14<<A <<<w\n\
         word\n\
         A\n\
         read -u 14 x; echo \"x=$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=word\n", "stdout={stdout:?}");
}

#[test]
fn issue_script_group_two_heredocs_bind_both_control() {
    // Control: the same two bodies on a brace group's redirect list
    // already bound both (bind_numbered_heredoc_redirects iterates
    // the whole list) — the loss was specific to redirect-only exec.
    let (stdout, stderr, code, _dir) = run_script(
        "group-two",
        "ulimit -n 16\n\
         { read -u 14 x; read -u 15 y; echo \"x=$x y=$y\"; } 14<<A 15<<B\n\
         first\n\
         A\n\
         second\n\
         B\n",
    );
    assert_eq!(code, Some(0), "stderr={stderr:?}");
    assert_eq!(stdout, "x=first y=second\n", "stdout={stdout:?}");
}
