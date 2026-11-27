// niubash#166: bare-name `tr` with OPTIONS in a pipeline stage was silently
// short-circuited — the inline translate fast path read `tr -d b` as
// SET1="-d", and input containing neither '-' nor 'd' passed through
// unchanged with rc=0 and empty stderr. Options must always run the real
// external tr; only two plain set operands may take the fast path.

use std::process::Command;

fn run(cmd: &str) -> (String, i32) {
    let exe = env!("CARGO_BIN_EXE_rubash");
    let out = Command::new(exe)
        .arg("-c")
        .arg(cmd)
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn tr_delete_option_runs_in_pipeline() {
    let (out, rc) = run("printf 'abc\\n' | tr -d 'b'");
    assert_eq!(
        (out.as_str(), rc),
        ("ac\n", 0),
        "tr -d must delete, not pass through"
    );
}

#[test]
fn tr_squeeze_option_runs_in_pipeline() {
    let (out, rc) = run("printf 'x   y\\n' | tr -s ' '");
    assert_eq!((out.as_str(), rc), ("x y\n", 0), "tr -s must squeeze");
}

#[test]
fn tr_delete_downstream_count() {
    let (out, rc) = run("printf 'abc\\n' | tr -d 'b' | wc -c");
    assert_eq!((out.as_str().trim(), rc), ("3", 0));
}

#[test]
fn plain_translate_fast_path_still_works() {
    let (out, rc) = run("echo hello | tr a-z A-Z");
    assert_eq!((out.as_str(), rc), ("HELLO\n", 0));
}

#[test]
fn tr_long_options_also_bypass_the_fast_path() {
    let (out, rc) = run("printf 'abc\\n' | tr --delete 'b'");
    assert_eq!(
        (out.as_str(), rc),
        ("ac\n", 0),
        "--delete must not be read as a set"
    );
}

#[test]
fn tr_complement_option_runs() {
    let (out, rc) = run("printf 'abc\\n' | tr -c 'a\\n' x");
    assert_eq!(
        (out.as_str(), rc),
        ("axx\n", 0),
        "-c complement belongs to coreutils"
    );
}
