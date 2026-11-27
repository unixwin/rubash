//! rubash#2423 — fork-shaped respawns must survive any function-table
//! size: the `__RUBASH_FORK_FN_SPANS` stamp (rubash#2202) grows ~34+
//! bytes per function definition, so past a few thousand definitions a
//! single env string crosses MAX_ARG_STRLEN (128 KiB) and the spawn
//! dies with E2BIG before the child runs anything.
//!
//! History: the failure was adjudicated from niubash's workspace test
//! (`background_spawn_args::large_function_table_background_command_runs`)
//! against niubash's rubash pin 76f611d5, which predates the fix. The
//! mechanism fix is rubash#2356: every fork-shaped respawn builder
//! (background `&`, coproc, both process-substitution forms, external
//! respawns) runs its composed environment through
//! `Executor::spill_respawn_env_entries`, which moves entries past a
//! size budget — by size class, never by key — into the
//! `__RUBASH_STATE_FILE` channel. The plain background-builtin shape is
//! also pinned in tests/regression.rs (issue_niubash_362 case_1); the
//! pins here add the crossing region and the sibling fork shapes the
//! 362 pin does not cover.
//!
//! Expectations are the transport contract (GNU has no equivalent
//! transport — its forked child inherits the table in memory): a valid
//! script with any function count spawns successfully, the child's
//! output is delivered, and statuses are the ordinary ones.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The 6000-definition runs are heavy (a full parse of the table per
/// run); serialize them inside this file like regression.rs does with
/// its heavy lock.
static HEAVY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn heavy_lock() -> std::sync::MutexGuard<'static, ()> {
    HEAVY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

fn run_script(dir: &Path, script: &str, limit: Duration) -> RunOutcome {
    let path = dir.join("case.sh");
    std::fs::write(&path, script).expect("write script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(windows) {
        for name in [
            "SystemRoot",
            "SystemDrive",
            "WINDIR",
            "COMSPEC",
            "PATHEXT",
            "OS",
            "TEMP",
            "TMP",
        ] {
            if let Ok(value) = std::env::var(name) {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait().expect("poll child") {
            Some(status) => break Some(status),
            None => {
                if start.elapsed() > limit {
                    timed_out = true;
                    let _ = child.kill();
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default()).into_owned();
    RunOutcome {
        code: status.and_then(|s| s.code()),
        stdout,
        stderr,
        timed_out,
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2423-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// The niubash seed's function shape: `local` + a quoted expansion, so
/// each body carries two span-bearing commands.
fn function_table(count: usize) -> String {
    let mut script = String::new();
    for i in 0..count {
        script.push_str(&format!("f{i}() {{ local v={i}; : \"$v\"; }}\n"));
    }
    script
}

fn assert_spawn_survived(out: &RunOutcome) {
    assert!(!out.timed_out, "timed out; stderr:\n{}", out.stderr);
    assert_eq!(out.code, Some(0), "stderr:\n{}", out.stderr);
    assert!(
        !out.stderr.contains("Argument list too long"),
        "spawn hit the per-string env cap; stderr:\n{}",
        out.stderr
    );
}

/// The adjudicated seed: 6000 definitions, then a background builtin.
/// At the pre-#2356 transport this died at the `&` line with E2BIG and
/// the child's output was lost.
#[test]
fn issue2423_background_spawn_at_6000_functions() {
    let _guard = heavy_lock();
    let dir = scratch("bg6000");
    let mut script = function_table(6000);
    script.push_str("echo hello &\nwait\necho large-fn-status:$?\n");
    let out = run_script(&dir, &script, Duration::from_secs(300));
    assert_spawn_survived(&out);
    assert!(
        out.stdout.contains("hello"),
        "background child output missing; stdout:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("large-fn-status:0"),
        "wait status wrong; stdout:\n{}",
        out.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The empirical crossing region: the codec simulation puts the
/// MAX_ARG_STRLEN crossing near ~3,100 definitions; the true crossing
/// measured against the pre-fix transport sits in (3600, 4800]. 4800
/// is past either figure, so this pin bites on the old transport and
/// must stay green on the state-channel transport.
#[test]
fn issue2423_background_spawn_crossing_region_4800() {
    let _guard = heavy_lock();
    let dir = scratch("bg4800");
    let mut script = function_table(4800);
    script.push_str("echo hello &\nwait\necho large-fn-status:$?\n");
    let out = run_script(&dir, &script, Duration::from_secs(300));
    assert_spawn_survived(&out);
    assert!(
        out.stdout.contains("hello"),
        "background child output missing; stdout:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("large-fn-status:0"),
        "wait status wrong; stdout:\n{}",
        out.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Control: a small table takes the same path with no spill pressure.
#[test]
fn issue2423_background_spawn_small_table_control() {
    let dir = scratch("bg25");
    let mut script = function_table(25);
    script.push_str("echo hello &\nwait\necho large-fn-status:$?\n");
    let out = run_script(&dir, &script, Duration::from_secs(120));
    assert_spawn_survived(&out);
    assert!(
        out.stdout.contains("hello"),
        "background child output missing; stdout:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("large-fn-status:0"),
        "wait status wrong; stdout:\n{}",
        out.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The class beyond the seed: a background FUNCTION call (the child
/// needs the transported function table itself, not just the span
/// stamp) and a process-substitution consumer, both at 6000
/// definitions. `wait` orders FN-RAN before the procsub output.
#[test]
fn issue2423_background_function_and_procsub_at_6000_functions() {
    let _guard = heavy_lock();
    let dir = scratch("shapes6000");
    let mut script = function_table(6000);
    script.push_str("probe_fn() { echo FN-RAN; }\n");
    script.push_str("probe_fn &\nwait\n");
    script.push_str("cat <(echo PROC)\n");
    script.push_str("echo shapes-status:$?\n");
    let out = run_script(&dir, &script, Duration::from_secs(300));
    assert_spawn_survived(&out);
    assert!(
        out.stdout.contains("FN-RAN"),
        "background function output missing; stdout:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("PROC"),
        "procsub output missing; stdout:\n{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("shapes-status:0"),
        "final status wrong; stdout:\n{}",
        out.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The coproc builder composes its respawn environment separately
/// from the `&` builder; pin it at the same table size.
#[test]
fn issue2423_coproc_at_6000_functions() {
    let _guard = heavy_lock();
    let dir = scratch("coproc6000");
    let mut script = function_table(6000);
    script.push_str("coproc { echo COP; }\n");
    script.push_str("read -r line <&${COPROC[0]}\n");
    script.push_str("echo \"got:$line\"\n");
    let out = run_script(&dir, &script, Duration::from_secs(300));
    assert_spawn_survived(&out);
    assert!(
        out.stdout.contains("got:COP"),
        "coproc output missing; stdout:\n{}",
        out.stdout
    );
    let _ = std::fs::remove_dir_all(&dir);
}
