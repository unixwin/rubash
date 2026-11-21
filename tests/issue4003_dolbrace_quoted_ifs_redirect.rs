//! rubash#4003 regressions: a quoted NON-BLANK IFS character inside the
//! default/alternate word of a `${parameter<op>word}` expansion keeps its
//! quote protection through field splitting — including when the expansion
//! is a redirect target.
//!
//! GNU owner: subst.c `parameter_brace_expand_rhs` (subst.c:7966) hands
//! the rhs to `expand_string_for_rhs` (subst.c:4462), which runs
//! `expand_word_internal` with `quoted == 0`. That walk's quoted-character
//! and backslash arms (subst.c:11639-11678) emit CTLESC + c for every
//! quoted/escaped character — whitespace AND non-whitespace IFS chars
//! alike — and the deferred `word_list_split` skips CTLESC-protected
//! delimiters. Rubash's rhs walkers only emitted the \x1c IFS_GLUE carrier
//! for whitespace, so `'a:b'`/`"a:b"`/`a\:b` under `IFS=': '` lost the `:`
//! protection and `${W:-'a:b'}` field-split into `a` `b` — an `ambiguous
//! redirect` where GNU bash 5.3.20 creates the file `a:b`.
//!
//! The same missing marks broke argument-position mixed words
//! (`x${W:-'a:b'}` split to `xa` `b`) and escaped non-IFS... non-blank IFS
//! chars (`${W:-a\:b}` split to `a` `b`). A BARE unquoted IFS char still
//! splits — `${W:-a:b}` stays `ambiguous redirect` in both shells.
//!
//! Every expectation below was verified byte-for-byte against WSL GNU
//! Bash 5.3.x (/usr/local/bin/bash, script-file batteries
//! target/probe4003*.sh, 2026-11-21).

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-ifsmark4003-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_script(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script);
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// The ticket form: `IFS=': '; W=; : > ${W:-'a:b'}` — the single-quoted
/// `:` is quoted data, so the redirect target is ONE word `a:b` and the
/// redirection succeeds (GNU creates the file `a:b`).
#[test]
fn quoted_nonblank_ifs_default_redirect_single_field() {
    assert_script(
        "IFS=': '\nW=\n: > ${W:-'a:b'}\necho \"rc=$?\"\ntest -f 'a:b' && echo 'file=a:b'\n",
        "rc=0\nfile=a:b\n",
    );
}

/// Same protection through a double-quoted default word.
#[test]
fn dquoted_nonblank_ifs_default_redirect_single_field() {
    assert_script(
        "IFS=': '\nW=\n: > ${W:-\"a:b\"}\necho \"rc=$?\"\ntest -f 'a:b' && echo 'file=a:b'\n",
        "rc=0\nfile=a:b\n",
    );
}

/// Same protection through a backslash-escaped non-blank IFS char.
#[test]
fn escaped_nonblank_ifs_default_redirect_single_field() {
    assert_script(
        "IFS=': '\nW=\n: > ${W:-a\\:b}\necho \"rc=$?\"\ntest -f 'a:b' && echo 'file=a:b'\n",
        "rc=0\nfile=a:b\n",
    );
}

/// Guard against over-protection: a BARE unquoted `:` in the default
/// word is a real IFS delimiter, so the redirect still fails with
/// `ambiguous redirect` exactly like GNU.
#[test]
fn bare_nonblank_ifs_default_redirect_still_ambiguous() {
    let (stdout, stderr, code) = rubash_file("IFS=': '\nW=\n: > ${W:-a:b}\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "stdout mismatch");
    assert!(
        stderr.contains("ambiguous redirect"),
        "stderr missing ambiguous redirect: {stderr}"
    );
    assert_eq!(code, Some(0), "exit code mismatch");
}

/// Argument position, mixed words: the quoted/escaped `:` keeps its
/// protection while affixes glue onto the same field — the mut-walker
/// half of the same rhs-walk invariant.
#[test]
fn quoted_nonblank_ifs_default_mixed_word_single_field() {
    assert_script(
        "IFS=': '\nW=\nprintf '<%s>\\n' x${W:-'a:b'}\nprintf '<%s>\\n' ${W:-'a:b'}x\nprintf '<%s>\\n' ${W:-a\\:b}\nprintf '<%s>\\n' ${W:-a\"b:c\"d}\n",
        "<xa:b>\n<a:bx>\n<a:b>\n<ab:cd>\n",
    );
}

/// Whole-word argument position (already correct via the alternate
/// re-parse path) stays green alongside the redirect fix.
#[test]
fn quoted_nonblank_ifs_default_whole_word_argument() {
    assert_script(
        "IFS=': '\nW=\nprintf '<%s>\\n' ${W:-'a:b'}\nprintf '<%s>\\n' ${W:-\"a:b\"}\n",
        "<a:b>\n<a:b>\n",
    );
}

/// Assignment context stores the protected value verbatim.
#[test]
fn quoted_nonblank_ifs_default_assignment_value() {
    assert_script(
        "IFS=': '\nW=\nx=${W:-'a:b'}\nprintf '<%s>\\n' \"$x\"\n",
        "<a:b>\n",
    );
}
