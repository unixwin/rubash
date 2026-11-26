//! rubash#1091 + #1092 + #1093 (pattern-matching trio).
//!
//! GNU ground truth (bash 5.3.20, script-file mode, byte-compared).
//!
//! - #1091: `${var//*(pat)/rep}` replaces the EMPTY match at every gap
//!   (subst.c pat_subst global loop: match_upattern MATCH_ANY tries p1
//!   down to p itself, pat_subst:9286-9304 copies one char past a
//!   zero-length match). `${w//*(a)/Q}` on `abac` is `QQbQQc`, not `QbQc.
//! - #1092: the unquoted `[[ == ]]` RHS never undergoes quote removal
//!   (execute_cmd.c COND_BINARY patmatch arm, subst.c:4077
//!   cond_expand_word special=1), so `\\` stays a literal backslash for
//!   the matcher (sm_loop.c:121): `[[ ab == a\\b ]]` fails (rc=1) while
//!   `[[ 'a\b' == a\\b ]]` matches (rc=0).
//! - #1093: the script-file reader is incremental (shell.c reader_loop):
//!   a runtime `shopt -s extglob` (via eval) opens the parse-time extglob
//!   gate for LATER commands (shopt.def extglob_flag, parse.y:3502
//!   reset_parser), so a later `case` with `+(...)` arms parses. A `-c`
//!   string is one parse_and_execute call and keeps the upfront parse
//!   (GNU errors there too) — only script files take the grouped driver.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script(body: &str) -> (String, String, Option<i32>) {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "rubash-trio-{}.sh",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    {
        let mut file = std::fs::File::create(&path).expect("temp script");
        // LF only: the repo pins *.sh eol=lf (.gitattributes).
        file.write_all(body.as_bytes()).expect("write script");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1091_global_replace_matches_empty_extglob_spans() {
    let (stdout, stderr, rc) = run_script(
        "shopt -s extglob\nw='abac'; echo \"[${w//*(a)/Q}]\"\nv='x'; echo \"[${v//*(y)/Q}]\"\n",
    );
    assert_eq!(stdout, "[QQbQQc]\n[Qx]\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}

#[test]
fn issue1092_cond_rhs_double_backslash_is_literal() {
    let (stdout, _stderr, rc) = run_script(
        "if [[ ab == a\\\\b ]]; then echo T1; else echo F1; fi\nif [[ 'a\\b' == a\\\\b ]]; then echo T2; else echo F2; fi\n",
    );
    assert_eq!(stdout, "F1\nT2\n");
    assert_eq!(rc, Some(0));
}

#[test]
fn issue1093_runtime_shopt_enables_later_case_extglob() {
    let (stdout, stderr, rc) = run_script(
        "c() { if eval \"$2\"; then echo \"T: $1\"; else echo \"F: $1\"; fi; }\nc 'x' 'shopt -s extglob; [[ aaab == +(a)b ]]'\ncase aaab in +(a)b) echo MATCH;; *) echo NOMATCH;; esac\n",
    );
    assert_eq!(stdout, "T: x\nMATCH\n", "stderr: {stderr:?}");
    assert_eq!(rc, Some(0));
}
