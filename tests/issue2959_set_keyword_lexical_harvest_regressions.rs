//! rubash#2959 (Stage-2 ④ Phase 3 E1 residual) — `set -k` harvests
//! assignment-shaped words anywhere in a command line into the
//! command's temporary environment (GNU `place_keywords_in_env`).
//! Whether a word is an assignment word is a LEXICAL verdict, taken
//! on the raw token at parse time: parse.y stamps W_ASSIGNMENT via
//! `assignment(token)` (general.c:480) on the token as read — quotes
//! included — and the subst.c sweep moves exactly the stamped words.
//! A quoted word is therefore never an assignment word, however
//! assignment-shaped its cooked text looks.
//!
//! Pre-fix rubash judged each word of `cmd.words` — the COOKED token
//! text, quotes stripped — with `split_assignment_word`, so a quoted
//! `"v=$v"` or `"a=b c"` was harvested out of the command line: the
//! command ran with the word deleted and a spurious temporary
//! assignment installed. The misjudgment also ate the OBSERVATION
//! words of the bare-assignment face (`echo "v=$v"` printed nothing),
//! which is why the issue's second face looked like a persistence
//! failure: bare assignments under `-k` persist correctly once the
//! observation word survives.
//!
//! Every expectation below was measured against GNU bash 5.3.20.
//! Guard cells (expansion results, plain trailing assignments, the
//! leading run) already matched GNU pre-fix and pin the boundary so
//! the raw-token judgment cannot over- or under-harvest.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2959_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

fn assert_case(body: &str, expected_stdout: &str) {
    let run = run_script(body);
    assert_eq!(
        run.status, 0,
        "status for {body:?} (stderr: {})",
        run.stderr
    );
    assert_eq!(run.stdout, expected_stdout, "stdout for {body:?}");
    let _ = std::fs::remove_dir_all(&run.dir);
}

/// The issue reproducer: a quoted word whose cooked text is
/// assignment-shaped with a space is an ordinary argument.
#[test]
fn quoted_word_with_space_is_not_harvested() {
    assert_case("set -k\necho \"a=b c\"\necho end\n", "a=b c\nend\n");
}

/// The lexical pair: the SAME text unquoted is a raw assignment
/// token and is harvested (echo runs with no arguments); quoted, it
/// is an argument and expands normally.
#[test]
fn quoted_assignment_shaped_word_survives_unquoted_is_harvested() {
    assert_case("v=9\nset -k\necho v=$v\necho \"v=$v\"\n", "\nv=9\n");
}

/// Guard: a plain trailing assignment word still lands in the
/// callee's temporary environment and does not persist.
#[test]
fn trailing_assignment_reaches_function_tempenv() {
    assert_case(
        "f() { echo \"in:$y\"; }\nset -k\nf y=2\necho \"after:${y-unset}\"\n",
        "in:2\nafter:unset\n",
    );
}

/// The issue's second face: a bare assignment under `-k` is
/// persistent (an assignment-only command keeps GNU's permanent
/// semantics); the observation word must survive to show it.
#[test]
fn bare_assignment_persists_under_keyword() {
    assert_case("set -k\nv=9\necho \"v=$v\"\n", "v=9\n");
}

/// Guard: a word that only BECOMES assignment-shaped through
/// expansion was never stamped at read time — not harvested.
#[test]
fn expansion_result_is_not_harvested() {
    assert_case("w='a=b'\nset -k\necho $w\n", "a=b\n");
}

/// The harvested value is expanded like an assignment value
/// (arithmetic included) before it enters the temporary environment;
/// the function's own observation word is quoted and survives.
#[test]
fn harvested_value_is_expanded() {
    assert_case(
        "show() { echo \"z=$z\"; }\nset -k\nshow z=$((6*7))\necho \"after:${z-unset}\"\n",
        "z=42\nafter:unset\n",
    );
}

/// `name+=value` in the sweep appends to the caller's current value
/// for the temporary environment only; the shell's own variable is
/// untouched after the command.
#[test]
fn harvested_append_uses_current_value_for_tempenv_only() {
    assert_case(
        "q=pre\nshow() { echo \"q=$q\"; }\nset -k\nshow q+=post\necho \"shell:$q\"\n",
        "q=prepost\nshell:pre\n",
    );
}

/// Guard: with a command name present, the leading run and the
/// harvested remainder are BOTH temporary — nothing persists.
#[test]
fn leading_run_and_remainder_are_both_temporary() {
    assert_case(
        "set -k\nx=1 echo y=2\necho \"x:${x-unset}\"\n",
        "\nx:unset\n",
    );
}
