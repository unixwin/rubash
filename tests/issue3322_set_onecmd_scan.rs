//! rubash#3322 — `set_args_may_set_onecmd` truncated a flag bundle at
//! the first `'o'`, so `set -ot` (and `-oto`, `-ot <name>`, ...) was
//! never routed to the grouped reader and `onecmd` went unhonored on
//! the whole-file fast path (the flag itself was set — the executor
//! matches GNU — but the fast path has no unit boundary to consult).
//!
//! GNU mechanism (builtins/set.def:726-766, the apply loop): the loop
//! walks the bundle letter by letter; `'o'` takes its option name
//! from the NEXT WORD (set.def:737-762), never from the bundle
//! remainder, and after listing options (no next word) or setting the
//! named option it CONTINUES with the next letter. So a `'t'`
//! anywhere in the bundle is reached: `set -ot` sets onecmd AND
//! lists the option table (oracle 5.3.20: `FLAGS=htBc`, table
//! printed), exactly like `set -ov` sets verbose — the sibling
//! scanner's measured standard (`set_args_may_toggle_verbose`: "a
//! flag bundle carrying `v` in any position counts"). The old
//! `split('o')` truncation assumed the rest was swallowed as the
//! option name; it is not. The validation pass
//! (set.def:671-691, `internal_getopt` with optflags
//! `"+<flags>o;"`, flags.c:372-382, bashgetopt.c:104-131 where `';'`
//! means the option argument may be missing) does not reject `-ot`
//! either.
//!
//! Every expectation below was measured against GNU bash 5.3.20
//! (~/workspace/rubash-work/worktrees/issue-3308/third_party/bash/bash):
//! `set -t/-ot/-to/-oto/-ot onecmd/-ot verbose/-ot -v/-ot -o/-t foo`
//! all give `FLAGS=htBc` (or `htvBc` with verbose); `set -o` gives
//! `hBc` and only lists; `set -ot foo` / `set -o t` give `hBc` with
//! rc 2 (`invalid option name` — the `o` arm returns before the
//! `'t'` is reached); `set -ti` / `set -it` give `hBc` with rc 2
//! (the validation pass rejects `-i` before the apply loop runs).
//! The last three are router OVER-ADMISSIONS by design (a false
//! positive only costs the incremental driver, same class as the
//! verbose scanner's documented `-ovi`): the scan must not
//! under-admit, so they are pinned true here with the reason
//! recorded, not silently left to invite another truncation.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use rubash::script_driver::script_may_set_onecmd;

/// Forms where GNU sets onecmd: `'t'` anywhere in a `-` bundle
/// counts, before or after `'o'`, with or without a following word.
#[test]
fn onecmd_forms_route_to_grouped_reader() {
    for form in [
        "set -t",
        "set -ot",
        "set -to",
        "set -oto",
        "set -ot onecmd",
        "set -ot verbose",
        "set -ot -v",
        "set -ot -o",
        "set -t foo",
        "set -o onecmd",
        "set -o onecmd; echo x",
        "echo hi; set -ot",
        "  set -ot",
        "set -ot # comment",
        "set -tx",
        "set -et",
    ] {
        assert!(
            script_may_set_onecmd(form),
            "GNU sets onecmd for `{form}`, the scan must route it"
        );
    }
}

/// Forms where GNU does not set onecmd: no `'t'` in any `-` bundle,
/// or the `'t'` is a separate word (an option NAME, not a flag).
#[test]
fn non_onecmd_forms_do_not_route() {
    for form in [
        "set -o",
        "set -o t",
        "set -e",
        "set -v",
        "set -ov",
        "set -o verbose",
        "echo hi",
        "set +t",
        "set +ot",
    ] {
        assert!(
            !script_may_set_onecmd(form),
            "GNU does not set onecmd for `{form}`, the scan must not route it"
        );
    }
}

/// Documented over-admissions: GNU does NOT set onecmd here, but
/// the router admits them anyway — the scan is a routing whitelist
/// that may over-admit and must never under-admit, and modelling
/// the early returns (invalid `-o` name, validation rejecting `-i`)
/// would mean reimplementing `set_minus_o_option`'s table and the
/// `internal_getopt` pass in a text scan. Pinned true so the
/// boundary stays explicit and nobody "tightens" it back into a
/// truncation.
#[test]
fn documented_over_admissions_stay_routed() {
    for form in [
        // set.def:751-757: `-o` with an invalid next-word name
        // returns before the bundle's `'t'` is reached.
        "set -ot foo",
        // set.def:671-691: the validation pass rejects `-i` before
        // the apply loop runs, so the `'t'` is never applied.
        "set -ti",
        "set -it",
    ] {
        assert!(
            script_may_set_onecmd(form),
            "router over-admits `{form}` by design (see header comment)"
        );
    }
}

fn case_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3322_{tag}_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn run_script(body: &str) -> (i32, String, String) {
    let dir = case_dir("script");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let status = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    (status, stdout, stderr)
}

/// End to end: with the scan fixed, `set -ot` routes to the grouped
/// reader, which honors onecmd — the table prints, then the reader
/// stops before the next unit (eval.c:206-207). On the unpatched
/// tree the script takes the fast path and LINE2/LINE3 run.
#[test]
fn set_ot_script_honors_onecmd_end_to_end() {
    let (status, stdout, stderr) = run_script("set -ot\necho LINE2\necho LINE3\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        stdout.contains("onecmd"),
        "the `-o` arm still lists the option table (stdout: {stdout:?})"
    );
    assert!(
        !stdout.contains("LINE2") && !stdout.contains("LINE3"),
        "onecmd must stop the reader after the `set -ot` unit (stdout: {stdout:?})"
    );
}

/// Control: `set -to` already routed before the fix, so this is
/// green on both trees — it pins the grouped reader's behavior,
/// not the scan.
#[test]
fn set_to_script_control() {
    let (status, stdout, stderr) = run_script("set -to\necho LINE2\necho LINE3\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        !stdout.contains("LINE2") && !stdout.contains("LINE3"),
        "onecmd must stop the reader after the `set -to` unit (stdout: {stdout:?})"
    );
}
