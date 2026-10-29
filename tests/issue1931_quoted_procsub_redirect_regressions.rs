//! rubash#1931 — a DOUBLE-quoted `"<(...)"` redirection target was
//! treated as a real process substitution: the substitution body
//! RAN (with side effects) and its unbound parameters were diagnosed
//! twice — once by the nounset gate over the target, once by the
//! body's own expansion. GNU redir.c:298 redirection_expand()
//! expands the target as a word; quotes remove the substitution
//! meaning (parse.y:5305 read_token_word records a process
//! substitution span only for an UNQUOTED `<(`/`>(`), so the target
//! is the literal filename `<(echo hi)` and the open simply fails.
//! The trace of the bug was materialization sites pattern-matching
//! the quote-stripped target VALUE. Simple-command redirect
//! materialization now admits only targets whose recorded
//! process-substitution span covers the whole target
//! (`redirect_target_span_is_recorded`); a quoted shape falls to the
//! ordinary literal-file open, whose ENOENT diagnostic is the single
//! report GNU gives. Compound-command INPUT targets share the
//! quote-blind match but are deliberately not guarded here: the
//! compound bind layer has no literal-target error channel yet, so
//! declining the spawn would leave the compound reading ambient
//! stdin (a hang, worse than the wrong answer) — recorded as a known
//! adjacent divergence.
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
            std::env::temp_dir().join(format!("rubash-i1931-{tag}-{}-{nanos}", std::process::id()));
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
fn issue1931_quoted_input_span_never_runs_and_diagnoses_once() {
    let s = Scratch::new("sfx");
    // GNU: the body never runs (no SFX side effect) and NOPE is
    // reported exactly once. Before the fix the body ran, SFX
    // printed, and NOPE was reported twice.
    let (out, err, rc) = s.run("set -u\ncat < \"<(echo SFX >&2; awk '$NOPE')\"\n");
    assert_eq!(out, "");
    assert!(!err.contains("SFX"), "body must not run: {err}");
    assert_eq!(err.matches("NOPE: unbound variable").count(), 1, "{err}");
    let _ = rc;
}

#[test]
fn issue1931_quoted_input_span_is_a_literal_filename() {
    let s = Scratch::new("literalin");
    // GNU: opening the literal file fails with the ordinary
    // redirection diagnostic; nothing is substituted.
    let (out, err, rc) = s.run("cat < \"<(echo hi)\"\necho \"rc=$?\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1\n");
    assert_eq!(err, "SH: line 1: <(echo hi): No such file or directory\n");
    // A file literally named like the span IS read as that file.
    fs::write(s.dir.join("<(echo hi)"), "literal-body\n").unwrap();
    let (out, err, rc) = s.run("cat < \"<(echo hi)\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "literal-body\n");
    assert_eq!(err, "");
}

#[test]
fn issue1931_quoted_output_span_creates_the_literal_file() {
    let s = Scratch::new("literalout");
    // GNU: the output target is a literal filename; the inner
    // command never runs, so no `captured` file appears.
    let (out, err, rc) = s.run("echo out > \">(cat > captured)\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(err, "");
    assert_eq!(
        fs::read_to_string(s.dir.join(">(cat > captured)")).unwrap(),
        "out\n"
    );
    assert!(!s.dir.join("captured").exists());
}

#[test]
fn issue1931_real_spans_still_materialize() {
    let s = Scratch::new("realspans");
    // GNU agreement guards: unquoted spans keep working in both
    // directions, and a compound OUTPUT fake span writes the literal
    // file (its fall-through has a real error channel).
    let (out, err, rc) =
        s.run("cat < <(echo hi)\ndiff <(echo same) <(echo same)\necho \"diff=$?\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\ndiff=0\n");
    assert_eq!(err, "");
    let (out, err, rc) = s.run("{ echo out; } > \">(cat > captured)\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "");
    assert_eq!(err, "");
    assert_eq!(
        fs::read_to_string(s.dir.join(">(cat > captured)")).unwrap(),
        "out\n"
    );
    assert!(!s.dir.join("captured").exists());
}
