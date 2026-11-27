//! rubash#2773 — `set -v` must echo each input line to stderr as it is
//! READ (the physical line, verbatim: comments kept, `;`-joined commands
//! kept together, continuation lines echoed per physical line), not a
//! print_cmd-style rendering produced per command at execution time.
//!
//! GNU: flags.c:284 `echo_input_at_read = verbose_flag`; parse.y:2712
//! the reader prints `shell_input_line` when a line is read. Two timing
//! consequences are pinned below: a `set +v` line is itself echoed (the
//! flag is still on when the line is read), and a `-c` string produces
//! no echo for commands after an in-string `set -v` (the whole string
//! was already read before the flag took effect), while `-v -c` echoes
//! the whole string exactly once.
//!
//! Expectations verified against GNU bash 5.3.20 (script files and -c).

use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_args(args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_script(tag: &str, body: &str, extra_flags: &[&str]) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash-2773-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let mut args: Vec<&str> = extra_flags.to_vec();
    args.push(&script_arg);
    let run = run_args(&args);
    let _ = std::fs::remove_dir_all(&dir);
    run
}

const REPRO: &str = "echo before\nset -v\necho one; echo two # comment\nset +v\necho after\n";

#[test]
fn verbose_echoes_the_physical_line_once_with_its_comment() {
    let run = run_script("repro", REPRO, &[]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "before\none\ntwo\nafter\n");
    // The physical line is echoed once, as read — and the `set +v` line
    // is echoed too: verbose is still on when that line is read.
    assert_eq!(run.stderr, "echo one; echo two # comment\nset +v\n");
}

#[test]
fn verbose_echoes_each_physical_line_of_a_multiline_compound() {
    let run = run_script(
        "compound",
        "set -v\nfor i in 1 2\ndo\necho iter$i\ndone\necho end\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "iter1\niter2\nend\n");
    assert_eq!(
        run.stderr,
        "for i in 1 2\ndo\necho iter$i\ndone\necho end\n"
    );
}

#[test]
fn verbose_echoes_comment_and_blank_lines() {
    let run = run_script(
        "comment",
        "set -v\n# just a comment\n\necho x\nset +v\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "x\n");
    assert_eq!(run.stderr, "# just a comment\n\necho x\nset +v\n");
}

#[test]
fn verbose_echoes_continuation_per_physical_line() {
    let run = run_script("continuation", "set -v\necho one \\\ntwo\nset +v\n", &[]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "one two\n");
    assert_eq!(run.stderr, "echo one \\\ntwo\nset +v\n");
}

#[test]
fn dash_c_string_read_before_verbose_takes_effect_is_not_echoed() {
    // The whole -c string is read before `set -v` executes, so GNU
    // echoes nothing at all.
    let run = run_args(&["-c", "echo pre; set -v; echo post"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "pre\npost\n");
    assert_eq!(run.stderr, "");
}

#[test]
fn dash_v_dash_c_echoes_the_whole_string_once() {
    let run = run_args(&["-vc", "echo a; echo b"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "a\nb\n");
    assert_eq!(run.stderr, "echo a; echo b\n");
}

#[test]
fn dash_v_flag_echoes_from_the_first_line_until_plus_v_is_read() {
    let run = run_script("flagv", REPRO, &["-v"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "before\none\ntwo\nafter\n");
    // `echo after` is NOT echoed: it is read after `set +v` executed.
    assert_eq!(
        run.stderr,
        "echo before\nset -v\necho one; echo two # comment\nset +v\n"
    );
}

#[test]
fn verbose_state_toggles_feed_the_next_lines_read() {
    // Chained state: each toggle command changes whether the NEXT lines
    // read are echoed; a toggle line itself is judged by the state in
    // effect when it is read.
    let run = run_script(
        "toggle",
        "set -v\necho first\nset +v\necho hidden\nset -v\necho second\nset +v\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "first\nhidden\nsecond\n");
    assert_eq!(run.stderr, "echo first\nset +v\necho second\nset +v\n");
}
