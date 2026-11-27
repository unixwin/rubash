//! Issue rubash#3605 regression: a word whose `@`-list expansion comes
//! back empty is retained as one empty argument whenever the word saw a
//! quoted null, and quoted `"${@op...}"` words never re-split their
//! elements just because the cooked text lost its quote bytes.
//!
//! GNU anchors:
//! - subst.c:12026-12035 expand_word_internal: the all-empty discard
//!   branch is `had_quoted_null || (quoted_dollar_at == 0 &&
//!   PARTIALLY_QUOTED)` — had_quoted_null ALONE retains the word, so
//!   `''"${a[@]}"`, `""${@/*}"` and `"$x""${@#p}"` (x unset) all keep
//!   `<>`. The ''/"" spans add CTLNUL + had_quoted_null at
//!   11841-11847 / 11940-11944.
//! - subst.c:11229 expand_word_internal: field splitting only runs on
//!   the unquoted path — a quoted `"${@/*}"` keeps its (empty) elements
//!   whole even when an adjacent `''` cooked the quote bytes away.
//!
//! Every expectation below is byte-verified against GNU Bash 5.3.20
//! (/opt/homebrew/bin/bash, script-file probes /tmp/v4.sh + ctl.sh
//! 2026-11-16).

use std::process::Command;

/// Run a script FILE and return (stdout, stderr, code).
fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i3605nullanchor-{}-{}",
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

const RECHO: &str =
    "recho() { local i=0; for a; do i=$((i+1)); printf 'argv[%d] = <%s>\\n' \"$i\" \"$a\"; done; }\n";

/// `''"${@/*}"` and `''"${@#X}"` under `set -- X`: the pattern-modified
/// `$@` element is the empty string, and the adjacent quoted null keeps
/// it as one empty argument (dollar-at7 anchor family).
#[test]
fn quoted_null_anchors_modified_at_element() {
    let script = String::from(RECHO)
        + concat!(
            "set -- X\n",
            "recho a ''\"${@/*}\"\n",
            "recho b x''\"${@/*}\"\n",
            "recho d ''\"${@#X}\"\n",
        );
    let (stdout, stderr, code) = rubash_file(&script);
    assert_eq!(
        stdout,
        "argv[1] = <a>\nargv[2] = <>\nargv[1] = <b>\nargv[2] = <x>\nargv[1] = <d>\nargv[2] = <>\n"
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// had_quoted_null alone retains: `''"${a[@]}"` on an unset array and
/// `''"${*}"`/`''"${*:-}"`/`''"${@}"`/`''"$@"` with no positional
/// parameters are all one empty argument.
#[test]
fn quoted_null_retains_empty_list_sources() {
    let script = String::from(RECHO)
        + concat!(
            "set --\n",
            "recho i ''\"${@/*}\"\n",
            "recho j ''\"${a[@]}\"\n",
            "recho k ''\"${@}\"\n",
            "recho l ''\"$@\"\n",
            "recho m ''\"${*}\"\n",
            "recho n ''\"${*:-}\"\n",
            "x=''\n",
            "recho o \"$x\"\"${@/*}\"\n",
        );
    let (stdout, stderr, code) = rubash_file(&script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = <i>\nargv[2] = <>\n",
            "argv[1] = <j>\nargv[2] = <>\n",
            "argv[1] = <k>\nargv[2] = <>\n",
            "argv[1] = <l>\nargv[2] = <>\n",
            "argv[1] = <m>\nargv[2] = <>\n",
            "argv[1] = <n>\nargv[2] = <>\n",
            "argv[1] = <o>\nargv[2] = <>\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// Unquoted `${@/*}` still drops its empty element; `''`/`${@/*}''` keep
/// it because `''` sets had_quoted_null regardless of the expansion's
/// own quoting.
#[test]
fn unquoted_modified_at_still_splits() {
    let script = String::from(RECHO)
        + concat!(
            "set -- X\n",
            "recho c1 ${@/*}\n",
            "recho c2 x${@/*}\n",
            "recho c3 ''${@/*}\n",
            "recho c4 ${@/*}''\n",
        );
    let (stdout, stderr, code) = rubash_file(&script);
    assert_eq!(
        stdout,
        concat!(
            "argv[1] = <c1>\n",
            "argv[1] = <c2>\nargv[2] = <x>\n",
            "argv[1] = <c3>\nargv[2] = <>\n",
            "argv[1] = <c4>\nargv[2] = <>\n",
        )
    );
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}

/// `"$@"` with no positional parameters expands to ZERO words even
/// beside a quoted null on a DIFFERENT word — the retention is
/// per-word (quote4.sub `n "$@"''` is 1 not 0: the `''` word is the
/// one that stays).
#[test]
fn bare_quoted_at_still_drops_when_empty() {
    let script =
        String::from(RECHO) + concat!("set --\n", "recho n \"$@\"\n", "recho n \"$@\"''\n");
    let (stdout, stderr, code) = rubash_file(&script);
    assert_eq!(stdout, "argv[1] = <n>\nargv[1] = <n>\nargv[2] = <>\n");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0));
}
