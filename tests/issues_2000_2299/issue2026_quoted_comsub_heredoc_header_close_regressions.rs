//! rubash#2026 — a command substitution INSIDE a double-quoted word whose
//! heredoc header line carries the substitution's closing `)`:
//! `echo "$(cat <<EOF)"` used to die with a hard extraction error
//! (`unexpected EOF while looking for matching ')'`). GNU bash 5.3.20
//! dispatches a quoted `$(` to parse_comsub with the outer quote state
//! cleared (parse.y:4172-4174, `~P_DQUOTE`), the `)` is the
//! substitution's eof token wherever it sits (parse.y:4513/4519
//! PST_EOFTOKEN), the pending heredoc is warned and gathered
//! (parse.y:4563-4567 + make_cmd.c:627), and the word reprints with the
//! gathered body inside the `)` (parse.y:4655 print_comsub). The
//! lexer's finish-pass completion now models that reader order for the
//! quoted faces: the rebuild moves the header-line tail (the outer
//! word's closing `"`) after the body and the `)`, so the word the
//! executor sees is GNU's reprint.
//!
//! All expectations measured against GNU bash 5.3.20 (WSL), byte for
//! byte, including the warning line numbers.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i2026-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        // The diagnostic prefix carries the script path in whatever form
        // the host renders; normalize each line to `SH: ` at the
        // script-name boundary so the pins stay byte-exact about the
        // GNU-relevant part.
        let raw_err = read(&err_path);
        let trailing_newline = raw_err.ends_with('\n');
        let mut err = raw_err
            .lines()
            .map(|line| match line.find("case.sh: ") {
                Some(pos) => format!("SH: {}", &line[pos + "case.sh: ".len()..]),
                None => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        if trailing_newline {
            err.push('\n');
        }
        (read(&out_path), err, rc)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue2026_quoted_one_liner_warns_and_prints_empty_body() {
    let s = Scratch::new("oneliner");
    // The issue's exact shape, alone at EOF: GNU warns twice (the
    // unterminated here-document inside the substitution, then the
    // gather's delimited-by-end-of-file) and still runs the command —
    // echo with an empty substitution prints one empty line.
    let (out, err, rc) = s.run("echo \"$(cat <<EOF)\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n\
         SH: line 1: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}

#[test]
fn issue2026_quoted_with_body_prints_the_body() {
    let s = Scratch::new("body");
    // With the delimiter line present GNU prints the body and only the
    // header-close warning fires (rubash#1982's unquoted twin).
    let (out, err, rc) = s.run("echo \"$(cat <<EOF)\"\nhello\nworld\nEOF\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hello\nworld\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
}

#[test]
fn issue2026_quoted_word_in_a_larger_command() {
    let s = Scratch::new("larger");
    // The quoted word is one word among several; the substitution still
    // closes at the header `)`, the following line feeds the heredoc,
    // and the trailing word prints after the gathered body.
    let (out, err, rc) = s.run("echo \"$(cat <<EOF)\" tail\nbody line\nEOF\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "body line tail\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
}

#[test]
fn issue2026_quoted_assignment_shape_captures_the_body() {
    let s = Scratch::new("assign");
    // Assignment form, unquoted comsub inside the word but the word
    // itself quoted: the capture must be the gathered body.
    let (out, err, rc) = s.run("echo \"V=$(cat <<EOF)\"\nhello body\nEOF\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=hello body\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
}

#[test]
fn issue2026_unclosed_outer_quote_still_reports() {
    let s = Scratch::new("unclosed");
    // Guard: with the outer quote never closed (`$(cat <<EOF` with no
    // trailing `"`) the word is genuinely unterminated — the completion
    // must decline and the unclosed-construct report must stand, not a
    // rewritten line. The pre-existing report shape is pinned as-is
    // (pristine parity; the GNU quote-face wording is a separate face).
    let (out, err, rc) = s.run("echo \"$(cat <<EOF)\n");
    assert_ne!(rc, 0, "stderr: {err}");
    assert_eq!(out, "");
    assert!(
        err.contains("unexpected EOF"),
        "expected the unclosed-construct report, got: {err}"
    );
}
