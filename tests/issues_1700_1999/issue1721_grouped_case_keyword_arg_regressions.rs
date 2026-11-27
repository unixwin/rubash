//! rubash#1721 (ecosuite DIV-F): a script file mentioning `set -o
//! posix` takes the grouped (incremental) driver, whose completeness
//! scan folded every keyword-KIND token into its compound stack.
//! The lexer marks every keyword-VALUED word TokenKind::Keyword and
//! lets consumers demote it (scanner.rs record_token), but the scan
//! never demoted: a reserved word in ARGUMENT position inside a case
//! clause — modernish's `command -vkb if && set -o posix` at
//! bin/modernish line 747 — pushed a phantom `fi` frame, the clause's
//! `;;` then read as a stray terminator against that frame, the
//! gatherer ended the group mid-case, and the truncated text died
//! with `syntax error: unexpected end of file from `if' command on
//! line 747` (GNU bash 5.3.20: rc=0).
//!
//! GNU anchor: parse.y:3168 CHECK_FOR_RESERVED_WORD recognizes a
//! reserved word only when reserved_word_acceptable(last_read_token)
//! (parse.y:5899) holds; under PST_CASEPAT (parse.y:3177-3186) only
//! `esac` is recognized, and not after `|`. The reader now stamps
//! that verdict on each word-path keyword token
//! (Token::reserved_position) and every completeness fold
//! (GroupScanFeeder::fold_token, stdin_source_needs_more_posix,
//! unclosed_compound_open_line, innermost_unclosed_compound,
//! first_mismatched_closer, case_command_end_index) consumes only
//! structural keywords (Token::structural_keyword).
//!
//! Second layer of the same state-machine family: the scanner's
//! case-operand slot (record_token's is_case_operand) did not count
//! a Keyword token as the case word, so with a keyword-spelled
//! subject (`case if in`) the wait for the separator `in` never
//! armed, the first clause never entered pattern state, and its
//! pattern `)` was reported as a stray close. The operand slot now
//! accepts Keyword tokens, as the parser's collect_case_word and
//! GNU's grammar (a WORD follows `case`) both do.
//!
//! Expectations are the GNU bash 5.3.20 oracle's for the same files.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-1721-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn rubash_file(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }

    fn rubash_file_parse_only(&self, script: &str) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-n")
            .arg(&file)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.0)
            .output()
            .expect("run rubash");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The modernish yash-branch shape, reduced to its bash-executable
/// core: a case clause whose body probes for the `if` keyword and
/// flips posix mode, in a file that (by mentioning `set -o posix`)
/// takes the grouped driver. (The verbatim `-vkb` yash form is
/// pinned parse-only in issue1721_parse_only_yash_branch_shape —
/// under bash, `command -vkb` is an invalid-option error, so it
/// cannot carry execution assertions.)
#[test]
fn issue1721_case_clause_command_v_if_and_set_o_posix() {
    let scratch = Scratch::new("yash");
    let script = "\
case $? in
( 0 )	set +o posix; command -v if && set -o posix ;;
( 1 )	command -v if ;;
( * )	false ;;
esac
echo survived=$?
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "if\nsurvived=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Reserved words as plain arguments inside case clauses, one per
/// opener family, plus a clause after them and code after the case:
/// none of them opens a compound.
#[test]
fn issue1721_keyword_arguments_in_case_clauses() {
    let scratch = Scratch::new("args");
    let script = "\
set -o posix
for w in if while until for select case; do
	case $w in
	( if )		echo got-if ;;
	( while )	echo got-while ;;
	( * )		echo other-$w ;;
	esac
done
case x in
( x )	echo if; echo while; echo done ;;
esac
echo end
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(
        stdout,
        "got-if\ngot-while\nother-until\nother-for\nother-select\nother-case\nif\nwhile\ndone\nend\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Keyword spellings in the case SUBJECT and in PATTERN position are
/// pattern/subject text (PST_CASEPAT), never openers.
#[test]
fn issue1721_keyword_subject_and_pattern() {
    let scratch = Scratch::new("pat");
    let script = "\
set -o posix
case if in
( if )	echo subject-and-pattern ;;
( * )	echo other ;;
esac
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "subject-and-pattern\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The driver admission is a file-level text scan: even when the only
/// `set -o posix` sits in a trailing comment, the grouped driver runs
/// and must still gather the case correctly.
#[test]
fn issue1721_driver_switch_via_comment_text() {
    let scratch = Scratch::new("comment");
    let script = "\
case $? in
( 0 )	echo if ;;
esac
echo after
# set -o posix
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "if\nafter\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// A real `if` nested inside a case clause keeps working under the
/// grouped driver (the demotion must not inert genuine openers).
#[test]
fn issue1721_real_if_inside_case_clause() {
    let scratch = Scratch::new("realif");
    let script = "\
set -o posix
case x in
( x )	if true; then echo inner; fi ;;
esac
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "inner\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Parse-only (`-n`, the modernish failure shape) over the yash
/// branch: an if/elif chain whose elif condition is a brace group
/// holding the case with the keyword probe.
#[test]
fn issue1721_parse_only_yash_branch_shape() {
    let scratch = Scratch::new("parseonly");
    let script = "\
if isset FOO; then
	echo foo
elif isset YASH_VERSION && {
	PATH=$DEFPATH command test -o posix
	case $? in
	( 0 )	set +o posix; command -vkb if && set -o posix || ! set -o posix ;;
	( 1 )	command -vkb if ;;
	( * )	false ;;
	esac
}; then
	echo yash
fi
";
    let (stdout, stderr, code) = scratch.rubash_file_parse_only(script);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// The subject spelled `in` itself: the first `in` is the case word,
/// the second is the separator (GNU-verified). This pins the scanner
/// state machine's operand slot — a keyword-spelled subject must arm
/// the wait for the separator, or the first clause never enters
/// pattern state.
#[test]
fn issue1721_keyword_subject_spelled_in() {
    let scratch = Scratch::new("insubj");
    let script = "\
set -o posix
case in in
( in )	echo in-subj ;;
esac
";
    let (stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(stdout, "in-subj\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// Negative control: a genuinely unclosed `if` in a grouped-driver
/// file still gets the EOF diagnostic naming `if` — the demotion
/// must not over-admit and lose real incompleteness.
#[test]
fn issue1721_genuinely_unclosed_if_still_diagnosed() {
    let scratch = Scratch::new("unclosed");
    let script = "\
set -o posix
if true; then
	echo never
";
    let (_stdout, stderr, code) = scratch.rubash_file(script);
    assert_eq!(code, Some(2));
    assert!(
        stderr.contains("unexpected end of file from `if' command on line 2"),
        "stderr was: {stderr}"
    );
}
