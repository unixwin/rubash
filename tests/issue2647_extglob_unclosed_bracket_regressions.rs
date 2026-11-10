//! rubash#2647 — an unbalanced `[` inside an extglob group is NOT a
//! parse error in GNU: the `[` is literal pattern text and the script
//! runs on. rubash reported `unexpected EOF while looking for
//! matching ']'` and silently truncated the rest of the script.
//!
//! GNU anchors: parse.y:5466-5489 read_token_word — with
//! extended_glob live, a PATTERN_CHAR (`@*+?!`, syntax.h:90-92)
//! followed by `(` hands the whole group to parse_matched_pair
//! (parse.y:3877); the group's `[` never opens a P_ARRAYSUB hunt
//! (parse.y:5635-5643 runs only for a bare-word subscript). At match
//! time an unclosed `[` is literal (lib/glob/sm_loop.c PATSCAN /
//! BRACKMATCH), never a parse error.
//!
//! Expectations measured against GNU bash 5.3.20 (2026-11-10),
//! script-file mode, in a directory containing abcx / abcz / bbc so
//! the patterns below have files to (not) match.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2647-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_clean(tag: &str, run: &Run, stdout: &str) {
    assert_eq!(run.stdout, stdout, "{tag}: stdout");
    assert_eq!(run.stderr, "", "{tag}: stderr");
    assert_eq!(run.code, Some(0), "{tag}: rc");
}

#[test]
fn issue2647_issue_script_runs_to_completion() {
    // The issue's exact script: all three pattern lines echo
    // literally (no matches) and the trailing line still runs —
    // no `]` EOF, no truncation.
    let run = run_file(
        "issue",
        "shopt -s extglob\n\
         touch abcx abcz bbc\n\
         echo !([*)*\n\
         echo +(a|b[)*\n\
         echo [a*(]*)z\n\
         echo after\n",
    );
    assert_clean("issue", &run, "!([*)*\n+(a|b[)*\n[a*(]*)z\nafter\n");
}

#[test]
fn issue2647_five_operators_closed_group_unclosed_bracket() {
    // All five group operators, group CLOSED, `[` unclosed inside:
    // the word is consumed whole and echoes verbatim.
    let run = run_file(
        "five-ops",
        "shopt -s extglob\n\
         echo @(a[b)\n\
         echo *(a[b)\n\
         echo +(a[b)\n\
         echo ?(a[b)\n\
         echo !(a[b)\n\
         echo alive\n",
    );
    assert_clean(
        "five-ops",
        &run,
        "@(a[b)\n*(a[b)\n+(a[b)\n?(a[b)\n!(a[b)\nalive\n",
    );
}

#[test]
fn issue2647_unclosed_bracket_midline_does_not_truncate_line() {
    // Same-line neighbours of the bracket-bearing word still run.
    let run = run_file(
        "midline",
        "shopt -s extglob\n\
         touch abcx\n\
         echo before; echo [a*(]*)z; echo after\n",
    );
    assert_clean("midline", &run, "before\n[a*(]*)z\nafter\n");
}
