//! Issue rubash#325: `read -rt 0.1 <> <(:)` — the pbb read_sleep idiom
//! (section 074 "Use read as an alternative to sleep") — was a parse-time
//! syntax error (`syntax error near unexpected token `<'`, rc 2); GNU
//! parses and runs it.
//!
//! GNU semantics (vendored third_party/bash): the redirection grammar's
//! `<>` production is `LESS_GREATER WORD` (parse.y:614-619 — every
//! redirection operator takes a WORD operand), and read_token lexes a
//! word-initial `<(` as a process substitution — so `<> <(:)` is a
//! read-write fd-0 redirect whose filename is the substitution: fd 0
//! binds onto the empty pipe (verified: GNU `f <> <(:)` calls f with ZERO
//! arguments — the substitution is the TARGET, never an argument), and
//! `read -t` then times out on it (the self-pipe sleep trick).
//!
//! Root cause: rubash's main parse loop (token_actions.rs RedirectOut arm
//! -> redirect_assign.rs assign_redirect_out_target) only accepted the
//! `>(...)` output-substitution forms after `>`-family operators; an
//! INPUT substitution after `<>` fell to missing_redirect_target_node.
//! (collect_trailing_redirections had the same gap.)
//!
//! Fix: read_write_process_substitution_redirect_target
//! (parser/process_substitution.rs) recognizes `<` `(` after the `<>`
//! operator; both parse paths build the ReadWrite redirect with the
//! substitution target and mirror fd-0 into redirect_in, exactly like the
//! `< <(:)` arm.
//!
//! Expected outputs are byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file probes 2026-09-28).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_clean(stderr: &str) {
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
}

/// The issue's exact reproducer: parse + run + rc (through the `|| :`
/// guard the pbb idiom uses).
///
/// KNOWN RESIDUAL (runtime, documented in the commit): GNU's `<>` opens
/// the procsub pipe O_RDWR, so the shell itself holds a write end — `:`
/// exiting never yields EOF and `read -t` waits the FULL timeout
/// (rc 142). Rubash's text-transport model serves the captured bytes then
/// EOF (rc 1); through the idiom's `|| :` the observable output and rc
/// are identical. The held-open-pipe timeout port (read blocking past
/// exhaustion) is future work in the FUNCTION_STDIN model.
#[test]
fn read_sleep_idiom_parses_and_runs() {
    let (stdout, stderr, code) =
        rubash("read_sleep() {\n  read -rt 0.1 <> <(:) || :\n}\nread_sleep\necho \"p7-ok rc=$?\"");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "p7-ok rc=0\n");
}

/// The minimal shape `: <> <(:)`.
#[test]
fn bare_readwrite_with_empty_procsub_parses() {
    let (stdout, stderr, code) = rubash(": <> <(:)\necho \"direct rc=$?\"");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "direct rc=0\n");
}

/// The substitution is the redirect TARGET, never an argument (GNU
/// `f <> <(:)` -> N=0).
#[test]
fn procsub_is_the_target_not_an_argument() {
    let (stdout, stderr, code) = rubash("f(){ echo \"N=$# A=[${1:-none}]\"; }\nf <> <(:)");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "N=0 A=[none]\n");
}

/// `<> <(echo x)` — fd 0 carries the substitution's output.
#[test]
fn readwrite_procsub_output_reaches_read() {
    let (stdout, stderr, code) = rubash("read -rt 1 <> <(echo x)\necho \"got=$REPLY rc=$?\"");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got=x rc=0\n");
}

/// Numbered form `exec 3<> <(echo y)` binds descriptor 3.
#[test]
fn numbered_readwrite_procsub_binds_fd() {
    let (stdout, stderr, code) =
        rubash("exec 3<> <(echo y)\nread -rt 1 -u 3 line\necho \"fd3=[$line]\"\nexec 3<&-");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "fd3=[y]\n");
}

/// The pre-existing `< <(:)` family is unchanged.
#[test]
fn classic_input_procsub_unaffected() {
    let (stdout, stderr, code) = rubash("cat < <(echo direct-in)");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "direct-in\n");
}

/// `<>` with a plain file word is unchanged (creates the file read-write).
#[test]
fn readwrite_plain_file_target_unaffected() {
    let (stdout, stderr, code) =
        rubash("f=$(mktemp)\n: <> \"$f\"\n[ -f \"$f\" ] && echo created\nrm -f \"$f\"");
    assert_clean(&stderr);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "created\n");
}
