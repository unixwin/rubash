//! rubash#2424 — the declare/local and tempenv doors for DIRSTACK
//! assignment must go through the same stack setter as plain
//! assignment (rubash#2408), and plain bindings that shadow the
//! dynamic variable must own the name outright.
//!
//! GNU keeps ONE write path: every bind of the global dynamic
//! variable lands in variables.c:1612 assign_dirstack ->
//! builtins/pushd.def:657 set_dirstack_element (element 0 ignored,
//! in-bounds replaced, out-of-range dropped, never cleared or
//! grown). That covers plain assignment, declare/typeset operands,
//! and the setattr family (export/readonly share declare.def's
//! operand handling) alike. Three other GNU shapes matter here:
//!
//! - `local DIRSTACK=...` creates a brand-new PLAIN variable in the
//!   function frame (make_local_variable via make_new_variable — no
//!   get/assign funcs, no value copy): binds in the frame never
//!   touch the directory stack, and the dynamic face returns when
//!   the frame pops. `declare -g` in a function binds the global
//!   variable instead, through the setter.
//! - A tempenv word (`DIRSTACK=(...) cmd`) is variables.c:3536
//!   assign_in_env: a fresh SCALAR tempvar holding the literal list
//!   text, shadowing every read for the command's duration; the
//!   stack is untouched during and after. A subscript tempenv word
//!   is not a valid identifier and is skipped with a diagnostic.
//! - A readonly DIRSTACK refuses every bind before the assign
//!   function runs.
//!
//! Before this fix the declare door's own compound walker wrote the
//! list into the env mirror as a private typed cell: `dirs` kept
//! showing the real stack while ${DIRSTACK[1]} read the phantom,
//! `declare -p DIRSTACK` panicked on the cell/mirror disagreement,
//! scalar/subscript declare binds wrote the read faces, the tempenv
//! splice poisoned the binding (post-window count read 0), and a
//! bare `$DIRSTACK` read leaked the parked storage repr ("\u{1d}()")
//! whenever the mirror had never been materialized.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle
//! with script-file probes; the scratch path is normalized to WORK.
//! Every script builds the same stack: WORK /tmp / (top first).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2424-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `script` as a script FILE whose $1 is the scratch dir; return
/// (stdout, stderr, success) with the scratch path normalized to WORK.
fn run_script(dir: &Path, script: &str) -> (String, String, bool) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .arg(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    let normalize = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes).replace(&dir.to_string_lossy().into_owned(), "WORK")
    };
    (
        normalize(&output.stdout),
        normalize(&output.stderr),
        output.status.success(),
    )
}

fn run_script_ok(dir: &Path, script: &str) -> String {
    let (stdout, stderr, success) = run_script(dir, script);
    assert!(success, "probe failed: {stderr}");
    assert!(!stderr.contains("panicked"), "probe panicked: {stderr}");
    stdout
}

/// Script prelude: a three-element stack WORK /tmp / (top first).
const SETUP: &str = r#"cd / || exit 1
pushd /tmp >/dev/null || exit 1
pushd "$1" >/dev/null || exit 1
"#;

// ---------------------------------------------------------------------
// The declare door
// ---------------------------------------------------------------------

#[test]
fn issue2424_declare_compound_routes_through_setter() {
    let dir = scratch("decl-compound");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK=(X Y Z)
echo "dirs: $(dirs)"
echo "e1=${{DIRSTACK[1]}} e2=${{DIRSTACK[2]}} count=${{#DIRSTACK[@]}}"
declare -p DIRSTACK
"#
        ),
    );
    assert_eq!(
        out,
        "dirs: WORK Y Z\ne1=Y e2=Z count=3\ndeclare -a DIRSTACK=([0]=\"WORK\" [1]=\"Y\" [2]=\"Z\")\n"
    );
}

