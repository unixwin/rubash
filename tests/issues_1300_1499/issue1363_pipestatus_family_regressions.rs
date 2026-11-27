//! PIPESTATUS / dynamic call-stack variable family — regression pins for
//! rubash#1051, rubash#1110, rubash#1282, rubash#1363, rubash#1364,
//! rubash#1384, rubash#1387, rubash#1394 (and the #1398/#1400 assoc
//! case-attribute forms that share the verification matrix).
//!
//! These issues were closed on workspace-probe evidence (the z2 cluster
//! matrix and repro scripts under rubash-fix3) that never landed in the
//! repo, so a future drift would have been invisible to CI. Every
//! expectation below is the GNU bash 5.3.20 behaviour for the same form,
//! captured from the oracle binary: `-c` cases run under `env -i`
//! exactly like the original matrix harness, script-file cases run from
//! a file with arguments `a "b c" d` like the original run-diff harness.
//!
//! Shape notes (GNU source): PIPESTATUS is a dynamic array rewritten by
//! the pipeline executor after every pipeline (execute_cmd.c), and
//! BASH_ARGC/BASH_ARGV are the call-frame stack arrays (variables.c);
//! scalar expansion of an array yields element 0.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// `-c` form under a cleared environment (mirrors the matrix harness).
fn rubash_c(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env_clear()
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rubash-bfa-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    /// Script-file form: file in a fresh cwd, PATH/HOME pinned, extra args.
    fn rubash_file(&self, script: &str, args: &[&str]) -> (String, String, Option<i32>) {
        let file = self.0.join("case.sh");
        fs::write(&file, script).expect("write script");
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&file)
            .args(args)
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

// ---------------------------------------------------------------------
// The z2 verification matrix: 33 active forms (name, script, expected
// stdout, expected stderr substring). Expectations are GNU bash 5.3.20
// byte-for-byte on stdout; process exit status is 0 for every case.
// The six first-touch forms and the 40th form ("elem assign fresh",
// the R3 sparse case) are pinned separately below — the first-touch
// pins activated with PR #1526, the R3 sparse pins with the
// sparse-representation rework (fix5/r3).
// ---------------------------------------------------------------------

const MATRIX: &[(&str, &str, &str, &str)] = &[
    // --- PIPESTATUS basics / writer timing ---
    ("W1 false", "false; echo \"[$PIPESTATUS]\"", "[1]\n", ""),
    ("W3 assign", "x=5; echo \"[$PIPESTATUS]\"", "[0]\n", ""),
    ("W4 unset then read", "unset PIPESTATUS; echo \"[$PIPESTATUS] s:[${PIPESTATUS+set}]\"", "[0] s:[set]\n", ""),
    ("W5 pipe unset read", "false|true; unset PIPESTATUS; echo \"[$PIPESTATUS]\"", "[0]\n", ""),
    ("unset declare", "unset PIPESTATUS; declare -p PIPESTATUS; echo rc=$?", "declare -a PIPESTATUS=([0]=\"0\")\nrc=0\n", ""),
    ("first read", "echo \"[$PIPESTATUS] s:[${PIPESTATUS+set}]\"", "[] s:[]\n", ""),
    ("first declare", "declare -p PIPESTATUS; echo rc=$?", "rc=1\n", "declare: PIPESTATUS: not found"),
    ("pipe values", "false|true|false; declare -p PIPESTATUS", "declare -a PIPESTATUS=([0]=\"1\" [1]=\"0\" [2]=\"1\")\n", ""),
    ("pipe scalar read", "false|true; echo \"${PIPESTATUS} ${PIPESTATUS[1]} ${#PIPESTATUS[@]}\"", "1 0 2\n", ""),
    // --- unset / readonly interplay (T-series) ---
    ("T1 unset+declare after pipe", "false|true; unset PIPESTATUS; declare -p PIPESTATUS", "declare -a PIPESTATUS=([0]=\"0\")\n", ""),
    ("T3 unset then readonly", "unset PIPESTATUS; readonly PIPESTATUS; echo \"val:[$PIPESTATUS]\"; declare -p PIPESTATUS; false|true; declare -p PIPESTATUS", "val:[0]\ndeclare -ar PIPESTATUS=([0]=\"0\")\ndeclare -ar PIPESTATUS=([0]=\"1\" [1]=\"0\")\n", ""),
    ("unset readonly unset", "false|true; unset PIPESTATUS; readonly PIPESTATUS; declare -p PIPESTATUS", "declare -ar PIPESTATUS=([0]=\"0\")\n", ""),
    ("readonly after live", "true; readonly PIPESTATUS; declare -p PIPESTATUS; echo \"v:[$PIPESTATUS]\"; false|true; declare -p PIPESTATUS", "declare -ar PIPESTATUS=([0]=\"0\")\nv:[0]\ndeclare -ar PIPESTATUS=([0]=\"1\" [1]=\"0\")\n", ""),
    ("declare -a first", "declare -a PIPESTATUS; echo \"val:[$PIPESTATUS] set:[${PIPESTATUS+set}]\"; declare -p PIPESTATUS", "val:[] set:[]\ndeclare -a PIPESTATUS\n", ""),
    ("readonly then unset fails", "true; readonly PIPESTATUS; unset PIPESTATUS; echo rc=$?; declare -p PIPESTATUS", "rc=1\ndeclare -ar PIPESTATUS=([0]=\"0\")\n", "unset: PIPESTATUS: cannot unset: readonly variable"),
    // --- scalar cell ---
    ("scalar then declare -a", "PIPESTATUS=99; declare -a PIPESTATUS; declare -p PIPESTATUS; false|true; declare -p PIPESTATUS", "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"1\" [1]=\"0\")\n", ""),
    ("scalar elem assign", "PIPESTATUS=99; PIPESTATUS[0]=7; declare -p PIPESTATUS; false; declare -p PIPESTATUS", "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"1\")\n", ""),
    ("compound assign", "PIPESTATUS=(7 8); declare -p PIPESTATUS; true; declare -p PIPESTATUS", "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"0\")\n", ""),
    ("assign after unset", "unset PIPESTATUS; PIPESTATUS=5; declare -p PIPESTATUS; false|true; declare -p PIPESTATUS; echo \"v:[$PIPESTATUS]\"", "declare -a PIPESTATUS=([0]=\"0\")\ndeclare -a PIPESTATUS=([0]=\"1\" [1]=\"0\")\nv:[0]\n", ""),
    ("live export print", "true; export PIPESTATUS; export -p | grep PIPESTATUS; readonly -p | grep PIPESTATUS; echo done", "declare -ax PIPESTATUS=([0]=\"0\")\ndone\n", ""),
    ("listing contains", "true; declare -p | grep -c PIPESTATUS; echo rc=$?", "2\nrc=0\n", ""),
    // --- BASH_ARGC/ARGV at top level (#1394) ---
    ("argc declare", "declare -p BASH_ARGC BASH_ARGV", "declare -a BASH_ARGC=([0]=\"0\")\ndeclare -a BASH_ARGV=()\n", ""),
    ("argc elem", "echo \"${BASH_ARGC[0]} ${BASH_ARGV[0]}\"", "0 \n", ""),
    // --- assoc case-attribute forms (#1398/#1400) ---
    ("1398 one-shot -uA", "declare -uA h=([k]=v); declare -p h; echo \"elem:[${h[k]}]\"", "declare -Au h=([k]=\"V\" )\nelem:[V]\n", ""),
    ("1398 one-shot -lA", "declare -lA h=([K]=V); declare -p h; echo \"elem:[${h[K]}]\"", "declare -Al h=([K]=\"v\" )\nelem:[v]\n", ""),
    ("1398 attr only", "declare -A h=([k]=v); declare -u h; declare -p h; echo \"elem:[${h[k]}]\"", "declare -Au h=([k]=\"v\" )\nelem:[v]\n", ""),
    ("1400 elem store -u", "declare -uA h; h[k]=abc; declare -p h; echo \"elem:[${h[k]}]\"", "declare -Au h=([k]=\"ABC\" )\nelem:[ABC]\n", ""),
    ("1400 elem store -l", "declare -lA h; h[K]=ABC; declare -p h; echo \"elem:[${h[K]}]\"", "declare -Al h=([K]=\"abc\" )\nelem:[abc]\n", ""),
    ("1400 append", "declare -uA h; h[k]=abc; h[k]+=def; declare -p h", "declare -Au h=([k]=\"ABCDEF\" )\n", ""),
    ("1400 declare elem", "declare -uA h; declare h[k]=abc; declare -p h; echo \"elem:[${h[k]}]\"", "declare -Au h=([k]=\"ABC\" )\nelem:[ABC]\n", ""),
    ("1400 one-shot elem", "declare -uA h[k]=abc; declare -p h", "declare -Au h=([k]=\"ABC\" )\n", ""),
    ("scalar -u sanity", "declare -u s=abc; s=def; echo \"$s\"", "DEF\n", ""),
    ("indexed -u elem", "declare -u a; a[0]=abc; echo \"${a[0]}\"; declare -p a", "ABC\ndeclare -au a=([0]=\"ABC\")\n", ""),
];

#[test]
fn pipestatus_matrix_forms_match_gnu() {
    for &(name, script, want_out, want_err_part) in MATRIX {
        let (stdout, stderr, code) = rubash_c(script);
        assert_eq!(stdout.as_str(), want_out, "case {name}: script {script}");
        assert_eq!(code, Some(0), "case {name}: script {script}");
        if want_err_part.is_empty() {
            assert_eq!(stderr.as_str(), "", "case {name}: script {script}");
        } else {
            assert!(
                stderr.contains(want_err_part),
                "case {name}: stderr {stderr:?} must contain {want_err_part:?}"
            );
        }
    }
}

const FIRST_TOUCH_PENDING: &[(&str, &str, &str, &str)] = &[
    ("T2 readonly first", "readonly PIPESTATUS; echo \"val:[$PIPESTATUS] set:[${PIPESTATUS+set}]\"; declare -p PIPESTATUS", "val:[] set:[]\ndeclare -r PIPESTATUS\n", ""),
    ("readonly pipe update", "readonly PIPESTATUS; false|true; declare -p PIPESTATUS", "declare -r PIPESTATUS\n", ""),
    ("export first", "export PIPESTATUS; echo \"val:[$PIPESTATUS] set:[${PIPESTATUS+set}]\"; declare -p PIPESTATUS", "val:[] set:[]\ndeclare -x PIPESTATUS\n", ""),
    ("declare first", "declare PIPESTATUS; echo \"val:[$PIPESTATUS] set:[${PIPESTATUS+set}]\"; declare -p PIPESTATUS", "val:[] set:[]\ndeclare -- PIPESTATUS\n", ""),
    ("scalar freeze", "PIPESTATUS=99; echo \"[$PIPESTATUS]\"; false|true; echo \"[$PIPESTATUS]\"; declare -p PIPESTATUS", "[99]\n[99]\ndeclare -- PIPESTATUS=\"99\"\n", ""),
    ("scalar export print", "PIPESTATUS=99; export PIPESTATUS; export -p | grep PIPESTATUS", "declare -x PIPESTATUS=\"99\"\n", ""),
];

/// rubash#1364 first-touch family: a PIPESTATUS cell that has never
/// held a pipeline value must render in `declare -p` WITHOUT the array
/// attribute (`declare -r` / `declare -x` / `declare --`), and a plain
/// scalar assignment must freeze the variable (later pipelines stop
/// rewriting it). These six forms were the open half of #1364 (the
/// 1364.sh/1364b.sh pins below are its other two forms); the fix landed
/// on master via PR #1526 (commit 75cbab32), so the placeholders were
/// activated when the bf-a pins were integrated.
#[test]
fn pipestatus_first_touch_forms_pending_issue1364() {
    for &(name, script, want_out, want_err_part) in FIRST_TOUCH_PENDING {
        let (stdout, stderr, code) = rubash_c(script);
        assert_eq!(stdout.as_str(), want_out, "case {name}: script {script}");
        assert_eq!(code, Some(0), "case {name}: script {script}");
        if want_err_part.is_empty() {
            assert_eq!(stderr.as_str(), "", "case {name}: script {script}");
        } else {
            assert!(
                stderr.contains(want_err_part),
                "case {name}: stderr {stderr:?} must contain {want_err_part:?}"
            );
        }
    }
}

/// R3 (rubash#1364): assigning a fresh sparse element
/// (`PIPESTATUS[2]=5` before PIPESTATUS exists) must keep the sparse
/// index — GNU prints `([2]="0")` both before and after the next
/// pipeline rewrites the value. The sparse-representation rework
/// (variables.c:6291 set_pipestatus_array over the ARRAY object)
/// landed with the fix5/r3 cluster, activating this pin.
#[test]
fn pipestatus_sparse_element_assignment_keeps_index() {
    let (stdout, stderr, code) =
        rubash_c("PIPESTATUS[2]=5; declare -p PIPESTATUS; true; declare -p PIPESTATUS");
    assert_eq!(
        stdout,
        "declare -a PIPESTATUS=([2]=\"0\")\ndeclare -a PIPESTATUS=([2]=\"0\")\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// R3 (rubash#1364): the sparse writer's grow path — a following
/// pipeline with MORE statuses than the cell has elements replaces the
/// existing elements in index order and inserts the remaining statuses
/// AT their indices (array.c:516 array_insert replaces on collision),
/// so the sparse index survives and the count can stay below the
/// pipeline length. A pipeline with FEWER statuses rebuilds densely.
#[test]
fn pipestatus_sparse_writer_grow_and_shrink_match_gnu() {
    for (script, want) in [
        (
            "PIPESTATUS[2]=5; true|false; declare -p PIPESTATUS",
            "declare -a PIPESTATUS=([1]=\"1\" [2]=\"0\")\n",
        ),
        (
            "PIPESTATUS[2]=5; true|false|true; declare -p PIPESTATUS",
            "declare -a PIPESTATUS=([1]=\"1\" [2]=\"0\")\n",
        ),
        (
            "PIPESTATUS[2]=5; true|true|true|true; declare -p PIPESTATUS",
            "declare -a PIPESTATUS=([1]=\"0\" [2]=\"0\" [3]=\"0\")\n",
        ),
        // Shrink: fewer statuses than elements flushes, dense rebuild.
        (
            "PIPESTATUS[2]=5; PIPESTATUS[0]=7; declare -p PIPESTATUS",
            "declare -a PIPESTATUS=([0]=\"0\")\n",
        ),
    ] {
        let (stdout, stderr, code) = rubash_c(script);
        assert_eq!(stdout, want, "script {script}");
        assert_eq!(stderr, "", "script {script}");
        assert_eq!(code, Some(0), "script {script}");
    }
}

/// R3 (rubash#1364): reads on a sparse live cell resolve by INDEX —
/// `$PIPESTATUS` and arithmetic read element 0 (unset on a sparse
/// cell, never the first stored entry), `${PIPESTATUS[@]}` lists the
/// values in index order, `${!PIPESTATUS[@]}` the real indices, and a
/// negative subscript resolves against the MAXIMUM index with holes
/// reading as unset in silence.
#[test]
fn pipestatus_sparse_reads_resolve_by_index() {
    let (stdout, stderr, code) = rubash_c(
        "PIPESTATUS[5]=3; echo \"all:[${PIPESTATUS[@]}] keys:[${!PIPESTATUS[@]}] \
         n:${#PIPESTATUS[@]} s:[$PIPESTATUS] e5:[${PIPESTATUS[5]}]\"",
    );
    assert_eq!(stdout, "all:[0] keys:[5] n:1 s:[] e5:[0]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    let (stdout, stderr, code) = rubash_c(
        "PIPESTATUS[2]=5; echo \"m1:[${PIPESTATUS[-1]}] m2:[${PIPESTATUS[-2]}] \
         m3:[${PIPESTATUS[-3]}]\"; PIPESTATUS[2]=5; false; \
         echo \"s:[$PIPESTATUS] a:$((PIPESTATUS)) ap:$((PIPESTATUS[0])) a2:$((PIPESTATUS[2]))\"",
    );
    assert_eq!(stdout, "m1:[0] m2:[] m3:[]\ns:[] a:0 ap:0 a2:1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));

    // Arithmetic element reads consult the live cell, not the env
    // mirror (which never carries pipeline results): a dense
    // pipeline's [1] and a sparse cell's surviving [2] both read
    // their real statuses.
    let (stdout, stderr, code) =
        rubash_c("true|false; echo \"a1:$((PIPESTATUS[1])) a0:$((PIPESTATUS[0]))\"");
    assert_eq!(stdout, "a1:1 a0:0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// R3 (rubash#1364): unsetting the only sparse element empties the
/// cell, so the unset command's own completion writes its status
/// densely at index 0 (GNU oracle-verified).
#[test]
fn pipestatus_sparse_unset_element_rebuilds_dense() {
    let (stdout, stderr, code) =
        rubash_c("PIPESTATUS[2]=5; unset PIPESTATUS[2]; declare -p PIPESTATUS");
    assert_eq!(stdout, "declare -a PIPESTATUS=([0]=\"0\")\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------
// rubash#1051 — BASH_ARGV must stay unpopulated without extdebug, and
// ${#BASH_ARGC[@]} must agree with the (empty) array it describes.
// ---------------------------------------------------------------------

#[test]
fn issue1051_bash_argv_hidden_without_extdebug() {
    let (stdout, stderr, code) = rubash_c(
        "shopt -u extdebug; f() { echo \"argv=[${BASH_ARGV[*]-UNSET}]\"; echo \"len=${#BASH_ARGC[@]}\"; declare -p BASH_ARGC; }; f a b c",
    );
    assert_eq!(stdout, "argv=[UNSET]\nlen=0\ndeclare -a BASH_ARGC=()\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1051_bash_argv_populated_with_extdebug() {
    let (stdout, stderr, code) = rubash_c(
        "shopt -s extdebug; f() { echo \"argc=${BASH_ARGC[0]} argv=[${BASH_ARGV[*]}] len-argv=${#BASH_ARGV[@]} len-argc=${#BASH_ARGC[@]}\"; }; f a b c",
    );
    assert_eq!(stdout, "argc=3 argv=[c b a] len-argv=3 len-argc=2\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------
// rubash#1110 — compat-ignored behaviours batch.
// ---------------------------------------------------------------------

#[test]
fn issue1110_compat43_break_in_function_breaks_caller_loop() {
    let (stdout, stderr, code) = rubash_c(
        "shopt -s compat43; f(){ break; echo NO; }; for i in 1 2; do f; echo \"iter$i\"; done; echo done",
    );
    assert_eq!(stdout, "done\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1110_compat44_assignment_prefix_export_persists() {
    let (stdout, stderr, code) = rubash_c(
        "shopt -s compat44; f(){ local v=outer; v=inner export v; }; unset v; f; echo \"v=${v-UNSET}\"",
    );
    assert_eq!(stdout, "v=inner\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1110_compat44_bash_argc_argv_lengths() {
    let (stdout, stderr, code) =
        rubash_c("shopt -s compat44; f(){ echo \"${#BASH_ARGC[@]} ${#BASH_ARGV[@]}\"; }; f a b");
    assert_eq!(stdout, "1 0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

/// `time -p` after `set -o posix` on its own line: with POSIX mode in
/// effect at parse time, a word starting with `-` after `time` keeps
/// `time` from being a keyword, so it is looked up as an external
/// command (and PATH has been emptied). The same text on one line is
/// parsed before `set` runs and stays a keyword — the multi-line form
/// is the filed trigger (cf. rubash#1218).
#[test]
fn issue1110_posix_time_dash_p_is_not_a_keyword() {
    let scratch = Scratch::new("1110-time");
    let (stdout, stderr, code) =
        scratch.rubash_file("set -o posix\nPATH=/nonexistent\ntime -p true\n", &[]);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(127));
}

// ---------------------------------------------------------------------
// rubash#1282 — scalar expansion of the call-stack arrays must render
// element 0, never the internal carrier repr (a stray 0x1D + parens).
// ---------------------------------------------------------------------

#[test]
fn issue1282_scalar_call_stack_vars_at_top_level() {
    let (stdout, stderr, code) = rubash_c("echo \"argc=[$BASH_ARGC] argv=[$BASH_ARGV]\"");
    assert_eq!(stdout, "argc=[0] argv=[]\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1282_scalar_call_stack_vars_inside_function() {
    let scratch = Scratch::new("1282");
    // The call to `f` is on line 8 by construction; GNU reports
    // BASH_LINENO element 0 as the caller's line number.
    let script = "shopt -s extdebug\nf() {\n  echo \"scalar-ARGC=[$BASH_ARGC]\"\n  echo \"scalar-ARGV=[$BASH_ARGV]\"\n  echo \"scalar-LINENO=[$BASH_LINENO]\"\n  echo \"idx0-ARGC=[${BASH_ARGC[0]}]\"\n}\nf x y\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, &[]);
    assert_eq!(
        stdout,
        "scalar-ARGC=[2]\nscalar-ARGV=[y]\nscalar-LINENO=[8]\nidx0-ARGC=[2]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

// ---------------------------------------------------------------------
// Script-file forms (run-diff harness shape: args `a "b c" d`).
// ---------------------------------------------------------------------

const RUN_ARGS: &[&str] = &["a", "b c", "d"];

#[test]
fn issue1363_declare_p_pipestatus_script_form() {
    let scratch = Scratch::new("1363");
    let script = "true\ndeclare -p PIPESTATUS; echo \"rc=$?\"\ntrue | false\ndeclare -p PIPESTATUS; echo \"rc=$?\"\nunset PIPESTATUS; echo \"${PIPESTATUS[@]}\"\ndeclare -p PIPESTATUS; echo \"rc=$?\"\nexport PIPESTATUS; declare -p PIPESTATUS; echo \"rc=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(
        stdout,
        "declare -a PIPESTATUS=([0]=\"0\")\nrc=0\ndeclare -a PIPESTATUS=([0]=\"0\" [1]=\"1\")\nrc=0\n0\ndeclare -a PIPESTATUS=([0]=\"0\")\nrc=0\ndeclare -ax PIPESTATUS=([0]=\"0\")\nrc=0\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1364_readonly_pipestatus_script_form() {
    let scratch = Scratch::new("1364");
    let script = "readonly PIPESTATUS\ntrue | false\necho \"ps=${PIPESTATUS[@]}\"\ndeclare -p PIPESTATUS; echo \"rc=$?\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(stdout, "ps=\ndeclare -r PIPESTATUS\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1364_declare_r_pipestatus_script_form() {
    let scratch = Scratch::new("1364b");
    let script = "declare -r PIPESTATUS\ntrue | false\necho \"ps=${PIPESTATUS[@]}\"\ndeclare -p PIPESTATUS\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(stdout, "ps=\ndeclare -r PIPESTATUS\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1384_pipestatus_survives_never_entered_while() {
    let scratch = Scratch::new("1384");
    let script = "true | true\nwhile false; do :; done; echo \"ps=${PIPESTATUS[@]}\"\nwhile true; do break; done; echo \"ps2=${PIPESTATUS[@]}\"\nfalse\nwhile false; do :; done; echo \"ps3=${PIPESTATUS[@]}\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(stdout, "ps=1\nps2=0\nps3=1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1387_pipestatus_after_for_loop_with_pipeline_body() {
    let scratch = Scratch::new("1387");
    let script = "for i in 1; do false | true; done; echo \"ps=${PIPESTATUS[@]}\"\nfor i in 1; do true | false; done; echo \"ps=${PIPESTATUS[@]}\"\nfor i in 1; do false; done; echo \"ps=${PIPESTATUS[@]}\"\nfor i in 1 2; do false | true; done; echo \"ps=${PIPESTATUS[@]}\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(stdout, "ps=1 0\nps=0 1\nps=1\nps=1 0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1394_bash_argc_argv_at_script_top_level() {
    let scratch = Scratch::new("1394");
    let script = "declare -p BASH_ARGC BASH_ARGV; echo \"rc=$?\"\necho \"argc0=${BASH_ARGC[0]} argv=${BASH_ARGV[@]} n=${#BASH_ARGV[@]}\"\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(
        stdout,
        "declare -a BASH_ARGC=([0]=\"3\")\ndeclare -a BASH_ARGV=([0]=\"d\" [1]=\"b c\" [2]=\"a\")\nrc=0\nargc0=3 argv=d b c a n=3\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue1394_bash_argc_argv_declare_rendering() {
    let scratch = Scratch::new("1394-argc");
    let script = "declare -p BASH_ARGC BASH_ARGV\necho \"argc-elem:${BASH_ARGC[0]} argv-elem:${BASH_ARGV[0]}\"\necho \"argv-all:${BASH_ARGV[@]}\"\nf() {\n  declare -p BASH_ARGC BASH_ARGV\n  echo \"in-fn argc:${BASH_ARGC[@]} argv:${BASH_ARGV[@]}\"\n}\nf x y\ndeclare -p BASH_ARGC BASH_ARGV\n";
    let (stdout, stderr, code) = scratch.rubash_file(script, RUN_ARGS);
    assert_eq!(
        stdout,
        "declare -a BASH_ARGC=([0]=\"3\")\ndeclare -a BASH_ARGV=([0]=\"d\" [1]=\"b c\" [2]=\"a\")\nargc-elem:3 argv-elem:d\nargv-all:d b c a\ndeclare -a BASH_ARGC=([0]=\"3\")\ndeclare -a BASH_ARGV=([0]=\"d\" [1]=\"b c\" [2]=\"a\")\nin-fn argc:3 argv:d b c a\ndeclare -a BASH_ARGC=([0]=\"3\")\ndeclare -a BASH_ARGV=([0]=\"d\" [1]=\"b c\" [2]=\"a\")\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
