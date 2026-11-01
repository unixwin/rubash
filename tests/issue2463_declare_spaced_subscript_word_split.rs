//! rubash#2463 — an UNQUOTED `declare DIRSTACK[abc + 1]=x` must be
//! word-split before it reaches the builtin: GNU emits three
//! invalid-identifier diagnostics, one per resulting word.
//!
//! GNU ground: read_token_word consumes a matched `[...]` span into
//! the word ONLY where an assignment word is acceptable —
//! `token_index > 0 && assignment_acceptable(last_read_token) &&
//! token_is_ident(...)`, or at a compound-list element start
//! (parse.y:5633-5651) — and assignment_acceptable is command-token
//! position (parse.y:3163). Everywhere else the whitespace inside the
//! brackets is an ordinary word boundary (syntax.h:29-30
//! shell_break_chars), and assignment() (general.c:480) and the
//! builtins see the split words.
//!
//! rubash's lexer glued the span at ANY position when the word's text
//! matched `name[...]=...`, so declare received one operand and the
//! generic door treated the whole span as a subscript. The lexer now
//! keeps the span atomic only for the compound-value form
//! (`name[sub]=(...)`, whose `(` cannot be reassembled from split word
//! tokens), and the parser reassembles the one-word form exactly
//! where GNU's assignment_acceptable holds (first-word position,
//! collect_split_array_element_assignment_word). The QUOTED form is
//! untouched: it still reaches the DIRSTACK router whole and keeps the
//! #2454 behavior.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes; the scratch path is normalized to WORK and the
//! script path to SCRIPT, mirroring tests/issue2454_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2463-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE; return (stdout, stderr) with the
/// scratch path normalized to WORK and the script path to SCRIPT.
fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    let normalize = |text: &str| {
        text.replace(&script_path.to_string_lossy().into_owned(), "SCRIPT")
            .replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        normalize(&String::from_utf8_lossy(&output.stdout)),
        normalize(&String::from_utf8_lossy(&output.stderr)),
    )
}

#[test]
fn unquoted_dirstack_operand_splits_into_three_words() {
    let dir = scratch("dirstack");
    let (stdout, stderr) = run_script(&dir, "declare DIRSTACK[abc + 1]=x; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `DIRSTACK[abc': not a valid identifier\n\
         SCRIPT: line 1: declare: `+': not a valid identifier\n\
         SCRIPT: line 1: declare: `1]=x': not a valid identifier\n"
    );
}

#[test]
fn unquoted_generic_array_operand_splits_the_same_way() {
    let dir = scratch("generic");
    let (stdout, stderr) = run_script(&dir, "declare a[abc + 1]=x; echo \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: declare: `a[abc': not a valid identifier\n\
         SCRIPT: line 1: declare: `+': not a valid identifier\n\
         SCRIPT: line 1: declare: `1]=x': not a valid identifier\n"
    );
}

#[test]
fn argument_positions_split() {
    let dir = scratch("positions");
    let (stdout, stderr) = run_script(
        &dir,
        "printf '<%s>\\n' DIRSTACK[abc + 1]=x\n\
         printf '<%s>\\n' q a[bc + 1]=x\n\
         set -- DIRSTACK[abc + 1]=x; echo \"argc=$#\"\n",
    );
    assert_eq!(
        stdout,
        "<DIRSTACK[abc>\n<+>\n<1]=x>\n<q>\n<a[bc>\n<+>\n<1]=x>\nargc=3\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn quoted_form_is_unchanged() {
    let dir = scratch("quoted");
    let (stdout, stderr) = run_script(
        &dir,
        "declare 'DIRSTACK[abc + 1]=x'; echo \"rc=$?\"\n\
         declare 'a[abc + 1]=x'; echo \"rc=$?\"\n\
         declare -p a\n",
    );
    // The quoted span stays one operand: with `abc` unset the subscript
    // is 1, valid for both doors, and the generic door binds a[1].
    assert_eq!(stdout, "rc=0\nrc=0\ndeclare -a a=([1]=\"x\")\n");
    assert_eq!(stderr, "");
}

#[test]
fn prefix_assignment_still_binds_spaced_subscript() {
    let dir = scratch("prefix");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=40; a[abc + 1]=x; declare -p a\n\
         abc=40; x=1 a[abc + 1]=x; declare -p a\n",
    );
    // Command-token position is where GNU's assignment_acceptable
    // holds: the span is one assignment word and the spaced subscript
    // evaluates arithmetically, in first AND in later prefix position.
    assert_eq!(
        stdout,
        "declare -a a=([41]=\"x\")\ndeclare -a a=([41]=\"x\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn compound_shapes_are_unchanged() {
    let dir = scratch("compound");
    let (stdout, stderr) = run_script(
        &dir,
        "a[1 + 1]=(x y); echo \"rc=$?\"\n\
         declare arr=( [1 + 1]=two [0]=zero ); declare -p arr\n",
    );
    // The element compound form still parses as one word and fails at
    // execution with GNU's diagnostic; element-leading subscripts
    // inside a compound list still span their whitespace.
    assert_eq!(stdout, "declare -a arr=([0]=\"zero\" [2]=\"two\")\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 1: a[1 + 1]: cannot assign list to array member\n"
    );
}
