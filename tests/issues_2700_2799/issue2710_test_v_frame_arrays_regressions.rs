//! rubash#2710 — `test -v` (and `[[ -v ]]`) was blind to FUNCNAME and
//! BASH_SOURCE inside functions: every form (`-v FUNCNAME`,
//! `-v FUNCNAME[0]`, `-v BASH_SOURCE`, `-v BASH_SOURCE[0]`) returned
//! false while `declare -p` on the same names worked.
//!
//! GNU ground: test.c:650-677's unary `-v` is find_variable over the
//! live variable table, following nameref chains (variables.c:2011-2047
//! find_variable_nameref) — the frame arrays are ordinary (if dynamic)
//! cells there, so they answer like any stored array.
//!
//! Rubash composes the frame arrays from the call stacks
//! (dynamic_stack_array_values) and their env cells are refreshed only
//! at frame push/pop or not materialized at all (FUNCNAME), so the
//! env-only lookup in builtins::test::variable_is_set missed them.
//! The executor now overlays the composed storage — in the cells' own
//! serialized form — onto the env map handed to that lookup whenever a
//! `-v` operand names a frame carrier; the builtin's env-only
//! contract (and its #2073 no-process-env rule) is untouched.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file and -c probes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2710-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run_dash_c(script: &str) -> String {
    let output = Command::new(RUBASH)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn issue2710_frame_arrays_visible_to_test_v_in_script_files() {
    let dir = scratch("script");
    let script = "\
[ -v BASH_SOURCE ] && echo a1 || echo a0
[ -v BASH_SOURCE[0] ] && echo b1 || echo b0
[ -v FUNCNAME ] && echo c1 || echo c0
f() {
  [ -v FUNCNAME ] && echo d1 || echo d0
  [ -v FUNCNAME[0] ] && echo e1 || echo e0
  [ -v FUNCNAME[1] ] && echo f1 || echo f0
  [ -v BASH_SOURCE ] && echo g1 || echo g0
  [ -v BASH_SOURCE[0] ] && echo h1 || echo h0
  [[ -v FUNCNAME ]] && echo i1 || echo i0
  [[ -v BASH_SOURCE[0] ]] && echo j1 || echo j0
  [ -v FUNCNAME[@] ] && echo k1 || echo k0
  [ -v BASH_LINENO ] && echo l1 || echo l0
  [ -v BASH_ARGC ] && echo m1 || echo m0
}
f
";
    // Script-file mode has a main frame: FUNCNAME=(f main) inside f,
    // BASH_SOURCE names the script from the top level on, and BASH_ARGC
    // stays unset without extdebug.
    assert_eq!(
        run_script(&dir, script),
        "a1\nb1\nc0\nd1\ne1\nf1\ng1\nh1\ni1\nj1\nk1\nl1\nm0\n"
    );
}

#[test]
fn issue2710_frame_arrays_visible_to_test_v_in_dash_c() {
    // -c mode: no main frame, empty stack arrays at the top level.
    assert_eq!(
        run_dash_c(
            "[ -v FUNCNAME ] && echo a1 || echo a0; [ -v BASH_SOURCE ] && echo b1 || echo b0"
        ),
        "a0\nb0\n"
    );
    assert_eq!(
        run_dash_c(
            "f() { [ -v FUNCNAME ] && echo a1 || echo a0; [ -v FUNCNAME[0] ] && echo b1 || echo b0; [ -v BASH_SOURCE ] && echo c1 || echo c0; [ -v BASH_SOURCE[0] ] && echo d1 || echo d0; [[ -v FUNCNAME ]] && echo e1 || echo e0; [[ -v BASH_SOURCE[0] ]] && echo f1 || echo f0; }; f"
        ),
        "a1\nb1\nc1\nd1\ne1\nf1\n"
    );
    // Nested calls: FUNCNAME=(g f) — element 1 set, element 2 unset.
    assert_eq!(
        run_dash_c(
            "g() { [ -v FUNCNAME[1] ] && echo n1 || echo n0; [ -v FUNCNAME[2] ] && echo o1 || echo o0; }; f() { g; }; f"
        ),
        "n1\no0\n"
    );
}
