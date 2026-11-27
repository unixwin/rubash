//! rubash#2946 — associative array assignment evaluates the subscript
//! twice.
//!
//! GNU anchor: assign_array_element_internal (arrayfunc.c:392-398)
//! expands an associative subscript EXACTLY ONCE —
//! `akey = expand_subscript_string (sub, 0)` — and hands the already
//! expanded key to bind_assoc_variable; there is no whole-word expansion
//! pass for an assignment word at all (general.c:480 assignment() splits
//! the syntactic word first). Rubash instead ran its whole-word
//! expansion over `a[$(f)]=W` (evaluating the subscript's command
//! substitution) and then expanded the raw subscript a second time
//! while preparing the assignment, discarding the first result: the
//! side effect ran twice and the stored key came from the SECOND
//! evaluation.
//!
//! `shopt -s assoc_expand_once` governs the EXPANSION form
//! (${a[$(f)]}) only; GNU evaluates the assignment form once with the
//! option on or off, so both settings are pinned.
//!
//! Expectations cross-checked against GNU bash 5.3.20 (oracle probes on
//! master 4f747412): the plain, option-on, nested and append facets
//! diverge (counter 2, second key wins); the controls agree.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQUE: AtomicU64 = AtomicU64::new(0);

struct CaseDir(PathBuf);

impl CaseDir {
    fn new(label: &str) -> Self {
        let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "rubash_issue2946_{}_{}_{}",
            std::process::id(),
            label,
            n
        ));
        fs::create_dir_all(&dir).expect("create case dir");
        Self(dir)
    }

    fn run(&self, script: &str) -> (String, String) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .args(["-c", script])
            .current_dir(&self.0)
            .stdin(Stdio::null())
            .env_remove("BASH_ENV")
            .env_remove("WINUXSH_ROOT")
            .output()
            .expect("run rubash -c");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }
}

impl Drop for CaseDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// The seed counter function: each call appends one line to ./cnt and
// echoes the key `k`.
const F_DEF: &str = "f(){ echo >>cnt; echo k; }; ";

#[test]
fn assoc_assignment_subscript_evaluates_once() {
    // Regression seed: cnt must hold exactly one line and the value must
    // land under the (single) expanded key.
    let dir = CaseDir::new("plain");
    let script = format!("declare -A a; {F_DEF}a[$(f)]=W; echo \"val=${{a[k]}}\"");
    let (stdout, stderr) = dir.run(&script);
    assert_eq!(stdout, "val=W\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
}

#[test]
fn assoc_assignment_subscript_evaluates_once_with_assoc_expand_once() {
    // mD2: the option changes nothing for the assignment form.
    let dir = CaseDir::new("expand_once");
    let script = format!("shopt -s assoc_expand_once; declare -A a; {F_DEF}a[$(f)]=W");
    let (_, stderr) = dir.run(&script);
    assert_eq!(dir.read("cnt").lines().count(), 1, "stderr={stderr:?}");
}

#[test]
fn assoc_assignment_nested_subscripts_evaluate_once() {
    // mD9: in `a[$(f)]=${a[$(g)]}` BOTH subscripts evaluate once.
    let dir = CaseDir::new("nested");
    let script = "declare -A a=([k]=inner); \
                  f(){ echo >>cnt; echo k; }; \
                  g(){ echo >>cnt2; echo k; }; \
                  a[$(f)]=${a[$(g)]}; echo \"val=${a[k]}\"";
    let (stdout, stderr) = dir.run(script);
    assert_eq!(stdout, "val=inner\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
    assert_eq!(dir.read("cnt2").lines().count(), 1);
}

#[test]
fn assoc_assignment_stores_the_single_evaluations_key() {
    // If the subscript ran twice, the SECOND result (k2) would win the
    // store; GNU's single evaluation stores k1.
    let dir = CaseDir::new("firstkey");
    let script = "declare -A a; \
                  f(){ n=$(cat n 2>/dev/null || echo 0); n=$((n+1)); echo $n >n; echo k$n; }; \
                  a[$(f)]=W; echo \"keys=${!a[@]}\"; echo \"n=$(cat n)\"";
    let (stdout, stderr) = dir.run(script);
    assert_eq!(stdout, "keys=k1\nn=1\n", "stderr={stderr:?}");
}

#[test]
fn assoc_append_subscript_evaluates_once() {
    // The append form shares the assignment path end to end.
    let dir = CaseDir::new("append");
    let script = format!("declare -A a=([k]=V); {F_DEF}a[$(f)]+=W; echo \"val=${{a[k]}}\"");
    let (stdout, stderr) = dir.run(&script);
    assert_eq!(stdout, "val=VW\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
}

#[test]
fn indexed_assignment_subscript_evaluates_once() {
    // Control (mD5, both shells): indexed subscripts go through
    // arithmetic evaluation, which already ran once.
    let dir = CaseDir::new("indexed");
    let script = "declare -a a; f(){ echo >>cnt; echo 0; }; a[$(f)]=W; echo \"val=${a[0]}\"";
    let (stdout, stderr) = dir.run(script);
    assert_eq!(stdout, "val=W\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
}

#[test]
fn assoc_expansion_subscript_evaluates_once() {
    // Control (mD3, both shells): the expansion form evaluates the
    // subscript once per use.
    let dir = CaseDir::new("expansion");
    let script = format!("declare -A a=([k]=V); {F_DEF}echo \"val=${{a[$(f)]}}\"");
    let (stdout, stderr) = dir.run(&script);
    assert_eq!(stdout, "val=V\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
}

#[test]
fn assoc_assignment_value_comsub_evaluates_once() {
    // Control (both shells): a command substitution in the VALUE (with
    // a literal subscript) also runs exactly once.
    let dir = CaseDir::new("value");
    let script = "declare -A a; f(){ echo >>cnt; echo W; }; a[k]=$(f); echo \"val=${a[k]}\"";
    let (stdout, stderr) = dir.run(script);
    assert_eq!(stdout, "val=W\n", "stderr={stderr:?}");
    assert_eq!(dir.read("cnt").lines().count(), 1);
}
