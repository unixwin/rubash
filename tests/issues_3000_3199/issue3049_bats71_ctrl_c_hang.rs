//! Regression pins for rubash#3049 — bats-core's own suite test #71
//! ("Don't hang on CTRL-C (issue #353)") hangs forever under rubash:
//! the inner suite's test registration dies with
//! `BATS_TEST_TAGS: readonly variable`, the fixture never signals
//! its latch, and the outer test waits forever.
//!
//! Mechanism (captain-lane instrumentation of a bats tree copy,
//! 2026-11-09): the only `readonly BATS_TEST_TAGS` in bats is
//! bats-exec-test:380, which runs AFTER the registration assignment
//! (lib/bats-core/test_functions.bash:531, during source
//! evaluation). In the failing run the INNER bats-exec-test process
//! is *born* with `declare -ar BATS_TEST_TAGS` already in its
//! variable table — before its first line executes — and its born
//! environment carries 29 variables that exist only as unexported
//! shell variables in the outer session (GNU inner env: 40 entries;
//! rubash inner env: 69; the outer process's env is identical in
//! both shells, 34). GNU gives every descendant a fresh table plus
//! exactly the exported environment; rubash evidently transfers
//! session state — unexported variables and variable attributes —
//! across the spawn boundary somewhere in the perform-context spawn
//! chain (outer test body -> bats -> suite -> file -> exec-test).
//! Simple parent->child channels do NOT carry it (probed: plain /
//! function-mediated / background+set -m / three-generation spawns
//! all stay clean), which is why this only bites deep real-world
//! chains like bats'.
//!
//! GNU ownership: a child process's initial variable table is the
//! exported environment and nothing else (execve); attributes such
//! as readonly are process-local shell state (variables.c) and can
//! never cross an exec boundary.
//!
//! Cell A drives the real bats chain end-to-end. It needs a
//! bats-core checkout: set RUBASH_BATS_TREE or have one at a known
//! workspace location; without one the cell skips (it still runs
//! wherever the eco fixtures live). Cell B locks the simple spawn
//! channels that were probed clean, so a future widening of the
//! carriage turns this suite red even without a bats tree.
#![cfg(unix)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3049-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn bats_tree() -> Option<PathBuf> {
    if let Ok(tree) = std::env::var("RUBASH_BATS_TREE") {
        let p = PathBuf::from(tree);
        if p.join("libexec/bats-core/bats-exec-test").is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;
    for cand in ["workspace/eco-rerun-2901-release/work/tree"] {
        let p = Path::new(&home).join(cand);
        if p.join("libexec/bats-core/bats-exec-test").is_file() {
            return Some(p);
        }
    }
    None
}

/// SIGKILL any leftover process whose cmdline mentions `needle`
/// (the scratch tree path): the hung outer test leaves orphans in
/// their own process groups that a plain child kill cannot reach.
fn sweep_orphans(needle: &str) {
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            if let Ok(cmdline) = std::fs::read(entry.path().join("cmdline")) {
                let text = String::from_utf8_lossy(&cmdline);
                if text.contains(needle) {
                    if let Ok(pid) = name.parse::<i32>() {
                        unsafe {
                            libc_kill(pid);
                        }
                    }
                }
            }
        }
    }
}

// Minimal libc kill binding (the crate has no libc dev-dependency
// guarantee in tests; declare the symbol directly).
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}
unsafe fn libc_kill(pid: i32) {
    kill(pid, 9);
}

