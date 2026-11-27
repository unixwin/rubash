//! rubash#1933 — the `[[ ]]` parser rejected a word that STARTS
//! with a process substitution and continues with adjacent text:
//! `[[ -n <(echo hi)x ]]` was a syntax error. GNU parse.y:5305
//! read_token_word assembles ONE word from adjacent fragments — a
//! process substitution is a word fragment, and unspaced text after
//! its closing paren (another fragment, a literal run, a second
//! span) belongs to the same word; cond_expand_word then expands
//! the glued word, materializing the span in place. The compound
//! word collector stopped after the first span and then demanded an
//! operator or terminator. It now glues every adjacent fragment the
//! lexer hands it without intervening whitespace, which is also how
//! the case-command subject (the collector's other caller) reads
//! such words.
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
            std::env::temp_dir().join(format!("rubash-i1933-{tag}-{}-{nanos}", std::process::id()));
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
fn issue1933_word_initial_span_with_literal_suffix_parses() {
    let s = Scratch::new("suffix");
    // GNU: `<(echo hi)x` is one word; it materializes to
    // /dev/fd/Nx, so -n is true and the fd-prefix pattern matches.
    let (out, err, rc) = s.run(
        "[[ -n <(echo hi)x ]] && echo n-yes || echo n-no\n[[ <(echo hi)x == /dev/fd/*x ]] && echo glue-yes || echo glue-no\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "n-yes\nglue-yes\n");
    assert_eq!(err, "");
}

#[test]
fn issue1933_word_initial_span_with_span_and_variable_suffixes() {
    let s = Scratch::new("moreglue");
    // GNU: adjacent spans and $-fragments glue the same way.
    let (out, err, rc) = s.run(
        "suffix=Z\n[[ -n <(echo hi)>(cat) ]] && echo span-span-yes || echo span-span-no\n[[ -n <(echo hi)$suffix ]] && echo span-var-yes || echo span-var-no\n[[ <(echo hi)$suffix == /dev/fd/*Z ]] && echo var-glue-yes || echo var-glue-no\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "span-span-yes\nspan-var-yes\nvar-glue-yes\n");
    assert_eq!(err, "");
}

#[test]
fn issue1933_case_subject_with_span_suffix() {
    let s = Scratch::new("casesubj");
    // GNU: the case subject is collected by the same word assembly —
    // a glued span subject matches the fd pattern.
    let (out, err, rc) =
        s.run("case <(echo hi)x in\n  /dev/fd/*) echo fd-case ;;\n  *) echo other-case ;;\nesac\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "fd-case\n");
    assert_eq!(err, "");
}

#[test]
fn issue1933_spaced_span_still_needs_an_operator() {
    let s = Scratch::new("spaced");
    // GNU agreement guard: with whitespace, `<(echo hi) x` inside
    // [[ ]] is TWO words — a syntax error there, not a glued word.
    let (out, err, rc) = s.run("[[ <(echo hi) x ]]\n");
    assert_ne!(rc, 0);
    assert_eq!(out, "");
    assert!(err.contains("syntax error"), "unexpected stderr: {err}");
}