#[test]
fn issue2424_declare_compound_fewer_and_more_elements() {
    let dir = scratch("decl-compound-len");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK=(X Y)
echo "fewer: $(dirs) count=${{#DIRSTACK[@]}}"
declare -a DIRSTACK=(P Q R S)
echo "more: $(dirs) count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    // Fewer: only the in-bounds elements are replaced, nothing is
    // cleared. More: elements past the stack end drop silently.
    assert_eq!(out, "fewer: WORK Y / count=3\nmore: WORK Q R count=3\n");
}

#[test]
fn issue2424_declare_scalar_is_a_noop() {
    let dir = scratch("decl-scalar");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK=/scalar
echo "dirs: $(dirs) e0=${{DIRSTACK[0]}} count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    assert_eq!(out, "dirs: WORK /tmp / e0=WORK count=3\n");
}

#[test]
fn issue2424_declare_subscript_binds_live_elements() {
    let dir = scratch("decl-subscript");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK[1]=SUB
echo "sub: $(dirs)"
declare DIRSTACK[-1]=LAST
echo "neg: $(dirs)"
declare DIRSTACK[-3]=TOP
echo "neg0: $(dirs) rc=$?"
declare DIRSTACK[0]=ZERO
echo "zero: $(dirs) rc=$?"
"#
        ),
    );
    // Negative indices resolve against the live stack; an index
    // landing on element 0 (directly or via -len) is ignored.
    assert_eq!(
        out,
        "sub: WORK SUB /\nneg: WORK SUB LAST\nneg0: WORK SUB LAST rc=0\nzero: WORK SUB LAST rc=0\n"
    );
}

#[test]
fn issue2424_declare_subscript_before_stack_start_fails() {
    let dir = scratch("decl-subscript-oob");
    let (stdout, stderr, success) = run_script(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK[-4]=OOB 2>err.txt
echo "rc=$?"
echo "dirs: $(dirs)"
cat err.txt
"#
        ),
    );
    assert!(success, "script itself should complete: {stderr}");
    // The diagnostic is a builtin-phase error: the 2> redirect on
    // the declare captures it, so it comes back via `cat err.txt`.
    assert_eq!(
        stdout,
        "rc=1\ndirs: WORK /tmp /\nWORK/probe.sh: line 4: DIRSTACK[-4]: bad array subscript\n"
    );
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

#[test]
fn issue2424_declare_append_positional_drops_subscripted_lands() {
    let dir = scratch("decl-append");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare DIRSTACK+=(zz)
echo "append: $(dirs) count=${{#DIRSTACK[@]}}"
declare DIRSTACK+=([1]=QQ)
echo "subappend: $(dirs) count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    assert_eq!(
        out,
        "append: WORK /tmp / count=3\nsubappend: WORK QQ / count=3\n"
    );
}

#[test]
fn issue2424_declare_mixed_operands_bind_in_order() {
    let dir = scratch("decl-mixed");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}FOO=old
declare DIRSTACK=(A B) FOO=new DIRSTACK[2]=Z
echo "dirs=$(dirs) foo=$FOO e2=${{DIRSTACK[2]}}"
"#
        ),
    );
    assert_eq!(out, "dirs=WORK B Z foo=new e2=Z\n");
}

#[test]
fn issue2424_declare_attributes_transform_values() {
    let dir = scratch("decl-attrs");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare -i DIRSTACK=(1+2 3+4)
echo "int: $(dirs)"
declare -p DIRSTACK
declare -l DIRSTACK=(XX YY)
echo "lower: $(dirs)"
declare -u DIRSTACK[1]=ab
echo "upper: $(dirs)"
declare -i DIRSTACK[1]=3+4
echo "intsub: $(dirs)"
"#
        ),
    );
    // -i evaluates each element before the setter maps it (the first
    // list element lands on the ignored index 0). The integer
    // attribute then STANDS on the variable and preempts the case
    // attributes in later binds (make_variable_value: the integer
    // branch preempts the case branch), so the -l/-u binds below
    // still evaluate arithmetically.
    assert_eq!(
        out,
        "int: WORK 7 /\ndeclare -ai DIRSTACK=([0]=\"WORK\" [1]=\"7\" [2]=\"/\")\nlower: WORK 0 /\nupper: WORK 0 /\nintsub: WORK 7 /\n"
    );
}

