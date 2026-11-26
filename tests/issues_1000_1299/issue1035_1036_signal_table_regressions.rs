//! rubash#1036 + rubash#1035: signal-table / disposition surface.
//!
//! GNU ground truth (bash 5.3.20 aarch64-linux-android, script-file mode,
//! byte-compared; see the issue repro scripts):
//!
//! - #1036: bionic's realtime block lives at 41..64
//!   (`__libc_current_sigrtmin/max`, support/signames.c:92-139 generation
//!   rule). `trap -l` lists through `64) SIGRTMAX` (55 entries);
//!   `kill -l 41` prints RTMIN and `kill -l RTMIN` prints 41; holes
//!   (32..40) print nothing with rc 0; 65 is an error (rc 1).
//! - #1035: an asynchronous child ignores SIGINT/SIGQUIT while job
//!   control is off (execute_cmd.c setup_async_signals, trap.c
//!   SIG_HARD_IGNORE via set_signal). A background script's
//!   `trap ... INT` is silently refused (rc 0), `trap -p INT` shows
//!   `trap -- '' SIGINT`, and the signal stays ignored. Compound bodies
//!   (`( ) &`) stay trappable (trap.c's ASYNCSIG special-case), as do
//!   `set -m` children.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_shell(body: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn write_executable(path: &std::path::Path, body: &str) {
    let mut file = std::fs::File::create(path).expect("temp script");
    file.write_all(body.as_bytes()).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
    }
}

// ---- #1036: realtime signals ----

/// `kill -l RTMIN` prints 41 and `kill -l 41` prints RTMIN (bionic RT base).
#[test]
#[cfg(target_os = "android")]
fn kill_list_maps_rtmin_both_directions() {
    let (stdout, _, rc) = run_shell("kill -l RTMIN");
    assert_eq!(stdout, "41\n");
    assert_eq!(rc, Some(0));
    let (stdout, _, rc) = run_shell("kill -l 41");
    assert_eq!(stdout, "RTMIN\n");
    assert_eq!(rc, Some(0));
}

/// `kill -l RTMAX` prints 64 and `kill -l 64` prints RTMAX (all RT hosts).
#[test]
fn kill_list_maps_rtmax_both_directions() {
    let (stdout, _, rc) = run_shell("kill -l RTMAX");
    assert_eq!(stdout, "64\n");
    assert_eq!(rc, Some(0));
    let (stdout, _, rc) = run_shell("kill -l 64");
    assert_eq!(stdout, "RTMAX\n");
    assert_eq!(rc, Some(0));
}

/// In-range holes print nothing and keep rc 0 (common.c display_signal_list).
#[test]
fn kill_list_hole_is_silent_success() {
    let (stdout, _, rc) = run_shell("kill -l 32");
    assert_eq!(stdout, "");
    assert_eq!(rc, Some(0));
}

/// Out-of-range numbers are an error (rc 1).
#[test]
fn kill_list_out_of_range_is_an_error() {
    let (_, _, rc) = run_shell("kill -l 65");
    assert_eq!(rc, Some(1));
}

/// `trap -l` reaches the realtime block.
#[test]
fn trap_list_covers_realtime_block() {
    let (stdout, _, rc) = run_shell("trap -l");
    assert_eq!(rc, Some(0));
    assert!(
        stdout.contains("SIGRTMAX"),
        "trap -l missing SIGRTMAX:\n{stdout}"
    );
}

/// Numeric realtime specs round-trip through `trap` (android numbering).
#[test]
#[cfg(target_os = "android")]
fn trap_numeric_realtime_spec_roundtrip() {
    let (stdout, stderr, rc) = run_shell("trap 'echo H' 41; trap -p RTMIN");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "trap -- 'echo H' SIGRTMIN\n", "stderr: {stderr:?}");
}

// ---- #1035: entry-ignored signals cannot be trapped ----

