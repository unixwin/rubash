//! rubash#2655 — assignment through a nameref to an array element must
//! land on the target element exactly once, in the shell's single
//! variable store. On the 1.5.0 harvest tree the write updated only the
//! mirror string, leaving the typed cell stale, and the next
//! command-boundary coherence check panicked:
//! "typed indexed cell `a` disagrees with its mirror" (variable_state.rs
//! assertion; assoc twin for associative targets).
//!
//! GNU semantics (bash 5.3 sources):
//! - general.c:310 valid_nameref_value: a nameref cell may be a valid
//!   identifier OR (flags != 2) a valid array reference — `a[0]` is a
//!   legitimate assignment nameref cell.
//! - variables.c:3225 bind_variable: a nameref whose last cell is a
//!   valid array reference dispatches straight to
//!   assign_array_element(cell_text, value) (arrayfunc.c:334), which
//!   writes the one store via bind_array_element (arrayfunc.c:293).
//! - The self-name shape (local nameref named like its cell's base,
//!   `local -n a=$1; a=X` with $1='a[0]') instead fails: bind_array_
//!   variable (arrayfunc.c:262) finds the local nameref itself and
//!   find_variable_nameref_for_create (variables.c:2182) accepts bare
//!   identifiers only, so GNU reports `a[0]': not a valid identifier
//!   and writes NOTHING (plus the two circular warnings).
//!
//! Expectations measured byte-for-byte against GNU bash 5.3.20 with
//! the same -c harness shape (2026-11-09). Diagnostic lines carry a
//! program-name prefix that differs between shells, so cells with
//! diagnostics assert stdout/rc byte-exact plus required/forbidden
//! stderr substrings with occurrence counts.

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_c(script: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// Clean cell: exact stdout, empty stderr, exit 0.
fn assert_clean(script: &str, expected_stdout: &str) {
    let run = run_c(script);
    assert_eq!(run.stdout, expected_stdout, "script: {script}");
    assert_eq!(run.stderr, "", "script: {script}");
    assert_eq!(run.code, Some(0), "script: {script}");
}

/// Diagnostic cell: exact stdout and exit code; stderr must contain
/// each required substring exactly `count` times and none of the
/// forbidden substrings (the panic signature is forbidden everywhere).
fn assert_diag(
    script: &str,
    expected_stdout: &str,
    expected_code: i32,
    required: &[(&str, usize)],
    forbidden: &[&str],
) {
    let run = run_c(script);
    assert_eq!(run.stdout, expected_stdout, "script: {script}");
    assert_eq!(run.code, Some(expected_code), "script: {script}");
    for (needle, count) in required {
        assert_eq!(
            run.stderr.matches(needle).count(),
            *count,
            "stderr substring {needle:?} count, script: {script}, stderr: {:?}",
            run.stderr
        );
    }
    for needle in forbidden {
        assert!(
            !run.stderr.contains(needle),
            "forbidden stderr substring {needle:?}, script: {script}, stderr: {:?}",
            run.stderr
        );
    }
}

const NO_PANIC: &[&str] = &["panicked", "disagrees with its mirror"];

/// The issue's headline shape (nameref15.sub): a local nameref named
/// `a` whose cell is `a[0]`. GNU writes nothing: two circular warnings
/// plus the invalid-identifier refusal, the array keeps its old value,
/// and the script continues. The harvest build instead wrote the value
/// into the mirror only and panicked at the next coherence check.
#[test]
fn issue2655_self_named_local_nameref_element_assignment_is_refused() {
    assert_diag(
        r#"f() { local -n a=$1; a=X; }
a=(0); f 'a[0]'
declare -p a
echo "status=$?"
"#,
        "declare -a a=([0]=\"0\")\nstatus=0\n",
        0,
        &[
            ("circular name reference", 2),
            ("`a[0]': not a valid identifier", 1),
        ],
        &[
            "panicked",
            "disagrees with its mirror",
            "maximum nameref depth",
        ],
    );
}

/// nameref15.sub opening loops: the refused `a[0]` shape inside a
/// `while [[ -v a ]]` unset loop, then the whole-name `a` shape, where
/// GNU's max-depth fallback binds the GLOBAL namesake and a[0] does
/// become X (with a maximum-depth warning instead of the refusal).
#[test]
fn issue2655_nameref15_opening_loops() {
    assert_diag(
        r#"f() { local -n a=$1; a=X; }
a=(0); f 'a[0]'
while [[ -v a ]]; do declare -p a; unset a; done
echo "done"
a=(0); f 'a'
while [[ -v a ]]; do declare -p a; unset a; done
echo "done2"
"#,
        "declare -a a=([0]=\"0\")\ndone\ndeclare -a a=([0]=\"X\")\ndone2\n",
        0,
        &[
            ("circular name reference", 4),
            ("`a[0]': not a valid identifier", 1),
            ("maximum nameref depth (8) exceeded", 1),
        ],
        NO_PANIC,
    );
}

/// Cross-name element target (`f 'b[0]'` through a nameref named `a`)
/// resolves to the global array and writes cleanly; pointing the same
/// nameref at `a[0]` while only `b` exists refuses and creates nothing.
#[test]
fn issue2655_cross_name_element_write_and_missing_target() {
    assert_diag(
        r#"f() { local -n a=$1; a=X; }
b=(0); f 'b[0]'
typeset -p b
b=(0); f 'a[0]'
typeset -p b
typeset -p a
"#,
        "declare -a b=([0]=\"X\")\ndeclare -a b=([0]=\"0\")\n",
        1,
        &[
            ("`a[0]': not a valid identifier", 1),
            ("typeset: a: not found", 1),
        ],
        NO_PANIC,
    );
}

/// nameref23.sub: integer array element through a global nameref —
/// plain assignment, `b+=1`, `declare b+=1`, `declare b=42` all do
/// arithmetic on the element (arrayfunc.c bind_array_element with the
/// target's att_integer).
#[test]
fn issue2655_integer_element_append_through_nameref() {
    assert_clean(
        r#"declare -ai a
a[0]=4
declare -n b='a[0]'
b+=1
declare -p a
declare b+=1
declare -p a
declare b=42
declare -p a
"#,
        "declare -ai a=([0]=\"5\")\ndeclare -ai a=([0]=\"6\")\ndeclare -ai a=([0]=\"42\")\n",
    );
}

/// nameref23.sub global string append through an element nameref.
#[test]
fn issue2655_string_element_append_through_nameref() {
    assert_clean(
        r#"declare -a a=('' 'foo')
declare -n b=a[1]
echo "[$b]"
b+=\ bar
echo "[$b]"
declare -p a b
"#,
        "[foo]\n[foo bar]\ndeclare -a a=([0]=\"\" [1]=\"foo bar\")\ndeclare -n b=\"a[1]\"\n",
    );
}

/// nameref23.sub function-local shapes: local array + local element
/// nameref, string append and integer append forms.
#[test]
fn issue2655_local_element_nameref_appends() {
    assert_clean(
        r#"f()
{
	local -a a=('' 'foo')
	local -n b=a[1]
	echo "[$b]"
	b+=\ bar
	echo "[$b]"
	declare -p a b
}
f
g()
{
	local -ai a=(0 12)
	local -n b=a[1]
	echo "[$b]"
	b+=4
	echo "[$b]"
	declare -p a
}
g
"#,
        "[foo]\n[foo bar]\ndeclare -a a=([0]=\"\" [1]=\"foo bar\")\ndeclare -n b=\"a[1]\"\n[12]\n[16]\ndeclare -ai a=([0]=\"0\" [1]=\"16\")\n",
    );
}

/// Associative element through a nameref: write and append land on
/// the referenced key (the :1111 assoc coherence assertion is the
/// harvest panic twin for this shape).
#[test]
fn issue2655_assoc_element_write_through_nameref() {
    assert_clean(
        r#"declare -A A
A[k]=v
declare -n r='A[k]'
r=X
declare -p A
r+=Y
declare -p A
echo "r=$r"
"#,
        "declare -A A=([k]=\"X\" )\ndeclare -A A=([k]=\"XY\" )\nr=XY\n",
    );
}

/// Linked chain (nameref23 + nameref15): every step's state feeds the
/// next — append through the element nameref, read back directly,
/// append again, then write and unset an element through a
/// whole-array nameref, asserting the referenced array each time.
#[test]
fn issue2655_chained_element_writes_and_unset_through_namerefs() {
    assert_clean(
        r#"declare -ai a=(1)
declare -n b='a[0]'
b+=2
echo "direct=${a[0]}"
b+=3
declare -p a
echo "via-ref=$b"
v=(0 1)
typeset -n n=v
n[0]=X
echo "v0=${v[0]}"
typeset -p v
unset n[0]
typeset -p n v
"#,
        "direct=3\ndeclare -ai a=([0]=\"6\")\nvia-ref=6\nv0=X\ndeclare -a v=([0]=\"X\" [1]=\"1\")\ndeclare -n n=\"v\"\ndeclare -a v=([1]=\"1\")\n",
    );
}

/// nameref22.sub: `declare foo=...` / `declare foo+=...` through an
/// element nameref write the referenced element, not the nameref.
#[test]
fn issue2655_declare_assignments_through_element_nameref() {
    assert_clean(
        r#"declare -a bar
declare -n foo='bar[0]'
declare foo=spaces
declare -p foo bar
declare foo+=' still more spaces'
declare -p foo bar
"#,
        "declare -n foo=\"bar[0]\"\ndeclare -a bar=([0]=\"spaces\")\ndeclare -n foo=\"bar[0]\"\ndeclare -a bar=([0]=\"spaces still more spaces\")\n",
    );
}

/// nameref21.sub: a local nameref re-declared with `-A` and a compound
/// value converts the TARGET to an associative array; the nameref
/// itself survives. Two rounds: target unset, then target a scalar.
#[test]
fn issue2655_local_nameref_retargeted_to_assoc_compound() {
    assert_clean(
        r#"f()
{
	local -n ref=var
	local -A ref=([1]=)
	ref=([2]=)
	declare -p ref var
}
unset ref var
f
unset ref; var=0
f
"#,
        "declare -n ref=\"var\"\ndeclare -A var=([2]=\"\" )\ndeclare -n ref=\"var\"\ndeclare -A var=([2]=\"\" )\n",
    );
}

/// Guard (nameref18.sub): a COMPOUND append and `read -a` through an
/// element nameref are refused by GNU (find_variable_nameref_
/// for_create wants a bare identifier for array creation) — the
/// nameref is kept and nothing is written.
#[test]
fn issue2655_compound_and_read_through_element_nameref_are_refused() {
    assert_diag(
        r#"declare -n ref=XXX[0]
ref+=([2]=x)
declare -p ref XXX[0]
unset -n ref
declare -n ref=XXX[0]
read -a ref <<< "A B C"
declare -p XXX[0]
unset -n ref
"#,
        "declare -n ref=\"XXX[0]\"\n",
        0,
        &[
            ("`XXX[0]': not a valid identifier", 2),
            ("XXX[0]: not found", 2),
        ],
        NO_PANIC,
    );
}

/// Guard (nameref18.sub): `declare -A` through an element nameref and
/// `mapfile` through an element nameref are likewise refused.
#[test]
fn issue2655_declare_assoc_and_mapfile_through_element_nameref_are_refused() {
    assert_diag(
        r#"declare -n ref=XXX[0]
declare -A ref
ref[foo]=bar
declare -p XXX[0]
unset -n ref
declare -n ref=XXX[0]
mapfile ref <<< $'1\n2'
declare -p XXX[0]
"#,
        "",
        1,
        &[
            ("`XXX[0]': not a valid identifier", 2),
            ("XXX[0]: not found", 2),
        ],
        NO_PANIC,
    );
}

/// nameref15.sub scalar chain (regression-lock candidate): appending
/// through a plain scalar nameref whose name equals its target writes
/// the target, with GNU's own circular/max-depth warnings.
#[test]
fn issue2655_scalar_nameref_append_still_writes_target() {
    assert_diag(
        r#"add_X_echo()
{
	typeset -n ref=$1
	ref+=X
	echo "inside $ref"
}
ref=
add_X_echo ref
echo "outside [$ref]"
"#,
        "inside X\noutside [X]\n",
        0,
        &[
            ("circular name reference", 3),
            ("maximum nameref depth (8) exceeded", 1),
        ],
        NO_PANIC,
    );
}
