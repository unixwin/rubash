//! rubash#2238 — a foreground external command inheriting the
//! shell's stdin stream drained the unread remainder.
//!
//! GNU keeps ONE open file description for fd 0: the shell's own
//! reads take exactly the requested bytes off the shared kernel
//! offset (lib/sh/zread.c:99 zread), and a forked child inherits
//! that same description, so what the child reads is gone and what
//! it leaves stays readable. Rubash's virtual stdin channels (the
//! FUNCTION_STDIN mirror for heredoc/herestring/pipe-fed input, and
//! virtual fd-0 text endpoints) accounted a spawned external as
//! consuming the whole remainder up front, so a `while read` loop
//! whose body ran any external command stopped after iteration 1.
//! The fix hands the child a duplicate of one materialized
//! description and folds the measured consumption back onto the
//! stream cursor.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (`-c` string).

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

#[test]
fn heredoc_loop_external_expr_keeps_remaining_lines() {
    let (stdout, _, code) = rubash(
        "n=0; while read a b; do n=$((n+1)); expr \"$b\" : '[a-z]*' >/dev/null; done <<EOF\n\
         one sha1:x\n\
         two sha1:y\n\
         three sha1:z\n\
         EOF\n\
         echo \"iters=$n\"",
    );
    assert_eq!(stdout, "iters=3\n");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn heredoc_loop_nonreading_external_keeps_remaining_lines() {
    let (stdout, _, code) = rubash(
        "n=0; while read a b; do n=$((n+1)); /bin/true; done <<EOF\n\
         one x\n\
         two y\n\
         three z\n\
         EOF\n\
         echo \"iters=$n\"",
    );
    assert_eq!(stdout, "iters=3\n");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn herestring_reads_survive_interleaved_external() {
    let (stdout, _, _) =
        rubash("{ read a; /bin/true; read b; echo \"a=$a b=$b\"; } <<< \"one\ntwo\"");
    assert_eq!(stdout, "a=one b=two\n");
}

#[cfg(unix)]
#[test]
fn external_before_first_read_leaves_stream_untouched() {
    let (stdout, _, _) = rubash(
        "{ /bin/true; read a; read b; echo \"a=$a b=$b\"; } <<EOF\n\
         one\n\
         two\n\
         three\n\
         EOF",
    );
    assert_eq!(stdout, "a=one b=two\n");
}

#[cfg(unix)]
#[test]
fn pipe_fed_loop_external_keeps_remaining_lines() {
    let (stdout, _, _) = rubash(
        "printf 'one x\\ntwo y\\nthree z\\n' | while read a b; do echo iter; /bin/true; done",
    );
    assert_eq!(stdout, "iter\niter\niter\n");
}

#[cfg(unix)]
#[test]
fn external_partial_consumption_advances_cursor_by_measured_bytes() {
    // dd with bs=1 reads exactly two bytes per iteration, so the
    // loop's own reads resume mid-line: GNU shares the one kernel
    // offset, it does not round consumption up to a line or to EOF.
    let (stdout, _, _) = rubash(
        "while read line; do echo \"got:$line\"; dd bs=1 count=2 >/dev/null 2>/dev/null; done <<EOF\n\
         aaaa\n\
         bbbb\n\
         cccc\n\
         EOF",
    );
    assert_eq!(stdout, "got:aaaa\ngot:bb\ngot:cc\n");
}

#[cfg(unix)]
#[test]
fn external_with_own_stdin_redirect_never_touches_the_stream() {
    let (stdout, _, _) = rubash(
        "{ read a; /bin/true </dev/null; read b; echo \"a=$a b=$b\"; } <<EOF\n\
         one\n\
         two\n\
         EOF",
    );
    assert_eq!(stdout, "a=one b=two\n");
}

#[cfg(unix)]
#[test]
fn parked_text_endpoint_not_drained_by_nonreading_external() {
    // The virtual fd-0 text-endpoint leg (the probeS R5 mechanism in
    // miniature): f's call heredoc is parked as an fd-0 text endpoint
    // because the enclosing frame's stdin is a live File endpoint;
    // a non-reading external inside f's body then dispatched against
    // that endpoint, and the pre-fix accounting marked the whole
    // endpoint consumed before f's own reader ran. The endpoint
    // cursor may advance only by what a consumer actually reads.
    let dir = std::env::temp_dir().join(format!("rubash-2238-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::write(dir.join("data.txt"), "D1\nD2\n").expect("write data file");
    let script = "f() { /bin/true; cat; }; \
                  g() { f <<EOF\n\
                  payload\n\
                  EOF\n\
                  }; g < data.txt";
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "payload\n"
    );
    assert_eq!(output.status.code(), Some(0));
}
