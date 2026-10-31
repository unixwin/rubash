//! rubash#2332 — $LINENO intra-body fidelity in respawned inline
//! TEXT bodies (the #2202 residual). GNU's fork-shaped children
//! execute the parent's parsed COMMAND tree, whose nodes carry the
//! line make_cmd.c stamped at parse (make_cmd.c:196
//! `temp->line = lineno`); execute_cmd.c re-stamps line_number from
//! the executing node (execute_cmd.c:936 SET_LINE_NUMBER) and
//! $LINENO reads it back (variables.c:1861 INIT_DYNAMIC_VAR
//! get_lineno, variables.c:1462). A respawned `rubash -c` child
//! instead re-parses the re-rendered body text
//! (bash_command_sequence_text collapses the line structure), so
//! before this change every mid-body command reported the #2058
//! seed line. The spawn now stamps the root body's per-node source
//! spans (__RUBASH_FORK_ROOT_SPANS, sibling to the #2202
//! __RUBASH_FORK_FN_SPANS) at the async and coproc sites, and the
//! child's `-c` driver re-seeds the re-parsed body's node lines
//! from them (docs/design/respawn-root-line-spans.md).
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-10-31); the #2202 function-body pins are the sibling
//! controls in issue2202_lineno_span_transport_regressions.rs.
//!
//! Unix-only for the same reasons as the #2058/#2202 pins.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2332-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Same harness shape as the #2202 pins: the script gets two prelude
/// lines binding $OUT and a trailing `cat "$OUT"`; transported bodies
/// write their readings to $OUT (or stdout, synchronized by `wait`).
fn run_script(case: &Case, script: &str) -> (String, String) {
    let path = case.dir.join("case.sh");
    let full = format!(
        "OUT=\"{}\"\n: > \"$OUT\"\n{}\ncat \"$OUT\"\n",
        case.dir.join("out.txt").display(),
        script
    );
    std::fs::write(&path, full).expect("write fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&case.dir)
        .arg("case.sh")
        .arg("A")
        .arg("B")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// 1-based line number of the first line containing `needle`, in the
/// full script layout run_script produces (two prelude lines + body).
fn line_of(body: &str, needle: &str) -> usize {
    format!("OUT=\n: >\n{body}")
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
        .unwrap_or_else(|| panic!("needle {needle:?} not in script"))
}

/// Async `( { ...; } > file ) &` inside a function: every statement
/// of the transported brace group reports its own line (GNU 3/4/6 in
/// the bare probe layout; rubash reported the seed for all three).
#[test]
fn issue2332_asyncsub_group_in_function() {
    let case = make_case("asyncsub-group");
    let script = r#"f() {
  ( {
    echo "a1=$LINENO" >> "$OUT"
    echo "a2=$LINENO" >> "$OUT"

    echo "a3=$LINENO" >> "$OUT"
  } ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let a1 = line_of(script, "a1=");
    let a2 = line_of(script, "a2=");
    let a3 = line_of(script, "a3=");
    assert_eq!(stdout, format!("a1={a1}\na2={a2}\na3={a3}\n"));
}

/// Top-level `( ... ) &`: the whole subshell node is the transported
/// body (non-absorbed render), same per-line expectation.
#[test]
fn issue2332_asyncsub_toplevel() {
    let case = make_case("asyncsub-top");
    let script = r#"(
echo "t1=$LINENO" >> "$OUT"
echo "t2=$LINENO" >> "$OUT"
) &
wait"#;
    let (stdout, _) = run_script(&case, script);
    let t1 = line_of(script, "t1=");
    let t2 = line_of(script, "t2=");
    assert_eq!(stdout, format!("t1={t1}\nt2={t2}\n"));
}

/// Async `{ ...; } &` brace-group body (no parens to absorb).
#[test]
fn issue2332_async_brace_group() {
    let case = make_case("async-brace");
    let script = r#"{
echo "b1=$LINENO" >> "$OUT"
echo "b2=$LINENO" >> "$OUT"
} &
wait"#;
    let (stdout, _) = run_script(&case, script);
    let b1 = line_of(script, "b1=");
    let b2 = line_of(script, "b2=");
    assert_eq!(stdout, format!("b1={b1}\nb2={b2}\n"));
}

/// Coproc `{ ...; }` compound body: the child's main source is the
/// body list itself (no prelude).
#[test]
fn issue2332_coproc_group_body() {
    let case = make_case("coproc-group");
    let script = r#"coproc CP {
{
echo "c1=$LINENO"
echo "c2=$LINENO"

echo "c3=$LINENO"
} > "$OUT"
}
wait "$CP_PID""#;
    let (stdout, _) = run_script(&case, script);
    let c1 = line_of(script, "c1=");
    let c2 = line_of(script, "c2=");
    let c3 = line_of(script, "c3=");
    assert_eq!(stdout, format!("c1={c1}\nc2={c2}\nc3={c3}\n"));
}

/// Coproc `( ... )` subshell body: same body-list transport.
#[test]
fn issue2332_coproc_subshell_body() {
    let case = make_case("coproc-subshell");
    let script = r#"coproc CP (
{
echo "s1=$LINENO"
echo "s2=$LINENO"
} > "$OUT"
)
wait "$CP_PID""#;
    let (stdout, _) = run_script(&case, script);
    let s1 = line_of(script, "s1=");
    let s2 = line_of(script, "s2=");
    assert_eq!(stdout, format!("s1={s1}\ns2={s2}\n"));
}

