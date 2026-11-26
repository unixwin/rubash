//! rubash#2375 — `$[...]` in ASSIGNMENT position (and in prefixed /
//! suffixed unquoted words) did not evaluate a quoted body: the
//! unquoted word walk (remove_shell_quotes_inner_cursor) had no `$[`
//! dispatch arm, so the body was cooked as ordinary word text — an
//! inner `"` opened the walk's double-quote arm, which CTLESC-prefixed
//! the arithmetic characters and dropped the quotes, and the evaluator
//! reported the corrupted token `1\u{11}+1` instead of evaluating
//! `1+1`.
//!
//! GNU contract: parse.y:5527 read_token_word hands a `$[` unit to
//! parse_matched_pair(cd, '[', ']', P_ARITH) as one self-contained
//! span; subst.c:10912-10926 param_expand case '[' then extracts and
//! evaluates it in every word position. The fix copies the whole span
//! verbatim in the unquoted walk (copy_dollar_bracket_verbatim_cursor,
//! span by subst.c:1361 extract_delimited_string); the collectors
//! re-extract and evaluate it (rubash#2374's machine).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with the
//! exact script below (cluster probes/pins/pin2375.sh). On master,
//! lines 1-4 all fail with the corrupted-token diagnostic, line 7
//! silently assigns 2, and line 8 reports rc=0.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let root = std::env::temp_dir().join(format!("issue2375_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let script_path: PathBuf = root.join("pin2375.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2375.sh")
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

const SCRIPT: &str = "\
v=$[\"1+1\"]; echo \"a1 v=$v\"
echo x=$[\"1+1\"]
echo pre$[\"1+1\"]post
echo $[\"1+1\"]x
echo $[\\\"1+1\\\"]
echo \"a6 rc=$?\"
w=$['1+1']; echo \"a7 w=[$w]\"
echo \"a8 rc=$?\"
";

#[test]
fn issue2375_quoted_bodies_evaluate_in_assignment_and_word_positions() {
    let (stdout, stderr, code) = run_script(SCRIPT);
    assert_eq!(
        stdout,
        "a1 v=2\n\
         x=2\n\
         pre2post\n\
         2x\n\
         a6 rc=1\n\
         a8 rc=1\n"
    );
    assert_eq!(
        stderr,
        "pin2375.sh: line 5: \"1+1\": arithmetic syntax error: operand expected (error token is \"\"1+1\"\")\n\
         pin2375.sh: line 7: '1+1': arithmetic syntax error: operand expected (error token is \"'1+1'\")\n"
    );
    assert_eq!(code, Some(0));
}
