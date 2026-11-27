//! rubash#2956 (Stage-2 ④ Phase 3) — one mechanism in three
//! builtins: the numeric argument's VALIDITY was checked after the
//! surplus-operand (no_args) check, but GNU validates first.
//! `get_numeric_arg` (builtins/common.c:488-520; shift.def,
//! break.def/continue.def) and `get_exitstat` (builtins/common.c:
//! 525+; return.def) run `valid_number` (general.c:248) on the
//! first word BEFORE the no_args surplus tail, so `shift x y`
//! reports the numeric error (and, for shift/return, does NOT take
//! the DISCARD class), while a valid head with surplus still
//! reports too-many-arguments WITH the discard. Expectations are
//! byte-exact GNU bash 5.3.20 outputs (Phase 3 E1 differential).

use std::path::PathBuf;
use std::process::Command;

struct Run {
    status: i32,
    stdout: String,
    stderr: String,
    dir: PathBuf,
}

fn run_script(body: &str) -> Run {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("issue2956_{}_{}", std::process::id(), seq));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    std::fs::write(dir.join("case.sh"), body).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        status: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        dir,
    }
}

#[test]
fn shift_invalid_head_reports_numeric_not_too_many() {
    let run = run_script("set -- a b c d\nshift x y\necho \"rc=$? n=$# rest=$*\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(
        run.stdout, "rc=2 n=4 rest=a b c d\n",
        "stdout: {}",
        run.stdout
    );
    assert_eq!(
        run.stderr, "case.sh: line 2: shift: x: numeric argument required\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn shift_invalid_head_does_not_discard_the_list() {
    // neednumarg with fatal=0 returns; the eval body runs on.
    let run = run_script("set -- a b c d\neval 'shift x; echo INNER'\necho OUTER\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "INNER\nOUTER\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 2: shift: x: numeric argument required\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn shift_valid_head_with_surplus_still_discards() {
    // no_args (common.c:511): too-many + DISCARD — the eval body
    // is abandoned, the outer list continues.
    let run = run_script("set -- a b c d\neval 'shift 1 2; echo INNER'\necho OUTER\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "OUTER\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 2: shift: too many arguments\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn return_invalid_head_reports_numeric_not_too_many() {
    let run = run_script("f() { return x y; echo in-func-after; }\nf\necho \"rc=$? after\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=2 after\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 1: return: x: numeric argument required\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn return_valid_head_with_surplus_reports_too_many() {
    let run = run_script("f() { return 1 2; echo in-func-after; }\nf\necho \"rc=$? after\"\n");
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "rc=2 after\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 1: return: too many arguments\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn break_invalid_head_is_numeric_and_exits_the_shell() {
    // break.def: get_numeric_arg(list, interactive ? 2 : 1) — a
    // non-interactive invalid count is EXITPROG: the script dies
    // with status 2 at the builtin.
    let run = run_script(
        "for i in 1 2 3; do echo iter$i; break x y; echo after; done\necho \"rc=$? done\"\n",
    );
    assert_eq!(run.status, 2, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "iter1\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 1: break: x: numeric argument required\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}

#[test]
fn break_valid_head_with_surplus_reports_too_many_and_discards() {
    let run = run_script(
        "for i in 1 2 3; do echo iter$i; break 1 2; echo after; done\necho \"rc=$? done\"\n",
    );
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, "iter1\nrc=2 done\n", "stdout: {}", run.stdout);
    assert_eq!(
        run.stderr, "case.sh: line 1: break: too many arguments\n",
        "stderr: {}",
        run.stderr
    );
    let _ = run.dir;
}
