//! rubash#2924 — the inputrc parser behind `bind -f` and the
//! startup ~/.inputrc load dropped OLD-STYLE binding lines
//! (`Control-t: undo`): _rl_read_init_file feeds every line to
//! rl_parse_and_bind (lib/readline/bind.c:2260), whose old-style
//! arm (bind.c:1594) binds keyname specs exactly like quoted ones,
//! but rubash's parse_inputrc_with_env only routed lines through
//! the quoted-line parser and silently ignored the rest — the file
//! loaded, conditionals and `set` lines applied, and the bindings
//! never landed. The fix routes a line the quoted parser does not
//! claim through the same parse_old_style_binding the positional
//! `bind` argument uses (itself rubash#2914). Expectations are the
//! GNU bash 5.3.20 oracle's verbatim output for the same files.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Run {
    stdout: String,
    stderr: String,
}

fn run_with_inputrc(script: &str, inputrc: Option<&str>) -> Run {
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    let out = Command::new(RUBASH)
        .args(["-c", script])
        .env("INPUTRC", inputrc.unwrap_or(EMPTY_INPUTRC))
        .stdin(Stdio::null())
        .output()
        .expect("spawn rubash");
    let stderr = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.contains("line editing not enabled"))
        .map(|line| format!("{line}\n"))
        .collect();
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr,
    }
}

fn run(script: &str) -> Run {
    run_with_inputrc(script, None)
}

fn write_inputrc(tag: &str, content: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rubash-issue2924-{}-{tag}.inputrc",
        std::process::id()
    ));
    std::fs::write(&path, content).expect("write inputrc fixture");
    path
}

#[test]
fn issue2924_bind_f_loads_old_style_function_lines() {
    let file = write_inputrc("fn", "Control-t: undo\n");
    let script = format!(
        "bind -f {}; bind -q undo; bind -p | grep -F '\"\\C-t\"'",
        file.display()
    );
    let r = run(&script);
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\C-_\".\n\"\\C-t\": undo\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_bind_f_loads_old_style_macro_lines() {
    let file = write_inputrc("macro", "Control-y: \"echo hi\"\n");
    let script = format!("bind -f {}; bind -s", file.display());
    let r = run(&script);
    assert_eq!(r.stdout, "\"\\C-y\": \"echo hi\"\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_bind_f_loads_old_style_prefix_meta_lines() {
    // The keymap-valued value rides the same route (rubash#2923).
    let file = write_inputrc("meta", "Control-t: prefix-meta\n");
    let script = format!("bind -f {}; bind -q transpose-words", file.display());
    let r = run(&script);
    assert_eq!(
        r.stdout,
        "transpose-words can be invoked via \"\\C-tt\", \"\\et\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_old_style_diagnostic_carries_file_and_line() {
    let file = write_inputrc("diag", "# comment\nFoo-t: undo\n");
    let script = format!("bind -f {}; echo rc=$?", file.display());
    let r = run(&script);
    assert_eq!(r.stdout, "rc=0\n");
    assert_eq!(
        r.stderr,
        format!(
            "readline: {}: line 2: Foo-t: unknown key modifier\n",
            file.display()
        )
    );
}

#[test]
fn issue2924_startup_inputrc_loads_old_style_lines() {
    let file = write_inputrc("startup", "Control-t: undo\n");
    let r = run_with_inputrc("bind -q undo", Some(file.to_str().expect("utf8 path")));
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_old_style_lines_respect_conditionals() {
    let file = write_inputrc(
        "cond",
        "$if mode=emacs\nControl-t: undo\n$endif\n$if mode=vi\nControl-y: undo\n$endif\n",
    );
    let script = format!("bind -f {}; bind -q undo", file.display());
    let r = run(&script);
    assert_eq!(
        r.stdout,
        "undo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\C-_\".\n"
    );
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_old_style_lines_follow_set_editing_mode() {
    // A `set` line earlier in the file moves the keymap the
    // old-style line lands in, exactly like a quoted line.
    let file = write_inputrc("mode", "set editing-mode vi\nControl-t: undo\n");
    let script = format!("bind -f {}; bind -q undo", file.display());
    let r = run(&script);
    assert_eq!(r.stdout, "undo can be invoked via \"\\C-t\".\n");
    assert_eq!(r.stderr, "");
}

#[test]
fn issue2924_mixed_files_still_route_quoted_and_set_lines() {
    let file = write_inputrc(
        "mixed",
        "\"\\C-a\": kill-line\nControl-t: undo\nset completion-ignore-case on\n",
    );
    let script = format!(
        "bind -f {}; bind -p | grep -F '\"\\C-a\"'; bind -q undo; bind -v | grep completion-ignore-case",
        file.display()
    );
    let r = run(&script);
    assert_eq!(
        r.stdout,
        "\"\\C-a\": kill-line\nundo can be invoked via \"\\C-t\", \"\\C-x\\C-u\", \"\\C-_\".\nset completion-ignore-case on\n"
    );
    assert_eq!(r.stderr, "");
}
