//! rubash#2421 — `case` patterns whose text contains a backslash escape
//! of an expander-active character (`\"`, `\$`, a `\` inside a `${...}`
//! unit, a bracket-member `\"`) never matched: the quote-free pattern
//! branch protected only the `\` with a guard byte and fed the rest to
//! the expander, which re-read the now-bare character as syntax — `\"`
//! opened a quote span that quote removal deleted (the glob after it
//! collapsed to a literal), `\$` started a parameter expansion that ate
//! the `$`, and a `\` inside `${v:-...}` was rewritten before the
//! unit's own grammar could see it.
//!
//! GNU contract: read_token_word consumes `\X` as ONE quoted unit at
//! read time (parse.y:5366-5398 backslash branch sets
//! pass_next_character; parse.y:5358-5362 routes the char to
//! got_escaped_character), the pattern undergoes NO quote removal
//! (execute_cmd.c:3733-3736), and quote_string_for_globbing
//! (pathexp.c:212) re-emits the quoting for strmatch after
//! expand_word_leave_quoted (execute_cmd.c:3712-3725). The escaped
//! character is literal; an unquoted glob after it stays live.
//!
//! All expectations below were measured against GNU bash 5.3.20 (the
//! project oracle) with these exact script files. Tests only pin the
//! behavior; the fix lives in expand_case_pattern
//! (src/executor/compound_exec.rs).

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
            std::env::temp_dir().join(format!("rubash-2421-{tag}-{}-{nanos}", std::process::id()));
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

/// The two filed seeds, verbatim shapes from the issue.
#[test]
fn seed_patterns_with_leading_escaped_quote_match() {
    assert_script(
        "seeds",
        r#"x='"git", "delta"]'
case $x in
\"*) echo "seed1: MATCH" ;;
*) echo "seed1: NOMATCH" ;;
esac
y='"*"'
case $y in
\"*\") echo "seed2: MATCH" ;;
*) echo "seed2: NOMATCH" ;;
esac
"#,
        "seed1: MATCH\nseed2: MATCH\n",
    );
}

/// The escaped-quote family: the quote is literal data wherever the
/// escape appears — pattern start, middle, end, alone — and the glob
/// after it stays live. Negative controls pin that the quote is a real
/// literal (not dropped): `\"a` must not match `a`, `a\"b` must not
/// match `ab`, `\"*` must not match `*`.
#[test]
fn escaped_quote_family_matches_gnu() {
    assert_script(
        "escquote-family",
        r#"case '"' in
\") echo "quoteonly: MATCH" ;;
*) echo "quoteonly: NOMATCH" ;;
esac
case 'a"bc' in
a\"*) echo "mid: MATCH" ;;
*) echo "mid: NOMATCH" ;;
esac
case 'a"b' in
a\"b) echo "midlit: MATCH" ;;
*) echo "midlit: NOMATCH" ;;
esac
case 'ab' in
a\"b) echo "midlit-neg: MATCH" ;;
*) echo "midlit-neg: no" ;;
esac
case '"a' in
\"a) echo "startlit: MATCH" ;;
*) echo "startlit: NOMATCH" ;;
esac
case 'a' in
\"a) echo "startlit-neg: MATCH" ;;
*) echo "startlit-neg: no" ;;
esac
case '*' in
\"*) echo "staronly-neg: MATCH" ;;
*) echo "staronly-neg: no" ;;
esac
case 'x"y' in
x\"*y) echo "gapy: MATCH" ;;
*) echo "gapy: no" ;;
esac
case 'xy' in
x\"*y) echo "gapy-neg: MATCH" ;;
*) echo "gapy-neg: no" ;;
esac
case '"' in
[\"a]) echo "bracket: MATCH" ;;
*) echo "bracket: no" ;;
esac
case 'b' in
[\"a]) echo "bracket-neg: MATCH" ;;
*) echo "bracket-neg: no" ;;
esac
"#,
        "quoteonly: MATCH\nmid: MATCH\nmidlit: MATCH\nmidlit-neg: no\nstartlit: MATCH\n\
         startlit-neg: no\nstaronly-neg: no\ngapy: MATCH\ngapy-neg: no\nbracket: MATCH\n\
         bracket-neg: no\n",
    );
}

/// The other expander-active escape: `\$` is a literal dollar, at the
/// start or in the middle, and does NOT start an expansion (in a
/// function, `$*` is the function's own argument list — the pre-fix
/// pattern silently became whatever `$*` expanded to).
#[test]
fn escaped_dollar_is_literal_not_expansion() {
    assert_script(
        "escdollar",
        r#"case '$x' in
\$*) echo "start: MATCH" ;;
*) echo "start: NOMATCH" ;;
esac
case 'abc' in
\$*) echo "start-neg: MATCH" ;;
*) echo "start-neg: no" ;;
esac
case '$' in
\$) echo "only: MATCH" ;;
*) echo "only: NOMATCH" ;;
esac
case 'a$zz' in
a\$*) echo "mid: MATCH" ;;
*) echo "mid: NOMATCH" ;;
esac
probe() {
  case $1 in
  \$*) echo "fn: MATCH" ;;
  *) echo "fn: no" ;;
  esac
}
probe 'abc'
probe '$abc'
"#,
        "start: MATCH\nstart-neg: no\nonly: MATCH\nmid: MATCH\nfn: no\nfn: MATCH\n",
    );
}

