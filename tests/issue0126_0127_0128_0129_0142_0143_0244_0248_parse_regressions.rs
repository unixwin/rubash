//! W4 backfill slice 1 — parser faces.
//!
//! * rust#126 — a function body's reprint (`declare -f`) must not leak
//!   the engine's internal quote-carrier markers.
//! * rust#127 — `${var#prefix}` strips a backslash-containing prefix
//!   literally (no glob interpretation of the backslashes).
//! * rust#128 — a stray `)` as the first token of a later line must not
//!   poison the parse of the line before it.
//! * rust#129 — nested capture groups in `[[ =~ ]]` match correctly.
//! * rust#142 — text that merely looks like zsh syntax inside an
//!   untaken `if` branch must not break parsing of the whole script.
//! * rust#143 — with `extglob` off, `@(a|b)` in a case pattern is a
//!   parse-time syntax error (rc 2), not a silent literal match.
//! * rust#244 — a function name containing a mid-word `{...}` segment
//!   (`layer:{selection}/declare`) is a valid function definition.
//! * rust#248 — a herestring inside a function body feeds the command's
//!   stdin just like at top level.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
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
            std::env::temp_dir().join(format!("rubash-bfw4-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue126_function_reprint_has_no_carrier_markers() {
    let s = Scratch::new("i126");
    let (out, err, rc) = s.run(
        "dumper() { local -a a; a+=(\"x y\" 'z'); printf '%s|%s' \"${a[0]}\" \"${a[1]}\"; }\ndeclare -f dumper | grep -c '__RUBASH'\ndeclare -f dumper | grep -c 'x y'\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "0\n1\n");
}

#[test]
fn issue127_strip_prefix_with_backslashes() {
    // READJUDICATION (adj-semantics §issue127): the original pin's
    // Rust literal carried one escape level too many — the script
    // it wrote had DOUBLED backslashes in the value, so the
    // single-backslash quoted pattern was not a prefix and GNU
    // (like rubash) stripped nothing. The literal below writes
    // the intended single-backslash script; on it, GNU bash
    // 5.3.20 and rubash both produce `\alice\miniforge3`
    // byte-identically (quoted pattern treated literally, #127's
    // actual behavior).
    let s = Scratch::new("i127");
    let (out, err, rc) =
        s.run("rem=\'C:\\Users\\alice\\miniforge3\'\necho \"${rem#\"C:\\Users\"}\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "\\alice\\miniforge3\n");
}

#[test]
fn issue128_stray_paren_line_does_not_reject_earlier_line() {
    let s = Scratch::new("i128");
    // GNU reports the syntax error but the first line has already run.
    let (out, err, rc) = s.run("echo ')'\necho after\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, ")\nafter\n");
}

#[test]
fn issue129_nested_groups_in_regex_match() {
    let s = Scratch::new("i129");
    let (out, err, rc) = s
        .run("[[ ay =~ ^(a(x|y))$ ]] && echo match-ok\n[[ ab =~ ^(a(b|c))$ ]] && echo match2-ok\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "match-ok\nmatch2-ok\n");
}

#[test]
fn issue142_foreign_syntax_in_untaken_branch_still_parses() {
    let s = Scratch::new("i142");
    let (out, err, rc) = s.run("x=hello\nif false; then echo ${(M)x}; fi\necho parsed-ok\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "parsed-ok\n");
}

#[test]
fn issue143_extglob_pattern_without_extglob_is_parse_error() {
    let s = Scratch::new("i143");
    let (out, err, rc) = s.run(
        "shopt -u extglob\ncase x in @(a|b)) echo in-ext;; *) echo other;; esac\necho rc=$?\n",
    );
    assert_eq!(rc, 2, "stderr: {err}");
    assert_eq!(out, "");
    assert!(
        err.contains("syntax error near unexpected token `('"),
        "got: {err}"
    );
}

#[test]
fn issue244_function_name_with_brace_segment() {
    let s = Scratch::new("i244");
    let (out, err, rc) =
        s.run("layer:{selection}/declare() { echo fn-ok; }\nlayer:{selection}/declare\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "fn-ok\n");
}

#[test]
fn issue248_herestring_inside_function() {
    let s = Scratch::new("i248");
    let (out, err, rc) = s.run("cat <<< in\nf() { cat <<< in; }\nf\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "in\nin\n");
}
