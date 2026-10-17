//! rubash#758: a malformed parameter expansion must be a `bad substitution`
//! with the command abandoned at status 1 — not silently swallowed as an
//! empty string with status 0.
//!
//! GNU validates the `${...}` reference before the operator dispatch
//! (subst.c:10053 parameter_brace_expand -> valid_brace_expansion_word,
//! subst.c:7583) and the substring operand before slicing; every failure is
//! the subst.c:10276-10288 `bad substitution` default:
//!
//!   * junk after a well-formed subscript — `${v[0]x}`, `${v[a]b}`,
//!     `${v[@]x}`, even with an operator tail `${v[0]x:-d}` —
//!     fails `valid_array_reference` (arrayfunc.c:1290: the `]` must be the
//!     last byte);
//!   * an empty substring operand `${v:}` (and `${@:}`) fails
//!     `verify_substring_values`; `${v::}` and `${v: }` stay VALID.
//!
//! The diagnostic is the WHOLE word GNU was expanding (subst.c:10278
//! `report_error (_("%s: bad substitution"), string)`), so `echo "[${v[0]x}]"`
//! reports the literal brackets too. The exit status of the abandoned command
//! is 1 and the next line still runs; that is what `set -e` / `&&` / `||` key
//! off.
//!
//! Every expected byte was measured against WSL GNU Bash 5.3.0 (script-file
//! oracle, separated stdout/stderr, `cd` to the probe dir).
//!
//! Residual (NOT asserted here, out of this PR's scope): GNU's DISCARD
//! abandons the whole AND-OR list, so `echo "[${v:}]" && echo and || echo or`
//! prints neither `and` nor `or`. rubash still runs the `||` fallback because
//! `ExecuteError::ExpansionFailure` is swallowed by the AND-OR executor in
//! `pipeline_exec.rs` — a pre-existing gap shared by `failglob`
//! (`shopt -s failglob; echo nope* && echo and || echo or`), whose fix belongs
//! to the pipeline owner (#761).

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

struct Run {
    stdout: String,
    stderr: String,
    status: Option<i32>,
}

fn run(script: &str) -> Run {
    let seq = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rubash758-{}-{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    std::fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(&dir);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
    }
}

/// The issue's exact repro: `${v[0]x}` abandons the `echo`, reports the whole
/// bracketed word, and the next line sees `$?` = 1.
#[test]
fn junk_after_subscript_is_reported_and_status_one() {
    let out = run(concat!(
        "v=abc\n",
        "echo \"[${v[0]x}]\"\n",
        "echo \"status=$?\"\n",
    ));
    assert_eq!(out.stdout, "status=1\n", "stdout {:?}", out.stdout);
    assert_eq!(
        out.stderr, "probe.sh: line 2: [${v[0]x}]: bad substitution\n",
        "stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, Some(0), "script continues to the next line");
}

/// The truncated substring `${v:}` likewise reports and yields status 1.
#[test]
fn truncated_substring_is_reported_and_status_one() {
    let out = run(concat!(
        "v=abcdef\n",
        "echo \"[${v:}]\"\n",
        "echo \"status=$?\"\n",
    ));
    assert_eq!(out.stdout, "status=1\n", "stdout {:?}", out.stdout);
    assert_eq!(
        out.stderr, "probe.sh: line 2: [${v:}]: bad substitution\n",
        "stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, Some(0));
}

/// The diagnostic names the WHOLE word, including literal text around the
/// expansion (GNU subst.c:10278).
#[test]
fn diagnostic_names_the_whole_word() {
    let out = run("v=abc\necho A${v[0]x}B\n");
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "probe.sh: line 2: A${v[0]x}B: bad substitution\n"
    );
    assert_eq!(out.status, Some(1));

    let out = run("v=abc\necho pre${v:}post\n");
    assert_eq!(
        out.stderr,
        "probe.sh: line 2: pre${v:}post: bad substitution\n"
    );
    assert_eq!(out.status, Some(1));
}

/// Every junk-after-subscript class reports, with or without an operator tail.
#[test]
fn junk_after_subscript_classes_all_report() {
    let cases = [
        ("v[a]b", "line 2: ${v[a]b}: bad substitution"),
        ("v[@]x", "line 2: ${v[@]x}: bad substitution"),
        ("v[*]x", "line 2: ${v[*]x}: bad substitution"),
        ("v[0]x:-d", "line 2: ${v[0]x:-d}: bad substitution"),
    ];
    for (body, needle) in cases {
        let out = run(&format!("v=abc; a=0\necho \"${{{body}}}\"\n"));
        assert_eq!(out.stdout, "", "{body}: stdout {:?}", out.stdout);
        assert!(
            out.stderr.contains(needle),
            "{body}: stderr {:?}",
            out.stderr
        );
        assert_eq!(out.status, Some(1), "{body}: status");
    }
}

/// `set -e`: the bad substitution's status 1 stops the script, the next line
/// does not run.
#[test]
fn errexit_stops_on_bad_substitution() {
    let out = run(concat!(
        "set -e\n",
        "v=abc\n",
        "echo \"[${v[0]x}]\"\n",
        "echo after\n",
    ));
    assert_eq!(out.stdout, "", "stdout {:?}", out.stdout);
    assert_eq!(
        out.stderr, "probe.sh: line 3: [${v[0]x}]: bad substitution\n",
        "stderr {:?}",
        out.stderr
    );
    assert_eq!(out.status, Some(1));
}

/// Guard: the well-formed substring shapes and the `:`-led operators keep
/// their current behaviour (`${v: }` is a space offset = 0, `${v::}` is an
/// empty offset/length pair).
#[test]
fn valid_substring_and_operator_forms_keep_expanding() {
    let out = run(concat!(
        "v=abcdef\n",
        "printf 'a[%s]\\n' \"${v: }\"\n",
        "printf 'b[%s]\\n' \"${v:0}\"\n",
        "printf 'c[%s]\\n' \"${v::}\"\n",
        "printf 'd[%s]\\n' \"${v:0:1}\"\n",
        "printf 'e[%s]\\n' \"${v:+}\"\n",
        "printf 'f[%s]\\n' \"${v:?}\"\n",
        "printf 'g[%s]\\n' \"${v:-}\"\n",
        "printf 'h[%s]\\n' \"${v:0:}\"\n",
        "printf 'i[%s]\\n' \"${v:=}\"\n",
        "printf 'j[%s]\\n' \"${v:x}\"\n",
    ));
    assert_eq!(
        out.stdout,
        "a[abcdef]\nb[abcdef]\nc[]\nd[a]\ne[]\nf[abcdef]\ng[abcdef]\nh[]\ni[abcdef]\nj[abcdef]\n"
    );
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}

/// Guard: the pre-existing malformed-subscript form (`${x[]}`, rubash#536)
/// keeps reporting the bare `${...}` word and status 1 — the whole-word
/// change must not alter a word that already IS the fragment.
#[test]
fn existing_empty_subscript_report_unchanged() {
    let out = run("x=5\necho ${x[]}\necho \"status=$?\"\n");
    assert_eq!(out.stdout, "status=1\n");
    assert_eq!(out.stderr, "probe.sh: line 2: ${x[]}: bad substitution\n");
    assert_eq!(out.status, Some(0));
}

/// Guard: a well-formed subscript + operator tail (`${v[0]:-d}`) still
/// expands to the element, with no diagnostic.
#[test]
fn valid_subscript_with_operator_stays_clean() {
    let out = run("v=abc\necho \"[${v[0]:-d}]\"\n");
    assert_eq!(out.stdout, "[abc]\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.status, Some(0));
}
