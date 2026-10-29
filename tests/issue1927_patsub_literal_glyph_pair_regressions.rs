//! rubash#1927 — a registry-zone glyph typed literally into a patsub
//! replacement body leaked a U+E400 carrier in front of the glyph:
//! `${v/b/<U+E312>x}` printed `61 ee 90 80 ee 8c 92 78 63` instead of
//! GNU's `61 ee 8c 92 78 63`.
//!
//! Root cause: script/`-c` entry escaping
//! (substitution_metadata::escape_marker_zone_chars) encodes every
//! typed zone glyph as an E400 + glyph pair before the parser runs.
//! The replacement scan pass in expand_braced_replacement.rs copied
//! literal chars one at a time through push_replacement_literal, which
//! E400-escapes the pair's lead a second time; the output boundary
//! decodes each pair exactly once, so the double encoding surfaced as
//! the leaked U+E400. (The `$'\uE312'` spelling was already clean: the
//! ANSI-C decoder escapes `\u` codepoints itself and the decoded chars
//! ride verbatim.) GNU has no second encoding — subst.c:3950
//! expand_string_for_patsub treats replacement characters as plain
//! data and subst.c:3891 quote_string_for_repl applies its CTLESC
//! protection once.
//!
//! Fix: the scan pass copies an entry-escaped pair verbatim as a unit
//! (push_replacement_escape_pair) at every literal-copy site — the
//! unquoted/compat42 scans, the single-quoted span, the double-quoted
//! span and its backslash arm, the unquoted backslash arms, the
//! unexpandable-`$` arm, and ANSI-C body collection.
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (LC_ALL=C.UTF-8), 2026-10-29.

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
            std::env::temp_dir().join(format!("rubash-i1927-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (Vec<u8>, String, i32) {
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
        let mut out = Vec::new();
        fs::File::open(&out_path)
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        let mut err = String::new();
        fs::File::open(&err_path)
            .unwrap()
            .read_to_string(&mut err)
            .unwrap();
        (out, err, status.code().unwrap_or(-1))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1927_literal_glyph_all_replacement_forms() {
    let s = Scratch::new("forms");
    // The issue's form matrix: the typed glyph rides unquoted,
    // single-quoted, double-quoted, and through an assignment — GNU
    // prints a + U+E312 + x + c for every one, with no U+E400 in front.
    let g = '\u{E312}';
    let script = format!(
        "v=abc\n\
         printf '%s' \"${{v/b/{g}x}}\" | od -An -tx1\n\
         printf '%s' \"${{v/b/'{g}x'}}\" | od -An -tx1\n\
         printf '%s' \"${{v/b/\"{g}x\"}}\" | od -An -tx1\n\
         r=\"${{v/b/{g}x}}\"; printf '%s' \"$r\" | od -An -tx1\n\
         printf '%s' \"${{v/b/$'{g}x'}}\" | od -An -tx1\n"
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        b" 61 ee 8c 92 78 63\n 61 ee 8c 92 78 63\n 61 ee 8c 92 78 63\n \
           61 ee 8c 92 78 63\n 61 ee 8c 92 78 63\n"
            .to_vec(),
        "every literal-typed form must match GNU byte-for-byte"
    );
    assert_eq!(err, "");
}

#[test]
fn issue1927_typed_zone_edge_glyphs_are_data() {
    let s = Scratch::new("edges");
    // The E400 introducer itself and the U+E401 guard codepoint, typed
    // literally, are user data (GNU prints each as the bare glyph).
    let script = format!(
        "v=abc\n\
         printf '%s' \"${{v/b/\u{E400}x}}\" | od -An -tx1\n\
         printf '%s' \"${{v/b/\u{E401}x}}\" | od -An -tx1\n"
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, b" 61 ee 90 80 78 63\n 61 ee 90 81 78 63\n".to_vec());
    assert_eq!(err, "");
}

#[test]
fn issue1927_glyph_pair_next_to_backslash_and_dollar() {
    let s = Scratch::new("adjacent");
    // The pair is one user char wherever it lands: an unquoted
    // backslash quotes it (and is consumed), a double-quoted backslash
    // before it stays literal, and an unexpandable `$` stays a literal
    // dollar in front of it.
    let g = '\u{E312}';
    let script = format!(
        "v=abc\n\
         printf '%s' \"${{v/b/\\{g}x}}\" | od -An -tx1\n\
         printf '%s' \"${{v/b/\"\\{g}x\"}}\" | od -An -tx1\n\
         printf '%s' \"${{v/b/${g}x}}\" | od -An -tx1\n"
    );
    let (out, err, rc) = s.run(&script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        b" 61 ee 8c 92 78 63\n 61 5c ee 8c 92 78 63\n 61 24 ee 8c 92 78 63\n".to_vec()
    );
    assert_eq!(err, "");
}
