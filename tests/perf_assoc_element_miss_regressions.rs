//! Perf round 2026-10-28, family F3: an associative-array element
//! MISS on the plain `${name[key]}` path must be answered by the
//! typed cell (authoritative miss), not re-derived from the
//! serialized storage by the operator/array braced handler.
//!
//! The storage re-derivation was Θ(n) per missed read (whole-storage
//! clone + `assoc_value_at` linear scan; `arrays.assoc_get_miss`
//! measured 13.96× GNU in the corpus) and it evaluated a
//! side-effecting subscript a second time. GNU evaluates the
//! subscript exactly once per expansion (subst.c:9777
//! parameter_brace_expand -> arrayfunc.c assoc_reference) and reads
//! the element with one hash lookup (assoc.c assoc_reference), hit
//! or miss.
//!
//! These pins assert GNU-observable behavior only: values on hit and
//! miss, single subscript evaluation, operator forms, and the
//! `declare -p` shape after reads. Expectations verified
//! byte-for-byte against GNU bash 5.3.20.

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-perf-assoc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create perf-assoc scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The corpus shape: a loop-filled assoc read on hit and on miss.
/// A set-but-empty element is a hit whose value is empty; an unset
/// element after `unset` is a miss again.
#[test]
fn assoc_get_hit_and_miss_after_loop_fill() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=()\n\
         for ((j=0; j<50; j++)); do m[key$j]=v$j; done\n\
         m[empty]=\n\
         echo \"hit=[${m[key25]}] miss=[${m[nope]}] emptyhit=[${m[empty]}]\"\n\
         unset 'm[key25]'\n\
         echo \"after-unset=[${m[key25]}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hit=[v25] miss=[] emptyhit=[]\nafter-unset=[]\n");
    assert_eq!(stderr, "");
}

/// GNU evaluates the subscript once per expansion, on hits and on
/// misses alike. The storage re-derivation ran `assoc_subscript_key`
/// a second time, so a miss used to leave `i` at 2.
#[test]
fn assoc_subscript_side_effect_runs_once_on_hit_and_miss() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=([1]=one [2]=two)\n\
         i=0\n\
         x=${m[$((++i))]}\n\
         echo \"hit: i=$i x=[$x]\"\n\
         i=0\n\
         x=${m[$((++i+5))]}\n\
         echo \"miss: i=$i x=[$x]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "hit: i=1 x=[one]\nmiss: i=1 x=[]\n");
    assert_eq!(stderr, "");
}

/// Operator and length forms over the same miss keep their GNU
/// answers, and reads do not disturb the stored shape.
#[test]
fn assoc_miss_operator_forms_and_storage_shape() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=([b]=2 [a]=1)\n\
         x=${m[nope]}\n\
         echo \"def=[${m[nope]:-fallback}] len=${#m[nope]} set=${m[nope]+yes}\"\n\
         declare -p m\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "def=[fallback] len=0 set=\ndeclare -A m=([b]=\"2\" [a]=\"1\" )\n"
    );
    assert_eq!(stderr, "");
}

/// rubash#2004: the `${#name[key]}` LENGTH leg must be settled by the
/// typed cell too — hit, miss, and set-but-empty — not re-derived from
/// the serialized storage (whole-storage clone + rehash + linear scan
/// per call, Θ(n) in the table size, hit AND miss). Values verified
/// against GNU bash 5.3.20.
#[test]
fn assoc_length_hit_miss_and_empty_hit() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=()\n\
         for ((j=0; j<50; j++)); do m[key$j]=val$j; done\n\
         m[empty]=\n\
         echo \"len-hit=[${#m[key25]}] len-miss=[${#m[nope]}] len-emptyhit=[${#m[empty]}]\"\n\
         unset 'm[key25]'\n\
         echo \"after-unset=[${#m[key25]}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "len-hit=[5] len-miss=[0] len-emptyhit=[0]\nafter-unset=[0]\n"
    );
    assert_eq!(stderr, "");
}

/// rubash#2004: a side-effecting subscript in a length read is
/// evaluated exactly once per expansion, on the hit leg and on the
/// settled-miss leg (GNU subst.c:9777 parameter_brace_expand evaluates
/// the subscript once, then assoc.c assoc_reference does one lookup).
#[test]
fn assoc_length_subscript_evaluated_once_hit_and_miss() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=()\n\
         for ((j=0; j<50; j++)); do m[key$j]=val$j; done\n\
         i=0\n\
         echo \"se-hit=[${#m[key$((i++))]}] i=$i\"\n\
         echo \"se-miss=[${#m[no$((i++))]}] i=$i\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "se-hit=[4] i=1\nse-miss=[0] i=2\n");
    assert_eq!(stderr, "");
}

/// rubash#2004: the nameref-cell length leg (`declare -n ref=m`;
/// `${#ref[key]}`) takes the same settled lookup, and a value with
/// multibyte text measures like GNU's `${#}`.
#[test]
fn assoc_length_through_nameref_and_multibyte_value() {
    let (code, stdout, stderr) = run_script(
        "declare -A m=()\n\
         for ((j=0; j<50; j++)); do m[key$j]=val$j; done\n\
         m[uni]=héllo\n\
         declare -n ref=m\n\
         echo \"ref-hit=[${#ref[key7]}] ref-miss=[${#ref[zz]}] uni=[${#m[uni]}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "ref-hit=[4] ref-miss=[0] uni=[6]\n");
    assert_eq!(stderr, "");
}

/// rubash#2004: the empty-key `bad array subscript` error
/// (subst.c:7545-7552) fires before any lookup, settled or not.
#[test]
fn assoc_length_empty_key_still_errors() {
    let (code, stdout, stderr) = run_script(
        "declare -A w=([a]=x)\n\
         unset e\n\
         echo \"before\"\n\
         echo \"emptykey=[${#w[$e]}]\"\n\
         echo \"after rc=$?\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "before\nafter rc=1\n");
    assert!(
        stderr.contains("[$e]: bad array subscript"),
        "stderr was: {stderr}"
    );
}

/// The settled-miss shortcut must also hold for function-local
/// assoc arrays and through a nameref to the array.
#[test]
fn assoc_miss_in_local_and_nameref_reads() {
    let (code, stdout, stderr) = run_script(
        "declare -A g=([k]=gv)\n\
         f() { declare -A loc=([k]=lv); echo \"loc-hit=[${loc[k]}] loc-miss=[${loc[no]}]\"; }\n\
         f\n\
         declare -n ref=g\n\
         echo \"ref-hit=[${ref[k]}] ref-miss=[${ref[no]}]\"\n",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "loc-hit=[lv] loc-miss=[]\nref-hit=[gv] ref-miss=[]\n"
    );
    assert_eq!(stderr, "");
}
