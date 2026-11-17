//! rubash#3605 class B (run-func) — a `function NAME` definition whose
//! `{` sits on the FOLLOWING line must report an invalid identifier at
//! the definition's parse-end line (the `}` line), matching GNU
//! execute_cmd.c execute_intern_function's read of `line_number` after
//! the whole function_def command parses.
//!
//! The grouped script driver (chosen by script_runs_foreign_text's
//! textual `. `/`source`/`eval` markers — func5.sub's license comment
//! `... FOR A PARTICULAR PURPOSE.  See the` contains `". "`) feeds
//! pre-lexed tokens through run_source_impl's line_offset rebase. The
//! rebase shifted position/column/logical_line but left end_position
//! stale, so finish_compound_command's `max(position, end_position)`
//! (arithmetic_command.rs) collapsed onto the folded `{ ... }` token's
//! `{` line instead of its `}` line.
//!
//! Expectations verified against GNU bash 5.3.20 (script files,
//! 2026-11-15): `function sys$read` + `{`-on-next-line def reports the
//! `}` line on both the fast path and the grouped driver.

use std::io::Write;
use std::process::Command;

/// Run `script` as a script FILE and return (stdout, stderr lines with
/// the `<path>: ` prolog stripped to `line N: ...`).
fn run_script(script: &str) -> (String, Vec<String>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605-fn-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&path).expect("create probe");
    file.write_all(script.as_bytes()).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    let stderr = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(|line| match line.find(": line ") {
            Some(idx) => line[idx + 2..].to_string(),
            None => line.to_string(),
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        output.status.code(),
    )
}

const INVALID_ID: &str = "`sys$read': not a valid identifier";

/// Fast path (no foreign-text markers): `function NAME` + `{` on the
/// next line reports the `}` line.
#[test]
fn issue3605_function_keyword_def_reports_close_line() {
    let (_stdout, stderr, _rc) = run_script("x=1\nfunction sys$read\n{\n\tprintf \"x\\n\"\n}\n");
    assert_eq!(stderr, vec![format!("line 5: {INVALID_ID}")]);
}

/// Grouped driver: a `. ` anywhere in the text (here inside a comment,
/// like func5.sub's `PURPOSE.  See`) selects the grouped reader; the
/// same def must still report the `}` line, not the `{` line.
#[test]
fn issue3605_grouped_driver_def_reports_close_line() {
    let (_stdout, stderr, _rc) =
        run_script("# comment . trailing\nfunction sys$read\n{\n\tprintf \"x\\n\"\n}\n");
    assert_eq!(stderr, vec![format!("line 5: {INVALID_ID}")]);
}

/// The func5.sub layout: two valid `function NAME` defs before the
/// invalid one; later groups rebase tokens with a nonzero offset —
/// the stale end_position used to win max() on the wrong side.
#[test]
fn issue3605_grouped_driver_late_def_reports_close_line() {
    let (_stdout, stderr, _rc) = run_script(
        "# .\nfunction a=2\n{\n\tprintf \"x\\n\"\n}\n\nfunction 11111\n{\n\tprintf \"x\\n\"\n}\n\nfunction sys$read\n{\n\tprintf \"x\\n\"\n}\n",
    );
    assert_eq!(stderr, vec![format!("line 15: {INVALID_ID}")]);
}
