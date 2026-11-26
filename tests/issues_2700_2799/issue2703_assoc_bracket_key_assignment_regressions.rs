//! rubash#2703 — an associative assignment whose quoted subscript
//! contains a bracket stored the value WITH its quotes: `m['a]b']='v'`
//! stored `'v'` (len 3) instead of `v` (len 1).
//!
//! GNU ground: assignment() (general.c:480) splits the SYNTACTIC word
//! with skipsubscript on the unexpanded spelling — quoting still
//! present — so the subscript boundary is the quote-aware one, and the
//! value is then expanded once as an assignment string
//! (assign_array_element, arrayfunc.c:815-865).
//!
//! Rubash's parser reconciles two parses of the word: the cooked
//! (de-quoted) parse and the raw parse. When the cooked parse succeeded
//! at a wrong boundary, the raw parse owned the boundaries AND its
//! `value`/`subscript` fields were re-cooked with remove_shell_quotes.
//! When the cooked parse FAILED outright — exactly the bracket-in-quotes
//! shapes, whose de-quoted text has no `=` after its first `]` — the raw
//! parse was used with its fields still raw, so the downstream synthetic
//! expansion (which treats `'` as compound-element data) stored the
//! literal quotes. The fix applies the same re-cook whenever the raw
//! parse owns the boundaries.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2703-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

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
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn check(tag: &str, script: &str, expected_stdout: &str) {
    let dir = scratch(tag);
    let (stdout, stderr) = run_script(&dir, script);
    assert_eq!(
        stdout, expected_stdout,
        "stdout for {tag} (stderr: {stderr})"
    );
    assert_eq!(stderr, "", "stderr for {tag}");
}

#[test]
fn issue2703_quoted_bracket_keys_store_unquoted_values() {
    // The filing repro, plus the `[` and leading-bracket faces of the
    // same class and the double-quoted-key spelling.
    check(
        "rb",
        "declare -A m\nm['a]b']='v'\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"v\" )\n",
    );
    check(
        "lb",
        "declare -A m\nm['a[b']='w'\ndeclare -p m\n",
        "declare -A m=([\"a[b\"]=\"w\" )\n",
    );
    check(
        "lead",
        "declare -A m\nm['[a']='x'\ndeclare -p m\n",
        "declare -A m=([\"[a\"]=\"x\" )\n",
    );
    check(
        "dqkey",
        "declare -A m\nm[\"a]b\"]='v'\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"v\" )\n",
    );
    // The stored value itself (not just its declare rendering): length
    // and a read-back through a variable key.
    check(
        "readback",
        "declare -A m\nm['a]b']='v'\nk='a]b'\necho \"val=${m[$k]} len=${#m[$k]}\"\n",
        "val=v len=1\n",
    );
    // Value-side quoting classes on a bracket key: a quoted expansion
    // result and a mixed-quote value both de-quote exactly once.
    check(
        "dollar",
        "declare -A m\nm['a]b']='$v'\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"\\$v\" )\n",
    );
    check(
        "mixed",
        "declare -A m\nm['a]b']=pre'mid'post\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"premidpost\" )\n",
    );
}

#[test]
fn issue2703_control_shapes_unchanged() {
    // Controls that already agreed with GNU: plain quoted key, unquoted
    // key, `=`/space keys, unquoted and double-quoted values on a
    // bracket key, and an escaped-quote subscript.
    check(
        "plain",
        "declare -A m\nm['ab']='v'\ndeclare -p m\n",
        "declare -A m=([ab]=\"v\" )\n",
    );
    check(
        "eqkey",
        "declare -A m\nm['a=b']='y'\ndeclare -p m\n",
        "declare -A m=([a=b]=\"y\" )\n",
    );
    check(
        "spacekey",
        "declare -A m\nm['a b']='z'\ndeclare -p m\n",
        "declare -A m=([\"a b\"]=\"z\" )\n",
    );
    check(
        "unqval",
        "declare -A m\nm['a]b']=v\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"v\" )\n",
    );
    check(
        "dqval",
        "declare -A m\nm['a]b']=\"v\"\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"v\" )\n",
    );
    check(
        "append",
        "declare -A m\nm['a]b']='v'\nm['a]b']+='w'\ndeclare -p m\n",
        "declare -A m=([\"a]b\"]=\"vw\" )\n",
    );
}
