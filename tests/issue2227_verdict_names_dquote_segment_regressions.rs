//! rubash#2227 (face 1 completion) — the `<string>` GNU names in
//! `bad substitution: no closing `]' in <string>` for a `$[...]` span
//! cut by a double-quote segment end. GNU expand_word_internal
//! (subst.c:11692-11723) hands each double-quoted segment
//! (string_extract_double_quoted, subst.c:852, which stops at the
//! first unescaped `"` — it has no `$[` passthrough at subst.c:944) to
//! a RECURSIVE expansion level, so the failing extraction
//! (extract_arithmetic_subst subst.c:1301 -> extract_delimited_string
//! subst.c:1361, error site subst.c:1505) names THE SEGMENT — not the
//! whole word prefix and not the assignment's `name=` prefix.
//!
//! Before this fix rubash named: the whole RHS word for a plain quoted
//! assignment (`in a=$[1+1] b`), the `q=`/`a[0]=`/`lv=` prefix for
//! declare/export/local/array-element assignment words, the unquoted
//! pre-segment text for `pre"a=$["1+1"]"post` shapes, and leaked the
//! assignment channel's quote carriers into the string. The cookers
//! (lexer/quotes.rs, both the byte-cursor and char families) now
//! re-emit the doomed segment's opening `"` (reopen_doomed_segment)
//! when the `$[...]` span is cut, and the walkers reconstruct the
//! level string from it (dollar_bracket_no_closing_string). The
//! collector also decodes PROTECTED_ESCAPED_SQUOTE (0x16) — the
//! assignment-RHS channel's re-encoding of DATA_SQUOTE — so a plain
//! `'` inside a `$[...]` body in that channel arms as a quote unit and
//! the body reaches the evaluator spelled exactly as GNU extracted it.
//!
//! All expectations below were measured against GNU bash 5.3.20 (the
//! project oracle) with these exact script files, byte-for-byte
//! (stdout/stderr/rc), via `wsl bash <file>` script-file comparison.

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
            std::env::temp_dir().join(format!("rubash-2227v-{tag}-{}-{nanos}", std::process::id()));
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

