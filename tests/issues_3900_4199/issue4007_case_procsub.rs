//! rubash#4007 — `<(...)` / `>(...)` text carried in a variable's
//! VALUE is data in the `case` subject, never a process substitution.
//!
//! GNU anchor: execute_cmd.c:3686 execute_case_command expands the
//! subject once with `expand_word_leave_quoted`; the `<`/`>` arms of
//! expand_word_internal (subst.c:11365-11391) recognize a process
//! substitution only while scanning the parser WORD itself — gated by
//! `quoted & (Q_HERE_DOCUMENT|Q_DOUBLE_QUOTES)` and
//! `word->flags & W_NOPROCSUB` — never inside an expansion RESULT.
//! Rubash's subject path re-scanned the EXPANDED text for `<(`/`>(`,
//! so `v='x >(touch PWNED) y'; case "$v" in *zz*) ;; esac` executed
//! the value's literal text (and likewise materialized source-quoted
//! `"<(...)"` spans). The subject now materializes only the parser's
//! quote-aware scan of the raw word (word_metadata.process_substitutions)
//! before expansion — the same vehicle the case-pattern side uses
//! (rubash#2764).
//!
//! Expectations measured against WSL GNU bash 5.3.20
//! (/usr/local/bin/bash), file mode, one script per shape.

use std::path::PathBuf;
use std::process::Command;

struct Run {
    dir: PathBuf,
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_file(tag: &str, script: &str) -> Run {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash4007-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("case.sh"), script).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("case.sh")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &dir)
        .output()
        .expect("run rubash");
    Run {
        dir,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn assert_clean(run: &Run, script: &str) {
    assert_eq!(
        run.code,
        Some(0),
        "rc\nscript:\n{script}\nstdout: {:?}\nstderr: {:?}",
        run.stdout,
        run.stderr
    );
    assert_eq!(
        run.stderr, "",
        "stderr\nscript:\n{script}\nstdout: {:?}",
        run.stdout
    );
}

/// The issue's core repro: `>(...)` text in a quoted expansion result
/// must not execute — no PWNED marker, `*zz*` arm not taken.
#[test]
fn issue4007_quoted_var_output_procsub_text_is_data() {
    let script = "v='x >(: >PWNED) y'\ncase \"$v\" in *zz*) echo A;; *) echo B;; esac\n[ -e PWNED ] && echo LEAK || echo CLEAN\n";
    let run = run_file("dq-output", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "B\nCLEAN\n", "stdout: {:?}", run.stdout);
}

/// Same text unquoted: the whole value is still data — the case word
/// is not field-split (GNU expand_word_leave_quoted), so `*hi*`
/// matches the literal `<(echo hi)` text.
#[test]
fn issue4007_unquoted_var_input_procsub_text_is_data() {
    let script = "v='x <(echo hi) y'\ncase $v in *hi*) echo A;; *) echo B;; esac\n";
    let run = run_file("uq-input", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "A\n", "stdout: {:?}", run.stdout);
}

/// Every subject form of the issue: braced, concatenated, bare —
/// `<(`/`>(` contributed by the value never runs.
#[test]
fn issue4007_all_subject_forms_keep_value_text_literal() {
    let script = concat!(
        "v='x >(: >P1) y'\n",
        "case \"${v}\" in *zz*) echo A;; *) echo B;; esac\n",
        "case \"x$v\" in *zz*) echo C;; *) echo D;; esac\n",
        "w='y >(: >P2) z'\n",
        "case $w in *zz*) echo E;; *) echo F;; esac\n",
        "[ -e P1 ] && echo P1-LEAK || echo P1-CLEAN\n",
        "[ -e P2 ] && echo P2-LEAK || echo P2-CLEAN\n"
    );
    let run = run_file("all-forms", script);
    assert_clean(&run, script);
    assert_eq!(
        run.stdout, "B\nD\nF\nP1-CLEAN\nP2-CLEAN\n",
        "stdout: {:?}",
        run.stdout
    );
}

/// A procsub span that arrives via a command-substitution RESULT is
/// likewise data, not a live substitution.
#[test]
fn issue4007_comsub_result_procsub_text_is_data() {
    let script = "case \"$(printf '<(x)')\" in '<(x)') echo A;; *) echo B;; esac\n";
    let run = run_file("comsub-result", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "A\n", "stdout: {:?}", run.stdout);
}

/// Source-level input procsub still materializes: the subject is the
/// `/dev/fd/N` word form, matched by the fd-path pattern and by `*`.
#[test]
fn issue4007_source_input_procsub_still_materializes() {
    let script = concat!(
        "case <(echo hi) in /dev/fd/*|/proc/self/fd/*) echo OK-FD;; *) echo MISS;; esac\n",
        "case <(echo hi) in *) echo OK-STAR;; esac\n",
        "case x<(echo hi)y in *fd*) echo OK-EMBED;; *) echo MISS2;; esac\n"
    );
    let run = run_file("src-input", script);
    assert_clean(&run, script);
    assert_eq!(
        run.stdout, "OK-FD\nOK-STAR\nOK-EMBED\n",
        "stdout: {:?}",
        run.stdout
    );
}

/// Source-level output procsub materializes too.
#[test]
fn issue4007_source_output_procsub_still_materializes() {
    let script = "case >(:) in /dev/fd/*|/proc/self/fd/*) echo OK;; *) echo MISS;; esac\n";
    let run = run_file("src-output", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "OK\n", "stdout: {:?}", run.stdout);
}

/// A source-quoted `<(...)` span is literal data at the word level —
/// GNU's Q_DOUBLE_QUOTES gate (subst.c:11370) — and matches the same
/// literal pattern.
#[test]
fn issue4007_source_quoted_procsub_stays_literal() {
    let script = concat!(
        "case '<(echo hi)' in '<(echo hi)') echo SQ;; *) echo MISS;; esac\n",
        "case \"<(echo hi)\" in '<(echo hi)') echo DQ;; *) echo MISS2;; esac\n",
        "case a\"<(echo hi)\"b in 'a<(echo hi)b') echo GLUE;; *) echo MISS3;; esac\n"
    );
    let run = run_file("src-quoted", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "SQ\nDQ\nGLUE\n", "stdout: {:?}", run.stdout);
}

/// Pattern-side procsub is unaffected: it still materializes per arm
/// (rubash#2764), so the fd-path word matches.
#[test]
fn issue4007_pattern_procsub_unaffected() {
    let script = "case /dev/fd/63 in <(true)) echo MATCH;; *) echo NOMATCH;; esac\n";
    let run = run_file("pat-side", script);
    assert_clean(&run, script);
    assert_eq!(run.stdout, "MATCH\n", "stdout: {:?}", run.stdout);
}
