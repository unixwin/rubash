// niubash#709: the inline `dirname`/`basename` string edges. A leading `//`
// must be preserved (POSIX: exactly two slashes are implementation-defined
// and GNU keeps them), and an empty operand must yield `.` / the empty string.
// Neither is the `/a/b -> A:` operand translation tracked in niubash#346.

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

#[test]
fn dirname_leading_double_slash_preserved() {
    assert_eq!(run("dirname //"), (b"//\n".to_vec(), 0));
    assert_eq!(run("dirname //a"), (b"//\n".to_vec(), 0));
}

#[test]
fn dirname_empty_operand_is_dot() {
    assert_eq!(run("dirname ''"), (b".\n".to_vec(), 0));
}

#[test]
fn basename_empty_operand_is_empty() {
    assert_eq!(run("basename ''"), (b"\n".to_vec(), 0));
}

#[test]
fn plain_paths_no_regression() {
    assert_eq!(run("dirname /a/b/c"), (b"/a/b\n".to_vec(), 0));
    assert_eq!(run("basename /a/b/c.txt"), (b"c.txt\n".to_vec(), 0));
    assert_eq!(run("dirname foo"), (b".\n".to_vec(), 0));
    assert_eq!(run("basename foo"), (b"foo\n".to_vec(), 0));
    assert_eq!(run("dirname /"), (b"/\n".to_vec(), 0));
    // Three or more leading slashes collapse to a single `/` (GNU).
    assert_eq!(run("dirname ///"), (b"/\n".to_vec(), 0));
}
