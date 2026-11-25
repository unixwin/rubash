//! rubash#4128 regressions: a quoted `<<X` spanning lines inside `"$( ... )"`
//! was registered as a real heredoc header, and the delimiter-line branch
//! then spliced an IFS_GLUE byte (0x1c) in front of the closing `)` — the
//! byte landed on the last word before the `)` (silent argument corruption,
//! a phantom `$'\034'` command in the `;)` shape, or an outright syntax
//! error for the group shape).
//!
//! Root cause: `scan_line_for_comsub_heredoc_headers` tracked only two
//! quote booleans. Inside `"$(...)"` the enclosing double quote stayed
//! set, so a `'` could never open a single-quoted string (its arm is
//! gated on `!double`) and the `<<` arm never checked `!double` at all.
//! The fix gives the scanner the matched-pair stack discipline the
//! codebase already models in `complete_comsub_heredoc_header_close`
//! (GNU parse.y:4172-4174, parse_comsub with ~P_DQUOTE): a `$(` saves
//! and clears the enclosing double-quote flag, the matching `)` restores
//! it, and the `<<` arm requires `!double`.
//!
//! Second face (same report): the delimited-by-end-of-file warning for a
//! heredoc whose `)` sits on the delimiter line never fired when the
//! substitution sat in a double-quoted word, because that shape is
//! produced entirely by the readback entry of `heredoc_comsub_plan`,
//! which was kept silent by an entry flag. The warning is now
//! entry-independent, like the header-close warning (rubash#1982), with
//! once-ness owned by the existing `last_heredoc_warning_source` dedup.
//!
//! Every expectation below is byte-verified against the GNU Bash 5.3.20
//! oracle (script-file batteries; the C.UTF-8 and C locales agree on all
//! of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-comsub4128-{}-{}",
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
        .env("LC_ALL", lc_all)
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

const BATTERY: &str = r#"h() { echo "${#2}"; }
h2() { printf '%s' "$2" | od -An -tx1 | tr -d ' \n'; echo; }
echo "$(h 'x <<E
E' /s)"
echo "$(h "x <<E
E" /s)"
echo "$(h 'x <<E
E' /s;)"
echo "$(h 'x <<E
E' :)"
echo "$({ h 'x <<E
E' /s; })"
v="$(h 'x <<E
E' /s)"
echo "assign:$v"
echo $(h 'x <<E
E' /s)
echo "$(h 'x <<E
E' /s )"
echo "$(h2 'x <<E
E' /s)"
echo "$(echo "a << b")"
echo "$(echo 'a << b')"
echo "$(cat <<E
body
E
)"
cat <<EOF2
plain
EOF2
"#;

const BATTERY_EXPECTED: &str = "\
2
2
2
1
2
assign:2
2
2
2f73
a << b
a << b
body
plain
";

#[test]
fn quoted_comsub_heredoc_battery_utf8_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C.UTF-8");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn quoted_comsub_heredoc_battery_c_locale() {
    let (stdout, stderr, code) = rubash_file(BATTERY, "C");
    assert_eq!(stdout, BATTERY_EXPECTED, "stdout mismatch");
    assert!(stderr.is_empty(), "stderr not empty: {stderr}");
    assert_eq!(code, Some(0), "exit code mismatch");
}

const WARN_QUOTED: &str = r#"echo "$(cat <<E
body
E)"
echo "rc=$?"
"#;

const WARN_ASSIGNMENT_QUOTED: &str = r#"v="$(cat <<E
body
E)"
echo "v=$v"
"#;

const WARN_UNQUOTED: &str = r#"echo $(cat <<E
body
E)
"#;

fn assert_warns_once(script: &str, expected_stdout: &str) {
    let (stdout, stderr, code) = rubash_file(script, "C.UTF-8");
    assert_eq!(stdout, expected_stdout, "stdout mismatch");
    let warnings = stderr
        .matches("warning: here-document at line 1 delimited by end-of-file (wanted `E')")
        .count();
    assert_eq!(
        warnings, 1,
        "expected exactly one EOF warning, stderr: {stderr}"
    );
    assert_eq!(code, Some(0), "exit code mismatch");
}

#[test]
fn quoted_unterminated_heredoc_warns() {
    assert_warns_once(WARN_QUOTED, "body\nrc=0\n");
}

#[test]
fn assignment_quoted_unterminated_heredoc_warns_once() {
    assert_warns_once(WARN_ASSIGNMENT_QUOTED, "v=body\n");
}

#[test]
fn unquoted_unterminated_heredoc_warns_once() {
    assert_warns_once(WARN_UNQUOTED, "body\n");
}
