//! rubash#2212: bats-core's own suite (bats.bats) divergences —
//! the remainder after the CTRL-C family split into #2241.
//! Expectations are GNU bash 5.3.20 behavior, one test per face:
//!   01 — nounset `${!i}` indirection through a positional index
//!   02 — nounset element reads of the dynamic stack arrays
//!   03 — CLI argv after the script operand passes through verbatim
//!   04 — nounset diagnostics name the file the failing command is in
//!   05 — lowercase environment imports keep the export attribute
//!   06 — a `>(...)` consumer inherits the shell's ambient fds

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn rubash_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .env_remove("WINUXSH_ROOT")
        .env_remove("__RUBASH_SHELL_ROOT");
    command
}

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2212-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn run_script(dir: &Path, name: &str, body: &str) -> std::process::Output {
    let script = dir.join(name);
    fs::write(&script, body).expect("write script");
    rubash_command()
        .arg(shell_test_path(&script))
        .output()
        .expect("run script")
}

// 01 — bats lib/bats-core/common.bash `bats_all_in`/`bats_any_in`
// iterate `local search_value=${!i}` over "$@" under the caller's
// `set -u` (bats.bats test 51 "strict mode"). GNU (subst.c
// parameter_brace_expand_indir): a numeric indirect target names a
// positional parameter; the positional window adjudicates boundness
// exactly like the read path. Beyond the window both shells report.
#[test]
fn issue2212_01_nounset_indirect_positional_index() {
    let output = rubash_command()
        .args([
            "-c",
            "set -u; set -- alpha beta; i=1; echo \"in=${!i}\"; i=2; echo \"in2=${!i}\"",
        ])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "in=alpha\nin2=beta\n"
    );

    let output = rubash_command()
        .args(["-c", "set -u; set -- alpha beta; i=9; echo \"${!i}\""])
        .output()
        .expect("run rubash");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unbound variable"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// 02 — bats_capture_stack_trace reads ${BASH_LINENO[$i]},
// ${FUNCNAME[$i]} and ${BASH_SOURCE[$i]} element-wise from inside
// the DEBUG trap while `set -u` is in force (bats.bats tests 55/57/59
// traces; test 73 / bats#385 reads ${BASH_SOURCE[0]} at line 122 of
// libexec bats). GNU keeps these as real ARRAY objects (variables.c
// INIT_DYNAMIC_VAR over the live call stack), so an in-range element
// is bound. The nounset adjudication must consult the same live
// stack view the read path renders, not the env mirror.
#[test]
fn issue2212_02_nounset_stack_array_elements() {
    let output = rubash_command()
        .args([
            "-c",
            "set -u; f() { echo \"fn=${FUNCNAME[1]} src0=${BASH_SOURCE[0]} ln=${BASH_LINENO[0]}\"; }; f",
        ])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fn=main"), "stdout: {stdout}");
    assert!(stdout.contains("src0="), "stdout: {stdout}");

    // Out-of-range elements stay unbound under nounset (GNU reports).
    let output = rubash_command()
        .args(["-c", "set -u; echo \"${BASH_SOURCE[9]}\""])
        .output()
        .expect("run rubash");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unbound variable"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// 02b — the bats#385 shape reduced: in a fresh script process, a
// function reads stack-array elements from inside a command
// substitution while `set -u` is in force (libexec bats line 122
// reads ${BASH_SOURCE[0]} exactly this way). The env mirror holds
// no FUNCNAME/BASH_LINENO entry in this shape, so a mirror-only
// nounset adjudication reports them unbound; the live view does not.
#[test]
fn issue2212_02b_nounset_stack_elements_in_comsub() {
    let dir = test_dir("02b");
    let output = run_script(
        &dir,
        "child.sh",
        "set -u\nf() { x=\"${FUNCNAME[1]}|${BASH_LINENO[1]}|${BASH_SOURCE[0]}\"; echo \"got=$x\"; }\nout=\"$(f)\"\necho \"$out\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("got=main|"), "stdout: {stdout}");
    let _ = fs::remove_dir_all(&dir);
}

// 03 — bats.bats test 11: `bats -cf -- -- -tr`. GNU shell.c
// parse_shell_options consumes option words only up to the first
// operand; everything after the script name is the script's argv,
// verbatim — a bundled-flag shape there is data, not an option.
// Before the fix, rubash's argv pre-expansion rewrote the whole
// argv: `-cf` became `-f -c` (split, and reversed), so the script's
// own option parsing derailed. The pre-operand bundle expansion the
// scan exists for (`-ce 'cmd'`) must keep working.
#[test]
fn issue2212_03_argv_after_operand_verbatim() {
    let dir = test_dir("03");
    fs::write(
        dir.join("show.sh"),
        "printf 'argc=%s args=[%s] [%s] [%s] [%s]\\n' \"$#\" \"$1\" \"$2\" \"$3\" \"$4\"\n",
    )
    .expect("write script");
    let script = dir.join("show.sh");
    let output = rubash_command()
        .arg(shell_test_path(&script))
        .args(["-cf", "--", "--", "-tr"])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "argc=4 args=[-cf] [--] [--] [-tr]\n"
    );

    // An option-looking word after the script name is data too,
    // including an `-o` whose operand must not be eaten by the scan.
    let output = rubash_command()
        .arg(shell_test_path(&script))
        .args(["-o", "posix", "-c"])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "argc=3 args=[-o] [posix] [-c] []\n"
    );

    // Pre-operand bundles still expand: `-ce` runs the command string.
    let output = rubash_command()
        .args(["-ce", "echo bundle-ok"])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "bundle-ok\n");
    let _ = fs::remove_dir_all(&dir);
}

