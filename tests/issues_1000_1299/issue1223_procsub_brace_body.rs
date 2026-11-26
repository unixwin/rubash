//! Issue unixwin/rubash#1223: a brace-group body `{ ...; }` on a
//! write-side process substitution was rejected with `ambiguous redirect`,
//! and the argument form (`tee >({ cat; })`) left the external child a
//! literal `/dev/fd/N` it could not open.
//!
//! GNU owner: redir.c:298 redirection_expand -> expand_word_internal's
//! '<'/'>' cases (subst.c:11349-11378 extract_process_subst, forked by
//! subst.c:6362 process_substitute) return the substitution filename as the
//! single expanded field — the `{ cat; }` body is never a brace-expansion
//! candidate. Rubash kept the raw `>({ cat; })` word in
//! target_metadata.raw after materialization rewrote the target to the
//! carrier path, and expand_braces_with_optional_raw fed that raw back
//! into the fields pipeline where `{ cat; }` looked like a brace group.
//!
//! Verified against WSL GNU Bash 5.3.20 (script-file probes): the redirect
//! form prints `hello` then `done`; `tee >({ cat; })` prints `hello`.

use std::io::Write;
use std::process::{Command, Stdio};

fn rubash_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash stdin");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("collect stdin run");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn write_procsub_brace_group_body_accepted() {
    // GNU: `hello` then `done`, no diagnostic.
    let (stdout, stderr, code) = rubash_stdin("echo hello > >({ cat; })\nsleep 0.3\necho done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hello\ndone\n", "stderr: {stderr}");
    assert!(!stderr.contains("ambiguous redirect"), "stderr: {stderr}");
}

#[test]
fn write_procsub_brace_group_body_spaced() {
    let (stdout, stderr, code) = rubash_stdin("echo hello > >( { cat; } )\nsleep 0.3\necho done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hello\ndone\n", "stderr: {stderr}");
}

#[test]
fn write_procsub_argument_form_external() {
    // GNU: tee writes into the substitution, cat echoes hello.
    let (stdout, stderr, code) =
        rubash_stdin("echo hello | tee >({ cat; }) >/dev/null\nsleep 0.5\necho done\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hello\ndone\n", "stderr: {stderr}");
}
