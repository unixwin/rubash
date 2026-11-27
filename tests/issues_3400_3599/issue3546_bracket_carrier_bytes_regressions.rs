//! Issue unixwin/rubash#3546: a carrier byte (0x0c, 0x14..=0x1f) inside a
//! `[...]` bracket expression never matches in `${v//[...]/r}` (or the
//! anchored `${v#...}` / `${v%...}` forms); GNU matches it. Non-carrier
//! control bytes (0x01, 0x07, 0x7f) match on both sides.
//!
//! GNU anchor: subst.c:9184 pat_subst() hands the pattern to
//! lib/glob/sm_loop.c:398 BRACKMATCH(), which compares bracket members
//! against the candidate BYTE — carrier bytes are data there.
//!
//! The rubash defect was in the `${v//...}` scanning finder
//! (`find_parameter_pattern_match`, src/executor/parameter_replace.rs)
//! and its anchored twins (`find_parameter_prefix_match` /
//! `find_parameter_suffix_match`): `pattern_match_length_bound` counts
//! PATTERN atoms, but the candidate walk counted VALUE chars — and a
//! carrier byte is stored as a TWO-char raw-byte marker pair
//! (U+E000 + payload), i.e. one match unit spanning two chars. A
//! single-atom bracket pattern therefore had bound 1, so the only
//! candidate slice that could match (the two-char pair) was pruned
//! before the matcher ever saw it. Raw one-char bytes (0x01/0x07/0x7f)
//! fit the bound and matched, which is exactly the issue's pass/fail
//! split. The `${v#pat}` / `${v%pat}` removal walks already exclude
//! marker-pair split points (rubash#1261); the replace finders did not.
//! The fix counts match units on both sides of the bound by walking
//! unit boundaries only.
//!
//! Every expected value below is GNU bash 5.3.20 output for the same
//! script (issue #3546 repro table), re-checked against the local GNU
//! bash for the byte matrix.

use std::process::{Command, Stdio};

fn run_file(script: &str, tag: &str) -> (String, String, i32) {
    let path =
        std::env::temp_dir().join(format!("i3546_bracket_{}_{}.sh", tag, std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let out = child.wait_with_output().expect("wait rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Issue repro table, all seven bytes, both spellings: `[$c]` / `[x$c]`
/// variable expansion and `$'...'` written directly inside the brackets.
/// GNU: every row `a_b|a_b|a_b`.
#[test]
fn bracket_carrier_bytes_match_in_parameter_substitution() {
    for byte in [0x01u8, 0x07, 0x0c, 0x14, 0x1b, 0x1f, 0x7f] {
        let hex = format!("{byte:02x}");
        let script = format!(
            "c=$'\\x{hex}'\nw=\"a${{c}}b\"\nt=${{w//[$c]/_}}\nt2=${{w//[x$c]/_}}\nt3=${{w//[$'\\x{hex}']/_}}\nprintf '%s|%s|%s' \"$t\" \"$t2\" \"$t3\"\n"
        );
        let (out, err, code) = run_file(&script, &format!("byte_{hex}"));
        assert_eq!(code, 0, "byte 0x{hex}: exit {code}, stderr {err}");
        assert_eq!(
            out, "a_b|a_b|a_b",
            "byte 0x{hex}: bracket member must match in all three spellings"
        );
    }
}

/// Multi-member bracket over two carrier bytes (issue's second repro
/// line): `${h//[$'\033\x14']/_}` on `m<ESC>n<0x14>o` is `m_n_o`.
#[test]
fn bracket_two_carrier_bytes_match() {
    let script = "h=$'m\\033n\\x14o'\nprintf '%s' \"${h//[$'\\033\\x14']/_}\"\n";
    let (out, err, code) = run_file(script, "two_carrier");
    assert_eq!(code, 0, "exit {code}, stderr {err}");
    assert_eq!(out, "m_n_o", "both carrier members must match");
}

/// Same mechanism in the anchored and single-char forms: `${v#pat}` /
/// `${v%pat}` share the bound prune, and `?` must consume one whole
/// carried byte (one match unit), not one half of its marker pair.
#[test]
fn bracket_carrier_bytes_anchored_and_question_forms() {
    // (tag, script, expected stdout)
    for (tag, script, expected) in [
        (
            "prefix_carrier",
            "c=$'\\x14'\nw=\"${c}b\"\nprintf '%s' \"${w#[$c]}\"\n",
            "b",
        ),
        (
            "prefix_control",
            "c=$'\\x01'\nw=\"${c}b\"\nprintf '%s' \"${w#[$c]}\"\n",
            "b",
        ),
        (
            "suffix_carrier",
            "c=$'\\x14'\nw=\"a${c}\"\nprintf '%s' \"${w%[$c]}\"\n",
            "a",
        ),
        (
            "suffix_control",
            "c=$'\\x01'\nw=\"a${c}\"\nprintf '%s' \"${w%[$c]}\"\n",
            "a",
        ),
        (
            "question_carrier",
            "c=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w//?/_}\"\n",
            "___",
        ),
        (
            "range_carrier",
            "c=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w//[$'\\x01'-$'\\x1f']/_}\"\n",
            "a_b",
        ),
        (
            "single_carrier",
            "c=$'\\x14'\nw=\"a${c}b${c}c\"\nprintf '%s' \"${w/[$c]/_}\"\n",
            "a_b\u{14}c",
        ),
        (
            "no_match_carrier",
            "c=$'\\x14'\nw=\"a${c}b\"\nprintf '%s' \"${w//[z]/_}\"\n",
            "a\u{14}b",
        ),
        (
            "plain_ascii_guard",
            "w=\"abc\"\nprintf '%s' \"${w//[b]/_}\"\n",
            "a_c",
        ),
    ] {
        let (out, err, code) = run_file(script, tag);
        assert_eq!(code, 0, "{tag}: exit {code}, stderr {err}");
        assert_eq!(out, expected, "{tag}: stdout mismatch");
    }
}
