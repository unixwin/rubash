//! rubash#3221: bats junit-formatter's nested fixture-suite tap13
//! lines leak into the outer TAP stream in full-file runs
//! (`Executed 15 instead of expected 13 tests`, rc=1; GNU: 13 ok,
//! rc=0). Residual of #3149 (whose pin covers the sandwich
//! function-stage stream loss, green on the #3177 head): the leak
//! needs the full reentrant context — the inner `bats` process
//! runs inside `run`'s `$(...)` (its stdout is the comsub pipe,
//! its fd 3 the inherited outer TAP stream) and its outer pipeline
//! starts with `exec bats-exec-suite`.
//!
//! Reduced shape (from bats' `bats_tee`):
//!
//!   outer: exec 3>tap.log; out=$("$RUBASH_BIN" ./inner.sh)
//!   inner: exec ./producer.sh | tee_fn | cat        (mode B)
//!          ./producer.sh | tee_fn | cat               (mode A)
//!   tee_fn() { exec 3<&1; tee >(cat >&3) | cat > report.xml; }
//!
//! GNU: the 4 TAP lines reach stdout via the validator `cat`,
//! tap.log stays empty, report.xml holds the 4 lines.
//! Rubash (base = master with #3177): the `>(cat >&3)` consumer's
//! fd 3 lands on the ambient fd 3 (the outer TAP stream) instead
//! of the stage pipe, so tap.log holds the raw TAP lines.
//!
//! Expectations are GNU bash 5.3.20 behavior, one test per face:
//!   01 — the failing cell: mode B (exec stage-0, comsub, fd-3 file)
//!   02 — mode A (no exec stage-0): discriminates whether the
//!        leading `exec` matters
//!   03 — control: same shape without the procsub (green on base
//!        and head); with no fd-3 copy OUT is empty, lines land
//!        only in report.xml

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let dir = std::env::temp_dir().join(format!("rubash-issue3221-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_producer(dir: &Path) {
    fs::write(
        dir.join("producer.sh"),
        "#!/usr/bin/env bash\nprintf '1..2\\nsuite f1\\nbegin 1 t1\\nok 1 t1 in 10ms\\n'\n",
    )
    .expect("write producer");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dir.join("producer.sh"))
            .expect("producer metadata")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dir.join("producer.sh"), perms).expect("chmod producer");
    }
}

fn write_inner(dir: &Path, mode: &str) {
    let pipeline = if mode == "B" {
        "exec ./producer.sh | tee_fn | cat"
    } else {
        "./producer.sh | tee_fn | cat"
    };
    fs::write(
        dir.join("inner.sh"),
        format!(
            "#!/usr/bin/env bash\ntee_fn() {{ exec 3<&1; tee >(cat >&3) | cat > report.xml; }}\n{pipeline}\n"
        ),
    )
    .expect("write inner");
}

fn run_outer(dir: &Path) -> std::process::Output {
    let script = dir.join("outer.sh");
    fs::write(
        &script,
        "exec 3>tap.log\nout=$(\"$RUBASH_BIN\" ./inner.sh)\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')] REPORT:[$(cat report.xml | tr '\\n' '|')]\"\n",
    )
    .expect("write outer");
    let mut command = rubash_command();
    command
        .current_dir(dir)
        .arg(shell_test_path(&script))
        .env("RUBASH_BIN", env!("CARGO_BIN_EXE_rubash"));
    command.output().expect("run outer")
}

