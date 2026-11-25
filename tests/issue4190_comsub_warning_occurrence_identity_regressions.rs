//! rubash#4190 regressions: the consumer side of comsub heredoc warnings
//! keyed everything by the command's first line and by source text —
//! (A) every comsub of one command was attributed to the command's start
//! line, and (B) the `(source text, generation)` dedup swallowed a second,
//! DISTINCT occurrence whose body text was identical to the first.
//!
//! GNU warns per heredoc occurrence (make_cmd.c:627 make_here_document is
//! unconditional per parse; parse.y:3120 gather_here_documents runs per
//! command parse), each time with that occurrence's own lines: the
//! `at line M` is the line of the occurrence's `<<`, and the prefix line
//! is where its gather ended. The fix gives the comsub consumption path a
//! per-occurrence identity — (dispatch generation, element slot, ordinal
//! within the element) minted where the occurrence is extracted and
//! passed through to the heredoc plan — and seeds the body parse from the
//! occurrence's own header line instead of the command's first line.
//!
//! The #4140 contract is preserved: one occurrence planned through
//! several expansion paths of the same command (the two entry faces,
//! a declined typed plan retried by the readback entry) still warns
//! exactly once. The r3/r3q/r4 shapes are #4169's remaining pins,
//! delivered with this issue per the captain's split ruling.
//!
//! Every expectation below is byte-verified against the GNU Bash 5.3.20
//! oracle (script-file batteries; the C.UTF-8 and C locales agree on all
//! of these shapes).

use std::process::Command;

fn rubash_file(script: &str, lc_all: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-heredoc4190-{}-{}",
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

fn assert_both_locales(script: &str, expected_stdout: &str, expected_stderr: &str) {
    for lc_all in ["C.UTF-8", "C"] {
        let (stdout, stderr, code) = rubash_file(script, lc_all);
        assert_eq!(stdout, expected_stdout, "stdout mismatch ({lc_all})");
        assert_eq!(stderr, expected_stderr, "stderr mismatch ({lc_all})");
        assert_eq!(code, Some(0), "exit code mismatch ({lc_all})");
    }
}

const TWO_WARNINGS_3_1_5_3: &str =
    "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
     case.sh: line 5: warning: here-document at line 3 delimited by end-of-file (wanted `E')\n";

/// rubash#4169 r3: two textually identical comsubs in ONE echo command.
/// GNU warns for each occurrence, with the second occurrence's own
/// lines; the dedup must not merge them.
const R3: &str = r#"echo $(cat <<E
body
E) $(cat <<E
body
E)
"#;

#[test]
fn two_identical_comsubs_one_command_warn_per_occurrence() {
    assert_both_locales(R3, "body body\n", TWO_WARNINGS_3_1_5_3);
}

/// rubash#4169 r3q: r3 with the whole echo argument double-quoted —
/// both comsubs live in ONE word, told apart by their ordinal and
/// line offset within that word.
const R3Q: &str = r#"echo "$(cat <<E
body
E) $(cat <<E
body
E)"
"#;

#[test]
fn two_identical_comsubs_one_quoted_word_warn_per_occurrence() {
    assert_both_locales(R3Q, "body body\n", TWO_WARNINGS_3_1_5_3);
}

/// rubash#4169 r4: three comsubs in one command — three warnings, each
/// carrying its own occurrence's lines (defect A: all three used to
/// report the first occurrence's `line 3 / at line 1`).
const R4: &str = r#"echo $(cat <<E
b1
E) $(cat <<E
b2
E) $(cat <<E
b3
E)
"#;

#[test]
fn three_comsubs_one_command_warn_with_own_lines() {
    assert_both_locales(
        R4,
        "b1 b2 b3\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n\
         case.sh: line 5: warning: here-document at line 3 delimited by end-of-file (wanted `E')\n\
         case.sh: line 7: warning: here-document at line 5 delimited by end-of-file (wanted `E')\n",
    );
}

/// Defect A independent shape (no #4169 involvement): ONE comsub on the
/// second physical line of a backslash-continued command. GNU attributes
/// it to line 2; the command-start seed reported line 1.
const CONTINUED: &str = r#"echo x \
  $(cat <<E
body
E)
"#;

#[test]
fn comsub_on_continued_line_warns_with_its_own_line() {
    assert_both_locales(
        CONTINUED,
        "x body\n",
        "case.sh: line 4: warning: here-document at line 2 delimited by end-of-file (wanted `E')\n",
    );
}

/// Defect B through the quoted whole-word face: two separate quoted
/// words, each expanded by the readback entry — distinct occurrences,
/// both warn.
const QQ: &str = r#"echo "$(cat <<E
body
E)" "$(cat <<E
body
E)"
"#;

#[test]
fn two_quoted_comsub_words_warn_per_occurrence() {
    assert_both_locales(QQ, "body body\n", TWO_WARNINGS_3_1_5_3);
}

/// Defect B across entry faces: the first occurrence is produced by the
/// typed entry, the second (quoted whole word) by the readback entry.
/// Distinct occurrences still warn twice — face is not identity.
const MX: &str = r#"echo $(cat <<E
body
E) "$(cat <<E
body
E)"
"#;

#[test]
fn mixed_entry_faces_still_warn_per_occurrence() {
    assert_both_locales(MX, "body body\n", TWO_WARNINGS_3_1_5_3);
}

/// Guard (#4140 contract): ONE occurrence planned through both entry
/// faces of the same dispatch — here an array-subscript word, expanded
/// once by the typed walker and once by the readback pass — warns
/// exactly once.
const SUBSCRIPT_ONCE: &str = r#"a[$(cat <<E
body
E)]=x
echo done
"#;

#[test]
fn one_occurrence_two_faces_warns_once() {
    assert_both_locales(
        SUBSCRIPT_ONCE,
        "done\n",
        "case.sh: line 3: warning: here-document at line 1 delimited by end-of-file (wanted `E')\n",
    );
}
