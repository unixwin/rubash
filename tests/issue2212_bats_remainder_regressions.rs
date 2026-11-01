//! rubash#2212: bats-core's own suite (bats.bats) divergences —
//! the remainder after the CTRL-C family split into #2241.
//! Expectations are GNU bash 5.3.20 behavior, one test per face:
//!   01 — nounset `${!i}` indirection through a positional index
//!   02 — nounset element reads of the dynamic stack arrays
//!   03 — CLI argv after the script operand passes through verbatim
//!   04 — nounset diagnostics name the file the failing command is in
//!   05 — lowercase environment imports keep the export attribute
//!   06 — a `>(...)` consumer inherits the shell's ambient fds
//!   07 — a heredoc-bearing function survives the async respawn prelude
//!   08 — a `<(...)` producer inherits an ambient std-stream fd
//!   09 — a pipeline stage's `>(...)` consumer inherits ambient fds
//!   10 — an async group's ambient fd reaches its respawn
//!   11 — a `<(...)` child sees an ambient `exec 9>file` fd
//!   12 — the EXIT trap runs once per exit (handler re-registration)
//!   13 — the DEBUG trap does not fire inside the EXIT trap action

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
        .current_dir(dir)
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
//
// Shape note (collector adjudication, oracle-verified): this pin
// runs a SCRIPT FILE, not a `-c` string. In the `-c` shape GNU
// 5.3.20 itself reports `FUNCNAME[1]: unbound variable` — the
// `main` frame only exists for a script process — so a `-c` pin
// would assert non-GNU behavior. In the script shape GNU prints
// `fn=main src0=<path> ln=3` and rubash matches byte-for-byte.
#[test]
fn issue2212_02_nounset_stack_array_elements() {
    let dir = test_dir("02");
    let output = run_script(
        &dir,
        "main.sh",
        "set -u\nf() { echo \"fn=${FUNCNAME[1]} src0=${BASH_SOURCE[0]} ln=${BASH_LINENO[0]}\"; }\nf\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fn=main"), "stdout: {stdout}");
    assert!(stdout.contains("src0="), "stdout: {stdout}");
    assert!(stdout.contains("ln=3"), "stdout: {stdout}");

    // Out-of-range elements stay unbound under nounset (GNU reports;
    // verified in this same script shape against the oracle).
    let output = run_script(&dir, "oob.sh", "set -u\necho \"${BASH_SOURCE[9]}\"\n");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unbound variable"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
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

    // The export attribute is observed via `declare -p`:
    // `declare -x <name>` prints nothing on GNU 5.3.20 either
    // (oracle-verified), while master prints
    // `declare -- issue2212_lower="kept"` here (no -x: the bug).
    let output = rubash_command()
        .env("issue2212_lower", "kept")
        .args(["-c", "declare -p issue2212_lower"])
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

