//! rubash#2462 — the GENERIC declare/local/typeset array door must
//! reproduce GNU's negative-subscript contract: wrap against the
//! array's current maximum index, and when the wrap still leaves the
//! index negative, report the WRITTEN subscript via err_badarraysub
//! while the remaining operands still bind.
//!
//! GNU ground: assign_array_element (arrayfunc.c:334) receives the
//! written `name` string; its indexed arm (arrayfunc.c:382-390)
//! evaluates the subscript once, wraps a negative result with
//! `ind = array_max_index + 1 + ind`, and calls err_badarraysub(name)
//! (error.c:440-443) when the index is still negative. The failure is
//! per-operand (any_failed): later operands of the same declare still
//! bind, and a scalar counts as a one-element array whose maximum is 0
//! (convert_var_to_array promotes it to element [0]).
//!
//! rubash's executor rewrite pass evaluates declare-operand subscripts
//! up front, so the builtin used to see only `a[-5]`: a negative value
//! collapsed into the same silent fallthrough as an evaluation failure
//! (no diagnostic, no wrap, status 1). The fix keeps the evaluated
//! value in the builtin's indexed arm, wraps there against the live
//! entries snapshot, and recovers the written spelling from the
//! pre-rewrite operand list (WrittenOperandHints) for the diagnostic.
//! A sibling defect rode along: a failed operand left the executor's
//! typed-cell sync gated off (it ran only on status 0), so a sibling
//! operand that DID bind desynced the typed cell from its mirror and
//! the next read panicked; the sync now runs whenever the builtin ran,
//! as the oob abort path already did (rubash#635).
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes; the scratch path is normalized to WORK and the
//! script path to SCRIPT, mirroring tests/issue2454_*.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2462-{tag}-{}", std::process::id()));
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
fn bad_subscript_diagnostic_uses_written_spelling() {
    let dir = scratch("spelling");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=-5\n\
         declare a[abc]=x; echo \"rc=$?\"\n\
         declare -p a\n\
         typeset t[abc]=x; echo \"rc=$?\"\n",
    );
    assert_eq!(stdout, "rc=1\ndeclare -a a=()\nrc=1\n");
    assert_eq!(
        stderr,
        "SCRIPT: line 2: a[abc]: bad array subscript\n\
         SCRIPT: line 4: t[abc]: bad array subscript\n"
    );
}

#[test]
fn negative_subscript_wraps_against_current_max() {
    let dir = scratch("wrap");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=-5\n\
         b=(1 2 3 4 5 6)\n\
         declare b[abc]=x; echo \"rc=$?\"\n\
         declare b[-1]=y; echo \"rc=$?\"\n\
         declare -p b\n\
         declare d=(1 2 3 4 5 6) d[abc]=x; echo \"rc=$?\"\n\
         declare -p d\n\
         i=-1\n\
         w=(1 2 3)\n\
         declare w[i++]=x; echo \"rc=$? i=$i\"\n\
         declare -p w\n",
    );
    // `d[abc]` wraps against the array created by the FIRST operand of
    // the same declare (bind-time adjudication, not rewrite-time), and
    // `w[i++]` evaluates the subscript exactly once (i ends at 0).
    assert_eq!(
        stdout,
        "rc=0\n\
         rc=0\n\
         declare -a b=([0]=\"1\" [1]=\"x\" [2]=\"3\" [3]=\"4\" [4]=\"5\" [5]=\"y\")\n\
         rc=0\n\
         declare -a d=([0]=\"1\" [1]=\"x\" [2]=\"3\" [3]=\"4\" [4]=\"5\" [5]=\"6\")\n\
         rc=0 i=0\n\
         declare -a w=([0]=\"1\" [1]=\"2\" [2]=\"x\")\n"
    );
    assert_eq!(stderr, "");
}

#[test]
fn append_form_wraps_and_diagnoses_without_plus() {
    let dir = scratch("append");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=-5\n\
         b=(1 2 3 4 5 6)\n\
         declare b[abc]+=z; echo \"rc=$?\"\n\
         declare -p b\n\
         abc=-9\n\
         declare b[abc]+=z; echo \"rc=$?\"\n",
    );
    assert_eq!(
        stdout,
        "rc=0\n\
         declare -a b=([0]=\"1\" [1]=\"2z\" [2]=\"3\" [3]=\"4\" [4]=\"5\" [5]=\"6\")\n\
         rc=1\n"
    );
    // The append `+` is not part of the reported word.
    assert_eq!(stderr, "SCRIPT: line 6: b[abc]: bad array subscript\n");
}

