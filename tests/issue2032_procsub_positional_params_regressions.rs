//! Regression pins for rubash#2032 — bats ran ZERO tests under rubash:
//! bats-exec-suite builds its unique-filename array from `"$@"` inside
//! `< <(printf "%s\n" "$@" | nl | sort | uniq | sort -n | cut)`, and the
//! process-substitution body saw an EMPTY positional parameter list, so
//! the filename degenerated to the `nl` padding of an empty line
//! (`Testfile "       " not found`, 0 executed, rc=1).
//!
//! Root cause: the streamed input process substitution (rubash#1952,
//! commit 2cba5b6a) — like the async/subshell wrapper, the coproc child
//! and the output process substitution — runs the body in a RESPAWNED
//! `rubash -c` process. GNU runs every one of those bodies in a forked
//! child (subst.c:6362 process_substitute; execute_cmd.c:1576
//! execute_in_subshell; execute_cmd.c coproc make_child), and a fork
//! carries dollar_vars verbatim (variables.c). The respawn transported
//! env vars and functions but no positional parameters, so the body
//! saw `$#` = 0 and `"$@"` collapsed to one empty word. The collected
//! (in-process) fallback path was never affected — forcing the spawn
//! to fail reproduced GNU behavior exactly.
//!
//! The fix stamps the live positional window at every fork-shaped
//! spawn under `__RUBASH_POSITIONAL_PARAMS` (indexed-array storage
//! codec) and `Executor::new` consumes and strips it for internal
//! respawns (the `__RUBASH_BG_DEPTH` protocol shape). Every
//! expectation below is GNU bash 5.3.20 behavior, verified byte-for-byte
//! against the oracle before the fix was written.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2032-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `rubash -c <script> pin2032 <args...>` with a hard deadline; a
/// hang is a test failure, never a frozen suite.
fn rubash_c(
    dir: &PathBuf,
    script: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg("-c")
        .arg(script)
        .arg("pin2032")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn rubash");
    let deadline = Duration::from_secs(30);
    let t0 = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if t0.elapsed() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not finish within {deadline:?} for script: {script}");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_end(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_end(&mut err);
    }
    let status = child.wait().expect("wait");
    (
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
        status.code(),
    )
}

