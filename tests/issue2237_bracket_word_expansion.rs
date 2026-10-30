//! rubash#2237 — the `[ ]` builtin's word preparation skipped pathname
//! expansion: `[ -r /dev/nul* ]` tested the literal pattern text and
//! always failed, while the `test` spelling globbed. `[` is the `test`
//! builtin under another spelling (builtins/test.def), so its words
//! take the same full expansion pipeline as any simple command —
//! pathname expansion included. (The process-substitution half of the
//! same defect is pinned by issue1467_test_procsub_path_regressions.)
//!
//! Expectations verified against GNU bash 5.3.20 (script files).

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
fn issue2237_bracket_words_take_pathname_expansion() {
    let (stdout, stderr, code) = rubash(
        "[ -r /dev/nul* ] && echo OK-glob || echo FAIL-glob\n\
         [ -e /dev/zzz-no-such-* ] && echo BAD-nomatch || echo OK-nomatch\n\
         test -r /dev/nul* && echo OK-test-glob || echo FAIL-test-glob",
    );
    assert_eq!(stdout, "OK-glob\nOK-nomatch\nOK-test-glob\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
