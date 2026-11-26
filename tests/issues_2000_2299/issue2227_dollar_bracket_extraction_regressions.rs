//! rubash#2227 (extraction faces) — `$[...]` extraction is quote-state
//! aware: a quoted `]` inside the body does not close the span, a
//! quoted body with a leading blank still extracts, and nested array
//! subscripts inside the body nest instead of closing.
//!
//! GNU contract: subst.c:1301 extract_arithmetic_subst / subst.c:1361
//! extract_delimited_string (skip_single_quoted subst.c:1127,
//! skip_double_quoted subst.c:1014 skip the units whole), then
//! subst.c:3981 expand_arith_string (string_quote_removal subst.c:12199
//! for expansion-free bodies). These are the faces the #2374 machine +
//! the #2375 verbatim cooker arm deliver; the remaining #2227 face —
//! `$[...]` inside a double-quoted SEGMENT whose hunt is segment-scoped
//! in GNU's reader (`echo "a=$["1+1"] b"` → `bad substitution: no
//! closing ']' in a=$[`) — is NOT pinned here: it needs a cooked-word
//! carrier for a segment-scoped verdict and is tracked as the issue's
//! documented remainder.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with the
//! exact script below (cluster probes/pins/pin2227.sh). On master, e5
//! fails with the corrupted-token diagnostic (`1\u{11}+1`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let root = std::env::temp_dir().join(format!("issue2227_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create fixture dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture dir");
    let script_path: PathBuf = root.join("pin2227.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let child = Command::new(RUBASH)
        .arg("pin2227.sh")
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
a=(10 20 30)
echo \"e3=$[a[1]+2]\"
echo \"e4=$[1+2]]\"
echo e5=$[ \"1+1\"]
echo \"e6=$[1+1] b\"
echo \"e7=$['1+1']\"
echo \"m7 rc=$?\"
echo t8=$[\"]\"]
echo \"m8 rc=$?\"
";

#[test]
fn issue2227_extraction_faces_match_gnu() {
    let (stdout, stderr, code) = run_script(SCRIPT);
    assert_eq!(
        stdout,
        "e3=22\n\
         e4=3]\n\
         e5=2\n\
         e6=2 b\n\
         m7 rc=1\n\
         m8 rc=1\n"
    );
    assert_eq!(
        stderr,
        "pin2227.sh: line 6: '1+1': arithmetic syntax error: operand expected (error token is \"'1+1'\")\n\
         pin2227.sh: line 8: ]: arithmetic syntax error: operand expected (error token is \"]\")\n"
    );
    assert_eq!(code, Some(0));
}