#[test]
fn bats71_nested_registration_survives_and_the_test_completes() {
    let Some(tree) = bats_tree() else {
        eprintln!("no bats-core tree available (set RUBASH_BATS_TREE); skipping cell A");
        return;
    };
    let root = scratch("chain");
    // Work on a copy: the run writes suite tmpdirs into the tree's
    // own state files in some bats versions, and the fixture tree
    // must stay pristine for other lanes.
    let copy = root.join("tree");
    let status = Command::new("cp")
        .args(["-r", &tree.to_string_lossy(), &copy.to_string_lossy()])
        .status()
        .expect("copy bats tree");
    assert!(status.success(), "cp -r of bats tree failed");

    // Extract test #71 alone (plus the file's setup preamble) so the
    // run reaches the nested chain in seconds instead of gathering
    // all 146 tests of test/bats.bats.
    let bats_bats = std::fs::read_to_string(copy.join("test/bats.bats")).expect("read bats.bats");
    let start = bats_bats
        .find("@test \"Don't hang on CTRL-C (issue #353)\"")
        .expect("test #71 present in bats.bats");
    let block = &bats_bats[start..];
    let mut lines = Vec::new();
    for line in block.lines() {
        lines.push(line);
        if line == "}" {
            break;
        }
    }
    let mini = format!(
        "#!/usr/bin/env bats\n\nbats_require_minimum_version 1.5.0\n\nsetup() {{\n  load test_helper\n  fixtures bats\n  REENTRANT_RUN_PRESERVE+=(BATS_LIBEXEC)\n}}\n\n{}\n",
        lines.join("\n")
    );
    std::fs::write(copy.join("test/mini71.bats"), mini).expect("write mini71.bats");

    // Shim: `bash` resolves to the rubash under test, exactly like
    // the eco harness (bats' scripts all use `#!/usr/bin/env bash`).
    let shim = root.join("shim");
    std::fs::create_dir_all(&shim).expect("shim dir");
    std::os::unix::fs::symlink(BIN, shim.join("bash")).expect("shim symlink");
    let path = format!(
        "{}:{}:/usr/local/bin:/usr/bin:/bin",
        shim.display(),
        copy.join("bin").display()
    );

    let mut child = Command::new("timeout")
        .args(["-k", "10", "240", "bats", "test/mini71.bats"])
        .current_dir(&copy)
        .env_clear()
        .env("PATH", &path)
        .env("HOME", std::env::var("HOME").unwrap_or_default())
        .env("TERM", "xterm")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bats run");

    let deadline = Instant::now() + Duration::from_secs(280);
    let status = loop {
        match child.try_wait().expect("poll bats run") {
            Some(status) => break status,
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("bats run exceeded the 280s harness deadline");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let mut out = String::new();
    let mut err = String::new();
    if let Some(mut so) = child.stdout.take() {
        let _ = so.read_to_string(&mut out);
    }
    if let Some(mut se) = child.stderr.take() {
        let _ = se.read_to_string(&mut err);
    }
    sweep_orphans(&copy.to_string_lossy());

    assert!(
        !err.contains("readonly variable") && !out.contains("readonly variable"),
        "inner registration died on a carried readonly attribute:\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(
        status.success(),
        "bats #71 must complete with rc=0 (GNU: ok, ~4s); got {status:?}:\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(
        out.contains("ok 1 Don't hang on CTRL-C (issue #353)"),
        "expected the ok line for test #71:\nstdout:\n{out}\nstderr:\n{err}"
    );
}

/// Cell B (always runs): the direct spawn channels stay clean — a
/// child sees only the exported environment, and a name the parent
/// made readonly is a fresh writable variable in the child.
#[test]
fn direct_child_channels_do_not_carry_attributes_or_unexported_vars() {
    let root = scratch("direct");
    let write = |name: &str, body: &str| {
        let p = root.join(name);
        std::fs::write(&p, body).expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fixture");
        }
        p
    };
    let child = write(
        "child.sh",
        &format!(
            "#!{BIN}\necho \"env-marker=$(env | grep -c '^MARKER=' || true)\"\nARR=(z)\necho \"child-arr=${{ARR[*]}}\"\n"
        ),
    );
    let parent = write(
        "parent.sh",
        &format!(
            "#!{BIN}\nMARKER=x\nARR=(a b)\nreadonly ARR\nf() {{ \"{}\"; }}\nf\n",
            child.display()
        ),
    );
    let out = Command::new(BIN)
        .arg(&parent)
        .current_dir(&root)
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .expect("run parent");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(stdout, "env-marker=0\nchild-arr=z\n");
}
