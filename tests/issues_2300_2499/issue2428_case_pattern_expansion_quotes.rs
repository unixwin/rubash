//! rubash#2428 — a `case` pattern that arrives VIA EXPANSION whose
//! result contains quote characters: rubash decoded the expanded text
//! with source-level quote semantics (decode_parameter_pattern_quotes),
//! so a `*` between quote characters in the value was literal-marked
//! and stopped being a glob. GNU: quoting is a read-time property of
//! the pattern word (parse.y read_token_word / CTLESC marking);
//! expansion results carry no quoting, patterns undergo no quote
//! removal (execute_cmd.c:3733-3736), and glob characters in a value
//! stay live while its quote characters are ordinary data.
//!
//! All expectations below were measured against GNU bash 5.3.20 (the
//! project oracle) with these exact script files. Tests only pin the
//! behavior; the fix lives in decode_case_pattern_expansion
//! (src/executor/parameter_decode.rs) and its two case call sites in
//! src/executor/compound_exec.rs.

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
            std::env::temp_dir().join(format!("rubash-2428-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out: Output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn assert_script(tag: &str, script: &str, expected_stdout: &str) {
    let scratch = Scratch::new(tag);
    let (stdout, stderr, code) = scratch.run(script);
    assert_eq!(code, 0, "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout (stderr: {stderr})");
}

/// The filed repro, verbatim.
#[test]
fn seed_expansion_result_quotes_are_data() {
    assert_script(
        "seed",
        r#"p='"*"'
case '"xb"' in $p) echo "var-pattern: MATCH";; *) echo "var-pattern: nomatch";; esac
case '"*"'  in $p) echo "var-pattern-star: MATCH";; *) echo "var-pattern-star: nomatch";; esac
q='a"b'
case 'a"b' in $q) echo "var-quote-mid: MATCH";; *) echo "var-quote-mid: nomatch";; esac
"#,
        "var-pattern: MATCH\nvar-pattern-star: MATCH\nvar-quote-mid: MATCH\n",
    );
}

/// Other glob operators arriving from a value stay live too, and a
/// single quote in a value is an ordinary character.
#[test]
fn value_globs_and_squotes_stay_live() {
    assert_script(
        "globs",
        r#"r='a?c'
case 'abc' in $r) echo "qmark: MATCH";; *) echo "qmark: nomatch";; esac
s='[a]b'
case 'ab' in $s) echo "bracket: MATCH";; *) echo "bracket: nomatch";; esac
t="x'y"
case "x'y" in $t) echo "squote: MATCH";; *) echo "squote: nomatch";; esac
"#,
        "qmark: MATCH\nbracket: MATCH\nsquote: MATCH\n",
    );
}

/// A backslash in a value still quotes the next character for the
/// matcher (that part is data semantics, not source quoting).
#[test]
fn value_backslash_still_quotes_glob() {
    assert_script(
        "backslash",
        r#"u='a\*b'
case 'a*b' in $u) echo "bstar: MATCH";; *) echo "bstar: nomatch";; esac
case 'axb' in $u) echo "bstar-x: MATCH";; *) echo "bstar-x: nomatch";; esac
"#,
        "bstar: MATCH\nbstar-x: nomatch\n",
    );
}

/// Source-level quoting in the pattern itself is untouched: a quoted
/// `$p` keeps the value's glob literal (the pattern word's own quotes
/// are read-time quoting), and a literal source pattern still works.
#[test]
fn source_quoting_still_applies() {
    assert_script(
        "sourceq",
        r#"p='*'
case 'abc' in "$p") echo "quoted-var: MATCH";; *) echo "quoted-var: nomatch";; esac
case '*' in "$p") echo "quoted-var-lit: MATCH";; *) echo "quoted-var-lit: nomatch";; esac
case 'abc' in a*) echo "literal: MATCH";; *) echo "literal: nomatch";; esac
"#,
        "quoted-var: nomatch\nquoted-var-lit: MATCH\nliteral: MATCH\n",
    );
}
