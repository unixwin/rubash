//! Prompt-channel parity regressions from the powerbash10k investigation
//! (niubash 1.3.0 + oh-my-bash, owner's second machine, Bug B lane).
//!
//! The interactive stdin driver (src/script_driver.rs) renders the
//! expanded PS1 on stderr before each read — the channel the niubash
//! product's line editor also renders through
//! (`Shell::sync_bash_prompt_from_env` -> `expand_prompt_string_mut`).
//! Two GNU parity rules are pinned here:
//!
//! 1. readline display.c:437-463 (expand_prompt): the `\[`/`\]` prompt
//!    markers (RL_PROMPT_START/END_IGNORE) exist for width accounting
//!    only — the displayed prompt is assembled WITHOUT them, so the
//!    terminal never receives the \x01/\x02 bytes. WSL GNU bash 5.3.0
//!    piped-`-i` prompt stderr carries no marker bytes (byte-verified
//!    2026-10-03); rubash leaking them corrupted strict VT parsers (a
//!    pyte replay swallowed everything after \x01) — the rendering
//!    corruption family of oh-my-bash themes.
//! 2. PROMPT_DIRTRIM (GNU general.c:942 trim_pathname, applied at
//!    parse.y:6524): oh-my-bash sets PROMPT_DIRTRIM=2, so `\w` in the
//!    theme's top-left segment must be the trimmed `.../last/two` form,
//!    not the full path — otherwise the segment is dozens of columns
//!    longer than GNU's and the theme's right-aligned cursor geometry
//!    (`\033[${cols}G\033[1K\033[1A`) lands differently.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(stdin: &[u8], env: &[(&str, &str)]) -> (String, Vec<u8>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-i")
        .env_remove("PS1")
        .env_remove("PROMPT_DIRTRIM")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        output.stderr,
    )
}

/// The terminal-bound prompt stream must not carry \x01/\x02 (readline
/// strips the ignore markers before display; display.c:437-463).
#[test]
fn interactive_prompt_stderr_has_no_ignore_marker_bytes() {
    let (_stdout, stderr) = run_stdin(
        b"exit\n",
        &[("PS1", "\\[\\e[31m\\]red\\[\\e[0m\\] mark \\w $ ")],
    );
    assert!(
        !stderr.contains(&0x01),
        "\\x01 leaked to the prompt stream: {stderr:?}"
    );
    assert!(
        !stderr.contains(&0x02),
        "\\x02 leaked to the prompt stream: {stderr:?}"
    );
    // The color escapes themselves survive as real ESC bytes.
    assert!(
        stderr.windows(5).any(|w| w == b"\x1b[31m"),
        "ESC[31m missing from prompt: {stderr:?}"
    );
}

/// `\w` under PROMPT_DIRTRIM produces GNU's trimmed form in the rendered
/// prompt (powerbash10k + oh-my-bash lib/shopt.sh:22 sets DIRTRIM=2).
#[test]
fn interactive_prompt_renders_dirtrimmed_w() {
    // A real deep directory: the engine binds PWD from the process cwd at
    // startup (like GNU variables.c), so the trim is driven through a cd.
    // A shallow path is correctly NOT trimmed (general.c:987: an elided
    // span <= 3 characters never trims — GNU prints the full path for a
    // 3-component cwd under DIRTRIM=2 too).
    let deep = std::env::temp_dir()
        .join("rubash-pb10k-dirtrim")
        .join("a/bb/ccc/dddd/eeeee");
    std::fs::create_dir_all(&deep).expect("mkdir deep");
    let deep_msys = format!(
        "/c{}",
        deep.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("C:")
    );
    let (_stdout, stderr) = run_stdin(
        format!("cd '{deep_msys}'\nexit\n").as_bytes(),
        &[("PS1", "left <\\w> right "), ("PROMPT_DIRTRIM", "2")],
    );
    let text = String::from_utf8_lossy(&stderr);
    let rendered = text
        .lines()
        .find(|line| line.contains("left <..."))
        .unwrap_or_else(|| text.lines().last().unwrap_or(""));
    let inner = rendered
        .split_once("left <")
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(path, _)| path.to_string())
        .expect("prompt segment");
    assert_eq!(inner, ".../dddd/eeeee", "line {rendered:?}");
    let _ = std::fs::remove_dir_all(deep.parent().unwrap().parent().unwrap());
}
