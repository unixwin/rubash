//! `declare -p` must emit per-name diagnostics and listings in
//! argument order on a merged stream (rubash#3408; GNU bash 5.2.21
//! oracle-verified 2026-11-16).
//!
//! GNU `declare -p BADNAME v 2>&1 | cat` prints the `BADNAME`
//! diagnostic BEFORE the `v` listing: the builtin emits each name's
//! output or diagnostic sequentially as it walks the operands
//! (builtins/setattr.def `set_or_show_attributes`, each name either
//! printed to stdout or reported via `builtin_error` to stderr
//! immediately). Rubash recorded the `not found` diagnostic in the
//! `Diag` pending queue (`diag.free`) and only rendered it when the
//! dispatcher flushed after the call — so every listing overtook
//! every diagnostic on a merged stream. The pins drive script FILES
//! (diagnostic prefixes carry the script path; the harness
//! normalizes it to `S`) and assert stdout/stderr byte-for-byte.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    dir: PathBuf,
}

/// Run a script file; return normalized stdout/stderr, exit code, and
/// the temp dir (for asserting files the script created).
fn run_script(tag: &str, body: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("issue3408-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("case.sh");
    fs::write(&path, body).expect("write script");
    let output = Command::new(rubash_bin())
        .arg(&path)
        .env("LC_ALL", "C.UTF-8")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(path.to_str().expect("utf8 path"), "S")
    };
    Run {
        stdout: normalize(&output.stdout),
        stderr: normalize(&output.stderr),
        code: output.status.code(),
        dir,
    }
}

const DIAG_BADNAME: &str = "S: line 2: declare: BADNAME: not found\n";
const DIAG_BAD1: &str = "S: line 2: declare: BAD1: not found\n";
const DIAG_BAD2: &str = "S: line 2: declare: BAD2: not found\n";
const LISTING_V: &str = "declare -- v=\"1\"\n";
const LISTING_W: &str = "declare -- w=\"2\"\n";

#[test]
fn issue3408_declare_p_merged_diag_first() {
    // The issue's canonical shape: the diagnostic for the first
    // operand must precede the listing for the second.
    let run = run_script("canon", "v=1\ndeclare -p BADNAME v 2>&1 | cat\n");
    assert_eq!(run.stdout, format!("{DIAG_BADNAME}{LISTING_V}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_merged_list_first() {
    // The reverse order guards against an "always diagnostic first"
    // hack: production order is the operand order, either way.
    let run = run_script("list-first", "v=1\ndeclare -p v BADNAME 2>&1 | cat\n");
    assert_eq!(run.stdout, format!("{LISTING_V}{DIAG_BADNAME}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_merged_sandwich() {
    let run = run_script("sandwich", "v=1\nw=2\ndeclare -p v BADNAME w 2>&1 | cat\n");
    // The `declare -p` is on line 3 of this script, so the
    // diagnostic names line 3, not line 2.
    assert_eq!(
        run.stdout,
        format!("{LISTING_V}S: line 3: declare: BADNAME: not found\n{LISTING_W}")
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_merged_two_bad_then_good() {
    let run = run_script("two-bad", "v=1\ndeclare -p BAD1 BAD2 v 2>&1 | cat\n");
    assert_eq!(run.stdout, format!("{DIAG_BAD1}{DIAG_BAD2}{LISTING_V}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_merged_typeset_alias() {
    // `typeset -p` shares the same per-name printer, so the same
    // ordering rule applies.
    let run = run_script("typeset", "v=1\ntypeset -p BADNAME v 2>&1 | cat\n");
    assert_eq!(
        run.stdout,
        "S: line 2: typeset: BADNAME: not found\n".to_string() + LISTING_V
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_status_and_split_streams() {
    // Unmerged, the listing stays on stdout and the diagnostic on
    // stderr, and any missing name still fails the builtin (GNU
    // exit code 1).
    let run = run_script("split", "v=1\ndeclare -p BADNAME v\n");
    assert_eq!(run.stdout, LISTING_V);
    assert_eq!(run.stderr, DIAG_BADNAME);
    assert_eq!(run.code, Some(1));
}

#[test]
fn issue3408_declare_p_merged_file_redirect() {
    // The non-pipeline merged shape: both streams to one file must
    // keep operand order too (the top-level buffered route).
    let run = run_script("file", "v=1\ndeclare -p BADNAME v > out.txt 2>&1\n");
    let raw = fs::read_to_string(run.dir.join("out.txt")).expect("read out.txt");
    let script = run.dir.join("case.sh");
    let merged = raw.replace(script.to_str().expect("utf8 path"), "S");
    assert_eq!(merged, format!("{DIAG_BADNAME}{LISTING_V}"));
    assert_eq!(run.stdout, "");
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(1));
}

#[test]
fn issue3408_declare_p_merged_comsub() {
    // Inside `$(...)`, a body-level `2>&1` merges the diagnostic
    // into the capture in operand order (the trailing newline is
    // stripped by the substitution and re-added by printf).
    let run = run_script(
        "comsub",
        "v=1\nout=$(declare -p BADNAME v 2>&1)\nprintf '%s\\n' \"$out\"\n",
    );
    assert_eq!(run.stdout, format!("{DIAG_BADNAME}{LISTING_V}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}
