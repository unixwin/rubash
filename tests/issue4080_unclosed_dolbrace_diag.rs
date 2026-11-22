//! rubash#4080 — unclosed `${ ... }` (Bash 5.3 funsub, `parse.y:4472-4490`
//! `parse_comsub`) vs `${x` / `${x:-foo` (`parse_matched_pair`, `parse.y:3877`):
//!
//! When `${<funsub_char>...` is not closed by `}` in `reserved_word_acceptable`
//! position (e.g. `echo ${ case x in }`, `echo ${ in }`, `echo ${ then }`),
//! GNU `parse_comsub` (`parse.y:4519, 4549`) still runs `yyparse()` on the
//! body with `shell_eof_token = '}'`. A token-level syntax error inside the
//! body fails before EOF and `report_syntax_error` (`parse.y:6858-6865`)
//! prints `syntax error near unexpected token '<tok>' while looking for
//! matching '}'` + offending source line at the token's line, whereas
//! non-funsub `${x` / `${x:-foo` (`parse_matched_pair`, `parse.y:3908-3915`)
//! and funsub bodies that reach actual EOF (`echo ${ echo x }`) print
//! `unexpected EOF while looking for matching '}'`.
//!
//! Also pins `case x in ; esac` / `echo ${ case x in ; }` (`parse.y:1225-1236`
//! `case_command` only allows `newline_list`, not `;`).

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
            std::env::temp_dir().join(format!("rubash-i4080-{tag}-{}-{nanos}", std::process::id()));
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

#[test]
fn issue4080_funsub_token_errors_name_token_and_echo_line() {
    let scratch = Scratch::new("tok");

    let cases = [
        (
            "echo ${ case x in }\n",
            "line 1: syntax error near unexpected token `newline' while looking for matching `}'",
            "line 1: `echo ${ case x in }'",
        ),
        (
            "echo ${ in }\n",
            "line 1: syntax error near unexpected token `in' while looking for matching `}'",
            "line 1: `echo ${ in }'",
        ),
        (
            "echo ${ then }\n",
            "line 1: syntax error near unexpected token `then' while looking for matching `}'",
            "line 1: `echo ${ then }'",
        ),
        (
            "echo ${ fi }\n",
            "line 1: syntax error near unexpected token `fi' while looking for matching `}'",
            "line 1: `echo ${ fi }'",
        ),
        (
            "echo ${ case x in ; }\n",
            "line 1: syntax error near unexpected token `;' while looking for matching `}'",
            "line 1: `echo ${ case x in ; }'",
        ),
        (
            "echo ${| in }\n",
            "line 1: syntax error near unexpected token `in' while looking for matching `}'",
            "line 1: `echo ${| in }'",
        ),
        (
            "echo ${\nin }\n",
            "line 2: syntax error near unexpected token `in' while looking for matching `}'",
            "line 2: `in }'",
        ),
    ];

    for (script, want_diag, want_echo) in cases {
        let (_out, err, rc) = scratch.run(script);
        assert_eq!(rc, 2, "script {script:?} rc; stderr:\n{err}");
        assert!(
            err.contains(want_diag),
            "script {script:?} missing {want_diag:?} in stderr:\n{err}"
        );
        assert!(
            err.contains(want_echo),
            "script {script:?} missing {want_echo:?} in stderr:\n{err}"
        );
    }
}

#[test]
fn issue4080_unclosed_param_and_eof_funsub_keep_eof_diag() {
    let scratch = Scratch::new("eof");

    let cases = [
        (
            "echo ${x\n",
            "line 1: unexpected EOF while looking for matching `}'",
        ),
        (
            "echo ${x:-foo\n",
            "line 1: unexpected EOF while looking for matching `}'",
        ),
        (
            "echo ${ echo x }\n",
            "line 2: unexpected EOF while looking for matching `}'",
        ),
        (
            "echo ${ case x in\n",
            "line 2: unexpected EOF while looking for matching `}'",
        ),
        (
            "echo ${ if true; then\n",
            "line 2: unexpected EOF while looking for matching `}'",
        ),
    ];

    for (script, want_diag) in cases {
        let (_out, err, rc) = scratch.run(script);
        assert_eq!(rc, 2, "script {script:?} rc; stderr:\n{err}");
        assert!(
            err.contains(want_diag),
            "script {script:?} missing {want_diag:?} in stderr:\n{err}"
        );
    }
}

#[test]
fn issue4080_comsub_and_case_semicolon_errors() {
    let scratch = Scratch::new("comsub-case");

    let (_out, err, rc) = scratch.run("echo $( case x in )\n");
    assert_eq!(rc, 2, "stderr:\n{err}");
    assert!(
        err.contains("line 1: syntax error near unexpected token `)'"),
        "stderr:\n{err}"
    );
    assert!(
        err.contains("line 1: `echo $( case x in )'"),
        "stderr:\n{err}"
    );

    for script in ["case x in ; esac\n", "case x in a) :;; ; esac\n"] {
        let (_out, err, rc) = scratch.run(script);
        assert_eq!(rc, 2, "script {script:?} rc; stderr:\n{err}");
        assert!(
            err.contains("line 1: syntax error near unexpected token `;'"),
            "script {script:?} stderr:\n{err}"
        );
    }
}
