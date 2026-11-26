//! rubash#1703 regressions: a function called from the body of a
//! `while read … done < file` loop swallowed the loop's remaining
//! redirected input — the loop ended after the line on which the
//! function was first called. nvm's alias-file readers are exactly
//! this shape (`while read` loop + helper function calls).
//!
//! Root cause: a compound command's `< file` binds fd 0 to a live
//! FdReadEndpoint::File in the fd table. `function_call_stdin` staged
//! the call's stdin as text through `stdin_string_for_command`, whose
//! File branch `read_to_end`s the shared-offset dup to EOF — and the
//! staged text is shadowed by the fd-table endpoint for every body
//! reader, so the bytes were consumed AND discarded.
//!
//! GNU spec (vendored tree `bash.git @9c465866`, Bash-5.3 patch 20):
//! - execute_cmd.c:5181 execute_function runs the body in-process on
//!   the caller's descriptors (body dispatched at execute_cmd.c:5395
//!   via execute_command_internal); nothing in the call path copies
//!   or drains fd 0.
//! - execute_cmd.c:854: only a command's OWN redirections rebind
//!   descriptors, via do_redirections (RX_ACTIVE|RX_UNDOABLE), undone
//!   on completion — a call without `<` leaves fd 0 shared.
//!
//! Expectations verified byte-for-byte against GNU Bash 5.3.20
//! script-file probes (self-compiled oracle), stdout compared exactly.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1703-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    std::fs::write(dir.join("in.txt"), "a\nb\nc\n").expect("write in.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue1703_function_call_in_while_read_loop_keeps_stream() {
    let (stdout, _, code) = rubash_file(
        "f() { :; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_function_body_reads_the_shared_stream() {
    // The function's own read consumes the NEXT line of the loop's
    // stream (one shared cursor), it does not see EOF.
    let (stdout, _, code) = rubash_file(
        "f() { read -r X; echo \"f-ate:$X\"; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\nf-ate:b\ngot:c\nf-ate:\n");
}

#[test]
fn issue1703_damage_is_at_call_time_not_loop_entry() {
    // Calling the function only on the second line must leave the
    // first iteration untouched and the third line reachable.
    let (stdout, _, code) = rubash_file(
        "f() { :; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; if [ \"$L\" = b ]; then f; fi; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_read_after_call_in_same_body_continues_stream() {
    let (stdout, _, code) = rubash_file(
        "f() { :; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; IFS= read -r M || break; echo \"post:$M\"; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\npost:b\ngot:c\n");
}

#[test]
fn issue1703_function_in_loop_condition_keeps_stream() {
    let (stdout, _, code) =
        rubash_file("f() { :; }\nwhile IFS= read -r L && f; do echo \"got:$L\"; done < in.txt\n");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_nested_function_calls_keep_stream() {
    let (stdout, _, code) = rubash_file(
        "g() { :; }\nf() { g; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_loop_inside_function_keeps_stream() {
    let (stdout, _, code) = rubash_file(
        "f() { :; }\ng() { while IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt; }\ng\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_external_command_body_reads_shared_stream() {
    // An external command in the body inherits the shared open file
    // description: `cat` drains the rest, then the loop is done.
    let (stdout, _, code) = rubash_file(
        "f() { cat; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\nb\nc\n");
}

#[test]
fn issue1703_explicit_dup_read_inside_function_shares_stream() {
    let (stdout, _, code) = rubash_file(
        "f() { IFS= read -r Y <&0 || true; echo \"f-got:$Y\"; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\nf-got:b\ngot:c\nf-got:\n");
}

#[test]
fn issue1703_call_with_own_redirect_still_isolated() {
    // Control: a call carrying its own `<` redirection never touched
    // the loop stream (binding path) — that must not change.
    let (stdout, _, code) = rubash_file(
        "f() { :; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f < /dev/null; done < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}

#[test]
fn issue1703_call_with_heredoc_reads_heredoc_not_loop_stream() {
    // Control: a call's own heredoc is its stdin; the loop stream is
    // not consumed by staging it.
    let (stdout, _, code) = rubash_file(
        "f() { IFS= read -r Y || true; echo \"f:$Y\"; }\nwhile IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f <<XEOF\nhx\nXEOF\ndone < in.txt\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\nf:hx\ngot:b\nf:hx\ngot:c\nf:hx\n");
}

#[test]
fn issue1703_pipe_fed_loop_unaffected() {
    // Control: pipe-fed loops take the text-endpoint path, which was
    // never broken — pin it so the fix cannot regress it.
    let (stdout, _, code) = rubash_file(
        "f() { :; }\nprintf 'a\\nb\\nc\\n' | while IFS= read -r L || [ -n \"$L\" ]; do echo \"got:$L\"; f; done\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "got:a\ngot:b\ngot:c\n");
}