#[test]
fn failed_operand_does_not_stop_the_list() {
    let dir = scratch("continuation");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=-9\n\
         declare c=(1 2 3) c[abc]=x c[0]=z; echo \"rc=$?\"\n\
         declare -p c\n\
         echo \"after=${c[0]}\"\n",
    );
    assert_eq!(
        stdout,
        "rc=1\n\
         declare -a c=([0]=\"z\" [1]=\"2\" [2]=\"3\")\n\
         after=z\n"
    );
    assert_eq!(stderr, "SCRIPT: line 2: c[abc]: bad array subscript\n");
}

#[test]
fn scalar_and_empty_promotion_edges() {
    let dir = scratch("edges");
    let (stdout, stderr) = run_script(
        &dir,
        "s=hello\n\
         declare s[-1]=x; echo \"rc=$?\"\n\
         declare -p s\n\
         s2=hello\n\
         abc=-5\n\
         declare s2[abc]=x; echo \"rc=$?\"\n\
         declare -p s2\n\
         g=\n\
         declare g[-1]=x; echo \"rc=$?\"\n\
         declare -p g\n\
         h=\n\
         abc=-2\n\
         declare h[abc]=x; echo \"rc=$?\"\n\
         declare -p h\n\
         e=()\n\
         abc=-1\n\
         declare e[abc]=x; echo \"rc=$?\"\n\
         declare -p e\n\
         declare -a u\n\
         declare u[-1]=x; echo \"rc=$?\"\n\
         declare -p u\n\
         t2=\n\
         declare t2[1/0]=x; echo \"rc=$?\"\n\
         declare -p t2\n",
    );
    // A scalar is a one-element array: -1 wraps to element 0, and a
    // still-negative subscript leaves the scalar promoted at [0]. An
    // empty array has no maximum and always fails; a declared-but-unset
    // array stays cell-less (`declare -a u`, no `=()`). The evalerror
    // DISCARD on the last line swallows that line's own `echo`, exactly
    // as GNU — the t2 rc line never prints, but the variable is left
    // promoted with [0]="".
    assert_eq!(
        stdout,
        "rc=0\n\
         declare -a s=([0]=\"x\")\n\
         rc=1\n\
         declare -a s2=([0]=\"hello\")\n\
         rc=0\n\
         declare -a g=([0]=\"x\")\n\
         rc=1\n\
         declare -a h=([0]=\"\")\n\
         rc=1\n\
         declare -a e=()\n\
         rc=1\n\
         declare -a u\n\
         declare -a t2=([0]=\"\")\n"
    );
    assert_eq!(
        stderr,
        "SCRIPT: line 6: s2[abc]: bad array subscript\n\
         SCRIPT: line 13: h[abc]: bad array subscript\n\
         SCRIPT: line 17: e[abc]: bad array subscript\n\
         SCRIPT: line 20: u[-1]: bad array subscript\n\
         SCRIPT: line 23: 1/0: division by 0 (error token is \"0\")\n"
    );
}

#[test]
fn function_scope_local_and_global_doors() {
    let dir = scratch("function");
    let (stdout, stderr) = run_script(
        &dir,
        "abc=-5\n\
         f() { local a=(1 2 3); local a[abc]=x; echo \"rc=$?\"; declare -p a; }\n\
         f\n\
         g() { local a=(1 2 3); abc=-2; local a[abc]=x; echo \"rc=$?\"; declare -p a; }\n\
         g\n\
         h() { local DIRSTACK[abd]=x; echo \"rc=$?\"; }\n\
         abd=-6\n\
         h\n\
         ga=(1 2 3 4 5 6)\n\
         k() { declare -g ga[abc]=x; echo \"rc=$?\"; }\n\
         k\n\
         declare -p ga\n",
    );
    assert_eq!(
        stdout,
        "rc=1\n\
         declare -a a=([0]=\"1\" [1]=\"2\" [2]=\"3\")\n\
         rc=0\n\
         declare -a a=([0]=\"1\" [1]=\"x\" [2]=\"3\")\n\
         rc=1\n\
         rc=0\n\
         declare -a ga=([0]=\"1\" [1]=\"2\" [2]=\"3\" [3]=\"4\" [4]=\"x\" [5]=\"6\")\n"
    );
    assert_eq!(
        stderr,
        "SCRIPT: line 2: a[abc]: bad array subscript\n\
         SCRIPT: line 6: DIRSTACK[abd]: bad array subscript\n"
    );
}