#[test]
fn issue2424_declare_case_attributes_transform_values() {
    let dir = scratch("decl-case-attrs");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare -l DIRSTACK=(XX YY)
echo "lower: $(dirs)"
declare -u DIRSTACK[1]=ab
echo "upper: $(dirs)"
declare -p DIRSTACK
"#
        ),
    );
    assert_eq!(
        out,
        "lower: WORK yy /\nupper: WORK AB /\ndeclare -au DIRSTACK=([0]=\"WORK\" [1]=\"AB\" [2]=\"/\")\n"
    );
}

#[test]
fn issue2424_declare_print_after_stack_moves_never_panics() {
    let dir = scratch("decl-print-fresh");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}declare -p DIRSTACK >/dev/null
pushd / >/dev/null
echo "e0=${{DIRSTACK[0]}} e1=${{DIRSTACK[1]}} count=${{#DIRSTACK[@]}}"
declare -p DIRSTACK
popd >/dev/null
declare DIRSTACK+=(zz)
echo "after-append count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    // The materialized cell is a cache of the carrier: reads after a
    // stack move come from the live stack, and a fresh declare append
    // must not rebuild the cell from the stale init mirror.
    assert_eq!(
        out,
        "e0=/ e1=WORK count=4\ndeclare -a DIRSTACK=([0]=\"/\" [1]=\"WORK\" [2]=\"/tmp\" [3]=\"/\")\nafter-append count=3\n"
    );
}

#[test]
fn issue2424_declare_readonly_refusals() {
    let dir = scratch("decl-readonly");
    let (stdout, stderr, success) = run_script(
        &dir,
        &format!(
            r#"{SETUP}readonly DIRSTACK
declare DIRSTACK=/x
echo "scalar-rc=$?"
declare DIRSTACK[1]=ZZ 2>err.txt
echo "sub-rc=$?"
cat err.txt
declare DIRSTACK=(A B)
echo "compound-rc=$?"
echo "dirs: $(dirs)"
"#
        ),
    );
    assert!(success, "script itself should complete: {stderr}");
    // The subscript refusal is captured by the 2> redirect (the
    // `cat err.txt` line); the scalar and compound refusals go to
    // the script's stderr.
    assert_eq!(
        stdout,
        "scalar-rc=1\nsub-rc=1\nWORK/probe.sh: line 7: declare: DIRSTACK: readonly variable\ncompound-rc=1\ndirs: WORK /tmp /\n"
    );
    assert!(
        stderr.contains("DIRSTACK: readonly variable"),
        "expected readonly diagnostics, got: {stderr}"
    );
}

#[test]
fn issue2424_declare_g_in_function_binds_global() {
    let dir = scratch("decl-g");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}f() {{ declare -g DIRSTACK=(G1 G2 G3); }}
f
echo "dirs=$(dirs) e1=${{DIRSTACK[1]}}"
"#
        ),
    );
    assert_eq!(out, "dirs=WORK G2 G3 e1=G2\n");
}

#[test]
fn issue2424_export_and_readonly_doors() {
    let dir = scratch("setattr");
    let (stdout, stderr, success) = run_script(
        &dir,
        &format!(
            r#"{SETUP}export DIRSTACK=(A B)
echo "export: $(dirs)"
declare -p DIRSTACK
export DIRSTACK[1]=EX
echo "export-sub-rc=$? dirs: $(dirs)"
readonly DIRSTACK=(R1 R2)
echo "readonly: $(dirs)"
declare -p DIRSTACK
"#
        ),
    );
    assert!(success, "script itself should complete: {stderr}");
    // setattr shares declare's operand handling for whole-variable
    // binds, but a subscript lhs fails its identifier check.
    assert_eq!(
        stdout,
        "export: WORK B /\ndeclare -ax DIRSTACK=([0]=\"WORK\" [1]=\"B\" [2]=\"/\")\nexport-sub-rc=1 dirs: WORK B /\nreadonly: WORK R2 /\ndeclare -arx DIRSTACK=([0]=\"WORK\" [1]=\"R2\" [2]=\"/\")\n"
    );
    assert!(
        stderr.contains("`DIRSTACK[1]': not a valid identifier"),
        "expected invalid-identifier diagnostic, got: {stderr}"
    );
}

