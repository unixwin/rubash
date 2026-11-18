//! rubash#3410 — `set -- -t` still routes `-t` as an option after `--`
//! (over-acceptance in the script-reader scan).
//!
//! GNU mechanism (builtins/set.def:671-691 pre-scan, 716-772 apply loop,
//! flags.c:372-382): option interpretation stops at the first `--` (and
//! at a lone `-`, and at the first non-option word GNU's getopt would not
//! consume as a `-o` operand). Everything after `--` is a positional
//! parameter: `set -- -t` leaves `$1` as `-t` with no option touched,
//! `set -- -o onecmd` leaves `$1`/`$2` as `-o`/`onecmd`.
//!
//! The defect was in the reader scan (`src/script_driver.rs`), not the
//! executor: `set_args_may_set_onecmd` (and its verbose/history siblings)
//! walked every `-` bundle on the line without ever stopping at `--`,
//! so a `set -- -t` script was routed to the grouped reader as if it
//! set onecmd. The executor itself already assigns `$1 = -t` and
//! leaves onecmd off in the `-c`, stdin and script drivers (verified
//! on baseline 8100d76), so the routing assertions below are the
//! fail-first proof and the end-to-end tests are guards that the
//! routing change never disturbs the (already correct) semantics.
//!
//! Every expectation below was measured against GNU bash 5.2.21
//! (script-file probes: a script stops after the `set` unit iff onecmd
//! was really set; `set -- -t`, `set -- -ot`, `set -e -- -t`,
//! `set - -t`, `set -- -o onecmd`, `set -- -v`, `set -- -H` all run
//! every later line with the words as positionals, while `set -t`,
//! `set -o onecmd -- -t`, `set -- -t; set -t`, `set -o -t`,
//! `set -o onecmd -t` all stop the reader). The audited set covers the
//! onecmd scan, the sibling verbose (`-v`) scan, the `-o` long-name
//! path (`set -- -o onecmd` / `set -- -o verbose`) and the history
//! scan (`set -- -o history` / `set -- -H`).

use std::path::PathBuf;
use std::process::{Command, Stdio};

use rubash::script_driver::{
    script_may_set_onecmd, script_may_toggle_verbose, script_uses_history,
};

/// Forms where GNU sets no onecmd: `--` (or a lone `-`, or the first
/// non-option word) ends option interpretation, so a later `-t`
/// bundle is a positional, never a flag.
#[test]
fn double_dash_stops_onecmd_scan() {
    for form in [
        "set -- -t",
        "set -- -ot",
        "set -e -- -t",
        "set - -t",
        "set -- -o onecmd",
        "set -- foo -t",
        "set foo -t",
        "set -e foo -t",
        "set -- -t; set -- -t",
        "echo set -- -t",
        // The `-o` long-name path: the name is a positional after `--`,
        // and a comment mentioning it past `--` does not revive it.
        "set -- -t # onecmd",
        "set -- -o onecmd # comment",
        // No `set` command at all: a bare mention of the name in a
        // comment or an argument cannot set the option either.
        "echo onecmd",
        "# onecmd",
    ] {
        assert!(
            !script_may_set_onecmd(form),
            "GNU sets no onecmd for `{form}`, the scan must not route it"
        );
    }
}

/// Controls: GNU really does set onecmd here, so the scan must keep
/// routing — including the `-o` operand forms and a real `set -t`
/// after an earlier `--` terminated command.
#[test]
fn onecmd_forms_still_route() {
    for form in [
        "set -t",
        "set -ot",
        "set -to",
        "set -t -- -x",
        "set -o onecmd",
        "set -o onecmd; echo x",
        "set -oonecmd",
        "set -o -t",
        "set -o onecmd -t",
        "set -oo onecmd verbose -t",
        "set -- -t; set -t",
        "set -e; set -t",
        "set -t foo -t",
        "set -o onecmd -- -t",
    ] {
        assert!(
            script_may_set_onecmd(form),
            "GNU sets onecmd for `{form}`, the scan must route it"
        );
    }
}

/// Sibling bundle letter: `--` ends option interpretation for the
/// verbose scan too — `set -- -v` toggles nothing.
#[test]
fn double_dash_stops_verbose_scan() {
    for form in [
        "set -- -v",
        "set -- -ov",
        "set -e -- -v",
        "set - -v",
        "set -- -o verbose",
        "set -- foo -v",
        "set foo -v",
        "set -- -v # verbose",
        "echo verbose",
        "# verbose",
    ] {
        assert!(
            !script_may_toggle_verbose(form),
            "GNU toggles no verbose for `{form}`, the scan must not route it"
        );
    }
}