/// A backslash inside a `${...}` unit belongs to the unit's own
/// grammar (parse_matched_pair): `${v:-\"*}` with v unset must yield
/// the pattern `"*` (literal quote + live glob), and an expansion
/// adjacent to an escape pair keeps both halves (`$w\"*`).
#[test]
fn escapes_inside_and_beside_expansion_units() {
    assert_script(
        "units",
        r#"unset v
case '"q' in
${v:-\"*}) echo "unit: MATCH" ;;
*) echo "unit: no" ;;
esac
case 'q' in
${v:-\"*}) echo "unit-neg: MATCH" ;;
*) echo "unit-neg: no" ;;
esac
w=hi
case 'hi"z' in
$w\"*) echo "mix: MATCH" ;;
*) echo "mix: no" ;;
esac
case 'hi' in
$w\"*) echo "mix-neg: MATCH" ;;
*) echo "mix-neg: no" ;;
esac
"#,
        "unit: MATCH\nunit-neg: no\nmix: MATCH\nmix-neg: no\n",
    );
}

/// Controls that already agreed with GNU and must stay green: the
/// inert escapes (`\a`, `\\`, `\[`, `\*`, escaped space/backtick), a
/// plain glob, a fully-quoted literal pattern, a pattern taken from a
/// variable, and the quoted-segment branch (patterns containing real
/// quotes, which take the quote-aware path).
#[test]
fn control_shapes_stay_green() {
    assert_script(
        "controls",
        r#"case '\foo' in
\\*) echo "bsstart: MATCH" ;;
*) echo "bsstart: NOMATCH" ;;
esac
case 'abc' in
\a*) echo "esca: MATCH" ;;
*) echo "esca: NOMATCH" ;;
esac
case '[a' in
\[*) echo "escbracket: MATCH" ;;
*) echo "escbracket: NOMATCH" ;;
esac
case 'lit' in
\*) echo "escstar-neg: MATCH" ;;
*) echo "escstar-neg: no" ;;
esac
case '*' in
\*) echo "escstar: MATCH" ;;
*) echo "escstar: NOMATCH" ;;
esac
case '`x' in
\`*) echo "escbacktick: MATCH" ;;
*) echo "escbacktick: NOMATCH" ;;
esac
case "'a" in
\'*) echo "escsquote: MATCH" ;;
*) echo "escsquote: NOMATCH" ;;
esac
case 'a' in
\') echo "escsquote-neg: MATCH" ;;
*) echo "escsquote-neg: no" ;;
esac
case 'a b' in
a\ *) echo "escspace: MATCH" ;;
*) echo "escspace: NOMATCH" ;;
esac
case 'hello' in
h*) echo "glob: MATCH" ;;
*) echo "glob: NOMATCH" ;;
esac
case '"*"' in
"*") echo "quotedlit: MATCH" ;;
*) echo "quotedlit: no" ;;
esac
p='\"*'
case '"git"' in
$p) echo "varpat: MATCH" ;;
*) echo "varpat: NOMATCH" ;;
esac
case 'ab*' in
a"b"\*) echo "qseg: MATCH" ;;
*) echo "qseg: no" ;;
esac
case 'abx' in
a"b"\*) echo "qseg-neg: MATCH" ;;
*) echo "qseg-neg: no" ;;
esac
"#,
        "bsstart: MATCH\nesca: MATCH\nescbracket: MATCH\nescstar-neg: no\nescstar: MATCH\n\
         escbacktick: MATCH\nescsquote: MATCH\nescsquote-neg: no\nescspace: MATCH\nglob: MATCH\n\
         quotedlit: no\nvarpat: MATCH\nqseg: MATCH\nqseg-neg: no\n",
    );
}

/// The `[[ == ]]` face shares the matcher (case_pattern_matches) but
/// prepares its RHS through quote_aware_glob_rhs, which already kept
/// escape pairs intact; pin the family there so the two faces cannot
/// drift apart again.
#[test]
fn conditional_face_agrees() {
    assert_script(
        "cond",
        r#"w='"git", "delta"]'
[[ $w == \"* ]] && echo "cond1: MATCH" || echo "cond1: NOMATCH"
w='"*"'
[[ $w == \"*\" ]] && echo "cond2: MATCH" || echo "cond2: NOMATCH"
w='a"bc'
[[ $w == a\"* ]] && echo "condmid: MATCH" || echo "condmid: NOMATCH"
w='abc'
[[ $w == \$* ]] && echo "conddollar-neg: MATCH" || echo "conddollar-neg: no"
w='$abc'
[[ $w == \$* ]] && echo "conddollar: MATCH" || echo "conddollar: NOMATCH"
"#,
        "cond1: MATCH\ncond2: MATCH\ncondmid: MATCH\nconddollar-neg: no\nconddollar: MATCH\n",
    );
}
