//! Issue unixwin/rubash#2473: a procsub argv operand delivered through
//! the #1903 fifo-marker rendezvous was read by the consumer as the 7
//! literal `!<fifo>` marker bytes — `paste <(echo 1) <(echo x)` printed
//! `!<fifo>\t!<fifo>` where GNU prints `1\tx`.
//!
//! Root cause (see `procsub_operand_delivery`): the marker route was
//! granted on binary identity alone ("the consumer IS the WinuxCmd
//! runtime, so its reads bridge markers"). Measured against WinuxCmd
//! 1.1.20 that prediction is false on the READ side — its own `od`
//! does not rendezvous with its own `mkfifo` marker, and as rubash
//! consumers paste/wc/od/comm read the marker bytes plainly. GNU's
//! contract is per-occurrence (subst.c:11364-11385): every `<(...)`
//! word gets its own pipe (subst.c:6381) under its own /dev/fd name
//! (subst.c:6392-6394), and whatever consumer opens the name receives
//! the producer's bytes — so the default is now the byte-correct drain
//! for EVERY consumer, with the marker rendezvous re-armed only by the
//! machine-level proof key RUBASH_PROCSUB_FIFO_MARKER=1.
//!
//! The deterministic pins below need no WinuxCmd install: a stand-in
//! consumer of the aware CLASS is built by hard-linking the rubash
//! binary under a fake WinuxCmd tree as both `winuxcmd.exe` and
//! `paste.exe` (`winuxcmd_fifo_marker_aware` is hardlink identity), and
//! invoked as `paste -c 'cat "$0"'` so the child reads the delivered
//! operand path verbatim. The real-WinuxCmd pins run only when the
//! runtime is on PATH (the byte shapes the issue reproduced with).

#![cfg(windows)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash2473-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn rubash_c_env(script: &str, dir: &Path, env_extra: &[(String, String)]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .args(["-c", script])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env_extra {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

/// A fake WinuxCmd tree whose `usr\bin` holds hard links of the rubash
/// binary as `winuxcmd.exe` and `paste.exe`: `paste` then resolves to
/// the aware class (same_windows_file hardlink identity, WINUXCMD_HOME
/// under-tree) while the child is a programmable rubash `-c` reader.
struct StandinTree {
    root: PathBuf,
}

