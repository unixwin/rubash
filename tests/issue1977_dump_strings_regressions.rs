//! rubash#1977 — `--dump-strings` / `-D` / `--dump-po-strings` were
//! accepted by the binary's option parser and silently dropped: no
//! dump, and the script operand executed (side effects included).
//! GNU bash wires the flag as read_but_dont_execute (shell.c:506-510)
//! and prints each locale string from locale_expand (locale.c:550)
//! instead of executing anything.
//!
//! All expectations were measured against GNU bash 5.3.20 (the local
//! oracle build) running the same script files.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
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
        let dir = std::env::temp_dir().join(format!(
            "rubash-issue1977-{tag}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn base_cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_rubash"));
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir);
        c
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run_args(&self, args: &[&str]) -> (String, String, i32) {
        let out = self.base_cmd().args(args).output().unwrap();
        self.finish(out)
    }

    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }
}

const PROBE_SCRIPT: &str = "echo $\"hello world\"\n\
    echo plain\n\
    touch RAN_MARKER\n\
    x=$\"second string\"\n\
    echo $\"esc\\tape \\\"q\\\" end\"\n";

const PROBE_DUMP: &str = "\"hello world\"\n\
    \"second string\"\n\
    \"esc\\tape \\\"q\\\" end\"\n";

#[test]
fn dump_strings_lists_locale_strings_and_never_executes() {
    let s = Scratch::new("plain");
    let script = s.write_script("case.sh", PROBE_SCRIPT);
    let arg = script.to_str().unwrap().to_string();
    let (stdout, _stderr, code) = s.run_args(&["--dump-strings", &arg]);
    assert_eq!(code, 0);
    assert_eq!(stdout, PROBE_DUMP);
    assert!(
        !s.dir.join("RAN_MARKER").exists(),
        "the script must not execute under --dump-strings"
    );
}

#[test]
fn dump_strings_short_and_plus_forms_match_gnu() {
    // GNU shell.c:960-964: case 'D' fires for either sign.
    for flag in ["-D", "+D"] {
        let s = Scratch::new("short");
        let script = s.write_script("case.sh", PROBE_SCRIPT);
        let arg = script.to_str().unwrap().to_string();
        let (stdout, _stderr, code) = s.run_args(&[flag, &arg]);
        assert_eq!(code, 0, "{flag}");
        assert_eq!(stdout, PROBE_DUMP, "{flag}");
        assert!(!s.dir.join("RAN_MARKER").exists(), "{flag}");
    }
}

#[test]
fn dump_po_strings_uses_gnu_msgid_format_with_line_anchors() {
    let s = Scratch::new("po");
    s.write_script("case.sh", PROBE_SCRIPT);
    // The anchor is the script path exactly as given on argv.
    let (stdout, _stderr, code) = s.run_args(&["--dump-po-strings", "case.sh"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "#: case.sh:1\n\
         msgid \"hello world\"\n\
         msgstr \"\"\n\
         #: case.sh:4\n\
         msgid \"second string\"\n\
         msgstr \"\"\n\
         #: case.sh:5\n\
         msgid \"esc\\\\tape \\\\\\\"q\\\\\\\" end\"\n\
         msgstr \"\"\n"
    );
    assert!(!s.dir.join("RAN_MARKER").exists());
}

#[test]
fn dump_po_strings_multiline_body_splits_like_mk_msgstr() {
    let s = Scratch::new("pomulti");
    s.write_script(
        "case2.sh",
        "echo first\necho $\"multi\nline body\"\necho $\"after\"\n",
    );
    let (stdout, _stderr, code) = s.run_args(&["--dump-po-strings", "case2.sh"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "#: case2.sh:2\n\
         msgid \"\"\n\
         \"multi\\n\"\n\
         \"line body\"\n\
         msgstr \"\"\n\
         #: case2.sh:4\n\
         msgid \"after\"\n\
         msgstr \"\"\n"
    );
}

#[test]
fn dump_strings_command_string_form_dumps_and_never_executes() {
    let s = Scratch::new("cform");
    let (stdout, _stderr, code) =
        s.run_args(&["--dump-strings", "-c", "echo $\"from c\"; touch RAN2"]);
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"from c\"\n");
    assert!(!s.dir.join("RAN2").exists());
}

#[test]
fn dump_strings_stdin_form_dumps() {
    let s = Scratch::new("stdin");
    let mut child = s
        .base_cmd()
        .arg("--dump-strings")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo $\"from stdin\"\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let (stdout, _stderr, code) = s.finish(out);
    assert_eq!(code, 0);
    assert_eq!(stdout, "\"from stdin\"\n");
}

#[test]
fn dump_strings_missing_script_is_127() {
    let s = Scratch::new("missing");
    let (_stdout, _stderr, code) = s.run_args(&["--dump-strings", "nosuch.sh"]);
    assert_eq!(code, 127);
}

#[test]
fn dump_strings_recognition_classes_match_gnu() {
    // GNU 5.3.20 probes: comment and here-doc bodies never dump;
    // command substitutions, ${...} bodies (defaults, patterns,
    // subscripts), arithmetic text and function bodies do.
    let s = Scratch::new("classes");
    let script = s.write_script(
        "classes.sh",
        "# comment $\"in comment\"\n\
         echo $(echo $\"in comsub\")\n\
         echo ${x:-$\"in brace default\"}\n\
         echo ${x##$\"in pattern\"}\n\
         echo ${arr[$\"in subscript\"]}\n\
         echo $((1 + $\"in arithmetic\"))\n\
         cat <<HD\n$\"in heredoc\"\nHD\n\
         f() { echo $\"in function\"; }\n\
         echo $\"top\"\n",
    );
    let arg = script.to_str().unwrap().to_string();
    let (stdout, _stderr, code) = s.run_args(&["--dump-strings", &arg]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "\"in comsub\"\n\
         \"in brace default\"\n\
         \"in pattern\"\n\
         \"in subscript\"\n\
         \"in arithmetic\"\n\
         \"in function\"\n\
         \"top\"\n"
    );
}
