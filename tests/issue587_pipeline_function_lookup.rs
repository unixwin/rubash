//! rubash#587: a PIPELINE MEMBER that names a shell function fell through to
//! the PATH external command instead of running the function.
//!
//! Root cause (two dispatch sites skipped the function table):
//! 1. `execute_pipeline_stage_inner` (src/executor/pipeline_exec.rs) matched
//!    the stage's first word against its hardcoded ordinary-builtin arms and
//!    `execute_builtin_pipeline_stage` before any function lookup, so
//!    `echo(){ ...; }; echo hi | cat` ran the builtin echo — while GNU runs
//!    the function for every non-special name. The default arm below those
//!    arms did run functions, which is why non-pipeline calls and
//!    non-builtin-named sequential stages looked fine.
//! 2. The concurrent native-pipeline fast paths
//!    (`concurrent_external_stage_spec`, Windows; the unix twin loop in
//!    `execute_external_pipeline_concurrently`) classified members with
//!    `is_shell_builtin_name` only and handed everything else straight to
//!    `find_user_command`, so `date(){ echo FAKE; }; date | cat` executed
//!    the real date(1) and `t(){ ...; }; t git log | wc -l` silently ran a
//!    0-byte `t` stub found on PATH.
//!
//! GNU contract: execute_pipeline (execute_cmd.c:2620) routes every element
//! through execute_command -> execute_simple_command, which resolves
//! functions BEFORE the disk fallback — special builtins first
//! (execute_cmd.c:4657-4663), then `func = find_function(...)`
//! (execute_cmd.c:4674-4677), execute_builtin_or_function dispatch
//! (execute_cmd.c:4837-4861), and execute_from_filesystem /
//! execute_disk_command (execute_cmd.c:4919-4925) only when both lookups
//! missed. All expectations below verified against WSL GNU bash 5.3.0 via
//! script-file probes (target/probe587/g2.sh, 2026-10-13): `date | cat`
//! prints FAKE with PIPESTATUS `0 0`, `t git log | wc -l` prints 1, and
//! functions shadowing echo/wc/grep run as functions inside pipelines.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

fn run_rubash_script(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                use std::io::Read;
                child
                    .stdout
                    .take()
                    .expect("stdout pipe")
                    .read_to_end(&mut stdout)
                    .expect("read stdout");
                child
                    .stderr
                    .take()
                    .expect("stderr pipe")
                    .read_to_end(&mut stderr)
                    .expect("read stderr");
                return (stdout, stderr, status.code());
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not exit within {DEADLINE:?}: {script}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("wait rubash: {error}"),
        }
    }
}

/// The exact issue repro: a function named like a PATH external must run as
/// the producer of a two-member pipeline (GNU: FAKE, PIPESTATUS `0 0`).
#[test]
fn issue_repro_function_pipeline_member_runs_function() {
    let (stdout, _, code) = run_rubash_script(r"date(){ echo FAKE; }; date | cat");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FAKE\n");
}

/// PIPESTATUS records both members' statuses across the function producer.
#[test]
fn function_producer_pipestatus_is_reported() {
    let (stdout, _, code) =
        run_rubash_script(r#"date(){ echo FAKE; }; date | cat; echo "${PIPESTATUS[*]}""#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FAKE\n0 0\n");
}

/// The issue's second repro shape: a function whose PATH stub would win on
/// affected hosts (`t(){ ...; }; t git log | wc -l`) must run the function
/// as a prefix member, with the consumer receiving its output.
#[test]
fn function_prefix_member_feeds_downstream_stage() {
    let (stdout, _, code) = run_rubash_script(r"t(){ echo FROM-T; }; t git log | wc -l");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"1\n");
}

/// A function reached through an expanded first word ($v) is still found in
/// a pipeline member — GNU resolves find_function on the EXPANDED word
/// (execute_cmd.c:4674-4677, after expand_words).
#[test]
fn function_via_expanded_command_word_runs_in_pipeline() {
    let (stdout, _, code) = run_rubash_script(r"v=date; date(){ echo FAKE; }; $v | cat");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FAKE\n");
}

/// A user-defined function shadows the ordinary builtin `echo` inside a
/// pipeline member (GNU find_function precedes ordinary-builtin lookup;
/// func5.sub's default-mode shadowing, pipeline flavor).
#[test]
fn function_shadows_builtin_echo_in_pipeline() {
    let (stdout, _, code) =
        run_rubash_script(r#"echo(){ printf '%s\n' FUNC-ECHO; }; echo hi | cat"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FUNC-ECHO\n");
}

/// Same shadowing rule for the hardcoded consumer arms: `wc` (GNU runs the
/// function, printing FUNC-WC; the pipeline status is the function's 0).
#[test]
fn function_shadows_builtin_wc_consumer() {
    let (stdout, _, code) = run_rubash_script(r#"wc(){ printf '%s\n' FUNC-WC; }; echo x | wc -l"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FUNC-WC\n");
}

/// And for the inline `cat` consumer arm.
#[test]
fn function_shadows_builtin_cat_consumer() {
    let (stdout, _, code) =
        run_rubash_script(r#"cat(){ printf '%s\n' FUNC-CAT; }; printf 'x\n' | cat"#);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FUNC-CAT\n");
}

/// The same classification in posix mode: `date` is not a special builtin,
/// so the function still shadows it (GNU execute_cmd.c:4657-4663 precludes
/// function lookup only for special builtins under posix).
#[test]
fn posix_mode_function_pipeline_member_still_runs_function() {
    let (stdout, _, code) = run_rubash_script(r"set -o posix; date(){ echo FAKE; }; date | cat");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FAKE\n");
}

/// A genuine external with no function/builtin shadow still runs from PATH
/// through a pipeline (the fix must not invert the dispatch): `date | cat`
/// without the function prints the system date, so assert on a
/// deterministic external instead — a missing command keeps GNU's 127
/// diagnostic on the producer stage.
#[test]
fn unshadowed_missing_external_pipeline_member_still_not_found() {
    let (stdout, stderr, code) = run_rubash_script(r"nosuchcmd587q | cat");
    // The pipeline status is the last member's (cat: 0); the producer's
    // diagnostic reaches stderr and its 127 lands in PIPESTATUS[0].
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"");
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        stderr.contains("nosuchcmd587q") && stderr.contains("command not found"),
        "unexpected stderr: {stderr}"
    );
}

/// The non-pipeline call keeps running the function (control: the fix must
/// not disturb plain command dispatch).
#[test]
fn non_pipeline_function_call_unchanged() {
    let (stdout, _, code) = run_rubash_script(r"date(){ echo FAKE; }; date");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"FAKE\n");
}

/// Function producer in a three-member pipeline (the concurrent-prefix
/// admission shape): the sequential loop owns it after the bail-out, and
/// every member still runs in order.
#[test]
fn function_member_in_three_member_pipeline() {
    let (stdout, _, code) = run_rubash_script(r"f587(){ echo hi; }; f587 | grep -c h | cat");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, b"1\n");
}
