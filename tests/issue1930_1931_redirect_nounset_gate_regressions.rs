//! rubash#1930 / rubash#1931 — the redirect nounset gate scanned the
//! RAW target text with no quote awareness, and (for a live reference
//! inside double quotes) reported a diagnostic the real expansion then
//! repeated.
//!
//! #1930: `set -u; echo hi > '$NOPE'` — GNU creates the file `$NOPE`
//! and succeeds: inside single quotes there is no expansion at all
//! (redir.c:298 redirection_expand -> expand_words_no_vars ->
//! expand_word_internal, where a single-quoted span is literal data).
//! The gate read the raw text, saw `$NOPE`, and misfired.
//!
//! #1931: `set -u; cat < "<(awk '$NOPE')"` — the target is a double-
//! quoted string, so `$NOPE` IS a live reference (single quotes are
//! literal inside double quotes) and GNU reports `NOPE: unbound
//! variable` exactly ONCE, contained in the external command's child
//! (script continues, cat's status 1). Rubash printed it twice: the
//! redirect-target materialization judged the COOKED target by text
//! shape, treated the quoted span as a real process substitution and
//! spawned its body BEFORE the gate ran — the body's own expansion
//! (inner quotes already sentinel-decoded, so `$NOPE` was live in the
//! child) printed the diagnostic first, and the gate printed it again.
//! GNU never recognizes a substitution there: expand_word_internal's
//! '<'/'>' cases (subst.c:11365-11372) fall through to add_character
//! when the character is quoted, so `"<(...)"` is a literal filename.
//!
//! Fix shape: the gate judges the target the way expansion does —
//! single-quoted spans and backslash-escaped dollars in the raw text
//! are masked before the scan (quote awareness the cooked word faces
//! get from carrier encoding) — and redirect-target materialization
//! only treats a target as a process substitution when the parser's
//! quote-aware scan finds the span UNQUOTED in the raw target.
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (LC_ALL=C.UTF-8), 2026-10-29. Each shape runs as its own script
//! because a nounset fatal in a main-shell (builtin) command ends the
//! script — that containment difference is part of the expectations.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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
            std::env::temp_dir().join(format!("rubash-i193x-{tag}-{}-{nanos}", std::process::id()));
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
        let status = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .env("LC_ALL", "C.UTF-8")
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .status()
            .unwrap();
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path),
            status.code().unwrap_or(-1),
        )
    }

    fn file(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.dir.join(name)).ok()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1930_single_quoted_redirect_target_is_not_a_reference() {
    let s = Scratch::new("sq");
    let (out, err, rc) = s.run("set -u; echo hi > '$NOPE'; echo rc=$?; cat '$NOPE'\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=0\nhi\n");
    assert_eq!(err, "");
    assert_eq!(s.file("$NOPE").as_deref(), Some("hi\n"));
}

#[test]
fn issue1930_quoted_and_escaped_segments_are_not_references() {
    // GNU: a single-quoted SEGMENT (prefix or suffix around it), a
    // braced reference inside single quotes, and a backslash-escaped
    // dollar are all literal filename text.
    for (tag, script, name) in [
        (
            "mixpre",
            "set -u; echo hi > pre'$NOPE'; echo rc=$?; cat 'pre$NOPE'\n",
            "pre$NOPE",
        ),
        (
            "mixpost",
            "set -u; echo hi > '$NOPE'post; echo rc=$?; cat '$NOPEpost'\n",
            "$NOPEpost",
        ),
        (
            "braced",
            "set -u; echo hi > '${NOPE}'; echo rc=$?; cat '${NOPE}'\n",
            "${NOPE}",
        ),
        (
            "escaped",
            "set -u; echo hi > \\$NOPE; echo rc=$?; cat '$NOPE'\n",
            "$NOPE",
        ),
    ] {
        let s = Scratch::new(tag);
        let (out, err, rc) = s.run(script);
        assert_eq!(rc, 0, "{tag} stderr: {err}");
        assert_eq!(out, "rc=0\nhi\n", "{tag}");
        assert_eq!(err, "", "{tag}");
        assert_eq!(s.file(name).as_deref(), Some("hi\n"), "{tag}");
    }
}

#[test]
fn issue1930_live_redirect_references_still_report_once() {
    // Guards: bare and double-quoted targets keep the GNU fatal — one
    // diagnostic, the builtin's script ends with status 1.
    for (tag, script) in [
        ("bare", "set -u; echo hi > $NOPE; echo rc=$?\n"),
        ("dq", "set -u; echo hi > \"$NOPE\"; echo rc=$?\n"),
        ("dq_sq", "set -u; echo hi > \"'$NOPE'\"; echo rc=$?\n"),
    ] {
        let s = Scratch::new(tag);
        let (out, err, rc) = s.run(script);
        assert_eq!(rc, 1, "{tag} out: {out} err: {err}");
        assert_eq!(out, "", "{tag}");
        assert_eq!(
            err.matches("NOPE: unbound variable").count(),
            1,
            "{tag} stderr: {err}"
        );
    }
}

#[test]
fn issue1931_quoted_fake_procsub_reports_exactly_once() {
    let s = Scratch::new("fakeprocsub");
    // External target: the diagnostic is contained (GNU), the script
    // continues, and it appears exactly once — not gate + spawned body.
    let (out, err, rc) = s.run("set -u; cat < \"<(awk '$NOPE')\"; echo rc=$?; echo after\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "rc=1\nafter\n");
    assert_eq!(
        err.matches("NOPE: unbound variable").count(),
        1,
        "stderr: {err}"
    );
}

#[test]
fn issue1931_quoted_fake_procsub_body_never_executes() {
    // GNU: a quoted "<(...)" redirect target is a literal filename —
    // the open fails ENOENT and the body NEVER runs, in double or
    // single quotes alike. Master ran the body (the marker appeared).
    for (tag, script) in [
        ("dqfake", "cat < \"<(touch RAN_MARKER)\"; echo rc=$?\n"),
        ("sqfake", "cat < '<(touch RAN_MARKER)'; echo rc=$?\n"),
    ] {
        let s = Scratch::new(tag);
        let (out, err, rc) = s.run(script);
        assert_eq!(rc, 0, "{tag} stderr: {err}");
        assert_eq!(out, "rc=1\n", "{tag} stderr: {err}");
        assert!(
            err.contains("No such file or directory"),
            "{tag} stderr: {err}"
        );
        assert!(
            s.file("RAN_MARKER").is_none() && !s.dir.join("RAN_MARKER").exists(),
            "{tag}: quoted fake procsub body executed"
        );
    }
}

#[test]
fn issue1931_unquoted_real_procsub_still_materializes() {
    // Control: the raw-quote judgment must not block REAL process
    // substitutions — an unquoted redirect target still runs its body
    // and a word-form substitution still yields its /dev/fd name.
    let s = Scratch::new("realprocsub");
    let (out, err, rc) = s.run("cat < <(printf 'real\\n'); echo <(printf x); echo rc=$?\n");
    assert_eq!(rc, 0, "stderr: {err}");
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("real"), "stdout: {out}");
    let word_form = lines.next().unwrap_or("");
    assert!(
        word_form.starts_with("/dev/fd/"),
        "word-form procsub did not materialize: {out}"
    );
    assert_eq!(lines.next(), Some("rc=0"), "stdout: {out}");
}
