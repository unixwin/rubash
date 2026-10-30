//! rubash#1982 — the unterminated-here-document warning was missing
//! for the header-close shape `x=$(cat <<EOF)` (the `)` closes the
//! substitution on the header line, so the heredoc body runs to EOF
//! with no delimiter line): GNU warns
//! `warning: command substitution: 1 unterminated here-document` and
//! still substitutes the body; master was silent. The shape is owned
//! by the reader/feeder layer, not the expansion layer: the feeder
//! relocates the closing paren past the heredoc delimiter
//! (`print_cmd.c:164 print_comsub()` reprints the command the same
//! way), which erases the header-close evidence before the heredoc
//! plan runs — and the plan's surviving warning site was additionally
//! gated on typed-input warnings, which a script file never sets.
//! The relocation now marks the extracted stream when the paren that
//! closed the substitution sat on the header line, and the plan warns
//! whenever the substitution's own evidence says the heredoc was
//! unterminated by its header.
//!
//! All expectations measured against GNU bash 5.3.20.

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
            std::env::temp_dir().join(format!("rubash-i1982-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        // Written without a trailing newline where the shape needs it:
        // callers embed exactly the bytes they mean.
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
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
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1982_header_close_warns_and_still_substitutes_the_body() {
    let s = Scratch::new("headerclose");
    // No trailing newline: the input ends inside the heredoc body.
    let (out, err, rc) = s.run("x=$(cat <<EOF)\nhello body\nEOF");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
    // The substitution still yields the body text.
    let (out, err, rc) = s.run("x=$(cat <<EOF)\nhello body\nEOF\nprintf 'V=[%s]\\n' \"$x\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[hello body]\n");
    assert_eq!(
        err,
        "SH: line 1: warning: command substitution: 1 unterminated here-document\n"
    );
}

#[test]
fn issue1982_paren_on_own_line_stays_silent() {
    let s = Scratch::new("ownline");
    // GNU agreement guard: the well-formed shape's paren sits on its
    // own line after the delimiter — no warning.
    let (out, err, rc) = s.run("x=$(cat <<EOF\nhello body\nEOF\n)\nprintf 'V=[%s]\\n' \"$x\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[hello body]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1982_paren_on_delimiter_line_warns_delimited_only() {
    let s = Scratch::new("delimline");
    // GNU agreement guard: `EOF)` terminates the heredoc with the
    // delimiter, so only the delimited-by-EOF warning fires — never
    // the unterminated one.
    let (out, err, rc) = s.run("x=$(cat <<EOF\nhello body\nEOF)\nprintf 'V=[%s]\\n' \"$x\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "V=[hello body]\n");
    assert_eq!(
        err,
        "SH: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `EOF')\n"
    );
}
