//! rubash#3320 — `set -v` read-point coverage residuals:
//! 1. Multi-line `eval` strings sample `verbose` at the read point
//!    line-by-line / group-by-group (`builtins/evalstring.c:359-451`
//!    `parse_and_execute` -> `parse.y:2712` `shell_getc`).
//! 2. `trap` action strings echo under `set -v` at their read boundary
//!    (`trap.c:1071`, `trap.c:1200`, `trap.c:1285-1290`).
//! 3. Interactive stdin (`run_interactive_stdin` -> history & fallback
//!    `run_source_with_line_offset` routes) echoes accepted lines at the
//!    read point (`parse.y:2712`).

use std::io::Write as _;
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_script(tag: &str, body: &str, extra_flags: &[&str]) -> Run {
    let dir = std::env::temp_dir().join(format!("rubash-3320-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let mut args: Vec<&str> = extra_flags.to_vec();
    args.push(&script_arg);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn run_interactive_stdin(stdin_body: &str, extra_flags: &[&str]) -> Run {
    let mut args: Vec<&str> = vec!["--norc", "-i"];
    args.extend_from_slice(extra_flags);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(&args)
        .env("PS1", "P> ")
        .env("PS2", "S> ")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash -i");
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_body.as_bytes())
            .expect("write interactive stdin");
    }
    let output = child.wait_with_output().expect("wait rubash -i");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn eval_multiline_toggles_verbose_at_read_point() {
    let run = run_script(
        "eval-toggle",
        "eval $'echo e1\\nset -v\\necho e2\\necho e3\\nset +v\\necho e4'\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "e1\ne2\ne3\ne4\n");
    assert_eq!(run.stderr, "echo e2\necho e3\nset +v\n");
}

#[test]
fn eval_multiline_with_initial_verbose_stops_after_set_plus_v() {
    let run = run_script(
        "eval-init-v",
        "set -v\neval $'echo e5\\nset +v\\necho e6'\nset +v\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "e5\ne6\n");
    assert_eq!(
        run.stderr,
        "eval $'echo e5\\nset +v\\necho e6'\necho e5\nset +v\n"
    );
}

#[test]
fn eval_multiline_compound_defers_verbose_until_next_group() {
    let run = run_script(
        "eval-compound",
        "eval $'if true; then\\nset -v\\necho in_if\\nfi\\necho after_if'\nset +v\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "in_if\nafter_if\n");
    assert_eq!(run.stderr, "echo after_if\nset +v\n");
}

#[test]
fn eval_multiline_preserves_exit_status_across_trailing_comment_and_redirects() {
    let dir = std::env::temp_dir().join(format!("rubash-3320-eval-redir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let out_path = dir.join("out.txt");
    let out_str = out_path.to_string_lossy().replace('\\', "/");
    let script = format!(
        "eval $'false\\n# trailing comment'\necho rc=$?\neval $'echo line1\\necho line2' > '{out_str}'\n"
    );
    let run = run_script("eval-redir", &script, &[]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "rc=1\n");
    let written = std::fs::read_to_string(&out_path).expect("read redirected file");
    assert_eq!(written, "line1\nline2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn traps_echo_action_lines_under_verbose() {
    let run = run_script(
        "traps-v",
        "set -v\ntrap 'echo ERR_TRAP' ERR\nfalse\nf() {\n  trap 'echo RET_TRAP' RETURN\n  true\n}\nf\ntrap 'echo DBG_TRAP' DEBUG\necho body\ntrap - DEBUG\ntrap 'echo EXIT_TRAP' EXIT\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(
        run.stdout,
        "ERR_TRAP\nRET_TRAP\nDBG_TRAP\nbody\nDBG_TRAP\nEXIT_TRAP\n"
    );
    assert_eq!(
        run.stderr,
        concat!(
            "trap 'echo ERR_TRAP' ERR\n",
            "false\n",
            "echo ERR_TRAP\n",
            "f() {\n",
            "  trap 'echo RET_TRAP' RETURN\n",
            "  true\n",
            "}\n",
            "f\n",
            "echo RET_TRAP\n",
            "trap 'echo DBG_TRAP' DEBUG\n",
            "echo body\n",
            "echo DBG_TRAP\n",
            "trap - DEBUG\n",
            "echo DBG_TRAP\n",
            "trap 'echo EXIT_TRAP' EXIT\n",
            "echo EXIT_TRAP\n",
        )
    );
}

#[test]
fn exit_trap_not_echoed_after_set_plus_v() {
    let run = run_script(
        "exit-trap-plus-v",
        "set -v\ntrap 'echo EXIT_TRAP' EXIT\nset +v\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "EXIT_TRAP\n");
    assert_eq!(run.stderr, "trap 'echo EXIT_TRAP' EXIT\nset +v\n");
}

#[test]
fn interactive_stdin_echoes_lines_when_verbose_is_on() {
    let run = run_interactive_stdin(
        "set -v\necho one\necho two # comment\nset +v\necho three\n",
        &[],
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "one\ntwo\nthree\n");
    let stderr_lines: Vec<&str> = run.stderr.lines().collect();
    assert!(
        stderr_lines.contains(&"echo one"),
        "expected `echo one` in interactive stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        stderr_lines.contains(&"echo two # comment"),
        "expected `echo two # comment` in interactive stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        stderr_lines.contains(&"set +v"),
        "expected `set +v` in interactive stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        !stderr_lines.contains(&"set -v"),
        "did not expect `set -v` echoed as verbose line in interactive stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        !stderr_lines.contains(&"echo three"),
        "did not expect `echo three` echoed after `set +v`, got: {:?}",
        stderr_lines
    );
}

#[test]
fn interactive_stdin_with_dash_v_echoes_from_first_line() {
    let run = run_interactive_stdin("echo first\nset +v\necho second\n", &["-v"]);
    assert_eq!(run.code, Some(0));
    assert_eq!(run.stdout, "first\nsecond\n");
    let stderr_lines: Vec<&str> = run.stderr.lines().collect();
    assert!(
        stderr_lines.contains(&"echo first"),
        "expected `echo first` in interactive -v stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        stderr_lines.contains(&"set +v"),
        "expected `set +v` in interactive -v stderr, got: {:?}",
        stderr_lines
    );
    assert!(
        !stderr_lines.contains(&"echo second"),
        "did not expect `echo second` after `set +v`, got: {:?}",
        stderr_lines
    );
}