/// A `$[...]` verdict line: stderr names `bad substitution: no closing
/// `]' in <segment>` at `line`, the command is discarded (status 1),
/// stdout stays empty, and execution CONTINUES with the next line
/// (GNU exp_jump_to_top_level(DISCARD) abandons the command, not the
/// script).
fn assert_verdict(tag: &str, script: &str, line: u32, segment: &str) {
    let scratch = Scratch::new(tag);
    let (stdout, stderr, code) = scratch.run(script);
    assert_eq!(code, 1, "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, "", "{tag}: stdout (stderr: {stderr})");
    assert!(
        stderr.contains(&format!(
            "line {line}: bad substitution: no closing `]' in {segment}"
        )),
        "{tag}: stderr: {stderr}"
    );
}

/// The issue's three seed shapes (rubash#2227 regression seed), pinned
/// byte-for-byte. Shape 3 is the arithmetic-error face of the same
/// extraction: `$[1\]]` inside double quotes keeps the backslash pass
/// (subst.c:1419-1424) and the body evaluates as `1]`.
#[test]
fn issue2227_seed_shapes_name_the_segment() {
    let scratch = Scratch::new("seed");
    let (stdout, stderr, code) = scratch.run("echo \"a=$[\"1+1\"] b\"\necho ok\n");
    assert_eq!(code, 0);
    assert_eq!(stdout, "ok\n");
    assert!(
        stderr.contains("line 1: bad substitution: no closing `]' in a=$["),
        "stderr: {stderr}"
    );

    let scratch = Scratch::new("seed2");
    let (stdout, stderr, code) = scratch.run("echo \"c=$[\"]\"] d\"\necho ok\n");
    assert_eq!(code, 0);
    assert_eq!(stdout, "ok\n");
    assert!(
        stderr.contains("line 1: bad substitution: no closing `]' in c=$["),
        "stderr: {stderr}"
    );

    let scratch = Scratch::new("seed3");
    let (stdout, stderr, code) = scratch.run("echo \"e=$[1\\]] f\"\necho ok\n");
    assert_eq!(code, 0);
    assert_eq!(stdout, "ok\n");
    assert!(
        stderr.contains(
            "1]: arithmetic syntax error: invalid arithmetic operator (error token is \"]\")"
        ),
        "stderr: {stderr}"
    );
}

/// Plain quoted assignment: the level string is the RHS segment only —
/// `a=$[`, not the whole cooked RHS `a=$[1+1] b` that rubash named
/// before (the assignment channel's carriers leaked into the string).
#[test]
fn issue2227_quoted_assignment_rhs_names_segment() {
    assert_verdict("assign-dq", "x=\"a=$[\"1+1\"] b\"\n", 1, "a=$[");
    assert_verdict("assign-cut", "y=\"c=$[\"]\"] d\"\n", 1, "c=$[");
}

/// Assignment-shaped words walked by the generic command-word path
/// (declare/export/local/readonly/array-element) must name the segment
/// without the `name=` / `a[0]=` / `lv=` prefix.
#[test]
fn issue2227_assignment_like_words_drop_the_name_prefix() {
    assert_verdict("declare", "declare q=\"a=$[\"1+1\"] b\"\n", 1, "a=$[");
    assert_verdict("export", "export r=\"a=$[\"1+1\"] b\"\n", 1, "a=$[");
    assert_verdict("array-elem", "a[0]=\"a=$[\"1+1\"] b\"\n", 1, "a=$[");
    // One-line definition+call: the `local` walk runs at the call, and
    // both shells agree the verdict names the segment at that line.
    assert_verdict(
        "local-fn",
        "f() { local lv=\"a=$[\"1+1\"] b\"; }; f\n",
        1,
        "a=$[",
    );
    assert_verdict("readonly", "readonly rv=\"a=$[\"1+1\"] b\"\n", 1, "a=$[");
}

/// Unquoted text glued to the doomed segment stays OUT of the named
/// string: GNU expands `"..."` as its own level, so only the segment
/// content through the failing `$[` (plus the body the collector
/// consumed before the cut) is named.
#[test]
fn issue2227_presegment_text_is_excluded() {
    assert_verdict("glued-pre", "echo pre\"a=$[\"1+1\"]post\"\n", 1, "a=$[");
    assert_verdict("two-segments", "echo \"a\"\"b=$[\"1+1\"]\"\n", 1, "b=$[");
    assert_verdict(
        "word-then-segment",
        "echo \"one\"x\"a=$[\"1+1\"]\"\n",
        1,
        "a=$[",
    );
    assert_verdict("assign-glued", "x=pre\"a=$[\"1+1\"]post\"\n", 1, "a=$[");
}

/// The named string is the WHOLE segment — body bytes the extraction
/// consumed before the segment's closing quote are included
/// (`"a=$[1"2"]"` names `a=$[1`, not `a=$[`).
#[test]
fn issue2227_verdict_names_the_whole_segment() {
    assert_verdict("body-before-cut", "echo \"a=$[1\"2\"]\"\n", 1, "a=$[1");
    assert_verdict(
        "body-with-pre",
        "echo \"pre a=$[1\"2\"] tail\"\n",
        1,
        "pre a=$[1",
    );
}

/// Single quotes inside a `$[...]` body in the assignment-RHS channel
/// (which re-encodes DATA_SQUOTE as PROTECTED_ESCAPED_SQUOTE before
/// the walk): the quote unit is skipped whole (a `]` inside it never
/// closes) and the body keeps its quotes, exactly as GNU extracts it
/// (the arithmetic evaluator then reports the quoted token).
#[test]
fn issue2227_assignment_channel_single_quote_units() {
    let scratch = Scratch::new("sq-body");
    let (stdout, stderr, code) = scratch.run("x=\"a=$['1+1']\"\necho \"x=[$x]\"\n");
    assert_eq!(code, 0);
    assert_eq!(stdout, "x=[]\n");
    assert!(
        stderr.contains(
            "'1+1': arithmetic syntax error: operand expected (error token is \"'1+1'\")"
        ),
        "stderr: {stderr}"
    );

    // A `]` inside the single-quoted unit must not close the span.
    let scratch = Scratch::new("sq-close");
    let (stdout, stderr, code) = scratch.run("x=\"a=$[']']\"\n");
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("']': arithmetic syntax error: operand expected (error token is \"']'\""),
        "stderr: {stderr}"
    );

    let scratch = Scratch::new("sq-bracket");
    let (stdout, stderr, code) = scratch.run("x=\"a=$['a]b']\"\n");
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(
        stderr
            .contains("'a]b': arithmetic syntax error: operand expected (error token is \"'a]b'\""),
        "stderr: {stderr}"
    );

    // A source `\'` keeps its backslash: the backslash-pass arm consumes
    // the pair, the quote never opens a unit, and the evaluator sees
    // `\'1+1\'` (GNU: error token "\'1+1\'").
    let scratch = Scratch::new("sq-escaped");
    let (stdout, stderr, code) = scratch.run("x=\"a=$[\\'1+1\\']\"\n");
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains(
            "\\'1+1\\': arithmetic syntax error: operand expected (error token is \"\\'1+1\\'\""
        ),
        "stderr: {stderr}"
    );
}

/// The unquoted assignment/channel control that already agreed with GNU
/// before this fix must not regress.
#[test]
fn issue2227_unquoted_assignment_controls() {
    let scratch = Scratch::new("unq");
    let (stdout, stderr, code) = scratch.run("y=$[\"1+1\"]\necho \"y=$y\"\n");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "y=2\n");

    let scratch = Scratch::new("unq-sq");
    let (stdout, stderr, code) = scratch.run("z=$['1+1']\n");
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(
        stderr
            .contains("'1+1': arithmetic syntax error: operand expected (error token is \"'1+1'\""),
        "stderr: {stderr}"
    );

    let scratch = Scratch::new("unq-cut");
    let (stdout, stderr, code) = scratch.run("s=$[\"]\"]\n");
    assert_eq!(code, 1);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("]: arithmetic syntax error: operand expected (error token is \"]\""),
        "stderr: {stderr}"
    );
}
