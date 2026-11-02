//! rubash#2580 — `__RUBASH_SCRIPT_NAME` / `__RUBASH_CURRENT_LINE` dual
//! store: the canonical store is the executor's `env_vars` map, but six
//! builtins (alias, help, kill, shift, shopt, setattr) read the PROCESS
//! environment mirror, and that mirror was only written for commands
//! whose RAW words matched a 15-word whitelist
//! (`command_needs_process_line_env`). An indirectly invoked builtin
//! (`x=shift; $x bogus` — the raw word is `$x`, not `shift`) therefore
//! read a stale/missing mirror and printed its diagnostic with the
//! generic `rubash: ` prefix instead of `<script>: line <n>: `.
//!
//! GNU has exactly one `line_number`: a process-global C int written by
//! SET_LINE_NUMBER (execute_cmd.c:295) and read by every builtin's
//! error path (builtins/common.c builtin_error). The fix writes BOTH
//! storages on every line stamp (the #2564 COLUMNS both-storages shape)
//! and deletes the whitelist gate.
//!
//! Oracle expectation (GNU bash 5.3.20, script file):
//!   $x bogus  →  "<path>: line 2: shift: bogus: numeric argument required"

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_script(script: &str) -> (String, String, Option<i32>, PathBuf) {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash-2580-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(path.to_str().expect("utf8 path"))
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let out = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
        path.clone(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    out
}

#[test]
fn issue_2580_indirect_shift_diagnostic_carries_script_and_line() {
    let (stdout, stderr, _code, path) = run_script("x=shift\n$x bogus\necho done\n");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    let expected_prefix = format!("{}: line 2: shift:", path.display());
    assert!(
        stderr.contains(&expected_prefix),
        "indirectly invoked shift must see the live line store; \
         expected prefix {expected_prefix:?}, stderr: {stderr}"
    );
}

#[test]
fn issue_2580_direct_shift_diagnostic_still_carries_script_and_line() {
    // Control: the direct (previously whitelisted) spelling kept working.
    let (stdout, stderr, _code, path) = run_script("shift bogus\necho done\n");
    assert_eq!(stdout, "done\n", "stderr: {stderr}");
    let expected_prefix = format!("{}: line 1: shift:", path.display());
    assert!(
        stderr.contains(&expected_prefix),
        "expected prefix {expected_prefix:?}, stderr: {stderr}"
    );
}
