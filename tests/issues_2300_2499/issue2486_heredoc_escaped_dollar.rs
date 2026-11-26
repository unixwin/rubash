//! rubash#2486 (face A) — an unquoted here-document's `\$` is data, not an
//! expansion, so `set -u` must never see the escaped name as a reference.
//!
//! GNU anchor: redir.c:348 heredoc_expand() sends an unquoted heredoc body
//! through `expand_string_to_string (redirectee->word, Q_HERE_DOCUMENT)`
//! (redir.c:374). In subst.c:11229 expand_word_internal() the backslash
//! branch (subst.c:11614-11661) consumes `\x` ATOMICALLY before any `$`
//! dispatch: under Q_HERE_DOCUMENT the escape class is CBSHDOC = `\ ` $`
//! (syntax.h:27 slashify_in_here_document, syntax.h:59; tflag selection at
//! subst.c:11628-11629), and the escaped `$`/`` ` `` become CTLESC-protected
//! data (subst.c:11659-11661) — parameter expansion, and with it the nounset
//! verdict, is never entered for them. A quoted delimiter (`<<'EOF'`) skips
//! expansion entirely (redir.c:361-367).
//!
//! Expectations verified against WSL GNU bash 5.3.20 script files: every
//! escaped shape prints its literal text under both `set +u` and `set -u`
//! with rc=0 and empty stderr, `\\$set` collapses to one data backslash plus
//! the real expansion, and a bare unbound `$name` in the body still raises
//! `name: unbound variable` (script-fatal via the external-cat containment
//! class: the script continues, the command reports 1).

use std::path::PathBuf;
use std::process::Command;

fn rubash_file(tag: &str, script: &str) -> (String, String, Option<i32>) {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rubash2486a-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    std::fs::write(dir.join("probe.sh"), script).expect("write probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("probe.sh")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The installer shape from the issue: `\$` inside the body of an unquoted
/// heredoc under `set -u` must stay literal and must not report the escaped
/// name unbound.
#[test]
fn issue2486_setu_heredoc_escaped_dollar_stays_literal() {
    let script = concat!(
        "set -u\n",
        "PKGV=xyz\n",
        "cat <<EOF\n",
        "PREFIX=\"\\${0%/*}/..\"\n",
        "LIBDIR=\"\\$PREFIX/lib\"\n",
        "plain=$PKGV\n",
        "EOF\n",
        "echo \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = rubash_file("installer", script);
    assert_eq!(
        stdout,
        concat!(
            "PREFIX=\"${0%/*}/..\"\n",
            "LIBDIR=\"$PREFIX/lib\"\n",
            "plain=xyz\n",
            "rc=0\n",
        ),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// The whole escape class in both `set -u` states: `\$` and `\${...}` are
/// data; `\\\$x` is a data backslash plus data `$`; `\\$set` collapses the
/// backslash pair and STILL expands the `$set` (GNU consumes `\\` and then
/// dispatches the `$`); the plain `$set` expands in both states. `set +u`
/// and `set -u` must print byte-identical bodies here.
#[test]
fn issue2486_heredoc_escape_class_matches_setu_states() {
    const BODY: &str = concat!(
        "cat <<EOF\n",
        "A1=\"\\${0%/*}/..\"\n",
        "A2=\"\\$PREFIX/lib\"\n",
        "A3=\\\\\\$QUOTED\n",
        "A4=\\\\$PKGV\n",
        "A5=$PKGV\n",
        "EOF\n",
    );
    let expected = concat!(
        "A1=\"${0%/*}/..\"\n",
        "A2=\"$PREFIX/lib\"\n",
        "A3=\\$QUOTED\n",
        "A4=\\xyz\n",
        "A5=xyz\n",
    );
    for (tag, script) in [
        ("plusu", format!("set +u\nPKGV=xyz\n{BODY}")),
        ("setu", format!("set -u\nPKGV=xyz\n{BODY}")),
    ] {
        let (stdout, stderr, code) = rubash_file(tag, &script);
        assert_eq!(stdout, expected, "state {tag} stdout: {stdout}");
        assert_eq!(stderr, "", "state {tag} stderr: {stderr}");
        assert_eq!(code, Some(0), "state {tag}");
    }
}

/// A quoted delimiter disables expansion entirely: even a bare `$name` is
/// literal data and `set -u` cannot fire (redir.c:361-367).
#[test]
fn issue2486_quoted_delimiter_skips_expansion_under_setu() {
    let script = concat!(
        "set -u\n",
        "cat <<'EOF'\n",
        "PREFIX=\"${0%/*}\"\n",
        "LIBDIR=\\$PREFIX/lib\n",
        "EOF\n",
        "echo \"rc=$?\"\n",
    );
    let (stdout, stderr, code) = rubash_file("qdelim", script);
    assert_eq!(
        stdout,
        concat!("PREFIX=\"${0%/*}\"\n", "LIBDIR=\\$PREFIX/lib\n", "rc=0\n",),
        "stdout: {stdout}"
    );
    assert_eq!(stderr, "", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

/// The fix must not over-suppress: a genuinely unbound `$name` in an
/// unquoted heredoc body still reports `name: unbound variable` under
/// `set -u` (GNU expands the body, the nounset fatal fires). For an
/// external `cat` target GNU contains the fatal in the forked child, so
/// the script continues and the command reports 1.
#[test]
fn issue2486_setu_heredoc_real_unbound_still_fires() {
    let script = concat!(
        "set -u\n",
        "cat <<EOF\n",
        "E1=$NOPE_REAL_UNBOUND\n",
        "EOF\n",
        "echo \"after rc=$?\"\n",
    );
    let (stdout, stderr, code) = rubash_file("realunbound", script);
    assert_eq!(stdout, "after rc=1\n", "stdout: {stdout}");
    assert!(
        stderr.contains("NOPE_REAL_UNBOUND: unbound variable"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}
