//! rubash#1930 — the nounset gate for redirection targets had no
//! quote awareness: `echo hi > '$NOPE'` was rejected with
//! `NOPE: unbound variable` although the target is one literal word.
//! GNU redir.c:298 redirection_expand() expands the target as a word
//! — single-quoted spans hold literal text, and the unbound report
//! only exists for parameters that are actually expanded. The gate
//! scanned the raw target text with a scanner whose contract is
//! carrier-encoded text it could not honor for quoted spans, escaped
//! characters, or `$'...'`. The gate now masks quoted spans out of
//! the raw target before scanning: single-quoted spans and `$'...'`
//! collapse away, backslash pairs are neutralized exactly where the
//! shell neutralizes them (in double quotes only before $ ` " \),
//! and `$(...)` / `${...}` / backtick units are copied verbatim so
//! the scanner still owns parameters that ARE expanded inside a
//! dquoted target.
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
            std::env::temp_dir().join(format!("rubash-i1930-{tag}-{}-{nanos}", std::process::id()));
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
fn issue1930_single_quoted_target_is_a_literal_filename() {
    let s = Scratch::new("squote");
    // GNU: the redirect creates a file literally named $NOPE and the
    // command succeeds; the gate used to reject it as unbound.
    let (out, err, rc) = s.run("set -u\necho hi > '$NOPE'\necho \"rc=$?\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\n");
    assert_eq!(err, "");
    assert_eq!(
        fs::read_to_string(s.dir.join("$NOPE")).unwrap(),
        "hi\n",
        "the literal $NOPE file must hold the redirected output"
    );
}

#[test]
fn issue1930_escaped_and_dollar_quoted_targets_are_literal_too() {
    let s = Scratch::new("escapes");
    // GNU: `\$NOPE` is literal text; `$'$NOPE'` is the ANSI-C literal
    // dollar-N-O-P-E (no parameter is expanded in either).
    let (out, err, rc) =
        s.run("set -u\necho hi > \\$NOPE\necho one=$?\necho yo > $'$NOPE'\necho two=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "one=0\ntwo=0\n");
    assert_eq!(err, "");
    assert!(s.dir.join("$NOPE").exists());
}

#[test]
fn issue1930_expanded_spans_still_report_unbound_exactly_once() {
    let s = Scratch::new("stillfires");
    // GNU agreement guards (the mask must not overreach): a parameter
    // in a double-quoted target and an unquoted one after a quoted
    // prefix still abort the script with exactly one report; one
    // inside $() in a dquoted target fires once there, after which
    // the empty result fails the open as GNU's second line shows.
    let (out, err, rc) = s.run("set -u\necho hi > \"$NOPE\"\n");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(err, "SH: line 2: NOPE: unbound variable\n");
    let (out, err, rc) = s.run("set -u\necho hi > 'pre'$NOPE\n");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(err, "SH: line 2: NOPE: unbound variable\n");
    let (out, err, rc) = s.run("set -u\necho hi > \"$(echo $NOPE)\"\n");
    assert_eq!(rc, 1, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(
        err,
        "SH: line 2: NOPE: unbound variable\nSH: line 2: : No such file or directory\n"
    );
}
