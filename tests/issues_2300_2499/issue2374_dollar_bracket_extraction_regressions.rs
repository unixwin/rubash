//! rubash#2374 — `$[...]` arithmetic diagnostics name the wrong token
//! when the body contains an escaped bracket: the extraction was plain
//! bracket counting, so `$[1\]]` collected the body `1\` and the eval
//! error named `\`, where GNU names `]`.
//!
//! GNU contract: subst.c:1301 extract_arithmetic_subst runs
//! subst.c:1361 extract_delimited_string — a backslash or CTLESC passes
//! the next character (it can neither delimit nor nest), quote/backquote
//! units are skipped whole, nested `[` / `$[` openers nest. The extracted
//! string then goes through subst.c:3981 expand_arith_string: with no
//! expansion character present, subst.c:12199 string_quote_removal
//! (Q_DOUBLE_QUOTES|Q_ARITH) drops a backslash before `[`/`]`/`~` and
//! the CBSDQUOTE set and drops unescaped `"` — so `$[1\]]` evaluates
//! `1]` (error token `]`) while `$[1\+1]` keeps `\+` (error token `\+1`
//! in the string as a whole).
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with the
//! cluster probe table (probes/pins/pin2374.sh, same shapes): every
//! marker line is rc=1 in both shells; on master the diagnostics for
//! the escaped-bracket shapes name `\`, a raw CTLESC byte, or nothing
//! (literal print / silent evaluation) instead of GNU's tokens.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let root = std::env::temp_dir().join(format!("issue2374_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let script_path: PathBuf = root.join("pin2374.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2374.sh")
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
echo \"t1=$[1\\]]\"
echo \"m1 rc=$?\"
v=$[1\\]]
echo \"m2 v=[$v] rc=$?\"
echo \"t3=$[1\\[2]\"
echo \"m3 rc=$?\"
echo \"t5=$[1\\+1]\"
echo \"m5 rc=$?\"
echo \"t6=$[1\\\\]+2\"
echo \"m6 rc=$?\"
echo $(( 1\\] ))
echo \"m7 rc=$?\"
";

#[test]
fn issue2374_escaped_bracket_tokens_match_gnu() {
    let (stdout, stderr, code) = run_script(SCRIPT);
    assert_eq!(
        stdout,
        "m1 rc=1\n\
         m2 v=[] rc=1\n\
         m3 rc=1\n\
         m5 rc=1\n\
         m6 rc=1\n\
         m7 rc=1\n"
    );
    assert_eq!(
        stderr,
        "pin2374.sh: line 1: 1]: arithmetic syntax error: invalid arithmetic operator (error token is \"]\")\n\
         pin2374.sh: line 3: 1]: arithmetic syntax error: invalid arithmetic operator (error token is \"]\")\n\
         pin2374.sh: line 5: 1[2: arithmetic syntax error: invalid arithmetic operator (error token is \"[2\")\n\
         pin2374.sh: line 7: 1\\+1: arithmetic syntax error: invalid arithmetic operator (error token is \"\\+1\")\n\
         pin2374.sh: line 9: 1\\: arithmetic syntax error: invalid arithmetic operator (error token is \"\\\")\n\
         pin2374.sh: line 11: 1] : arithmetic syntax error: invalid arithmetic operator (error token is \"] \")\n"
    );
    assert_eq!(code, Some(0));
}
