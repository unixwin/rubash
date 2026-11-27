//! rubash#868 regressions: `mark_quoted_ifs_from_raw` is carrier-aware.
//!
//! The simulator re-parses the raw word and only applies per-char IFS marks
//! when its simulated cook equals the actual cooked word; on mismatch it
//! bails and the splitter falls back to the coarse split-everything-or-
//! nothing behavior. Master's simulator treated every markers.rs carrier the
//! cook produces (ANSI-C `$'...'` decode output, `$"..."` bodies, escaped
//! quotes, escaped IFS blanks and pattern chars) as an opaque data byte, so
//! any carrier-laden raw word compared unequal on every char and silently
//! lost the per-char rule — while the coarse fallback's literal marks kept
//! the *non*-whitespace IFS faces looking correct by accident.
//!
//! Second face in the same function (exposed by the #868 battery): the
//! apply loop added a second \x1c glue in front of a char that
//! `mark_literal_ifs_chars` had already marked. The splitter consumes each
//! glue as "protect the next char", so the doubled glue spent the first on
//! the second and released the IFS char itself — with IFS=:, `"a:b"$w` split
//! its quoted colon (and so did the plain-ASCII `'a:b'$w`).
//!
//! Every expectation below is byte-verified against WSL GNU Bash 5.3.0
//! (/usr/local/bin/bash, script-file batteries 2026-10-19, artifacts under
//! the wt868 lane); the same faces ride the
//! `scripts/regression-catalog/cases/issue-868.sh` catalogue case.

use std::process::Command;

/// Run a script FILE in its own scratch directory and return
/// (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-ifsmark868-{}-{}",
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

/// The #868 acceptance probe: a carrier-laden raw word whose quoted IFS char
/// must be protected per-char. The `$'...'` span cooks to decoded data (the
/// simulator must run the authoritative ANSI-C pipeline to match the cooked
/// word); the quoted `:` must stay glued while the unquoted `$w` result
/// still splits.
#[test]
fn carrier_laden_word_keeps_per_char_ifs_marking() {
    assert_script(
        concat!(
            "w=x:y\n",
            "IFS=:\n",
            "printf '<%s>' \"a:b\"$'x'$w; echo\n",
            "printf '<%s>' \"a:b\"$'x'$'y'$w; echo\n",
            "printf '<%s>' x$'y'$'z'$w; echo\n",
        ),
        "<a:bxx><y>\n<a:bxyx><y>\n<xyzx><y>\n",
    );
}

/// Decoded-blank faces: `$'...'` blanks are ANSI_C_IFS_GUARD pairs, and the
/// guard must never take a second glue (the splitter converts the guard to
/// IFS_GLUE; a doubled glue would re-release the blank). The quoted source
/// space is protected per-char while `$w` splits.
#[test]
fn ansi_c_blank_guard_not_double_marked() {
    assert_script(
        concat!(
            "W='c d'\n",
            "printf '<%s>' \"a b\"$'x'$W; echo\n",
            "printf '<%s>' $'a b'$W; echo\n",
            "printf '<%s>' a\\ b$W; echo\n",
        ),
        "<a bxc><d>\n<a bc><d>\n<a bc><d>\n",
    );
}

/// `$"..."` locale bodies enter the double-quote rules with the `$` dropped
/// (cooked `x`, no live `$`); the quoted colon stays protected.
#[test]
fn locale_quote_body_matches_cook() {
    assert_script(
        concat!(
            "w=x:y\n",
            "IFS=:\n",
            "printf '<%s>' \"a:b\"$\"x\"$w; echo\n",
            "printf '<%s>' \"a:b\"$\"x:y\"$w; echo\n",
        ),
        "<a:bxx><y>\n<a:bx:yx><y>\n",
    );
}

/// Escaped-quote and escaped-IFS-char carriers outside quote spans: `\'`
/// cooks to DATA_SQUOTE (the #795 guard), `\:` to a CTLESC pair, `\$` to
/// DATA_DOLLAR — all must simulate equal to the cooked word and keep their
/// IFS chars glued.
#[test]
fn escaped_char_carriers_match_cook() {
    assert_script(
        concat!(
            "w=x:y\n",
            "IFS=:\n",
            "printf '<%s>' \"a:b\"\\'z\\'$w; echo\n",
            "printf '<%s>' a\\:b$w; echo\n",
            "printf '<%s>' \"a:b\"\\$q:c$w; echo\n",
            "q=x\n",
            "printf '<%s>' \"a:b\"\\$q:c$w; echo\n",
        ),
        "<a:b'z'x><y>\n<a:bx><y>\n<a:b$q:cx><y>\n<a:b$q:cx><y>\n",
    );
}

/// The double-mark face: `mark_literal_ifs_chars` already glued the literal
/// `:`; the quoted-position pass must not add a second glue. Plain-ASCII
/// words with a custom IFS get their quoted separators protected exactly
/// like GNU (master split them through the doubled glue).
#[test]
fn quoted_separator_custom_ifs_not_double_marked() {
    assert_script(
        concat!(
            "w=x:y\n",
            "IFS=:\n",
            "printf '<%s>' \"a:b\"$w; echo\n",
            "printf '<%s>' 'a:b'$w; echo\n",
            "printf '<%s>' \"a\"'b:c'd$w; echo\n",
            "IFS='.: '\n",
            "printf '<%s>' \"a.b:c d\"$w; echo\n",
        ),
        "<a:bx><y>\n<a:bx><y>\n<ab:cdx><y>\n<a.b:c dx><y>\n",
    );
}

/// Double-quote span carriers: `'` cooks to DATA_SQUOTE, the glob set to
/// CTLESC pairs, a non-expansion `$` to DATA_DOLLAR, `\"`/`\\` to their
/// carriers — the simulation must match the cook and keep the quoted IFS
/// chars protected. Nested quoted segments (`"a"'b:c'd`) ride the same
/// path.
#[test]
fn dquote_span_carriers_match_cook() {
    assert_script(
        concat!(
            "w=x:y\n",
            "IFS=:\n",
            "printf '<%s>' \"a'b:c\"$w; echo\n",
            "IFS='*:'\n",
            "printf '<%s>' \"a*b\"$w; echo\n",
            "IFS=:\n",
            "printf '<%s>' \"a\\$b:c\"$w; echo\n",
            "printf '<%s>' \"a:b\"\"x:y\"$w; echo\n",
        ),
        "<a'b:cx><y>\n<a*bx><y>\n<a$b:cx><y>\n<a:bx:yx><y>\n",
    );
}

/// Unset/empty expansions keep the quoted separators protected without
/// inventing fields, and a fully-quoted word stays one field.
#[test]
fn unset_expansion_and_quoted_whole_word_stay_green() {
    assert_script(
        concat!(
            "IFS=:\n",
            "unset w\n",
            "printf '<%s>' \"a:b\"$w; echo\n",
            "printf '<%s>' \"a:b\"; echo\n",
            "set -- p:q\n",
            "printf '<%s>' \"$@\"; echo\n",
        ),
        "<a:b>\n<a:b>\n<p:q>\n",
    );
}
