//! `declare -p` mixed diagnostic/output must keep its per-operand order
//! on a merged stream (rubash#3408; follow-up to #3324).
//!
//! GNU `builtins/declare.def:383-406` walks the `-p` operands in order:
//! each name is shown by `show_name_attributes` (setattr.def:539-552,
//! `printf` to stdout) or reported at once by `sh_notfound`
//! (common.c:190-193 -> `builtin_error`, vfprintf to stderr); both
//! streams are line-buffered (shell.c:1921-1922), so on one merged
//! stream the diagnostic for operand N precedes the listing for
//! operand N+1. Rubash recorded the `not found` diagnostic in the
//! `Diag` pending queue (flushed only after the builtin returned)
//! and emitted the builtin's two buffers stdout-block-then-stderr-
//! block (`write_buffered_builtin_output`), so the listing overtook
//! the diagnostic. The pins drive script FILES (diagnostic prefixes
//! carry the script path; the harness normalizes it to `S`) and
//! assert stdout/stderr byte-for-byte. Expectations verified against
//! GNU bash 5.3.20 with script files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Every spelling the shell may use for this script path. On Windows the
/// harness has the Win32 form (`C:\a\b\case.sh`) while rubash's diagnostics
/// carry the drive form (`/c/a/b/case.sh`); both must normalize to `S` or the
/// pins false-fail on path prefix alone (rubash#3408 round 3).
fn path_spellings(path: &Path) -> Vec<String> {
    let native = path.to_str().expect("utf8 path").to_string();
    let mut spellings = vec![native.clone(), native.replace('\\', "/")];
    let bytes = native.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let tail = native[3..].replace('\\', "/");
        spellings.push(format!("/{drive}/{tail}"));
    }
    spellings.sort_by_key(|s| std::cmp::Reverse(s.len()));
    spellings.dedup();
    spellings
}

fn rubash_bin() -> String {
    env!("CARGO_BIN_EXE_rubash").to_string()
}

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    #[allow(dead_code)]
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
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        for spelling in path_spellings(&path) {
            text = text.replace(&spelling, "S");
        }
        text
    };
    Run {
        stdout: normalize(&output.stdout),
        stderr: normalize(&output.stderr),
        code: output.status.code(),
        dir,
    }
}

const DIAG: &str = "S: line 2: declare: BADNAME: not found\n";
const LISTING: &str = "declare -- v=\"1\"\n";

#[test]
fn issue3408_declare_p_pipeline_diag_first() {
    // The issue's canonical shape: the diagnostic for the first
    // operand must precede the listing for the second.
    let run = run_script(
        "canon",
        "v=1\ndeclare -p BADNAME v 2>&1 | cat; echo rc=$?\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}rc=0\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_pipeline_list_first() {
    // The reverse order guards against an "always diagnostic first"
    // hack: production order is the operand order, either way.
    let run = run_script("list-first", "v=1\ndeclare -p v BADNAME 2>&1 | cat\n");
    assert_eq!(run.stdout, format!("{LISTING}{DIAG}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_pipeline_sandwich() {
    let run = run_script(
        "sandwich",
        "v=1\nw=hello\ndeclare -p BADNAME v BADNAME2 w 2>&1 | cat\n",
    );
    assert_eq!(
        run.stdout,
        "S: line 3: declare: BADNAME: not found\n".to_string()
            + LISTING
            + "S: line 3: declare: BADNAME2: not found\n"
            + "declare -- w=\"hello\"\n"
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_pipeline_multi_stage() {
    let run = run_script("multi", "v=1\ndeclare -p BADNAME v 2>&1 | cat | cat\n");
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_typeset_p_shares_the_merged_order() {
    // `typeset -p` runs the same declare-family dispatch; the
    // diagnostic carries the invoked name.
    let run = run_script("typeset", "v=1\ntypeset -p BADNAME v 2>&1 | cat\n");
    assert_eq!(
        run.stdout,
        "S: line 2: typeset: BADNAME: not found\n".to_string() + LISTING
    );
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_bare_merged_redirect_diag_first() {
    // No pipeline: `2>&1` alone merges the streams onto the same
    // capture, and the same per-operand order applies; the exit
    // status stays the builtin's own failure.
    let run = run_script(
        "bare-merged",
        "v=1\ndeclare -p BADNAME v 2>&1; echo rc=$?\n",
    );
    assert_eq!(run.stdout, format!("{DIAG}{LISTING}rc=1\n"));
    assert_eq!(run.stderr, "");
    assert_eq!(run.code, Some(0));
}

#[test]
fn issue3408_declare_p_split_streams_unchanged() {
    // Guard: when the streams are NOT merged, the listing stays on
    // stdout and the diagnostic stays on stderr (the fix must not
    // merge streams that the user kept apart).
    let run = run_script("split", "v=1\ndeclare -p BADNAME v; echo rc=$?\n");
    assert_eq!(run.stdout, format!("{LISTING}rc=1\n"));
    assert_eq!(run.stderr, DIAG);
    assert_eq!(run.code, Some(0));
}
