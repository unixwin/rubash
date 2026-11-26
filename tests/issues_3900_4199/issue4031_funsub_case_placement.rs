//! rubash#4031 — `case ... esac` inside a `${ ...; }` funsub nested
//! inside `$( ... )` (unquoted, double-quoted, assignment RHS, or nested
//! inside an outer `case`) must not confuse the enclosing `$(` scanner's
//! `case`-pattern tracker.
//!
//! GNU bash 5.3 (`parse.y:5494-5514 read_token_word()` -> `parse.y:4451
//! parse_comsub()`): when `read_token_word` encounters `${` with
//! `FUNSUB_CHAR` (`parser.h:85`: `' '`, `'\t'`, `'\n'`, `'|'`), it
//! delegates to `parse_comsub(cd, '{', '}', ...)` which saves parser state,
//! clears `PST_CASEPAT` (`parse.y:4511`), parses the funsub independently,
//! and restores parser state — so `case`, `in`, `)`, `;;`, and `esac`
//! inside `${ ...; }` never leak into the enclosing `$( ... )` parser.
//!
//! Expectations verified against WSL GNU Bash 5.3 (`/usr/local/bin/bash`)
//! via script-file probes.

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
            std::env::temp_dir().join(format!("rubash-i4031-{tag}-{}-{nanos}", std::process::id()));
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
            .stdin(Stdio::null())
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

fn assert_script_stdout(scratch: &Scratch, script: &str, want: &str) {
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
    assert_eq!(
        out, want,
        "stdout mismatch; stderr:\n{err}\nscript:\n{script}"
    );
}

#[test]
fn issue4031_funsub_case_in_comsub_unquoted_and_quoted() {
    let scratch = Scratch::new("basic");
    assert_script_stdout(
        &scratch,
        "echo out=$(echo ${ case a in a) echo ok;; esac; })\n",
        "out=ok\n",
    );
    assert_script_stdout(
        &scratch,
        "echo \"out=[$(echo ${ case a in a) echo ok;; esac; })]\"\n",
        "out=[ok]\n",
    );
    assert_script_stdout(
        &scratch,
        "echo \"out=[$(echo ${ case a in a) echo ok; esac; })]\"\n",
        "out=[ok]\n",
    );
}

#[test]
fn issue4031_funsub_case_nested_inside_outer_case_in_comsub() {
    let scratch = Scratch::new("nested");
    assert_script_stdout(
        &scratch,
        "echo out=$(case b in b) echo ${ case a in a) echo nested;; esac; };; esac)\n",
        "out=nested\n",
    );
    assert_script_stdout(
        &scratch,
        "echo \"out=[$(case b in b) echo ${ case a in a) echo nested;; esac; };; esac)]\"\n",
        "out=[nested]\n",
    );
    assert_script_stdout(
        &scratch,
        "echo \"out=[$(case ${ case a in a) echo x;; esac; } in x) echo matched;; esac)]\"\n",
        "out=[matched]\n",
    );
}

#[test]
fn issue4031_funsub_case_variants_and_assignment_rhs() {
    let scratch = Scratch::new("variants");
    let script = concat!(
        "echo \"m05=[$(echo ${ case a in (a) echo ok;; esac; })]\"\n",
        "echo \"m06=[$(echo ${ case a in a|b) echo or;; esac; })]\"\n",
        "echo \"m07=[$(echo ${ case a in a) echo 'b)';; esac; })]\"\n",
        "echo \"m08=[$(echo ${ case a in a) (echo sub);; esac; })]\"\n",
        "echo \"m10=[$(echo ${| case a in a) REPLY=valsub;; esac; })]\"\n",
        "echo m11=$(echo ${| case a in a) REPLY=valsub;; esac; })\n",
        "echo \"m12=[$(echo ${\ncase a in a) echo nl;; esac; })]\"\n",
        "echo \"m13=[$(echo ${\tcase a in a) echo tab;; esac; })]\"\n",
        "v1=$(echo ${ case a in a) echo ok;; esac; })\n",
        "v2=\"$(echo ${ case a in a) echo ok;; esac; })\"\n",
        "echo \"m15=[$v1:$v2]\"\n",
    );
    let want = concat!(
        "m05=[ok]\n",
        "m06=[or]\n",
        "m07=[b)]\n",
        "m08=[sub]\n",
        "m10=[valsub]\n",
        "m11=valsub\n",
        "m12=[nl]\n",
        "m13=[tab]\n",
        "m15=[ok:ok]\n",
    );
    assert_script_stdout(&scratch, script, want);
}

/// Graft addition (captain lane): compound tails with NO trailing
/// separator before the funsub's closing `}` — `${ case ... esac }`,
/// `${ if ...; fi }`. The opaque skip is only as correct as
/// `skip_funsub_body`'s extent on these tails (the face rubash#4026's
/// FunsubWordTracker fixed); the original 15 shapes all use `esac; }`.
/// These also assert stderr is empty, not just stdout + rc.
#[test]
fn issue4031_funsub_compound_tail_no_separator() {
    let scratch = Scratch::new("tail");
    let shapes: [(&str, &str); 6] = [
        (
            "echo out=$(echo ${ case a in a) echo ok;; esac })\n",
            "out=ok\n",
        ),
        (
            "echo \"out=[$(echo ${ case a in a) echo ok;; esac })]\"\n",
            "out=[ok]\n",
        ),
        (
            "echo out=$(case b in b) echo ${ case a in a) echo nested;; esac };; esac)\n",
            "out=nested\n",
        ),
        (
            "echo \"out=[$(case ${ case a in a) echo x;; esac } in x) echo matched;; esac)]\"\n",
            "out=[matched]\n",
        ),
        (
            "echo out=$(echo ${ if true; then case a in a) echo deep;; esac; fi })\n",
            "out=deep\n",
        ),
        (
            "v=$(echo ${ case a in a) echo ok;; esac })\necho \"rhs=[$v]\"\n",
            "rhs=[ok]\n",
        ),
    ];
    for (script, want) in shapes {
        let (out, err, rc) = scratch.run(script);
        assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
        assert_eq!(out, want, "stdout mismatch; script:\n{script}");
        assert_eq!(err, "", "stderr not empty; script:\n{script}");
    }
}
