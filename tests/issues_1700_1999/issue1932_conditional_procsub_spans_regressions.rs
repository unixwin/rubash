//! rubash#1932 — `[[ ]]` comparison operands never materialized
//! their process-substitution spans: `[[ <(echo hi) == /dev/fd/* ]]`
//! was false and `[[ -n <(echo hi) ]]` reduced the span to the empty
//! literal text, because the conditional word expander had no
//! materialization step at all while the #1915 recorded-span table
//! was only consulted for operator positions. GNU cond_expand_word
//! expands the operand as a word; `<(...)` becomes the /dev/fd path
//! of the running substitution. The expander now splices each
//! recorded span of the operand into a live fd word (input direction
//! runs the child and serves its output; output direction reserves
//! the fd word and registers the drain with the same conditional
//! wrapper lifecycle command words use), restricted to spans the
//! parse actually recorded for THIS word, so a quoted `"<(echo hi)"`
//! operand stays literal text.
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
            std::env::temp_dir().join(format!("rubash-i1932-{tag}-{}-{nanos}", std::process::id()));
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
fn issue1932_input_span_operand_materializes_to_an_fd_word() {
    let s = Scratch::new("inputspan");
    // GNU: the operand becomes /dev/fd/N — non-empty, an fd path,
    // and string-equal to another materialization of the same fd
    // class only through the pattern.
    let (out, err, rc) = s.run(
        "[[ <(echo hi) == /dev/fd/* ]] && echo pattern-yes || echo pattern-no\n[[ -n <(echo hi) ]] && echo n-yes || echo n-no\n[[ -p <(echo hi) ]] && echo p-yes || echo p-no\n[[ <(echo hi) == '<(echo hi)' ]] && echo lit-eq || echo not-lit\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "pattern-yes\nn-yes\np-yes\nnot-lit\n");
    assert_eq!(err, "");
}

#[test]
fn issue1932_span_compares_equal_to_direct_substitution_word() {
    let s = Scratch::new("selfeq");
    // GNU: two distinct spans are two distinct fd words; a span
    // compared against its own capture is equal. Using one span
    // twice through a variable keeps this timing-free.
    let (out, err, rc) = s.run(
        "w=$(cat < <(echo payload))\n[[ $w == payload ]] && echo body-ok || echo body-bad\n[[ $(cat < <(echo zz)) == zz ]] && echo direct-ok || echo direct-bad\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "body-ok\ndirect-ok\n");
    assert_eq!(err, "");
}

#[test]
fn issue1932_quoted_span_operand_stays_literal() {
    let s = Scratch::new("quoted");
    // GNU agreement guard: a quoted operand is the literal text —
    // non-empty, equal to itself, and the body never runs.
    let (out, err, rc) = s.run(
        "[[ \"<(echo hi)\" == '<(echo hi)' ]] && echo lit-yes || echo lit-no\n[[ -n \"<(echo hi)\" ]] && echo n-yes || echo n-no\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "lit-yes\nn-yes\n");
    assert_eq!(err, "");
    let marker = s.dir.join("ran");
    let script = format!(
        "[[ -n \"<(touch {})\" ]] && echo checked\n",
        marker.display()
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "checked\n");
    assert_eq!(err, "");
    assert!(!marker.exists(), "quoted span body must not run");
}

#[test]
fn issue1932_output_span_operand_registers_an_fd_word() {
    let s = Scratch::new("outputspan");
    // GNU: an output span in operand position also becomes an fd
    // word (the reader drains after the conditional finishes).
    let (out, err, rc) =
        s.run("[[ >(cat) == /dev/fd/* ]] && echo pattern-yes || echo pattern-no\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "pattern-yes\n");
    assert_eq!(err, "");
}