// 04 — bats.bats tests 55/57/59 + 64: GNU error.c get_name_for_error
// names the file the failing command was read from (BASH_SOURCE's
// innermost element), not the top-level script. A nounset failure
// inside a function defined in a sourced file blames that file, at
// the command's own line.
#[test]
fn issue2212_04_nounset_prefix_names_failing_file() {
    let dir = test_dir("04");
    fs::write(
        dir.join("lib.sh"),
        "setup_like() {\n  set -u\n  echo \"$unset_parameter_2212\"\n}\n",
    )
    .expect("write lib");
    let output = run_script(&dir, "main.sh", "source ./lib.sh\nsetup_like\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lib.sh: line 3: unset_parameter_2212: unbound variable"),
        "stderr: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 05 — bats.bats test 79 ("Setting BATS_TMPDIR is ignored"): the
// driver exports lowercase `expected` before invoking bats; GNU
// variables.c initialize_shell_variables imports every environment
// entry as an exported shell variable, so it survives the wrapper's
// `exec env ...` hop into the grandchild. The standalone shell
// process must mark lowercase imports exported too (the anti-leak
// case heuristic only governs in-process executors).
#[test]
fn issue2212_05_lowercase_env_import_stays_exported() {
    let output = rubash_command()
        .env("issue2212_lower", "kept")
        .args(["-c", "env"])
        .output()
        .expect("run rubash");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("issue2212_lower=kept"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let output = rubash_command()
        .env("issue2212_lower", "kept")
        .args(["-c", "declare -x issue2212_lower"])
        .output()
        .expect("run rubash");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("declare -x issue2212_lower=\"kept\""),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

// 06 — bats.bats test 118 (report formatter): bats runs
// `exec 3<&1` then `tee >(cat >&3)`. GNU process_substitute
// (subst.c:6362) forks the consumer with the shell's whole ambient
// fd table, so an fd opened by an earlier `exec 3>file` is visible
// inside `>(...)`. Before the fix only the owning command's
// redirect prefix crossed the consumer spawn.
#[test]
fn issue2212_06_procsub_consumer_inherits_ambient_fd() {
    let dir = test_dir("06");
    let out = dir.join("out.txt");
    let body = format!(
        "exec 3>{out}\necho hello > >(cat >&3)\nexec 3>&-\n",
        out = shell_test_path(&out)
    );
    let output = run_script(&dir, "main.sh", &body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("Bad file descriptor"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The consumer is asynchronous; poll for the delivered payload.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = String::new();
    while Instant::now() < deadline {
        delivered = fs::read_to_string(&out).unwrap_or_default();
        if delivered.contains("hello") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(delivered.contains("hello"), "out.txt: {delivered:?}");
    let _ = fs::remove_dir_all(&dir);
}