/// A heredoc inside the body forces a multi-line render; the
/// statements around it still report their own lines.
#[test]
fn issue2332_asyncsub_heredoc_body() {
    let case = make_case("asyncsub-heredoc");
    let script = r#"f() {
  (
    echo "pre=$LINENO" >> "$OUT"
    cat <<'XEOF' >> "$OUT"
literal
XEOF
    echo "post=$LINENO" >> "$OUT"
  ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let pre = line_of(script, "pre=");
    let post = line_of(script, "post=");
    assert_eq!(stdout, format!("pre={pre}\nliteral\npost={post}\n"));
}

/// A failing command mid-body: the `script: line N:` diagnostic and
/// the following statements both use the body's real lines.
#[test]
fn issue2332_diagnostic_line_inside_body() {
    let case = make_case("diag");
    let script = r#"f() {
  (
    echo "before=$LINENO"
    nosuchcmd2332
    echo "after=$LINENO"
  ) &
  wait
}
f"#;
    let (stdout, stderr) = run_script(&case, script);
    let diag = line_of(script, "nosuchcmd2332");
    let before = line_of(script, "before=");
    let after = line_of(script, "after=");
    assert_eq!(stdout, format!("before={before}\nafter={after}\n"));
    assert!(
        stderr.contains(&format!("line {diag}:")),
        "diagnostic should name line {diag}, got: {stderr}"
    );
}

/// A transported function called from inside the text body sees the
/// call's real line in BASH_LINENO (its own body lines are the #2202
/// control and stay definition-relative).
#[test]
fn issue2332_bash_lineno_call_line_from_text_body() {
    let case = make_case("bash-lineno");
    let script = r#"g() {
  echo "call_line=${BASH_LINENO[0]} in_g=$LINENO"
}
f() {
  (
    echo "pre=$LINENO"
    g # callsite
    echo "post=$LINENO"
  ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let pre = line_of(script, "pre=");
    let call = line_of(script, "callsite");
    let in_g = line_of(script, "call_line=");
    let post = line_of(script, "post=");
    assert_eq!(
        stdout,
        format!("pre={pre}\ncall_line={call} in_g={in_g}\npost={post}\n")
    );
}

/// Nested text bodies: the inner respawn stamps from the outer
/// child's re-seeded tree, so lines stay in the original source's
/// line space at depth 2.
#[test]
fn issue2332_nested_text_bodies() {
    let case = make_case("nested");
    let script = r#"f() {
  (
    echo "o1=$LINENO"
    (
      echo "i1=$LINENO"
      echo "i2=$LINENO"
    ) &
    wait
    echo "o2=$LINENO"
  ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let o1 = line_of(script, "o1=");
    let i1 = line_of(script, "i1=");
    let i2 = line_of(script, "i2=");
    let o2 = line_of(script, "o2=");
    assert_eq!(stdout, format!("o1={o1}\ni1={i1}\ni2={i2}\no2={o2}\n"));
}

/// A body that defines and uses an alias takes the child's grouped
/// `-c` driver (alias text present); GNU parses the whole group
/// before executing it, so the alias use is a plain command-not-found
/// at its own line, and the following $LINENO is the body's line.
#[test]
fn issue2332_grouped_driver_alias_body() {
    let case = make_case("alias-body");
    let script = r#"f() {
  ( {
    alias hi='echo aliased'
    hi # use-site
    echo "n=$LINENO" >> "$OUT"
  } ) &
  wait
}
f"#;
    let (stdout, stderr) = run_script(&case, script);
    let hi = line_of(script, "use-site");
    let n = line_of(script, "n=");
    assert_eq!(stdout, format!("n={n}\n"));
    assert!(
        stderr.contains(&format!("line {hi}:")),
        "alias-use diagnostic should name line {hi}, got: {stderr}"
    );
}

/// With `expand_aliases` live in the parent, the seed line itself
/// used to drift (the prelude-newline accounting in the #2058 seed);
/// the span re-seed is absolute and restores every body line
/// regardless of the seed.
#[test]
fn issue2332_expand_aliases_parent_seed_drift() {
    let case = make_case("expand-aliases");
    let script = r#"shopt -s expand_aliases
f() {
  ( {
    echo "a1=$LINENO" >> "$OUT"
    echo "a2=$LINENO" >> "$OUT"

    echo "a3=$LINENO" >> "$OUT"
  } ) &
  wait
}
f"#;
    let (stdout, _) = run_script(&case, script);
    let a1 = line_of(script, "a1=");
    let a2 = line_of(script, "a2=");
    let a3 = line_of(script, "a3=");
    assert_eq!(stdout, format!("a1={a1}\na2={a2}\na3={a3}\n"));
}

/// Forgery control: a user-exported __RUBASH_FORK_ROOT_SPANS on an
/// ordinary (non-respawn) invocation is provenance-gated and
/// stripped — it must neither re-seed lines nor surface in `env`.
#[test]
fn issue2332_forged_root_spans_ignored() {
    let case = make_case("forged");
    let script = r#"__RUBASH_FORK_ROOT_SPANS='( [0]="0" [1]="99.99.99" )' "$THIS_SH" -c 'echo "inner=$LINENO"; env' > "$OUT" 2>&1
echo "done=$LINENO" >> "$OUT""#;
    let (stdout, _) = run_script(&case, script);
    assert!(
        stdout.contains("inner=1\n"),
        "forged spans must not re-seed an ordinary -c child: {stdout}"
    );
    assert!(
        !stdout.contains("__RUBASH_FORK_ROOT_SPANS"),
        "the carrier must be stripped at child birth: {stdout}"
    );
    let done = line_of(script, "done=");
    assert!(stdout.contains(&format!("done={done}")), "got: {stdout}");
}