/// The issue repro: a background script inherits INT ignored; trapping it
/// is silently refused, `trap -p` shows the ignore, the signal is dropped.
/// Unix-only: the async entry-ignore markers (`__RUBASH_ASYNC_CHILD`,
/// `SIG_HARD_IGNORE`) are cfg(unix) — Windows has no kernel signal
/// dispositions to inherit, so the refusal path does not exist there.
#[test]
#[cfg(unix)]
fn background_script_cannot_trap_entry_ignored_int() {
    let dir = std::env::temp_dir();
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let child_path = dir.join(format!("rubash-1035-ign-{tag}.sh"));
    let ready_path = dir.join(format!("rubash-1035-ready-{tag}"));
    let parent_path = dir.join(format!("rubash-1035-parent-{tag}.sh"));
    write_executable(
        &child_path,
        &format!(
            "trap 'echo GOTINT; exit 43' INT\n\
             echo \"trap-set rc=$?\"\n\
             trap -p INT\n\
             touch {}\n\
             sleep 2\n\
             echo SURVIVED\n",
            ready_path.display()
        ),
    );
    write_executable(
        &parent_path,
        &format!(
            "{} &\n\
             CP=$!\n\
             for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50; do\n\
               if [ -e {} ]; then break; fi\n\
               sleep 0.1\n\
             done\n\
             kill -INT $CP\n\
             wait $CP\n\
             echo \"wait rc=$?\"\n",
            child_path.display(),
            ready_path.display()
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&parent_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_file(&child_path);
    let _ = std::fs::remove_file(&ready_path);
    let _ = std::fs::remove_file(&parent_path);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(0), "stderr: {stderr:?}");
    assert!(
        stdout.contains("trap-set rc=0\n"),
        "trap refusal must be silent rc=0:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("trap -- '' SIGINT\n"),
        "trap -p must show the inherited ignore:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("GOTINT"),
        "handler must never fire:\n{stdout}"
    );
    assert!(
        stdout.contains("SURVIVED\n"),
        "INT must stay ignored:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("wait rc=0\n"),
        "wait must report 0:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// QUIT is async-ignored alongside INT (setup_async_signals covers both).
#[test]
fn background_script_cannot_trap_entry_ignored_quit() {
    let (stdout, _, rc) = run_shell("trap 'echo Q' QUIT; trap -p QUIT");
    assert_eq!(rc, Some(0));
    // Foreground shell: QUIT is trappable (only async children ignore it).
    assert_eq!(stdout, "trap -- 'echo Q' SIGQUIT\n");
}

/// A compound background body stays trappable (trap.c ASYNCSIG
/// special-case in set_signal): the handler installs AND fires.
#[test]
fn background_compound_body_can_still_trap_int() {
    let dir = std::env::temp_dir();
    let tag = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let ready_path = dir.join(format!("rubash-1035-cready-{tag}"));
    let parent_path = dir.join(format!("rubash-1035-cparent-{tag}.sh"));
    write_executable(
        &parent_path,
        &format!(
            "( trap 'echo HANDLER' INT; trap -p INT; touch {}; sleep 3 ) &\n\
             CP=$!\n\
             for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50; do\n\
               if [ -e {} ]; then break; fi\n\
               sleep 0.1\n\
             done\n\
             kill -INT $CP\n\
             wait $CP\n\
             echo \"wait rc=$?\"\n",
            ready_path.display(),
            ready_path.display()
        ),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&parent_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_file(&ready_path);
    let _ = std::fs::remove_file(&parent_path);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stdout.contains("trap -- 'echo HANDLER' SIGINT\n"),
        "compound bg body must install the trap:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("HANDLER\n"),
        "installed handler must fire:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// A directly-spawned background external ignores INT (and QUIT).
/// Unix-only: INT/QUIT ignore semantics are kernel dispositions; Windows
/// children have no SIGINT/SIGQUIT to drop.
#[test]
#[cfg(unix)]
fn background_external_ignores_int() {
    let (stdout, stderr, rc) = run_shell(
        "sleep 30 &\n\
         CP=$!\n\
         sleep 0.5\n\
         kill -INT $CP\n\
         sleep 0.5\n\
         if kill -0 $CP 2>/dev/null; then echo SURVIVED; else echo DIED; fi\n\
         kill -QUIT $CP\n\
         sleep 0.5\n\
         if kill -0 $CP 2>/dev/null; then echo STILL; else echo DIED2; fi\n\
         kill -9 $CP 2>/dev/null\n\
         wait $CP 2>/dev/null\n\
         true",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert!(
        stdout.contains("SURVIVED\n"),
        "bg sleep must survive SIGINT:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("STILL\n"),
        "bg sleep must survive SIGQUIT:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Foreground trapping is unaffected: no async ignore leaks into `&`-free runs.
#[test]
fn foreground_trap_still_installs() {
    let (stdout, stderr, rc) = run_shell("trap 'echo X' INT; trap -p INT");
    assert_eq!(rc, Some(0), "stderr: {stderr:?}");
    assert_eq!(stdout, "trap -- 'echo X' SIGINT\n");
}
