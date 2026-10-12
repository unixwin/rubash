//! Issue rubash#533: an unterminated `${` inside an arithmetic body is
//! P_ARITH literal text (GNU parse.y parse_matched_pair: only the `${|`
//! funsub form nests), so it must neither swallow the arithmetic unit's own
//! closer at parse time nor abort the script at rc 2. GNU answers with an
//! expansion-stage `<string>: bad substitution` (subst.c bad_substitution
//! label, rc 1, command discarded, script continues); the unclosed forms
//! keep reading and name the arithmetic closer (`)` / `]`) at EOF.
//!
//! Rust semantic owners: lexer/brace_scan.rs param_residuals_advance (the
//! arithmetic-unit skip), lexer/continuation.rs unclosed_input_close_char_
//! posix + close_char_residuals_advance (P_ARITH pushes + `$[` unit),
//! executor/parameter_errors.rs + arithmetic/mod.rs (the expansion-stage
//! bad substitution).

#![cfg(windows)]

use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_script(lines: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue533-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue533 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, lines).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn stderr_of(out: &RunOutcome) -> String {
    out.stderr.lines().next().unwrap_or_default().to_string()
}

#[test]
fn closed_units_parse_and_report_expansion_bad_substitution() {
    // WSL GNU bash 5.3.0: `bash: line 1: 1+${x: bad substitution`, rc 1.
    for script in [
        "echo $((1+${x))\n",
        "echo $[1+${x]\n",
        "echo before\n((1+${x))\n",
    ] {
        let out = run_script(script);
        assert_eq!(
            out.code,
            Some(1),
            "rc for {script:?}: stderr={:?}",
            out.stderr
        );
        let err = stderr_of(&out);
        assert!(
            err.ends_with("1+${x: bad substitution"),
            "stderr for {script:?}: {err:?}"
        );
    }
}

#[test]
fn unclosed_units_name_the_arithmetic_closer() {
    // WSL GNU bash 5.3.0: `)' for $(( and ((, `]' for $[, rc 2.
    let cases = [
        ("echo $((1+${x", ')'),
        ("((1+${x", ')'),
        ("echo $[1+${x", ']'),
    ];
    for (script, closer) in cases {
        let out = run_script(&format!("{script}\n"));
        assert_eq!(out.code, Some(2), "rc for {script:?}");
        let err = stderr_of(&out);
        assert!(
            err.contains(&format!(
                "unexpected EOF while looking for matching `{closer}'"
            )),
            "stderr for {script:?}: {err:?}"
        );
    }
}

#[test]
fn terminated_forms_and_continuation_stay_correct() {
    let out = run_script("x=5\necho $((1+${x}))\n");
    assert_eq!(out.stdout, "6\n");
    assert_eq!(out.code, Some(0));

    let out = run_script("echo $((1+2))\n");
    assert_eq!(out.stdout, "3\n");

    // GNU discards only the failing command; the arithmetic-COMMAND form
    // continues with the next line (`(( ))` DISCARD scope, eval.c:98-107).
    let out = run_script("echo before\n((1+${x))\necho after\n");
    assert_eq!(out.stdout, "before\nafter\n");
    assert_eq!(out.code, Some(0));

    // Inside one `;` list GNU's DISCARD abandons the remaining list —
    // `echo after` on the SAME line does not run (matches GNU).
    let out = run_script("echo $((1+${x)) ; echo after\n");
    assert_eq!(out.stdout, "");
    assert_eq!(out.code, Some(1));
}
