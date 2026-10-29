//! rubash#2025 — a heredoc whose header line carries the substitution's
//! closing `)` (`x=$(cat <<EOF file)`) with NO delimiter line anywhere
//! after it used to die at end of input with
//! `unexpected EOF while looking for matching ')'` (rc 2). GNU bash
//! 5.3.20 closes the substitution at that header `)` (parse.y:4513/4519
//! parse_comsub sets PST_EOFTOKEN and `shell_eof_token = close`), warns
//! `command substitution: 1 unterminated here-document`
//! (parse.y:4563-4567), gathers the pending body from the following
//! lines to end of input warning `here-document at line N delimited by
//! end-of-file` per make_cmd.c:627, and still runs the command —
//! print_comsub (parse.y:4655, print_cmd.c:164) reprints it with the
//! gathered body inside the `)`. The lexer's finish pass now completes
//! the accumulated line to that reprint (two IFS_GLUE marks carry the
//! eof-gather evidence to the expansion-side warning arm).
//!
//! Layout discipline the pins encode: the gather consumes every physical
//! line AFTER the header line, so a command meant to observe the
//! substitution's result must share the header line's command (or run
//! before it) — a following line becomes heredoc body, exactly as GNU's
//! gather-to-EOF consumes it.
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
            std::env::temp_dir().join(format!("rubash-i2025-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// The host environment is inherited (issue1371 precedent): the
    /// substitutions here run real/emulated `cat`, which must resolve on
    /// PATH for the shape to reach the faces under test. A cat-less
    /// PATH turns the operand faces into `cat: command not found` (rc
    /// 127), which is GNU-correct for a cat-less system but not this
    /// file's subject.
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
        // the host renders (`C:\...`, `/c/...`); normalize each line to
        // `SH: ` at the script-name boundary so the pins stay byte-exact
        // about the GNU-relevant part.
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
fn issue2025_file_operand_no_body_delivers_the_file() {
    let s = Scratch::new("fileoperand");
    // The issue's w6 shape on the final line: header close + file
    // operand + no body/EOF line. GNU: rc 0, the substitution yields
    // the FILE's contents (the operand wins over the empty heredoc),
    // and both warnings fire at the comsub's line.
    fs::write(s.dir.join("operand.txt"), "file body\n").unwrap();
    let script = "echo [$(cat <<EOF operand.txt)]\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[file body]\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n\
         SH: line 1: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}

#[test]
fn issue2025_no_operand_body_line_gathered_to_eof() {
    let s = Scratch::new("gather");
    // With no operand the gathered body IS the output: the following
    // physical line feeds the heredoc, and the delimited-by-EOF warning
    // reports at the line the gather ended (start + body lines).
    let script = "echo $(cat <<EOF)\nhi\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n\
         SH: line 2: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}

#[test]
fn issue2025_gather_consumes_the_rest_of_the_script() {
    let s = Scratch::new("rest");
    // GNU's gather runs to end of input: a script line AFTER the
    // header-close shape is body text, never a command. The printf line
    // must therefore not execute — GNU prints only the two warnings,
    // numbered from the comsub's line.
    fs::write(s.dir.join("operand.txt"), "file body\n").unwrap();
    let script = "x=$(cat <<EOF operand.txt)\nprintf 'NEVER' \"$x\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n\
         SH: line 2: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}

#[test]
fn issue2025_terminated_header_close_keeps_1982_behavior() {
    let s = Scratch::new("terminated");
    // Regression guard (rubash#1982): with the delimiter line present
    // the completion must stay silent — one warning at the comsub's
    // line, and the operand still wins for cat.
    fs::write(s.dir.join("operand.txt"), "file body\n").unwrap();
    let script = "x=$(cat <<EOF operand.txt)\nhello body\nEOF\nprintf '[%s]' \"$x\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[file body]");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
}

#[test]
fn issue2025_paren_on_own_line_stays_silent() {
    let s = Scratch::new("ownline");
    // Regression guard: the well-formed shape (paren on its own line
    // after the delimiter) must not complete or warn.
    let script = "x=$(cat <<EOF\nhello body\nEOF\n)\nprintf '[%s]' \"$x\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "[hello body]");
    assert_eq!(err, "");
}

#[test]
fn issue2025_plain_top_level_unterminated_heredoc_unchanged() {
    let s = Scratch::new("plain");
    // Regression guard: a top-level `cat <<EOF` with no substitution is
    // the pre-existing gather-to-EOF machinery (rubash#1028 family) —
    // one delimited warning at the last physical line, body delivered.
    let script = "cat <<EOF\nhi\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n");
    assert_eq!(
        err,
        "SH: line 2: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}
