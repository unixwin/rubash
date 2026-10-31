//! rubash#2202 — LINENO intra-body fidelity in respawned function
//! bodies. GNU's fork-shaped children execute the parent's parsed
//! COMMAND tree, whose nodes carry the line make_cmd.c stamped at
//! parse (make_cmd.c:196 `temp->line = lineno`), and execute_cmd.c
//! re-stamps line_number from the executing node's own line
//! (execute_cmd.c:936 for simple commands; :5351 seeds a function's
//! entry from its body command's line) — so a body command in an
//! async/coproc/procsub child reports its DEFINITION line, and
//! $LINENO (variables.c:1861 INIT_DYNAMIC_VAR get_lineno) reads it.
//! A respawned `rubash -c` child instead re-parses the prelude's
//! re-rendered bodies (bash_command_sequence_text collapses the line
//! structure), so the spawn now stamps each transported function's
//! per-node source span (__RUBASH_FORK_FN_SPANS) and the child's
//! definition path re-seeds the re-parsed body's node lines from it.
//! Expectations measured against GNU bash 5.3.20 with script files
//! (2026-10-31). The #2058 start-point/call-site pins are the
//! controls; they live in issue2058_respawn_transport_regressions.rs.
//!
//! Unix-only for the same reasons as the #2058 pins.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

struct Case {
    dir: PathBuf,
}

fn make_case(tag: &str) -> Case {
    let dir = std::env::temp_dir().join(format!("rubash2202-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    Case { dir }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Same harness shape as the #2058 pins: the script gets two prelude
/// lines binding $OUT and a trailing `cat "$OUT"`; transported bodies
/// append their readings to $OUT.
fn run_script(case: &Case, script: &str) -> String {
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
    String::from_utf8_lossy(&output.stdout).into_owned()
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

/// A multi-line body with a blank line and a comment line: every body
/// command of an async child reports its own definition line, not the
/// call-site seed #2058 had to settle for.
#[test]
fn issue2202_asyncfn_child_reports_each_body_line() {
    let case = make_case("asyncfn");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"
  echo \"b=$LINENO\" >> \"$OUT\"

  # a comment line inside the body
  echo \"c=$LINENO\" >> \"$OUT\"
}
f &
wait
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let b = line_of(body, "b=$LINENO");
    let c = line_of(body, "c=$LINENO");
    assert_eq!(out, format!("a={a}\nb={b}\nc={c}\n"));
}

/// The coproc child is a respawn too: a function called inside the
/// coproc body reports its definition lines.
#[test]
fn issue2202_coproc_fn_child_reports_each_body_line() {
    let case = make_case("coproc");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"
  echo \"b=$LINENO\" >> \"$OUT\"
}
coproc C { f; }
sleep 0.5
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let b = line_of(body, "b=$LINENO");
    assert_eq!(out, format!("a={a}\nb={b}\n"));
}

/// Process substitution (input form): the producer child calling a
/// transported function reports definition lines.
#[test]
fn issue2202_procsub_fn_child_reports_each_body_line() {
    let case = make_case("procsub");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"
  echo \"b=$LINENO\" >> \"$OUT\"
}
cat < <(f) > /dev/null
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let b = line_of(body, "b=$LINENO");
    assert_eq!(out, format!("a={a}\nb={b}\n"));
}

/// Compound bodies inside the transported function: if/for bodies
/// keep their own lines through the span transport (the traversal is
/// structural, not line-count based).
#[test]
fn issue2202_asyncfn_child_compound_body_lines() {
    let case = make_case("compounds");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"
  if true; then
    echo \"b=$LINENO\" >> \"$OUT\"
  fi
  for i in 1; do
    echo \"c=$LINENO\" >> \"$OUT\"
  done
  echo \"d=$LINENO\" >> \"$OUT\"
}
f &
wait
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let b = line_of(body, "b=$LINENO");
    let c = line_of(body, "c=$LINENO");
    let d = line_of(body, "d=$LINENO");
    assert_eq!(out, format!("a={a}\nb={b}\nc={c}\nd={d}\n"));
}

/// A function defined INSIDE a transported body: its commands ride
/// the outer body's span (they are nodes of that body), so calling it
/// in the respawn child reports the inner definition lines.
#[test]
fn issue2202_asyncfn_child_nested_function_lines() {
    let case = make_case("nested");
    let body = "\
f() {
  g() {
    echo \"g=$LINENO\" >> \"$OUT\"
  }
  g
  echo \"f=$LINENO\" >> \"$OUT\"
}
f &
wait
";
    let out = run_script(&case, body);
    let g = line_of(body, "g=$LINENO");
    let f = line_of(body, "f=$LINENO");
    assert_eq!(out, format!("g={g}\nf={f}\n"));
}

/// Two commands sharing one source line both report that line;
/// subshell and brace-group bodies inside the function keep theirs.
#[test]
fn issue2202_asyncfn_child_same_line_and_group_lines() {
    let case = make_case("oneline");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"; echo \"b=$LINENO\" >> \"$OUT\"
  ( echo \"sub=$LINENO\" >> \"$OUT\" )
  { echo \"grp=$LINENO\" >> \"$OUT\"; }
}
f &
wait
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let c = line_of(body, "sub=$LINENO");
    let d = line_of(body, "grp=$LINENO");
    assert_eq!(out, format!("a={a}\nb={a}\nsub={c}\ngrp={d}\n"));
}

/// Nested respawn: the async child runs the transported f, which
/// itself spawns g in the background — the grandchild's stamp is
/// taken from the child's (re-seeded) function table, so g's body
/// lines survive two respawns.
#[test]
fn issue2202_nested_respawn_keeps_function_body_lines() {
    let case = make_case("grandchild");
    let body = "\
g() {
  echo \"g1=$LINENO\" >> \"$OUT\"
  echo \"g2=$LINENO\" >> \"$OUT\"
}
f() {
  g &
  wait
}
f &
wait
";
    let out = run_script(&case, body);
    let g1 = line_of(body, "g1=$LINENO");
    let g2 = line_of(body, "g2=$LINENO");
    assert_eq!(out, format!("g1={g1}\ng2={g2}\n"));
}

/// Control: the plain in-process call was never broken and must not
/// move — same body, no respawn anywhere.
#[test]
fn issue2202_direct_call_body_lines_unchanged() {
    let case = make_case("direct");
    let body = "\
f() {
  echo \"a=$LINENO\" >> \"$OUT\"

  echo \"b=$LINENO\" >> \"$OUT\"
}
f
";
    let out = run_script(&case, body);
    let a = line_of(body, "a=$LINENO");
    let b = line_of(body, "b=$LINENO");
    assert_eq!(out, format!("a={a}\nb={b}\n"));
}

/// Control (#2058): the start-point seed for a text body is exact —
/// the first statement of a multi-line async subshell body reports
/// the body's first line, not its closing line.
#[test]
fn issue2202_asyncsub_first_statement_line_control() {
    let case = make_case("asyncsub-start");
    let body = "\
(
  echo \"first=$LINENO\" >> \"$OUT\"
) &
wait
";
    let out = run_script(&case, body);
    let first = line_of(body, "first=$LINENO");
    assert_eq!(out, format!("first={first}\n"));
}
