//! rubash#431 regressions: PS1 `\[`/`\]` ignore-markers must never reach a
//! terminal-bound render path — neither as the raw \x01/\x02 bytes nor as
//! literal `\[`/`\]` text.
//!
//! The piped `-i` driver filter (wt70) covered only the terminal-write
//! channel. The leak persisted in the remaining render consumers: the host
//! line-editor channel (`Shell::expand_prompt_string_mut`, the bridge
//! niubash's BashPrompt renders through) and the `${var@P}` prompt
//! transform. The fix funnels every PS1 consumer through
//! `prompt_expansion::strip_prompt_ignore_markers`, keeping real control
//! bytes (ESC color sequences) intact so cursor-positioning geometry
//! survives.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_stdin(stdin: &[u8], env: &[(&str, &str)]) -> Vec<u8> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-i")
        .env_remove("PS1")
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
    [output.stdout, output.stderr].concat()
}

/// The oh-my-bash inretio theme shape: every colored segment wrapped in
/// `\[`/`\]`. The rendered prompt stream must carry the ESC color bytes but
/// neither the marker bytes nor literal bracket text. The PS1 arrives via
/// the environment, so nothing in the echoed input can smuggle the markers.
#[test]
fn inretio_theme_shape_renders_no_markers() {
    let ps1 = "\\[\\e[01;32m\\]\\u@\\h\\[\\e[00m\\]:\\[\\e[01;34m\\]\\w\\[\\e[00m\\]\\$ ";
    let stream = run_stdin(b"exit\n", &[("PS1", ps1)]);
    assert!(!stream.contains(&0x01), "\\x01 leaked: {stream:?}");
    assert!(!stream.contains(&0x02), "\\x02 leaked: {stream:?}");
    assert!(
        !stream.windows(2).any(|w| w == b"\\["),
        "literal \\[ leaked: {}",
        String::from_utf8_lossy(&stream)
    );
    assert!(
        !stream.windows(2).any(|w| w == b"\\]"),
        "literal \\] leaked: {}",
        String::from_utf8_lossy(&stream)
    );
    assert!(
        stream.windows(8).any(|w| w == b"\x1b[01;32m"),
        "ESC[01;32m missing — stripping destroyed the color payload: {}",
        String::from_utf8_lossy(&stream)
    );
}

/// bash-it theme shape (lib/themes base): PS1 assembled from many
/// `\[`-wrapped segments, partly delivered through PROMPT_COMMAND and a
/// command substitution. The PS1 text is built with printf octal escapes
/// (`\134` = backslash, `\133` = `[`), so the echoed input line cannot
/// contaminate the scan. Fragment count across the whole stream must be 0.
#[test]
fn bashit_theme_shape_leaks_zero_fragments() {
    let stream = run_stdin(
        concat!(
            "PROMPT_COMMAND='PS1=\"$(printf \"\\134[\\033[33m\\134]dir\\134[\\033[0m\\134]\\134[\")\\134[\\044 \"'\n",
            "true\n",
            "exit\n"
        )
        .as_bytes(),
        &[],
    );
    let text = String::from_utf8_lossy(&stream);
    let fragments = text.matches("\\[").count() + text.matches("\\]").count();
    assert_eq!(fragments, 0, "marker fragments leaked: {text:?}");
    assert!(!stream.contains(&0x01), "\\x01 leaked: {text:?}");
    assert!(!stream.contains(&0x02), "\\x02 leaked: {text:?}");
    assert!(
        text.contains("dir"),
        "theme text lost from the rendered prompt: {text:?}"
    );
}

/// The host line-editor channel (`expand_prompt_string_mut`): a themed PS1
/// (octal-built so the echo stays clean) plus a starship-style PS0/arithmetic
/// assignment — the expanded prompt the host renders must be marker-free.
#[test]
fn host_channel_expanded_prompt_is_marker_free() {
    let stream = run_stdin(
        concat!(
            "PS1=\"$(printf '\\134[\\033[36m\\134]> \\134[\\033[0m\\134] ')\"\n",
            "STARSHIP_START_TIME=0\n",
            "true\n",
            "exit\n"
        )
        .as_bytes(),
        &[],
    );
    assert!(!stream.contains(&0x01), "\\x01 leaked: {stream:?}");
    assert!(!stream.contains(&0x02), "\\x02 leaked: {stream:?}");
}

/// `${PS1@P}` (GNU subst.c string_transform 'P') routes through the same
/// strip: stdout is a terminal-bound channel, so neither the marker bytes
/// nor literal `\[`/`\]` may appear — while the ESC payload survives.
#[test]
fn prompt_transform_strips_ignore_markers() {
    let stream = run_stdin(
        concat!(
            "PS1=\"$(printf '\\134[\\033[35m\\134]inretio\\134[\\033[0m\\134] \\044 ')\"\n",
            "printf '%s' \"${PS1@P}\" | od -An -c\n",
            "exit\n"
        )
        .as_bytes(),
        &[],
    );
    let text = String::from_utf8_lossy(&stream);
    assert!(!stream.contains(&0x01), "\\x01 leaked: {text:?}");
    assert!(!stream.contains(&0x02), "\\x02 leaked: {text:?}");
    assert!(
        !text.contains("\\[") && !text.contains("\\]"),
        "literal markers leaked through @P: {text:?}"
    );
    assert!(
        text.contains("i   n   r   e   t   i   o"),
        "@P lost the prompt text: {text:?}"
    );
    assert!(
        text.contains("033") && text.contains("3   5   m"),
        "@P lost the ESC color payload: {text:?}"
    );
}