#[test]
fn issue2032_input_procsub_redirect_form_inherits_positional_params() {
    let dir = scratch("redir");
    let (out, err, code) = rubash_c(
        &dir,
        "cat < <(printf '<%s>\\n' \"$@\")",
        &["AAA", "BBB"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "<AAA>\n<BBB>\n");
}

#[test]
fn issue2032_input_procsub_argument_form_inherits_positional_params() {
    let dir = scratch("argform");
    let (out, err, code) = rubash_c(
        &dir,
        "cat <(printf '<%s>\\n' \"$@\")",
        &["AAA", "BBB"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "<AAA>\n<BBB>\n");
}

#[test]
fn issue2032_procsub_body_sees_count_star_and_zero() {
    let dir = scratch("faces");
    let (out, err, code) = rubash_c(
        &dir,
        "cat < <(printf 'n=%s star=[%s] zero=[%s]\\n' \"$#\" \"$*\" \"$0\")",
        &["AAA", "BBB"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "n=2 star=[AAA BBB] zero=[pin2032]\n");
}

#[test]
fn issue2032_procsub_body_observes_the_shifted_window() {
    let dir = scratch("shift");
    let (out, err, code) = rubash_c(
        &dir,
        "shift; cat < <(printf '<%s>\\n' \"$@\")",
        &["AAA", "BBB"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "<BBB>\n");
}

#[test]
fn issue2032_function_args_are_the_positional_params_inside_procsub() {
    let dir = scratch("func");
    let (out, err, code) = rubash_c(
        &dir,
        "f() { cat < <(printf '<%s>\\n' \"$@\"); }; f F1 F2",
        &[],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "<F1>\n<F2>\n");
}

#[test]
fn issue2032_nested_procsub_inherits_through_both_levels() {
    let dir = scratch("nested");
    let (out, err, code) = rubash_c(
        &dir,
        "cat < <(cat < <(printf 'deep: <%s>\\n' \"$@\"))",
        &["AAA", "BBB"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "deep: <AAA>\ndeep: <BBB>\n");
}

#[test]
fn issue2032_output_procsub_body_inherits_positional_params() {
    let dir = scratch("outbody");
    let script = "printf 'hello\\n' > >(read -r x; printf 'p1=[%s] x=[%s]\\n' \"$1\" \"$x\" > result.txt)\n\
                  for i in 1 2 3 4 5 6 7 8 9 10; do [ -f result.txt ] && break; sleep 0.2; done\n\
                  cat result.txt";
    let (out, err, code) = rubash_c(&dir, script, &["AAA", "BBB"], &[]);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "p1=[AAA] x=[hello]\n");
}

#[test]
fn issue2032_async_child_inherits_positional_params() {
    let dir = scratch("async");
    let script = "g() { printf 'async: <%s>\\n' \"$@\" > result.txt; }; g \"$@\" & wait; cat result.txt";
    let (out, err, code) = rubash_c(&dir, script, &["AAA", "BBB"], &[]);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "async: <AAA>\nasync: <BBB>\n");
}

#[test]
fn issue2032_coproc_body_inherits_positional_params() {
    let dir = scratch("coproc");
    let script = "coproc C { printf 'p1=[%s] n=%s\\n' \"$1\" \"$#\"; }\n\
                  read -r line <&\"${C[0]}\"\n\
                  printf 'got: %s\\n' \"$line\"";
    let (out, err, code) = rubash_c(&dir, script, &["AAA", "BBB"], &[]);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "got: p1=[AAA] n=2\n");
}

#[test]
fn issue2032_special_values_round_trip_through_procsub() {
    let dir = scratch("values");
    let (out, err, code) = rubash_c(
        &dir,
        "cat < <(printf '<%s>\\n' \"$@\")",
        &["with space", "", "it's", "$HOME", "a\\b", "line1\nline2"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(
        out,
        "<with space>\n<>\n<it's>\n<$HOME>\n<a\\b>\n<line1\nline2>\n"
    );
}

#[test]
fn issue2032_transport_key_never_surfaces_as_a_variable() {
    let dir = scratch("leak");
    // Inside the respawned body the stamp must already be consumed.
    let (out, err, code) = rubash_c(
        &dir,
        "cat < <(if [[ -v __RUBASH_POSITIONAL_PARAMS ]]; then echo LEAK; else echo CLEAN; fi)",
        &["AAA"],
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "CLEAN\n");
    // A plain (non-respawn) shell handed the key in its environment
    // ignores and strips it: no forged parameters, no visible variable.
    let (out, err, code) = rubash_c(
        &dir,
        "printf 'n=%s v=%s\\n' \"$#\" \"${__RUBASH_POSITIONAL_PARAMS-UNSET}\"",
        &[],
        &[("__RUBASH_POSITIONAL_PARAMS", "forged")],
    );
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "n=0 v=UNSET\n");
}

/// The exact bats-exec-suite shape that produced the blank filename:
/// the unique-filename array is read from a process substitution whose
/// pipeline starts from `"$@"` (bats-exec-suite line ~215).
#[cfg(unix)]
#[test]
fn issue2032_bats_exec_suite_filename_shape() {
    let dir = scratch("bats");
    let script = "IFS=$'\\n' read -d '' -r -a arr < <(printf \"%s\\n\" \"$@\" | nl | sort -k 2 | uniq -f 1 | sort -n | cut -f 2-)\n\
                  printf 'file=[%s] n=%s\\n' \"${arr[0]}\" \"${#arr[@]}\"";
    let (out, err, code) = rubash_c(&dir, script, &["/some/where/trivial.bats"], &[]);
    assert_eq!(code, Some(0), "stderr: {err}");
    assert_eq!(out, "file=[/some/where/trivial.bats] n=1\n");
}