// 07 — bats.bats test 118's prelude face: bats' libexec respawns
// carry every defined function through the background prelude
// serializer. A function whose body ends with a heredoc delimiter
// line must not have the prelude's closer appended ON that line —
// GNU print_cmd.c:1053 prints deferred heredocs so the delimiter
// never shares its line (the same rule as the #2388 closer in
// function source text). Before the fix, any `&` respawn from a
// table containing a heredoc-bearing function died at parse with
// "here-document delimited by end-of-file" + a syntax error.
#[test]
fn issue2212_07_heredoc_function_survives_async_prelude() {
    let dir = test_dir("07");
    let body = "usage() {\n  cat <<'HELP'\nhelp body\nHELP\n}\necho hi &\nwait\necho ok\n";
    let output = run_script(&dir, "main.sh", body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hi\nok\n");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("here-document"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

// 08 — an input process substitution's producer inherits an
// ambient fd bound to a standard stream (`exec 3<&1`, the shape
// bats' own `run` uses for its fd-3 output channel). GNU's
// process_substitute fork (subst.c:6362) inherits the descriptor
// itself; the std-stream endpoints carry no FileFd handle, so the
// delivery transports used to drop them and the producer died
// with EBADF.
#[test]
fn issue2212_08_input_procsub_inherits_ambient_std_fd() {
    let dir = test_dir("08");
    let body = "exec 3>&1\nx=$(cat <(echo via-input >&3))\necho \"got=[$x]\"\n";
    let output = run_script(&dir, "main.sh", body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "via-input\ngot=[]\n"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("Bad file descriptor"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = fs::remove_dir_all(&dir);
}

// 09 — the failing half of pin 06's shape: the ambient fd must
// also reach a `>(...)` consumer spawned from inside a PIPELINE
// STAGE (bats.bats test 118 runs `tee >(cat >&3)` as a stage).
// The stage's own spawn adjudication carried the fd; the nested
// consumer spawn dropped it.
#[test]
fn issue2212_09_pipeline_stage_procsub_inherits_ambient_fd() {
    let dir = test_dir("09");
    let cap = dir.join("cap.txt");
    let body = format!(
        "exec 3>{cap}\necho hello | tee >(cat >&3) >/dev/null\n",
        cap = shell_test_path(&cap)
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = String::new();
    while Instant::now() < deadline {
        delivered = fs::read_to_string(&cap).unwrap_or_default();
        if delivered.contains("hello") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(delivered.contains("hello"), "cap.txt: {delivered:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 10 — an async brace group's respawn inherits an ambient fd
// bound to a standard stream (`{ echo >&3; } &` after
// `exec 3>file`). The async ambient pass keyed deliveries by
// FileFd identity only, so std-stream endpoints never crossed.
#[test]
fn issue2212_10_async_group_inherits_ambient_fd() {
    let dir = test_dir("10");
    let cap = dir.join("cap.txt");
    let body = format!(
        "exec 3>{cap}\n{{ echo via-async >&3; }} &\nwait\n",
        cap = shell_test_path(&cap)
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delivered = String::new();
    while Instant::now() < deadline {
        delivered = fs::read_to_string(&cap).unwrap_or_default();
        if delivered.contains("via-async") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(delivered.contains("via-async"), "cap.txt: {delivered:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 11 — a `<(...)` child sees an ambient `exec 9>file` fd
// (rubash#2466's shape, surfaced by the #2163 mapping and fixed
// by the same delivery pass as 08: the input-procsub spawn had
// no ambient pass at all). The foreign-child half needs the raw
// descriptor pinned at its slot; the rubash-child half rides
// the env channel.
#[test]
fn issue2212_11_input_procsub_child_sees_ambient_file_fd() {
    let dir = test_dir("11");
    let body =
        "exec 9>ps.txt\ngot=$(cat <(echo in-child >&9; echo to-pipe))\necho \"pipe=[$got]\"\n";
    let output = run_script(&dir, "main.sh", body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pipe=[to-pipe]\n");
    let ps = fs::read_to_string(dir.join("ps.txt")).unwrap_or_default();
    assert!(ps.contains("in-child"), "ps.txt: {ps:?}");
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn issue2212_11_input_procsub_foreign_child_sees_ambient_file_fd() {
    let dir = test_dir("11f");
    let body = "exec 9>ps.txt\ngot=$(cat <(/usr/bin/bash -c 'echo in-child >&9; echo to-pipe'))\necho \"pipe=[$got]\"\n";
    let output = run_script(&dir, "main.sh", body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "pipe=[to-pipe]\n");
    let ps = fs::read_to_string(dir.join("ps.txt")).unwrap_or_default();
    assert!(ps.contains("in-child"), "ps.txt: {ps:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 12 — the EXIT trap runs ONCE per shell exit (GNU shell.c
// exit_shell -> trap.c run_exit_trap). bats' exec-suite teardown
// re-registers the EXIT action from inside the running handler
// ("avoid being called twice"); a second funnel entry for the
// same exit must not fire the new registration. Before the fix
// the `exit` builtin's dispatch ran the trap and the script
// finish ran it again, so the re-registered action fired too
// (bats.bats test 133's doubled failure block).
#[test]
fn issue2212_12_exit_trap_runs_once_per_exit() {
    let dir = test_dir("12");
    let body = "inner() { echo INNER; trap 'echo SECOND' EXIT; }\nwrapper() { echo WRAPPER; inner; }\ntrap wrapper EXIT\nexit 3\n";
    let output = run_script(&dir, "main.sh", body);
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "WRAPPER\nINNER\n");
    let _ = fs::remove_dir_all(&dir);
}

// 13 — GNU gates every run_debug_trap call site on
// `running_trap == 0` (execute_cmd.c:704/3047/3519/...): commands
// inside a trap action take no DEBUG firing. Before the fix the
// EXIT handler's own commands were DEBUG-traced, which shifted
// bats' captured stack ring (bats.bats tests 55/57/59/64).
#[test]
fn issue2212_13_debug_trap_silent_inside_exit_trap() {
    let dir = test_dir("13");
    let body = "trap 'echo \"DEBUG $BASH_COMMAND\" >> dbg.log' DEBUG\ntrap 'echo in-handler' EXIT\necho body\n";
    let output = run_script(&dir, "main.sh", body);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "body\nin-handler\n"
    );
    let dbg = fs::read_to_string(dir.join("dbg.log")).unwrap_or_default();
    assert_eq!(
        dbg, "DEBUG trap 'echo in-handler' EXIT\nDEBUG echo body\n",
        "dbg.log: {dbg:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}
