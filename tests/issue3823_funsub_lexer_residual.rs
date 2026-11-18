//! rubash#3823 — funsub `${ ... }` bodies with `#` comments/heredocs, and
//! `${x:-$(...)}` whose closing `}` leaked into the output (true-lexer
//! layer residual of #3813).
//!
//! Root causes (three scanners, one defect class): the funsub/`${...}`
//! extent scanners treated quote characters inside `#` comments and
//! heredoc bodies as real quotes, so an apostrophe in a comment
//! (`# don't`) opened a PHANTOM single-quote that swallowed the rest of
//! the input — and a `}` line inside a heredoc body either closed the
//! funsub early or was swallowed with it. The `${x:-$(...)}` leak is the
//! same class one layer down: the walker's `${...}` body collector had no
//! `$(...)` unit arm, so the `'` in the comsub's comment swallowed the
//! closing `}` and it leaked into the default word as literal text.
//!
//! The fix gives each scanner the opaque-span discipline the corrected
//! paren scanner (`skip_parenthesized_unit_corrected`, skip.rs) already
//! carries:
//! - `Lexer::skip_braced` funsub branch (skip.rs): `#`-comment arm
//!   (token-boundary gated) plus a `<<` heredoc arm.
//! - `extract_funsub_body_tail` (command_substitution.rs): the same two
//!   arms for the immutable walker's funsub collector.
//! - `collect_braced_parameter_body_into` (command_subst_helpers.rs):
//!   skip `$(...)` as one unit via the corrected paren scanner.
//! - `skip_parenthesized` (dolbrace.rs): `#`-comment arm (comment-only;
//!   heredocs stay with the corrected scanner).
//! - `skip_dollar_brace` (alias_stream.rs): funsub branch delegating to
//!   `skip_funsub_body`, so the pre-scan's main loop no longer swallows
//!   a funsub word to EOF and hides the closing alias `E`.
//!
//! GNU anchors: parse.y:3631-3645 `shell_getc` — `#` at a shell_getc
//! position ALWAYS comments ("Discard until EOL or EOF"); parse.y:4451
//! `parse_comsub` parses the body with the real parser, so comments and
//! heredocs inside `$(...)` / `${ ... }` are opaque spans; parse.y:5506
//! funsub (parser.h:85 FUNSUB_CHAR); make_cmd.c:512 `make_here_document`
//! reads the body as raw lines; subst.c:1825 `extract_dollar_brace_string`
//! hands a `$(...)` span inside `${...}` to parse_comsub.
//! Probed on GNU bash 5.3.20 (~/workspace/rubash-audit/gnu-bash-5.3.20):
//! all four shapes are rc=0 there with the expected values.
//!
//! NOTE: these four pins were split out of the #3813 pin file (commit
//! 7c8d28de) as issue3813_* and are re-established here as issue3823_*.
//! The d1 pin turns green with this PR's fix. The f1/h2/h3 pins also
//! need the mut walker's funsub collector
//! (`expand_current_shell_braced_substitution`, embedded_mutations.rs),
//! which is BLOCKED on the in-flight #3817 (same file) — see REPORT.md.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-i3823-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn alias_frame(body: &str) -> String {
    // `f` is invoked after the group closes: the parse verdict (rc) is
    // the defect signal, but the body must actually RUN for the
    // `v=[...]` stdout pins to mean anything.
    format!(
        "shopt -s expand_aliases\nalias T='{{ f() {{'\nalias E='}}; }}'\nT\n{body}\nE\nf\necho done\n"
    )
}

fn assert_parses(scratch: &Scratch, script: &str, want_stdout: &str) {
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}");
    assert!(
        !err.contains("syntax error"),
        "unexpected syntax error:\n{err}"
    );
    assert!(out.contains(want_stdout), "stdout mismatch:\n{out}");
    assert!(out.contains("done"), "trailing marker missing:\n{out}");
}

/// f1: `#` comment with an apostrophe inside a funsub body — the `'`
/// must not open a phantom quote that swallows the closing `}`.
#[test]
fn issue3823_funsub_comment_quote() {
    let scratch = Scratch::new("funsub");
    let script = alias_frame("  v=${ echo hi # don't\n}\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}

/// h2: heredoc inside a funsub body whose body carries an apostrophe —
/// the body is raw text, so the quote must not leak into the extent scan.
#[test]
fn issue3823_funsub_heredoc_quote() {
    let scratch = Scratch::new("funsubhd");
    let script = alias_frame("  v=${ cat <<EOF\nit's here\nEOF\n}\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[it's here]");
}

/// h3: heredoc inside a funsub body whose body carries a lone `}` line —
/// the line is heredoc data, not the funsub's closer.
#[test]
fn issue3823_funsub_heredoc_brace_line() {
    let scratch = Scratch::new("funsubhb");
    let script = alias_frame("  v=${ cat <<EOF\n}\nEOF\n}\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[}]");
}

/// d1: `${x:-$(...)}` whose `$(...)` body carries a `#` comment with an
/// apostrophe — the closing `}` must close the `${...}`, not leak into
/// the default word as literal text.
#[test]
fn issue3823_nested_comsub_in_brace_expansion() {
    let scratch = Scratch::new("brace");
    let script = alias_frame("  v=${x:-$(echo hi # don't\n)}\n  echo \"v=[$v]\"");
    assert_parses(&scratch, &script, "v=[hi]");
}
