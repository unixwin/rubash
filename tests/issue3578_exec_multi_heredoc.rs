//! rubash#3578: a redirect-only `exec` must bind EVERY heredoc /
//! here-string entry of its redirect list at that entry's list
//! position (GNU redir.c here_document_to_fd, redir.c:423, applied in
//! do_redirections' list walk at redir.c:1065-1072; exec's
//! redirect-only form keeps the binds persistent). The old tail-only
//! lookup bound just the last `heredoc_redirects` entry, so
//! `exec 14<<A 15<<B` left fd 14 unbound.

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