// ---------------------------------------------------------------------
// The local door: plain frame variables, never the stack
// ---------------------------------------------------------------------

#[test]
fn issue2424_local_compound_is_a_plain_frame_cell() {
    let dir = scratch("local-compound");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}f() {{
  local DIRSTACK=(X Y Z)
  echo "in: dirs=$(dirs) e1=${{DIRSTACK[1]}} e2=${{DIRSTACK[2]}} count=${{#DIRSTACK[@]}}"
  declare -p DIRSTACK
}}
f
echo "out: $(dirs) e1=${{DIRSTACK[1]}} count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    assert_eq!(
        out,
        "in: dirs=WORK /tmp / e1=Y e2=Z count=3\ndeclare -a DIRSTACK=([0]=\"X\" [1]=\"Y\" [2]=\"Z\")\nout: WORK /tmp / e1=/tmp count=3\n"
    );
}

#[test]
fn issue2424_local_fresh_frame_shapes() {
    let dir = scratch("local-fresh");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}f() {{
  local DIRSTACK=(X Y)
  echo "compound: count=${{#DIRSTACK[@]}} e0=${{DIRSTACK[0]}} e1=${{DIRSTACK[1]}} e2=[${{DIRSTACK[2]:-EMPTY}}]"
}}
g() {{
  local DIRSTACK=/scalar
  echo "scalar: count=${{#DIRSTACK[@]}} e0=${{DIRSTACK[0]}}"
  declare -- DIRSTACK
}}
h() {{
  local DIRSTACK[1]=SUB
  echo "subscript: count=${{#DIRSTACK[@]}} e1=${{DIRSTACK[1]}}"
}}
i() {{
  local DIRSTACK+=(zz)
  echo "append: count=${{#DIRSTACK[@]}} e0=${{DIRSTACK[0]}}"
}}
j() {{
  local DIRSTACK
  echo "bare: count=${{#DIRSTACK[@]}} set=${{DIRSTACK+set}}"
}}
f
g
h
i
j
echo "after: $(dirs) count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    // A local starts EMPTY (no copy of the stack): the fresh cell's
    // own shape is all any read in the frame can see.
    assert_eq!(
        out,
        "compound: count=2 e0=X e1=Y e2=[EMPTY]\nscalar: count=1 e0=/scalar\nsubscript: count=1 e1=SUB\nappend: count=1 e0=zz\nbare: count=0 set=\nafter: WORK /tmp / count=3\n"
    );
}

#[test]
fn issue2424_local_shadow_and_stack_are_decoupled() {
    let dir = scratch("local-decoupled");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}f() {{
  local DIRSTACK=(X Y)
  pushd / >/dev/null
  echo "in: dirs=$(dirs) e1=${{DIRSTACK[1]}}"
  popd >/dev/null
  DIRSTACK[1]=PLAIN
  DIRSTACK=(P Q R)
  echo "plain-assign: ${{DIRSTACK[*]}} dirs=$(dirs)"
}}
f
echo "out: ${{DIRSTACK[*]}}"
"#
        ),
    );
    // pushd inside the frame moves the REAL stack while the local
    // cell keeps reading back; plain binds in the frame write the
    // local cell, never the stack.
    assert_eq!(
        out,
        "in: dirs=/ WORK /tmp / e1=Y\nplain-assign: P Q R dirs=WORK /tmp /\nout: WORK /tmp /\n"
    );
}

// ---------------------------------------------------------------------
// The tempenv door: a scalar tempvar shadow, stack untouched
// ---------------------------------------------------------------------

const SHOW: &str = r#"show() {
  echo "in-cmd: dirs: $(dirs) | e0=${DIRSTACK[0]} e1=${DIRSTACK[1]:-EMPTY} count=${#DIRSTACK[@]} whole=$DIRSTACK"
}
"#;