/// Verbose controls: a `v` bundle (either sign) or a `-o verbose`
/// operand before any `--` still toggles verbose and must route.
#[test]
fn verbose_forms_still_route() {
    for form in [
        "set -v",
        "set -ov",
        "set +v",
        "set -v -- -t",
        "set -o verbose",
        "set -o verbose -- -v",
        "set +v -- -v",
        "set -o -v",
    ] {
        assert!(
            script_may_toggle_verbose(form),
            "GNU toggles verbose for `{form}`, the scan must route it"
        );
    }
}

/// History scan: `--` ends option interpretation, so `set -- -H` and
/// `set -- -o history` enable nothing.
#[test]
fn double_dash_stops_history_scan() {
    for form in [
        "set -- -H",
        "set -- -o history",
        "set -- -o histexpand",
        "set - -H",
        "set -- foo -H",
        "set foo -H",
    ] {
        assert!(
            !script_uses_history(form),
            "GNU enables no history for `{form}`, the scan must not route it"
        );
    }
}

/// History controls: `-H`, a `-o history`/`histexpand` operand, and
/// a bundle-`o` operand (`set -eo history`, `set -o -H`) still enable
/// history and must route.
#[test]
fn history_forms_still_route() {
    for form in [
        "set -H",
        "set -o history",
        "set -o histexpand",
        "set -o history -- -t",
        "set -eo history",
        "set -o -H",
    ] {
        assert!(
            script_uses_history(form),
            "GNU enables history for `{form}`, the scan must route it"
        );
    }
}

fn case_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue3410_{tag}_{}_{}", std::process::id(), seq));
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

/// Guard (green before and after the fix — the executor was already
/// right): `set -- -t` leaves `-t` as `$1` and the script runs on.
#[test]
fn set_double_dash_t_is_positional_end_to_end() {
    let (status, stdout, stderr) =
        run_script("set -- -t\necho \"[$1] [$#]\"\necho LINE2\necho LINE3\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        stdout.contains("[-t] [1]"),
        "`-t` must land in $1 untouched (stdout: {stdout:?})"
    );
    assert!(
        stdout.contains("LINE2") && stdout.contains("LINE3"),
        "no onecmd may stop the reader (stdout: {stdout:?})"
    );
}

/// Guard: `set -- -o onecmd` leaves both words as positionals.
#[test]
fn set_double_dash_o_onecmd_is_positional_end_to_end() {
    let (status, stdout, stderr) = run_script("set -- -o onecmd\necho \"[$1] [$2]\"\necho LINE2\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        stdout.contains("[-o] [onecmd]"),
        "`-o onecmd` must land in $@ untouched (stdout: {stdout:?})"
    );
    assert!(
        stdout.contains("LINE2"),
        "no onecmd may stop the reader (stdout: {stdout:?})"
    );
}

/// Guard: `set -- -v` toggles no verbose, so the input lines are not
/// echoed and the script runs on.
#[test]
fn set_double_dash_v_toggles_nothing_end_to_end() {
    let (status, stdout, stderr) = run_script("set -- -v\necho hi\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert_eq!(stdout, "hi\n", "verbose must stay off (stdout: {stdout:?})");
}

/// Control: onecmd set before `--` still stops the reader.
#[test]
fn set_o_onecmd_then_double_dash_stops_end_to_end() {
    let (status, stdout, stderr) = run_script("set -o onecmd -- -t\necho LINE2\necho LINE3\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        !stdout.contains("LINE2") && !stdout.contains("LINE3"),
        "onecmd set before `--` must stop the reader (stdout: {stdout:?})"
    );
}

/// Control: a real `set -t` after an earlier `--` terminated command
/// still stops the reader.
#[test]
fn set_double_dash_t_then_real_t_stops_end_to_end() {
    let (status, stdout, stderr) = run_script("set -- -t; set -t\necho LINE2\necho LINE3\n");
    assert_eq!(status, 0, "status (stdout: {stdout:?}, stderr: {stderr})");
    assert!(
        !stdout.contains("LINE2") && !stdout.contains("LINE3"),
        "the later `set -t` must stop the reader (stdout: {stdout:?})"
    );
}