// 01 — the failing cell (see the file header): mode B, the real
// bats shape. On the base the consumer leaks the raw TAP lines
// into tap.log; on the head tap.log stays empty and the lines
// reach stdout via the validator cat.
#[test]
fn issue3221_01_reentrant_exec_stage_procsub_consumer_leaks_to_ambient_fd3() {
    let dir = test_dir("01");
    write_producer(&dir);
    write_inner(&dir, "B");
    let output = run_outer(&dir);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[1..2\nsuite f1\nbegin 1 t1\nok 1 t1 in 10ms] TAP:[] REPORT:[1..2|suite f1|begin 1 t1|ok 1 t1 in 10ms|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = fs::read_to_string(dir.join("tap.log")).unwrap_or_default();
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let report = fs::read_to_string(dir.join("report.xml")).unwrap_or_default();
    assert_eq!(
        report, "1..2\nsuite f1\nbegin 1 t1\nok 1 t1 in 10ms\n",
        "report.xml: {report:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 02 — mode A (no leading `exec`): discriminates whether the
// leading `exec` stage-0 matters. Same expectations as 01.
#[test]
fn issue3221_02_same_shape_without_leading_exec() {
    let dir = test_dir("02");
    write_producer(&dir);
    write_inner(&dir, "A");
    let output = run_outer(&dir);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[1..2\nsuite f1\nbegin 1 t1\nok 1 t1 in 10ms] TAP:[] REPORT:[1..2|suite f1|begin 1 t1|ok 1 t1 in 10ms|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = fs::read_to_string(dir.join("tap.log")).unwrap_or_default();
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let report = fs::read_to_string(dir.join("report.xml")).unwrap_or_default();
    assert_eq!(
        report, "1..2\nsuite f1\nbegin 1 t1\nok 1 t1 in 10ms\n",
        "report.xml: {report:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// 03 — control: the same outer shape with the procsub removed
// from the inner pipeline (`tee | cat > report.xml`). With no
// fd-3 copy, nothing reaches stdout: OUT is empty and the lines
// land only in report.xml. Green on base and head.
#[test]
fn issue3221_03_same_shape_without_procsub_flows_on_base() {
    let dir = test_dir("03");
    write_producer(&dir);
    fs::write(
        dir.join("inner.sh"),
        "#!/usr/bin/env bash\ntee_fn() { tee | cat > report.xml; }\nexec ./producer.sh | tee_fn | cat\n",
    )
    .expect("write inner");
    let output = run_outer(&dir);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[] TAP:[] REPORT:[1..2|suite f1|begin 1 t1|ok 1 t1 in 10ms|]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = fs::read_to_string(dir.join("tap.log")).unwrap_or_default();
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let report = fs::read_to_string(dir.join("report.xml")).unwrap_or_default();
    assert_eq!(
        report, "1..2\nsuite f1\nbegin 1 t1\nok 1 t1 in 10ms\n",
        "report.xml: {report:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn run_single_process(dir: &Path, script: &str) -> std::process::Output {
    let path = dir.join("single.sh");
    fs::write(&path, script).expect("write single");
    let mut command = rubash_command();
    command.current_dir(dir).arg(shell_test_path(&path));
    command.output().expect("run single")
}

// 04 — V1 (rework): the `>(...)` consumer in a DIRECT comsub pipeline,
// no function stage (`out=$(exec 3<&1; printf 'hello\n' | tee >(cat
// >&3))`). Same mechanism family as 01/02 (fd 3 rebound by `exec 3<&1`
// must reach the consumer as the capture pipe). Root cause (rework):
// the assignment materialization pre-claimed the `>(...)` nested inside
// the `$(...)` — the embedded procsub scan was nesting-blind — so the
// body ran at the outer assignment's finish via the persistent route
// (`execute_persistent_output_process_substitution`, trap_exec.rs:6170),
// where the capture is already down and fd 3 is the ambient tap.log.
// The fix stops the outer word claiming comsub-nested spans; the clone's
// stage machinery then materializes it via the spawn route (already
// stamped). GNU 5.3.20: OUT carries hello twice, tap.log empty.
// Single-process shape, as measured in variant-v1.sh (three independent
// head builds consistently red).
#[test]
fn issue3221_04_direct_comsub_tee_procsub_consumer_reaches_capture_pipe() {
    let dir = test_dir("04");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=$(exec 3<&1; printf 'hello\\n' | tee >(cat >&3))\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello\nhello] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = fs::read_to_string(dir.join("tap.log")).unwrap_or_default();
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}

// 05 — V3 (rework): `>(...)` as a REDIRECT target, not a tee argument
// (`out=$(exec 3<&1; printf 'hello\n' > >(cat >&3))`). Same scan bug
// as 04: the redirect target is nested inside the `$(...)` in the
// assignment word, so the outer assignment materialization pre-claimed
// it; once un-claimed, the clone's redirect-target arms materialize it
// via the spawn route (same stamped transport as 04). GNU 5.3.20: OUT
// carries hello once, log empty. Single-process shape, as in
// variant-v3.sh.
#[test]
fn issue3221_05_redirect_target_procsub_consumer_reaches_capture_pipe() {
    let dir = test_dir("05");
    let output = run_single_process(
        &dir,
        "exec 3>tap.log\nout=$(exec 3<&1; printf 'hello\\n' > >(cat >&3))\necho \"OUT:[${out}] TAP:[$(cat tap.log | tr '\\n' '|')]\"\n",
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OUT:[hello] TAP:[]\n",
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let tap = fs::read_to_string(dir.join("tap.log")).unwrap_or_default();
    assert_eq!(tap, "", "tap.log: {tap:?}");
    let _ = fs::remove_dir_all(&dir);
}
