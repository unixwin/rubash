//! rubash#4126 — a pattern character that arrives quoted or escaped is
//! literal data, never pattern syntax (GNU pathexp.c quote_string marks
//! EVERY character of a quoted span; lib/glob/sm_loop.c then treats the
//! marked character as plain data). Four independent sites dropped the
//! literal marking for part of the character set:
//!
//! * parameter-expansion patterns (`push_quoted_pattern_char` marked
//!   only `* ? [ \`): a quoted `]` closed a bracket expression early, a
//!   quoted `-` became a range, quoted `!`/`^` became bracket negation,
//!   quoted `#`/`%` became patsub anchors, a quoted `+` opened an
//!   extglob group;
//! * `[[ == ]]` operands (`append_literal_glob_text` escaped only
//!   `* ? [ \ ( ) |`): the same `]` / `-` losses inside brackets;
//! * `${var^^pat}` / `${var,,pat}` (the casemod pattern bypassed
//!   `expand_parameter_pattern_word` through the generic walker, which
//!   strips quotes without leaving the literal marks);
//! * pathname expansion (lexer double-quote removal CTLESC-marked only
//!   `* ? [ @ + !`): a quoted `]` or `-` reached the glob matcher bare.
//!
//! Expectations verified against GNU Bash 5.3.20 (cloud oracle,
//! ~/oracle/bash-5.3.20) with script-file probes under LC_ALL=C; the
//! probe matrix is byte-identical between the oracle and the fixed
//! build (66/66 lines).

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
            std::env::temp_dir().join(format!("rubash-i4126-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    /// Run `script` as a script FILE under LC_ALL=C with the scratch
    /// dir as cwd/HOME; returns (stdout, stderr, rc).
    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env("HOME", &self.dir)
            .env("LC_ALL", "C")
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

    fn assert_script(&self, script: &str, want_stdout: &str) {
        let (out, err, rc) = self.run(script);
        assert_eq!(rc, 0, "rc mismatch; stderr:\n{err}\nscript:\n{script}");
        assert_eq!(
            out, want_stdout,
            "stdout mismatch; stderr:\n{err}\nscript:\n{script}"
        );
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Main face: ${var/pat/repl}, ${var#pat}, ${var%pat} and case patterns
/// fed from a variable.
#[test]
fn issue4126_parameter_patterns_treat_quoted_chars_as_literal() {
    let s = Scratch::new("main");
    s.assert_script(
        r##"
x='a[b]c'; echo "M1:[${x//[\[\]]/@}]"
x='a[b]c'; echo "M2:[${x//["[]"]/@}]"
s=b; echo "M3:[${s//[a\-z]/@}]"
s=-; echo "M3b:[${s//[a\-z]/@}]"
s=a; echo "M4:[${s//[\!a]/@}]"
x='a#b'; r=${x/"#"/X}; echo "M5:[$r]"
s=a; echo "M6a:[${s//[\^a]/@}]"
s=b; echo "M6b:[${s//[\^a]/@}]"
x='a%b'; r=${x/"%"/X}; echo "M7:[$r]"
shopt -s extglob; s=aa; echo "M8:[${s//"+"(a)/Q}]"
x='[b]c'; echo "M9:[${x#[\[\]]}]"
x='a[b]'; echo "M10:[${x%[\[\]]}]"
q='[a\]b]'
for c in a b ']' x; do case $c in $q) echo "M13-$c:match";; *) echo "M13-$c:no";; esac; done
"##,
        "M1:[a@b@c]\n\
         M2:[a@b@c]\n\
         M3:[b]\n\
         M3b:[@]\n\
         M4:[@]\n\
         M5:[aXb]\n\
         M6a:[@]\n\
         M6b:[b]\n\
         M7:[aXb]\n\
         M8:[aa]\n\
         M9:[b]c]\n\
         M10:[a[b]\n\
         M13-a:match\n\
         M13-b:match\n\
         M13-]:match\n\
         M13-x:no\n",
    );
}

/// Main face guards: unquoted pattern machinery must not move.
#[test]
fn issue4126_unquoted_patterns_unchanged() {
    let s = Scratch::new("guards");
    s.assert_script(
        r#"
x='abc'; echo "G1:[${x//b/@}]"
s=b; echo "G2:[${s//[a-z]/@}]"
s=b; echo "G3:[${s//[!a]/@}]"
x='a*b'; echo "G4:[${x//"*"/@}]"
s=b; echo "G5:[${s//[[:alpha:]]/@}]"
x='a/b/c'; echo "G6:[${x##*/}]"
"#,
        "G1:[a@c]\nG2:[@]\nG3:[@]\nG4:[a@b]\nG5:[@]\nG6:[c]\n",
    );
}

/// Face A: `[[ == ]]` bracket expressions with quoted members.
#[test]
fn issue4126_conditional_patterns_treat_quoted_chars_as_literal() {
    let s = Scratch::new("cond");
    s.assert_script(
        r#"
for c in '[' ']' a '-' b; do if [[ $c == ["[]"] ]]; then echo "A1-$c:T"; else echo "A1-$c:F"; fi; done
for c in a '-' b z; do if [[ $c == [a"-"z] ]]; then echo "A2-$c:T"; else echo "A2-$c:F"; fi; done
for c in '^' a b; do if [[ $c == ["^"a] ]]; then echo "A3-$c:T"; else echo "A3-$c:F"; fi; done
for c in '!' a b; do if [[ $c == ["!"a] ]]; then echo "A4-$c:T"; else echo "A4-$c:F"; fi; done
if [[ abc == a* ]]; then echo "AG1:T"; else echo "AG1:F"; fi
if [[ b == [a-z] ]]; then echo "AG2:T"; else echo "AG2:F"; fi
"#,
        "A1-[:T\n\
         A1-]:T\n\
         A1-a:F\n\
         A1--:F\n\
         A1-b:F\n\
         A2-a:T\n\
         A2--:T\n\
         A2-b:F\n\
         A2-z:T\n\
         A3-^:T\n\
         A3-a:T\n\
         A3-b:F\n\
         A4-!:T\n\
         A4-a:T\n\
         A4-b:F\n\
         AG1:T\n\
         AG2:T\n",
    );
}

/// Face B: casemod patterns keep quoted bracket members literal.
#[test]
fn issue4126_casemod_patterns_treat_quoted_chars_as_literal() {
    let s = Scratch::new("casemod");
    s.assert_script(
        r#"
x=abc; echo "B1:[${x^^["[]"a]}]"
x='a]b[c'; echo "B2:[${x^^["[]"a]}]"
x=xyz; echo "B3:[${x^^["[]"a]}]"
x=abc; echo "BG2:[${x^^[a]}]"
x=ABC; echo "BG4:[${x,,[a]}]"
"#,
        "B1:[Abc]\nB2:[A]b[c]\nB3:[xyz]\nBG2:[Abc]\nBG4:[ABC]\n",
    );
}

/// Face C: pathname expansion with quoted `]` / `-` inside brackets.
#[test]
fn issue4126_pathname_glob_treats_quoted_chars_as_literal() {
    let s = Scratch::new("glob");
    for name in ["a", "b", "x", "]", "-", "^", "!"] {
        fs::write(s.dir.join(name), "").unwrap();
    }
    s.assert_script(
        r#"
echo "C1:" [a"]"b]
echo "C2:" [a"-"z]
echo "C3:" [a']'b]
echo "C4:" [a'-'z]
echo "C5:" ["^"az]
echo "C6:" ['^'az]
echo "C7:" [[":"alpha:]]
echo "CG1:" [ab]
echo "CG2:" "[a]"
echo "CG3:" [a-z]
"#,
        "C1: ] a b\nC2: - a\nC3: ] a b\nC4: - a\nC5: ^ a\nC6: ^ a\nC7: [[:alpha:]]\nCG1: a b\nCG2: [a]\nCG3: a b x\n",
    );
}

/// ANSI-C decoded bytes are quoted source data too (GNU parse.y:5560-5575
/// hands them through the same quoted-character machinery). In particular a
/// decoded `]` or `-` inside a bracket expression is a member, not bracket
/// syntax; UTF-8 subject bytes around the ASCII metacharacter must pass
/// through unchanged.
#[test]
fn issue4126_ansi_c_and_utf8_pattern_chars_stay_literal() {
    let s = Scratch::new("ansi-c-utf8");
    for name in ["a", "b", "z", "]", "-"] {
        fs::write(s.dir.join(name), "").unwrap();
    }
    s.assert_script(
        r#"
x='é]é'; printf 'U01:[%s]\n' "${x//"]"/X}"
x='a\b'; printf 'U02:[%s]\n' "${x//["\\"]/X}"
printf 'U04:'; printf '<%s>' [a$'\x5d'b]; printf '\n'
printf 'U05:'; printf '<%s>' [a$'\x2d'z]; printf '\n'
"#,
        "U01:[éXé]\nU02:[aXb]\nU04:<]><a><b>\nU05:<-><a><z>\n",
    );
}

/// Carrier boundary guard: heredoc and here-string byte streams are data,
/// not pattern text. A `-` or `]` that passed through the double-quoted
/// expansion machinery must lose its CTLESC protection at the stdin-body
/// boundary (GNU subst.c:4807 dequote_string before redir.c writes the
/// here-document), never as a raw 0x11 byte. Verified against WSL GNU Bash
/// 5.3.20 with the same LF script file.
#[test]
fn issue4126_heredoc_and_here_string_bodies_dequote_pattern_chars() {
    let s = Scratch::new("body-boundary");
    s.assert_script(
        "cat <<EOF\nhd-a]b\nEOF\ncat <<<\"hs-a]b\"\nexec 8<<<\"stored-a]b\"\ncat <&8\nexec 8<&-\n",
        "hd-a]b\nhs-a]b\nstored-a]b\n",
    );
}