impl StandinTree {
    fn build(tag: &str) -> Option<StandinTree> {
        // Same volume as the rubash binary: a hard link cannot cross
        // volumes, and the binary lives in the build target dir.
        let exe = PathBuf::from(env!("CARGO_BIN_EXE_rubash"));
        let root = exe
            .parent()?
            .join(format!("rubash2473-standin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bin = root.join("usr").join("bin");
        std::fs::create_dir_all(&bin).ok()?;
        std::fs::hard_link(&exe, bin.join("winuxcmd.exe")).ok()?;
        std::fs::hard_link(&exe, bin.join("paste.exe")).ok()?;
        Some(StandinTree { root })
    }

    fn bin(&self) -> PathBuf {
        self.root.join("usr").join("bin")
    }

    fn env(&self) -> Vec<(String, String)> {
        let path = format!(
            "{};{}",
            self.bin().to_string_lossy(),
            std::env::var("PATH").unwrap_or_default()
        );
        vec![
            ("PATH".to_string(), path),
            (
                "WINUXCMD_HOME".to_string(),
                self.root.to_string_lossy().into_owned(),
            ),
        ]
    }
}

impl Drop for StandinTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Where the PATH-visible `paste` lives, if anywhere (the real-WinuxCmd
/// pins skip when the runtime is not installed).
fn resolved_paste() -> Option<PathBuf> {
    let run = rubash_c_env("command -v paste", &std::env::temp_dir(), &[]);
    let line = run.stdout.trim_end().to_string();
    if line.is_empty() {
        return None;
    }
    Some(PathBuf::from(line))
}

/// The default delivery for an aware-class consumer is the byte-correct
/// drain: the stand-in consumer reads `one`, not the marker. Pre-fix
/// this printed `!<fifo>` (the #2473 reproducer, single operand).
#[test]
fn issue2473_aware_class_consumer_gets_producer_bytes_by_default() {
    let Some(tree) = StandinTree::build("default") else {
        panic!("could not build the stand-in WinuxCmd tree");
    };
    let dir = tempdir("default-run");
    let run = rubash_c_env("paste -c 'cat \"$0\"' <(echo one)", &dir, &tree.env());
    assert_eq!(run.stdout, "one\n", "stderr: {}", run.stderr);
    assert_eq!(run.code, Some(0));
}

/// The multi-operand shape from the issue: every operand is its own
/// substitution (subst.c:11364-11385), so every operand must carry its
/// own producer's bytes — no `!<fifo>` anywhere.
#[test]
fn issue2473_two_operands_carry_their_own_bytes() {
    let Some(tree) = StandinTree::build("two") else {
        panic!("could not build the stand-in WinuxCmd tree");
    };
    let dir = tempdir("two-run");
    let run = rubash_c_env(
        "paste -c 'cat \"$0\"; cat \"$1\"' <(echo 1) <(echo x)",
        &dir,
        &tree.env(),
    );
    assert_eq!(run.stdout, "1\nx\n", "stderr: {}", run.stderr);
    assert!(!run.stdout.contains("!<fifo>"));
}

/// Three procsub operands — the per-occurrence assembly scales past two.
#[test]
fn issue2473_three_operands_carry_their_own_bytes() {
    let Some(tree) = StandinTree::build("three") else {
        panic!("could not build the stand-in WinuxCmd tree");
    };
    let dir = tempdir("three-run");
    let run = rubash_c_env(
        "paste -c 'cat \"$0\"; cat \"$1\"; cat \"$2\"' <(echo 1) <(echo 2) <(echo 3)",
        &dir,
        &tree.env(),
    );
    assert_eq!(run.stdout, "1\n2\n3\n", "stderr: {}", run.stderr);
}

/// rubash#2473 re-arm route: with the machine-level proof key the
/// marker rendezvous IS the delivery for the aware class — proven from
/// both sides of the protocol. A non-bridging reader (`cmd /c type`,
/// which opens the operand plainly) then sees the 7 marker bytes, so
/// the delivery really is the marker path (the drain would show real
/// bytes); a bridging reader (rubash's own cat, whose open layer
/// rendezvouses with the pump) receives the producer's `one` through
/// the live pipe, so the armed machinery still works end-to-end.
#[test]
fn issue2473_proof_key_rearms_the_marker_rendezvous() {
    let Some(tree) = StandinTree::build("proof") else {
        panic!("could not build the stand-in WinuxCmd tree");
    };
    let dir = tempdir("proof-run");
    let mut env = tree.env();
    env.push(("RUBASH_PROCSUB_FIFO_MARKER".to_string(), "1".to_string()));

    let run = rubash_c_env("paste -c 'cmd /c type \"$0\"' <(echo one)", &dir, &env);
    assert_eq!(run.stdout, "!<fifo>", "stderr: {}", run.stderr);

    let run = rubash_c_env("paste -c 'cat \"$0\"' <(echo one)", &dir, &env);
    assert_eq!(run.stdout, "one\n", "stderr: {}", run.stderr);
}

/// The proof key never rescues a consumer outside the runtime class: a
/// System32 tool drains to real bytes with or without the key.
#[test]
fn issue2473_proof_key_never_reaches_non_runtime_consumers() {
    let dir = tempdir("findstr");
    let run = rubash_c_env(
        "findstr /R one <(echo one)",
        &dir,
        &[("RUBASH_PROCSUB_FIFO_MARKER".to_string(), "1".to_string())],
    );
    assert_eq!(run.stdout, "one\n", "stderr: {}", run.stderr);
    assert!(!run.stdout.contains("!<fifo>"));
}

/// Single-operand no-regression: the in-shell consumer face keeps its
/// bytes (this shape was green before #2473 and must stay green).
#[test]
fn issue2473_single_operand_internal_consumer_unchanged() {
    let dir = tempdir("internal");
    let run = rubash_c_env("cat <(echo one)", &dir, &[]);
    assert_eq!(run.stdout, "one\n", "stderr: {}", run.stderr);
}

/// Real-runtime pins (run only when a WinuxCmd `paste` is on PATH — the
/// byte shapes the issue reproduced with; CI without the runtime skips).
#[test]
fn issue2473_real_winuxcmd_operands_carry_producer_bytes() {
    let Some(paste) = resolved_paste() else {
        eprintln!("skipping: no paste on PATH");
        return;
    };
    if !paste.to_string_lossy().to_lowercase().contains("winuxcmd") {
        eprintln!("skipping: paste is not the WinuxCmd runtime: {paste:?}");
        return;
    }
    let dir = tempdir("real");

    // The issue's reproducer, exactly.
    let run = rubash_c_env("paste <(echo 1) <(echo x)", &dir, &[]);
    assert_eq!(run.stdout, "1\tx\n", "stderr: {}", run.stderr);

    // wc counts the producer's bytes (5), not the 7 marker bytes.
    let run = rubash_c_env("wc -c <(printf hello)", &dir, &[]);
    assert!(
        run.stdout.trim_start().starts_with("5 "),
        "wc counted marker bytes: {}",
        run.stdout
    );

    // comm actually compares the two streams (identical lines land in
    // the common column, two leading tabs); with two leaked markers it
    // printed the marker as the common line instead.
    let run = rubash_c_env("comm <(printf 'a\\n') <(printf 'a\\n')", &dir, &[]);
    assert_eq!(run.stdout, "\t\ta\n", "stderr: {}", run.stderr);

    // diff must SEE the difference: two leaked identical markers made
    // GNU-shaped diff report "no difference" (rc 0) — silent wrong data.
    let run = rubash_c_env("diff <(echo a) <(echo b); echo rc=$?", &dir, &[]);
    assert!(
        run.stdout.contains("< a") && run.stdout.contains("> b"),
        "diff lost the streams: {}",
        run.stdout
    );
    assert!(run.stdout.contains("rc=1"), "diff rc: {}", run.stdout);
    assert!(!run.stdout.contains("!<fifo>"));
}
