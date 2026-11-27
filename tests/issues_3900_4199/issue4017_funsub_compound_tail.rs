//! rubash#4017 — a funsub body (`${ cmds; }` / `${| cmds; }`) ending with
//! a compound command and no trailing `;` or newline must be accepted:
//! GNU bash 5.3's `funsub: DOLBRACE compound_list '}'` (parse.y:1115)
//! parses the body with the real parser, and read_token_word's
//! PST_FUNSUBST `}` special case (parse.y:5407-5416) fires whenever
//! reserved_word_acceptable(last_read_token) (parse.y:5899-5947) —
//! compound-command keywords/closers (`fi`, `done`, `esac`, `]]`, `)`),
//! command terminators, and the FUNCTION/COPROC-name fallback
//! (parse.y:5940-5943) all qualify. A body ending in plain word text
//! (`${ echo x }`) stays unterminated (GNU: "unexpected EOF").
//!
//! Expectations verified against WSL GNU Bash 5.3.20 (/usr/local/bin/bash)
//! script-file probes under target/funsub-4017-cases/.

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
            std::env::temp_dir().join(format!("rubash-i4017-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// Run `script` as a script FILE (read-time EOF diagnostics differ
    /// from `-c`, matching how the GNU probes were taken).
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

/// `body` is the funsub body text; the script is
/// `echo "out=[$(echo ${ BODY })]"` so the funsub is nested inside a
/// `$()` inside double quotes — the hardest extent-scanning placement.
/// GNU prints `x` (or the body's own output) with rc=0.
fn assert_funsub(scratch: &Scratch, body: &str, want: &str) {
    let script = format!("echo \"out=[$(echo ${{ {body} }})]\"\n");
    let (out, err, rc) = scratch.run(&script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
    assert!(
        out.contains(&format!("out=[{want}]")),
        "stdout mismatch:\n{out}\nscript:\n{script}"
    );
}

/// Same as assert_funsub but with the funsub directly inside double
/// quotes (`"${ BODY }"`) — used for `case` bodies, whose `)`/`;;`/`esac`
/// tokens still confuse the `$(`-nested scanners' case-pattern tracker
/// (skip_cmd_subst's update_command_substitution_case_depth sees inside
/// the nested funsub; a known residual, see issue report).
fn assert_funsub_quoted(scratch: &Scratch, body: &str, want: &str) {
    let script = format!("echo \"out=[${{ {body} }}]\"\n");
    let (out, err, rc) = scratch.run(&script);
    assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
    assert!(
        out.contains(&format!("out=[{want}]")),
        "stdout mismatch:\n{out}\nscript:\n{script}"
    );
}

/// The issue's headline shapes: subshell, brace group and `if` tails.
#[test]
fn issue4017_subshell_brace_if_tails() {
    let scratch = Scratch::new("tails");
    assert_funsub(&scratch, "(echo x)", "x");
    assert_funsub(&scratch, "{ echo x; }", "x");
    assert_funsub(&scratch, "if true; then echo x; fi", "x");
    assert_funsub(&scratch, "if false; then echo y; else echo x; fi", "x");
}

/// Every compound-command keyword tail GNU's reserved_word_acceptable
/// accepts (parse.y:5903-5936): the body ends ON the compound command —
/// no trailing `;` or newline.
#[test]
fn issue4017_compound_keyword_tails() {
    let scratch = Scratch::new("kw");
    assert_funsub(&scratch, "for i in 1; do echo x; done", "x");
    assert_funsub(
        &scratch,
        "i=0; while [ $i -lt 1 ]; do echo x; i=1; done",
        "x",
    );
    assert_funsub(
        &scratch,
        "i=0; until [ $i -ge 1 ]; do echo x; i=1; done",
        "x",
    );
    assert_funsub_quoted(&scratch, "case a in a) echo x;; esac", "x");
    assert_funsub_quoted(&scratch, "case x in x) ;; esac", "");
    assert_funsub(&scratch, "echo x && [[ a == a ]]", "x");
    assert_funsub(&scratch, "for ((i=0;i<1;i++)); do echo x; done", "x");
    // GNU select on EOF stdin prints its menu to stderr and runs no
    // body iteration, so the funsub captures nothing.
    assert_funsub(&scratch, "select s in a; do echo $s; break; done", "");
}

/// `${| cmds; }` valsub form, glued mid-word placement, and nested
/// funsubs.
#[test]
fn issue4017_valsub_and_placement() {
    let scratch = Scratch::new("val");
    let (out, err, rc) = scratch.run("echo \"a=[$(echo ${| echo x; })]\"\n");
    assert_eq!(rc, 0, "stderr:\n{err}");
    assert!(out.contains("a=[x]"), "stdout:\n{out}");

    let (out, err, rc) = scratch.run(
        "echo \"b=[$(echo a${ if true; then echo x; fi }b)]\"\n\
         echo \"c=[$(echo ${ (echo ${ (echo y) }) })]\"\n",
    );
    assert_eq!(rc, 0, "stderr:\n{err}");
    assert!(out.contains("b=[axb]"), "stdout:\n{out}");
    assert!(out.contains("c=[y]"), "stdout:\n{out}");
}

/// `function name { ... }` / `name() { ... }` / `coproc name { ... }`:
/// the `{` opens via PST_ALLOWOPNBRC (parse.y:3444-3451, set at
/// 5853-5858) and the FUNCTION/COPROC-name fallback
/// (parse.y:5940-5943).
#[test]
fn issue4017_function_coproc_brace_bodies() {
    let scratch = Scratch::new("fn");
    // The funsub body IS the function definition; the definition's `}`
    // is itself a reserved-word-acceptable token, so the outer `}`
    // closes the funsub. (GNU runs the funsub in a subshell, so `f`
    // is not defined afterwards — the body just has to parse+run.)
    let script = "echo \"d=[$(echo ${ function f { echo x; } })]\"\n";
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "stderr:\n{err}\nscript:\n{script}");
    assert!(out.contains("d=[]"), "stdout:\n{out}");

    let script = "echo \"e=[$(echo ${ f2() { echo x; }; f2; })]\"\n";
    let (out, err, rc) = scratch.run(script);
    assert_eq!(rc, 0, "stderr:\n{err}\nscript:\n{script}");
    assert!(out.contains("e=[x]"), "stdout:\n{out}");

    // `coproc` runs asynchronously: GNU captures nothing, rc=0.
    let (out, err, rc) = scratch.run("echo \"g=[$(echo ${ coproc c { echo x; }; })]\"\n");
    assert_eq!(rc, 0, "stderr:\n{err}");
    assert!(out.contains("g=[]"), "stdout:\n{out}");
}

/// Unterminated simple-command bodies stay rejected exactly like GNU:
/// `${ echo x }` reads to EOF ("unexpected EOF while looking for
/// matching `}'", rc=2) — the `}` is word text after a WORD token.
#[test]
fn issue4017_unterminated_simple_body_still_rejected() {
    let scratch = Scratch::new("unterm");
    for body in ["echo x", "then", "case x in", "in", "fi"] {
        let script = format!("echo ${{ {body} }}\n");
        let (_out, err, rc) = scratch.run(&script);
        assert_eq!(rc, 2, "body {body:?} rc; stderr:\n{err}");
        assert!(
            err.contains("unexpected EOF") || err.contains("syntax error"),
            "body {body:?} stderr:\n{err}"
        );
    }
}

/// A funsub body that closes the extent but fails to parse reports the
/// funsub's own `}' closer — GNU's parse_comsub sets
/// `shell_eof_token = funsub ? '}' : ')'` (parse.y:4727) — never `)'.
#[test]
fn issue4017_body_error_names_brace_closer() {
    let scratch = Scratch::new("closer");
    for tail in ["echo x; fi", "then", "echo x;;"] {
        let script = format!("echo ${{ {tail} }}\n");
        let (_out, err, rc) = scratch.run(&script);
        assert_eq!(rc, 2, "tail {tail:?} rc; stderr:\n{err}");
        assert!(
            err.contains("while looking for matching `}'"),
            "tail {tail:?} stderr:\n{err}"
        );
        assert!(
            !err.contains("matching `)'"),
            "tail {tail:?} stderr named `)':\n{err}"
        );
    }
}
