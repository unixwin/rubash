//! rubash#2731 (B5-03) — interactive monitor mode must default to the
//! job-control ESTABLISHMENT result on every interactive route, not
//! to ON. GNU shell.c:1969 calls jobs.c:4731-4869
//! `initialize_job_control`, which ends (jobs.c:4869) with
//! `change_flag ('m', job_control ? '-' : '+')`: when the terminal
//! cannot be taken, monitor turns OFF (after the jobs.c:4841/4856
//! diagnostics) — including for an explicit `-m`, which does not
//! survive a terminal that cannot be taken.
//!
//! The sibling pin (issue2948) covers the `$-` letter and the
//! diagnostics through the engine's establishment funnel
//! (script_driver.rs `establish_interactive_job_control`). This pin
//! covers the face #2731's repro actually names — the `set -o`
//! monitor STATE — plus the chain the state feeds: background jobs
//! still list/run with monitor off, and `set -m` flips the state
//! (and `$-` follows it) mid-session.
//!
//! Measured against GNU bash 5.3.20 (setsid, no controlling tty):
//! | form | GNU set -o monitor | GNU `$-` |
//! |---|---|---|
//! | `-i` stdin pipe | off | hiBHs |
//! | `-i` pipe, after `set -m` | on | himBHs |
//! | `-i -c` | off | hiBHc |
//! | `-i script` | off | hiBH |
//! | `-i script` under PTY | on | himBH |
//! | `-c` (control) | off | hBc |
//! | `-m -c` no tty (control) | off | hBc |
//!
//! Harness shape (recorded per PTY discipline): no-tty cells pipe
//! all three streams and, on unix, setsid-detach in pre_exec so the
//! child has NO controlling terminal at all on any host. Env is
//! cleared and HOME points at an empty scratch dir so no rc file or
//! inherited carrier can seed the state.

use std::io::Write;
use std::process::{Command, Stdio};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2731-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn rubash_no_tty(
    args: &[&str],
    stdin_bytes: &[u8],
    home: &std::path::Path,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_bytes);
    }
    let out = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Every value `set -o` reports for `name`, in output order (a route
/// may dump the table twice when a chain flips the option midway).
fn set_o_values(dump: &str, name: &str) -> Vec<String> {
    dump.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(n), Some(v), None) if n == name => Some(v.to_string()),
                _ => None,
            }
        })
        .collect()
}

#[test]
fn piped_interactive_monitor_defaults_off() {
    let home = scratch_dir("pipe");
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], b"set -o\necho FLAGS=$-\nexit\n", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string()],
        "GNU: monitor off without a terminal, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("FLAGS=hiBHs"),
        "GNU: hiBHs without a terminal, got stdout: {stdout:?}"
    );
}

#[test]
fn piped_interactive_monitor_chain_jobs_and_set_m() {
    // Chain: the default-off state must not break background jobs,
    // and an explicit `set -m` flips the state (with `$-` following)
    // for the rest of the session.
    let home = scratch_dir("chain");
    let input =
        b"set -o\nsleep 0.3 &\njobs\nset -m\nset -o\necho FLAGS=$-\nwait\necho CHAINDONE\nexit\n";
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], input, &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string(), "on".to_string()],
        "GNU: monitor starts off, `set -m` turns it on, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("Running") && stdout.contains("sleep 0.3"),
        "GNU: `jobs` lists the background job even with monitor off, got stdout: {stdout:?}"
    );
    assert!(
        stdout.contains("FLAGS=himBHs"),
        "GNU: `$-` gains m after `set -m`, got stdout: {stdout:?}"
    );
    assert!(
        stdout.contains("CHAINDONE"),
        "GNU: `wait` reaps the job and the session continues, got stdout: {stdout:?}"
    );
}

#[test]
fn interactive_command_route_monitor_off() {
    let home = scratch_dir("dashc");
    let (stdout, stderr, _) = rubash_no_tty(&["-i", "-c", "set -o; echo FLAGS=$-"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string()],
        "GNU: monitor off on the -i -c route, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("FLAGS=hiBHc"),
        "GNU: hiBHc, got stdout: {stdout:?}"
    );
}

#[test]
fn interactive_script_route_monitor_off() {
    let home = scratch_dir("script");
    let script = home.join("probe.sh");
    std::fs::write(&script, "set -o\necho FLAGS=$-\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, _) = rubash_no_tty(&["-i", &script_arg], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string()],
        "GNU: monitor off on the -i script route, stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("FLAGS=hiBH"),
        "GNU: hiBH, got stdout: {stdout:?}"
    );
}

#[cfg(unix)]
#[test]
fn interactive_monitor_on_under_pty() {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("no script(1) on this host; skipping");
        return;
    }
    let home = scratch_dir("pty");
    let script = home.join("probe.sh");
    std::fs::write(&script, "set -o\necho FLAGS=$-\n").expect("write script");
    let cmdline = format!(
        "{} -i {}",
        env!("CARGO_BIN_EXE_rubash"),
        script.to_string_lossy()
    );
    let out = Command::new("script")
        .args(["-qec", &cmdline, "/dev/null"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &home)
        .env("TERM", "xterm")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn script(1)");
    let _ = std::fs::remove_dir_all(&home);
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        set_o_values(&text, "monitor"),
        vec!["on".to_string()],
        "GNU: monitor on when the terminal can be taken, got transcript: {text:?}"
    );
    assert!(
        text.contains("FLAGS=himBH"),
        "GNU: himBH under a terminal, got transcript: {text:?}"
    );
}

#[test]
fn noninteractive_monitor_controls() {
    let home = scratch_dir("controls");
    let (stdout, stderr, _) = rubash_no_tty(&["-c", "set -o"], b"", &home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string()],
        "GNU: plain -c keeps monitor off, stderr: {stderr:?}"
    );
    // An explicit -m does not survive a terminal that cannot be
    // taken (GNU: two diagnostics, monitor off, `hBc`).
    let (stdout, stderr, _) = rubash_no_tty(&["-m", "-c", "set -o"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        set_o_values(&stdout, "monitor"),
        vec!["off".to_string()],
        "GNU: explicit -m drops without a terminal, stderr: {stderr:?}"
    );
}
