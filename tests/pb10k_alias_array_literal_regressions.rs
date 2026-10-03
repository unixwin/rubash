//! Alias expansion must not leak into compound array-assignment literals
//! (niubash 1.3.0 + oh-my-bash powerbash10k, owner's second machine).
//!
//! Repro chain: a default convenience `alias 1='cd -'` is live when
//! `~/.niubashrc` sources oh-my-bash.sh, whose `OMB_VERSINFO=(1 0 0 0
//! master noarch)` got its `1` element alias-expanded (`cd -`) and
//! word-split into two elements; the later version arithmetic then died
//! with `niu: -: arithmetic syntax error: operand expected (error token is
//! "-")` — the bare `-` element.
//!
//! GNU spec: read_token_word (parse.y:5652-5673) consumes the whole
//! `( ... )` of `NAME=(...)` into the SAME word via
//! parse_compound_assignment (parse.y:7104), which reads element words
//! with `last_read_token = WORD`: "Plus it means we won't be in a command
//! position and so alias expansion won't happen" (parse.y:7113-7117).
//! alias_expand_token (parse.y:3249, predicate at :3254) therefore never
//! sees an element word, except through PST_ALEXPNEXT. Verified
//! byte-for-byte against WSL GNU Bash 5.3.0 (/usr/local/bin/bash,
//! script-file probes 2026-10-03).
//!
//! The `-i` piped-stdin shape is the tests/issue297_300 pattern; the probe
//! turns `expand_aliases` on itself so the case does not depend on the
//! interactive default.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(stdin: &[u8]) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The owner's corruption: array-literal elements are never alias
/// candidates, so `OMB_VERSINFO` keeps its literal words and the version
/// arithmetic on them stays well-formed.
#[test]
fn array_literal_elements_survive_a_live_digit_alias() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          alias 1='cd -'\n\
          OMB_VERSINFO=(1 0 0 0 master noarch)\n\
          printf 'elems: [%s]\\n' \"${OMB_VERSINFO[@]}\"\n\
          printf 'major: %d\\n' \"$((10#${OMB_VERSINFO[0]}))\"\n",
    );
    let expected = "elems: [1]\nelems: [0]\nelems: [0]\nelems: [0]\n\
                    elems: [master]\nelems: [noarch]\nmajor: 1\n";
    assert_eq!(stdout, expected);
    assert!(
        !stderr.contains("arithmetic syntax error"),
        "stderr leaked the arithmetic failure: {stderr}"
    );
    assert!(
        !stderr.contains("operand expected"),
        "stderr leaked the operand failure: {stderr}"
    );
}

/// GNU keeps expanding aliases in real command positions around the
/// literal — the fix must not over-reach (parse.y:3157
/// command_token_position: ASSIGNMENT_WORD leaves the next word in command
/// position; the first word of a subshell too).
#[test]
fn command_position_around_array_literal_still_expands() {
    let (stdout, stderr) = run_stdin(
        b"shopt -s expand_aliases\n\
          alias zq='echo ZQ'\n\
          A=(zq 0)\n\
          printf 'A: [%s]\\n' \"${A[@]}\"\n\
          A=(zq 0) zq hi\n\
          ( zq hi )\n",
    );
    let expected = "A: [zq]\nA: [0]\nZQ hi\nZQ hi\n";
    assert_eq!(stdout, expected);
    assert!(
        !stderr.contains("command not found"),
        "stderr leaked a failure: {stderr}"
    );
}