#[test]
fn issue2424_tempenv_compound_shadows_as_scalar() {
    let dir = scratch("tempenv-compound");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}{SHOW}DIRSTACK=(AA BB) show
echo "after: $(dirs) | e1=${{DIRSTACK[1]}} count=${{#DIRSTACK[@]}}"
declare -p DIRSTACK
DIRSTACK=(AA BB) /usr/bin/env | grep '^DIRSTACK='
"#
        ),
    );
    assert_eq!(
        out,
        "in-cmd: dirs: WORK /tmp / | e0=(AA BB) e1=EMPTY count=1 whole=(AA BB)\nafter: WORK /tmp / | e1=/tmp count=3\ndeclare -a DIRSTACK=([0]=\"WORK\" [1]=\"/tmp\" [2]=\"/\")\nDIRSTACK=(AA BB)\n"
    );
}

#[test]
fn issue2424_tempenv_scalar_and_append_shadows() {
    let dir = scratch("tempenv-scalar");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}{SHOW}DIRSTACK=/shadow show
echo "after-scalar: e0=${{DIRSTACK[0]}} count=${{#DIRSTACK[@]}}"
DIRSTACK+=(zz) show
echo "after-append: $(dirs) count=${{#DIRSTACK[@]}}"
"#
        ),
    );
    // The append shadow is element 0 of the live stack plus the
    // literal text (make_variable_value ASS_APPEND on the tempvar).
    assert_eq!(
        out,
        "in-cmd: dirs: WORK /tmp / | e0=/shadow e1=EMPTY count=1 whole=/shadow\nafter-scalar: e0=WORK count=3\nin-cmd: dirs: WORK /tmp / | e0=WORK(zz) e1=EMPTY count=1 whole=WORK(zz)\nafter-append: WORK /tmp / count=3\n"
    );
}

#[test]
fn issue2424_tempenv_subscript_word_is_rejected() {
    let dir = scratch("tempenv-subscript");
    let (stdout, stderr, success) = run_script(
        &dir,
        &format!(
            r#"{SETUP}{SHOW}DIRSTACK[1]=SUB show
echo "after: $(dirs) e1=${{DIRSTACK[1]}}"
"#
        ),
    );
    assert!(success, "script itself should complete: {stderr}");
    // The word is not a valid identifier: the command still runs,
    // with the real stack — including the bare $DIRSTACK read, which
    // is element 0 of the stack, never the parked storage repr.
    assert_eq!(
        stdout,
        "in-cmd: dirs: WORK /tmp / | e0=WORK e1=/tmp count=3 whole=WORK\nafter: WORK /tmp / e1=/tmp\n"
    );
    assert!(
        stderr.contains("`DIRSTACK[1]': not a valid identifier"),
        "expected invalid-identifier diagnostic, got: {stderr}"
    );
}

#[test]
fn issue2424_bare_dirstack_read_is_stack_top() {
    let dir = scratch("bare-read");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}f() {{ echo "in-fn: whole=$DIRSTACK"; }}
f
echo "top: whole=$DIRSTACK"
"#
        ),
    );
    assert_eq!(out, "in-fn: whole=WORK\ntop: whole=WORK\n");
}

#[test]
fn issue2424_tempenv_under_local_shadow_nests() {
    let dir = scratch("tempenv-nest");
    let out = run_script_ok(
        &dir,
        &format!(
            r#"{SETUP}inner() {{ echo "inner: ${{DIRSTACK[*]}} whole=$DIRSTACK"; }}
f() {{
  local DIRSTACK=(L1 L2)
  DIRSTACK=(T1 T2) inner
  echo "in-f after: ${{DIRSTACK[*]}}"
}}
f
echo "out: ${{DIRSTACK[*]}}"
"#
        ),
    );
    assert_eq!(
        out,
        "inner: (T1 T2) whole=(T1 T2)\nin-f after: L1 L2\nout: WORK /tmp /\n"
    );
}
