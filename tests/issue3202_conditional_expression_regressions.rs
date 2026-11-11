//! Regression locks for rubash#3202 — the conditional-expression
//! error face inside command substitution. The audit found every
//! shape already matching GNU on master; this file pins that
//! behaviour so the face cannot drift: a malformed `[[ ... ]]`
//! inside `$( )` produces GNU's three-line diagnostic (the
//! conditional-expression error, the near-token line with the
//! comsub's closing paren attached, and the echoed source line)
//! at status 2, and the script does not continue.
//!
//! All expectations are the GNU bash 5.3.20 outputs for the same
//! probe scripts under LC_ALL=C.UTF-8, measured against the oracle
//! binary (lane dossier probe battery, shapes 3202-D1f/D2/D3/D4).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3202-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::canonicalize(&dir).expect("canonical root")
}

struct Outcome {
    stdout: String,
    stderr: String,
    rc: i32,
}

fn run(root: &Path, body: &str) -> Outcome {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let out = Command::new(BIN)
        .arg("case.sh")
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    Outcome {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        rc: out.status.code().unwrap_or(-1),
    }
}

const D1_ERR: &str =
    "case.sh: line 1: syntax error in conditional expression: unexpected token `c'\n\
                      case.sh: line 1: syntax error near `c)'\n\
                      case.sh: line 1: `echo $([[ a == b c) )'\n";

/// D1f: the malformed conditional inside a command substitution.
#[test]
fn malformed_conditional_in_comsub() {
    let root = scratch("d1f");
    let out = run(&root, "echo $([[ a == b c) )\n");
    assert_eq!(out.rc, 2);
    assert_eq!(out.stderr, D1_ERR);
    assert_eq!(out.stdout, "");
}

/// D4: the parse failure discards the rest of the script.
#[test]
fn malformed_conditional_in_comsub_discards_rest() {
    let root = scratch("d4");
    let out = run(&root, "echo $([[ a == b c) )\necho AFTER\n");
    assert_eq!(out.rc, 2);
    assert_eq!(out.stderr, D1_ERR);
    assert_eq!(out.stdout, "");
}

/// D3: the bare conditional form — the near-token line names the
/// bare token (no comsub paren attached).
#[test]
fn malformed_conditional_bare() {
    let root = scratch("d3");
    let out = run(&root, "[[ a == b c ]]\n");
    assert_eq!(out.rc, 2);
    assert_eq!(
        out.stderr,
        "case.sh: line 1: syntax error in conditional expression: unexpected token `c'\n\
         case.sh: line 1: syntax error near `c'\n\
         case.sh: line 1: `[[ a == b c ]]'\n"
    );
}

/// D2: the well-formed form is silent and runs.
#[test]
fn wellformed_conditional_in_comsub() {
    let root = scratch("d2");
    let out = run(&root, "echo $([[ a == b ]] )\n");
    assert_eq!(out.rc, 0);
    assert_eq!(out.stderr, "");
    assert_eq!(out.stdout, "\n");
}
