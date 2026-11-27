//! niubash#165: `cat' as a PIPE reader (an inline pipeline stage reading
//! the captured stage input) silently dropped every formatting option —
//! `printf 'z\n' | cat -n' printed `z' instead of GNU's `     1\tz';
//! `-s'/`-A'/`-E'/long forms likewise dead, and `cat -Z' ran as identity
//! rc 0 instead of GNU's rc-1 diagnostic. Root cause: the inline stage
//! arm in src/executor/pipeline_exec.rs carried its own `-v'-only
//! mini-cat while the full option surface (rubash#415) lived only in the
//! simple-command path (external_cat). The fix routes the stage arm
//! through the same parse_cat_argv + cat_format pair.
//!
//! Behavioral reference: GNU coreutils 9.4 src/cat.c — main() parses the
//! option surface once (getopt_long) and passes the same option block to
//! cat() for every input fd, so file operands, `-', and the no-operand
//! stdin fallback all run the identical filter loop with one shared
//! line-filter state across the concatenation. Byte expectations below
//! verified against WSL GNU coreutils 9.4 via `wsl bash' script-file
//! probes (artifacts: target/issue-suites/results/i165/matrix-*.out,
//! 70/70 matrix cells byte-identical, 2026-10-03).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(20);

fn run_rubash_script(script: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                use std::io::Read;
                child
                    .stdout
                    .take()
                    .expect("stdout pipe")
                    .read_to_end(&mut stdout)
                    .expect("read stdout");
                child
                    .stderr
                    .take()
                    .expect("stderr pipe")
                    .read_to_end(&mut stderr)
                    .expect("read stderr");
                return (stdout, stderr, status.code());
            }
            Ok(None) => {
                if start.elapsed() > DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash did not exit within {DEADLINE:?}: {script}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("wait rubash: {error}"),
        }
    }
}

/// The exact issue repro: `printf 'z\n' | cat -n' must number the pipe
/// input `     1\tz' (GNU), not pass it through as `z'.
#[test]
fn issue_repro_pipe_cat_n_numbers_pipe_input() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'z\n' | cat -n");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"     1\tz\n");
}

#[test]
fn pipe_cat_squeeze_collapses_blank_runs() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'a\n\n\nb\n' | cat -s");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"a\n\nb\n");
}

#[test]
fn pipe_cat_show_all_renders_tabs_and_ends() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'z\ti\n' | cat -A");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"z^Ii$\n");
}

#[test]
fn pipe_cat_show_ends_marks_each_terminated_line_only() {
    // Unterminated final line gets neither `$' nor a newline (GNU cat.c).
    let (stdout, stderr, code) = run_rubash_script(r"printf 'x\ny' | cat -E");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"x$\ny");
}

#[test]
fn pipe_cat_n_s_combined_numbers_the_squeezed_stream() {
    // GNU runs the squeeze BEFORE numbering: the collapsed blank survives
    // as one numbered (empty) line 2 (WSL 9.4 probe, matrix cell
    // OPTS[-n -s]).
    let (stdout, stderr, code) =
        run_rubash_script(r"printf 'alpha\n\n\nbeta\tgamma\n' | cat -n -s");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        "     1\talpha\n     2\t\n     3\tbeta\tgamma\n"
    );
}

#[test]
fn pipe_cat_long_form_number_applies() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'z\n' | cat --number");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"     1\tz\n");
}

#[test]
fn pipe_cat_dash_operand_applies_options() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'z\n' | cat -n -");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"     1\tz\n");
}

#[test]
fn pipe_cat_invalid_option_is_rejected_like_gnu() {
    // GNU: no stdout, `cat: invalid option -- 'Z'' + Try line on stderr,
    // exit 1 — even mid-pipeline (the pipeline reports the stage status).
    let (stdout, stderr, code) = run_rubash_script(r"printf 'z\n' | cat -Z");
    assert_eq!(code, Some(1));
    assert_eq!(stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&stderr),
        "cat: invalid option -- 'Z'\nTry 'cat --help' for more information.\n"
    );
}

#[test]
fn pipe_cat_identity_passthrough_keeps_bytes() {
    let (stdout, stderr, code) = run_rubash_script(r"printf 'a\tb\n' | cat");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"a\tb\n");
}

#[test]
fn pipe_cat_file_operand_options_and_cross_operand_numbering() {
    // Options apply with FILE operands on the pipe path too, and the
    // numbering counter carries across the stdin/file boundary (one
    // filter pass over the concatenation — GNU cat.c).
    let dir = std::env::temp_dir().join("rubash-issue165-cat-pipe");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("operand.txt");
    std::fs::write(&file, b"file-line\n").unwrap();
    let script = format!(
        "printf 'z\\n' | cat -n - {}",
        file.to_string_lossy().replace('\\', "/")
    );
    let (stdout, stderr, code) = run_rubash_script(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"     1\tz\n     2\tfile-line\n");
}

#[test]
fn pipe_cat_squeeze_state_carries_across_operands() {
    // `-s' sees one concatenated stream: the pipe's trailing blank and
    // the file's leading blank collapse into ONE (GNU cat.c, WSL 9.4
    // probes pinned in rubash#415).
    let dir = std::env::temp_dir().join("rubash-issue165-cat-pipe");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("leading-blank.txt");
    std::fs::write(&file, b"\nb\n").unwrap();
    let script = format!(
        "printf 'a\\n\\n' | cat -s - {}",
        file.to_string_lossy().replace('\\', "/")
    );
    let (stdout, stderr, code) = run_rubash_script(&script);
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"a\n\nb\n");
}

#[test]
fn pipe_cat_v_still_renders_nonprinting_bytes() {
    // The pre-fix `-v' behavior (the only option the old inline arm
    // honored) must survive the unification: control bytes render as
    // ^X, TAB/LF pass through.
    let (stdout, stderr, code) = run_rubash_script(r"printf 'a\001b\tc\n' | cat -v");
    assert_eq!(code, Some(0));
    assert_eq!(stderr, b"");
    assert_eq!(stdout, b"a^Ab\tc\n");
}
