//! rubash#2733 (B5-16) — histexpand (`H` in `$-`) is seeded by the
//! interactive INVOCATION, not by the interactive reader, so the
//! `-i -c` and `-i script` routes carry it too. GNU shell.c:
//! 1829-1842 `init_interactive` sets the interactive state wholesale
//! and shell.c:1839 `histexp_flag = history_expansion;`;
//! bashhist.c:288 (`bash_history_enable`): for an interactive shell
//! `history_expansion = HISTEXPAND_DEFAULT` regardless of the input
//! route.
//!
//! Measured against GNU bash 5.3.20 (setsid, no controlling tty):
//! | form | GNU `$-` |
//! |---|---|
//! | `-i -c 'echo $-'` | hiBHc |
//! | `-i script` | hiBH |
//! | `-i` stdin pipe | hiBHs |
//! | `+H -i -c` (explicit opt-out) | hiBc |
//! | `-c` (control) | hBc |
//! | `-i -c 'set +H; echo $-'` | hiBc |
//!
//! The piped-route behavioral cell proves the flag is not cosmetic:
//! `!!` re-executes the previous command line on the reader route.
//!
//! Harness shape: same setsid/no-ctty + cleared-env + scratch-HOME
//! shape as issue2731.

use std::io::Write;
use std::process::{Command, Stdio};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2733-{tag}-{}-{}",
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

fn set_o_value(dump: &str, name: &str) -> Option<String> {
    dump.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(n), Some(v), None) if n == name => Some(v.to_string()),
            _ => None,
        }
    })
}

#[test]
fn interactive_command_route_has_histexpand() {
    let home = scratch_dir("dashc");
    let (stdout, stderr, _) = rubash_no_tty(&["-i", "-c", "echo FLAGS=$-; set -o"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains("FLAGS=hiBHc"),
        "GNU: -i -c carries H, got stdout: {stdout:?} stderr: {stderr:?}"
    );
    assert_eq!(
        set_o_value(&stdout, "histexpand").as_deref(),
        Some("on"),
        "GNU: `set -o histexpand` is on for -i -c, got stdout: {stdout:?}"
    );
}

#[test]
fn interactive_script_route_has_histexpand() {
    let home = scratch_dir("script");
    let script = home.join("probe.sh");
    std::fs::write(&script, "echo FLAGS=$-\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, _) = rubash_no_tty(&["-i", &script_arg], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        stdout.trim(),
        "FLAGS=hiBH",
        "GNU: -i script carries H, stderr: {stderr:?}"
    );
}

#[test]
fn piped_interactive_histexpand_control() {
    let home = scratch_dir("pipe");
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], b"echo FLAGS=$-\nexit\n", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        stdout.contains("FLAGS=hiBHs"),
        "GNU: the reader route carries H, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

#[test]
fn piped_interactive_history_expansion_fires() {
    // Behavioral chain: the seeded H must reach actual expansion —
    // `!!` re-runs the previous line, so the marker prints twice.
    let home = scratch_dir("bang");
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], b"echo HISTTARGET9\n!!\nexit\n", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        stdout.matches("HISTTARGET9").count(),
        2,
        "GNU: `!!` expands on the interactive reader route, got stdout: {stdout:?} stderr: {stderr:?}"
    );
}

#[test]
fn histexpand_optout_controls() {
    let home = scratch_dir("optout");
    // Explicit +H on the command line beats the interactive default.
    let (stdout, stderr, _) = rubash_no_tty(&["+H", "-i", "-c", "echo FLAGS=$-"], b"", &home);
    assert_eq!(
        stdout.trim(),
        "FLAGS=hiBc",
        "GNU: explicit +H survives -i, stderr: {stderr:?}"
    );
    // Non-interactive shells never get H by default.
    let (stdout, stderr, _) = rubash_no_tty(&["-c", "echo FLAGS=$-"], b"", &home);
    assert_eq!(
        stdout.trim(),
        "FLAGS=hBc",
        "GNU: plain -c has no H, stderr: {stderr:?}"
    );
    // An explicit `set +H` inside the session also survives.
    let (stdout, stderr, _) = rubash_no_tty(&["-i", "-c", "set +H; echo FLAGS=$-"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(
        stdout.trim(),
        "FLAGS=hiBc",
        "GNU: set +H drops H on the -i -c route, stderr: {stderr:?}"
    );
}
