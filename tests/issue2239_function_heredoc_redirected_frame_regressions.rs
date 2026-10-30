//! rubash#2239 — heredoc payloads attached to function CALLS
//! arrived empty inside a function frame whose stdin was
//! redirected (the git test-lib `test_eval_ </dev/null` shape).
//!
//! GNU installs a call's own redirections on the real descriptors
//! for the body's duration and undoes them on return
//! (execute_cmd.c:854 do_redirections with RX_UNDOABLE, applied by
//! execute_function, execute_cmd.c:5181): the call's heredoc
//! supersedes the inherited fd-0 binding outright. Rubash stages a
//! call heredoc as FUNCTION_STDIN text but left the frame's fd-0
//! entry in the table, and consumers that consult the fd table
//! before the mirror — the external spawn wiring, the stdin pull's
//! source predicate, inline readers — read the frame's binding
//! (classically /dev/null) instead of the payload. The fix displaces
//! the inherited fd-0 entry while the staged text serves the body
//! and reinstates it on return, the function-call twin of the
//! compound-command displacement (rubash#2170).
//!
//! The R5 pin also exercises the rubash#2238 leg of the same
//! cluster: a non-reading external (mkdir) inside the frame must
//! not drain a parked fd-0 text endpoint.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string,
//! plus the probeS file-content shape).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.status.code(),
    )
}

#[test]
fn call_heredoc_payload_survives_devnull_frame() {
    let (stdout, _, code) = rubash(
        "inner1() { cat; }; w1() { inner1 <<EOF\n\
         payload-v1\n\
         EOF\n\
         }; w1 </dev/null",
    );
    assert_eq!(stdout, "payload-v1\n");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn call_heredoc_payload_reaches_external_reader_in_devnull_frame() {
    let (stdout, _, _) = rubash(
        "inner8() { /bin/cat; }; w8() { inner8 <<EOF\n\
         payload-v8\n\
         EOF\n\
         }; w8 </dev/null",
    );
    assert_eq!(stdout, "payload-v8\n");
}

#[test]
fn call_heredoc_payload_survives_nested_calls_in_devnull_frame() {
    let (stdout, _, _) = rubash(
        "deep() { cat; }; mid() { deep <<EOF\n\
         payload-v7\n\
         EOF\n\
         }; top() { mid; }; top </dev/null",
    );
    assert_eq!(stdout, "payload-v7\n");
}

#[test]
fn write_hdr_shape_in_devnull_frame_delivers_both_payloads() {
    // The test-lib shape end to end: the call heredoc feeds the
    // trailing append, the nested call's own heredoc feeds the
    // header writer, both inside a </dev/null frame invoked
    // directly (probeS R8).
    let dir = std::env::temp_dir().join(format!("rubash-2239-r8-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = "write_hdr() { { echo \"#!HDR\" && cat; } >\"$1\"; }\n\
                  f_outer() { write_hdr \"$1.sh\" <<INNER &&\n\
                  test_description=sub\n\
                  INNER\n\
                  cat >>\"$1.sh\"; }\n\
                  direct() { f_outer sub <<EOF\n\
                  body1\n\
                  body2\n\
                  EOF\n\
                  }\n\
                  direct </dev/null\n\
                  cat sub.sh";
    let (stdout, code) = rubash_in(&dir, script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "#!HDR\ntest_description=sub\nbody1\nbody2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn eval_frame_with_data_file_stdin_delivers_both_payloads() {
    // probeS R5 verbatim in miniature: the heredoc rides inside the
    // eval string, the eval function's stdin is a DATA FILE, and
    // f_outer's body runs a non-reading external (mkdir) before the
    // payload consumer — both payloads must land in the product.
    let dir = std::env::temp_dir().join(format!("rubash-2239-r5-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("stdin-data.txt"), "STDIN-A\nSTDIN-B\n").expect("write data file");
    let script = "write_hdr() { { echo \"#!HDR\" && cat; } >\"$1\"; }\n\
                  f_outer() { name=\"$1\"; mkdir -p \"$name\" && write_hdr \"$name/$name.sh\" <<INNER &&\n\
                  test_description='sub'\n\
                  INNER\n\
                  cat >>\"$name/$name.sh\"; }\n\
                  inner() { eval \"$*\"; }\n\
                  inner <stdin-data.txt 'f_outer sub <<EOF\n\
                  body1\n\
                  body2\n\
                  EOF'\n\
                  cat sub/sub.sh";
    let (stdout, code) = rubash_in(&dir, script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "#!HDR\ntest_description='sub'\nbody1\nbody2\n");
    assert_eq!(code, Some(0));
}

#[test]
fn call_heredoc_payload_in_data_file_frame_still_works() {
    // Control (probeS V6 family): with a data-file frame the call
    // heredoc takes the fd-0 binding path — it worked before the fix
    // and must keep working.
    let dir = std::env::temp_dir().join(format!("rubash-2239-v6-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("data.txt"), "DATA-A\nDATA-B\n").expect("write data file");
    let script = "inner1() { cat; }; w1() { inner1 <<EOF\n\
                  payload-v6\n\
                  EOF\n\
                  }; w1 <data.txt";
    let (stdout, code) = rubash_in(&dir, script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "payload-v6\n");
    assert_eq!(code, Some(0));
}

#[test]
fn frame_stdin_is_back_after_the_heredoc_call_returns() {
    // The displacement is scoped to the call: once the heredoc call
    // returns, the frame's own stdin binding serves again (GNU:
    // RX_UNDOABLE unwinds the call's redirections on return).
    let (stdout, _, _) = rubash(
        "inner1() { cat; }; w1() { inner1 <<EOF\n\
         payload\n\
         EOF\n\
         read x; echo \"after:$x\"; }; \
         { read first; w1; } </dev/null; echo \"first:$first\"",
    );
    assert_eq!(stdout, "payload\nafter:\nfirst:\n");
}
