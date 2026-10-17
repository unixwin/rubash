// niubash#708: the inline `head` fast path skipped every `-…` word and then
// fell through `head_line_count(...).unwrap_or(10)`, so a malformed, missing
// or unknown count silently printed the first ten lines with rc 0, and GNU's
// negative form (`-n -1`, all but the last line) was lost because
// `"-1".parse::<usize>()` fails.
//
// Only the representable count forms stay inline; every other shape declines
// so the real external `head` reports its error (rc 1).

use std::process::Command;

fn run(cmd: &str) -> (Vec<u8>, i32) {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let out = Command::new(exe)
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("spawn rubash");
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// GNU `head -n -1` prints all but the last line.
#[test]
fn negative_count_is_all_but_last() {
    let (out, rc) = run("printf 'a\\nb\\nc\\n' | head -n -1");
    assert_eq!((out.as_slice(), rc), (b"a\nb\n".as_slice(), 0));
}

/// The same defect runs through command substitution's real executor.
#[test]
fn negative_count_through_command_substitution() {
    let (out, rc) = run("x=$(printf 'a\\nb\\nc\\n' | head -n -1); printf '%s' \"$x\"");
    assert_eq!((out.as_slice(), rc), (b"a\nb".as_slice(), 0));
}

/// Invalid, unknown and missing operands must reach the real head: no silent
/// first-ten-lines output, error status instead.
#[test]
fn malformed_counts_error() {
    for cmd in [
        "printf 'a\\nb\\nc\\n' | head -n abc",
        "printf 'a\\nb\\nc\\n' | head -x",
        "printf 'a\\nb\\nc\\n' | head -n",
        "printf 'a\\nb\\nc\\n' | head --lines",
    ] {
        let (out, rc) = run(cmd);
        assert!(out.is_empty(), "{cmd}: stdout must be empty, got {out:?}");
        assert_eq!(rc, 1, "{cmd}: rc must be 1");
    }
}

/// Every representable count form still works, including the default.
#[test]
fn valid_count_forms_no_regression() {
    for (args, expected) in [
        ("-n 2", b"a\nb\n".as_slice()),
        ("-n2", b"a\nb\n".as_slice()),
        ("-2", b"a\nb\n".as_slice()),
        ("--lines=2", b"a\nb\n".as_slice()),
        ("-n 0", b"".as_slice()),
        ("", b"a\nb\nc\n".as_slice()),
    ] {
        let (out, rc) = run(&format!("printf 'a\\nb\\nc\\n' | head {args}"));
        assert_eq!((out.as_slice(), rc), (expected, 0), "head {args}");
    }
}
