//! Issue rubash#2078: the process-level command parse/resolution cache
//! (executor/path.rs, introduced by rubash#159) was a **single-fingerprint**
//! store shared by every Session in the process. The `hashed` mirror —
//! the in-memory image of each session's user-visible `__RUBASH_HASH_TABLE`
//! — deliberately survives fingerprint resets (rubash#1137), so with two
//! Sessions in one process (the embed API's supported shape, contract Q5):
//!
//! * Session A's `hash -p` entry resolved Session B's commands (B's
//!   `find_user_command` consulted the one global mirror before its PATH
//!   walk), so B executed A's binary, and
//! * Session B's `hash -r` dropped Session A's remembered locations (the
//!   flush funnel cleared the one global mirror), so A's command vanished
//!   with a 127.
//!
//! Fix: the cache is a map of slices keyed by a per-executor session scope
//! token (SESSION_SCOPE_KEY, stamped by Executor::new); every cache entry
//! point routes through the caller's env-derived slice, so each Session
//! keeps the isolated single-fingerprint cache rubash#159 designed. These
//! pins drive two real Sessions through the Tier-C API in one process —
//! the exact shape that cross-contaminated — and need no SERIAL lock or
//! `hash -r` purges: other tests' sessions now own their own slices.
//!
//! The pure resolution slices are LRU-capped (16 scopes); the hash-table
//! mirror is NOT a slice and is exempt from eviction — it is user-visible
//! state, and GNU keeps a shell's hash table for the shell's lifetime.
//! Pin 5 guards that split.
//!
//! Pins 1 and 2 observe command EXECUTION, not `command -v`: the describe
//! face consults the session's own `__RUBASH_HASH_TABLE` first (which no
//! sibling can touch), so only the executor's `find_user_command` route —
//! the one the cache mirror serves — exposes the channel. All expectations
//! are GNU bash semantics for one shell (findcmd.c:367-380
//! search_for_command consults that shell's own hashed_filenames; nothing
//! in GNU shares a hash table between shells).

use std::fs;
use std::path::{Path, PathBuf};

use rubash::api::{RunOptions, Session};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2078-{name}-{}-{}",
        std::process::id(),
        std::time::Instant::now().elapsed().as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn opts() -> RunOptions {
    RunOptions::default()
}

/// The stem the pins resolve. Distinct from every other test's command
/// name so a mirror leak from a parallel test could not mask the
/// cross-session one.
const TOOL: &str = "i2078tool";

/// A runnable `TOOL` under `dir/bin` that appends its `tag` to `count`.
/// Windows needs the `.cmd` suffix for the PATH walk's PATHEXT probe and
/// real execution; Unix takes a shebang script. Only these two faces of
/// the tool matter: resolution (the cache's job) and "which binary ran".
fn make_tool(dir: &Path, tag: &str, count: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    let count_display = count.to_string_lossy().replace('\\', "/");
    let (file, content) = if cfg!(windows) {
        (
            bin_dir.join(format!("{TOOL}.cmd")),
            format!("@echo {tag}>> \"{count_display}\"\r\n"),
        )
    } else {
        (
            bin_dir.join(TOOL),
            format!("#!/bin/sh\necho {tag} >> '{count_display}'\n"),
        )
    };
    fs::write(&file, content).expect("write tool");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).expect("chmod tool");
    }
    file
}

/// One appended line per execution of a tool made by [`make_tool`].
fn count_lines(count: &Path) -> Vec<String> {
    fs::read_to_string(count)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// A session whose PATH is exactly `path`: an explicit PATH assignment
/// stands the fixture namespaces down (path_is_user_owned), so resolution
/// is the pure PATH walk plus the session's own hash table.
fn session(path: &str) -> Session {
    Session::builder()
        .env("PATH", path)
        .build()
        .expect("session builds")
}

/// Execute `TOOL` in `session` and return the exit status (the simple
/// command is the script's last command, so the run outcome is its status).
/// The deliberate post-`hash -r` miss in pin 4 reports 127 on stderr; the
/// redirect keeps the test output clean while the status still asserts it.
fn run_tool(session: &mut Session) -> i32 {
    let outcome = session
        .run(&format!("{TOOL} 2>/dev/null"), &opts())
        .expect("run");
    outcome.status
}

/// Pin 1 — the `hash -p` entry of one session must not resolve another
/// session's command. Before the fix, session B's lookup consulted the
/// process-global `hashed` mirror and executed session A's binary.
#[test]
fn issue2078_hash_p_entry_does_not_leak_into_another_session() {
    let root = temp_dir("leak");
    let count = root.join("count.txt");
    let dir_a = root.join("a");
    let dir_b = root.join("b");
    let tool_a = make_tool(&dir_a, "A", &count);
    let tool_b = make_tool(&dir_b, "B", &count);

    let mut a = session(&dir_a.join("bin").to_string_lossy());
    let mut b = session(&dir_b.join("bin").to_string_lossy());

    // Session A pins its own location for the name.
    let script = format!(
        "hash -p \"{}\" {TOOL}",
        tool_a.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0, "hash -p must succeed");

    // A executes its own pin.
    assert_eq!(run_tool(&mut a), 0);
    assert_eq!(count_lines(&count), ["A"], "A runs A's tool");

    // Session B has no pin; its PATH holds its own tool. Before the fix
    // the global mirror served A's pin here — B ran A's binary.
    assert_eq!(run_tool(&mut b), 0, "B's own tool must run");
    assert_eq!(
        count_lines(&count),
        ["A", "B"],
        "B must execute through its own PATH, not A's hash -p"
    );

    // A still resolves its pin after B's lookups.
    assert_eq!(run_tool(&mut a), 0);
    assert_eq!(count_lines(&count), ["A", "B", "A"]);

    let _ = fs::remove_dir_all(root);
}

/// Pin 2 — one session's `hash -r` must not drop another session's
/// remembered locations. Before the fix the flush funnel cleared the one
/// global mirror, so A's pinned command vanished with a 127 the moment B
/// flushed.
#[test]
fn issue2078_hash_r_in_one_session_keeps_another_sessions_entries() {
    let root = temp_dir("flush");
    let count = root.join("count.txt");
    let dir_a = root.join("a");
    let dir_b = root.join("b");
    fs::create_dir_all(&dir_a).expect("dir a");
    fs::create_dir_all(&dir_b).expect("dir b");
    // The pinned tool lives OUTSIDE both sessions' PATH: only the hash
    // table can resolve it, so losing the entry is observable.
    let pinned = make_tool(&root, "P", &count);

    let mut a = session(&dir_a.to_string_lossy());
    let mut b = session(&dir_b.to_string_lossy());

    let script = format!(
        "hash -p \"{}\" {TOOL}",
        pinned.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);

    assert_eq!(run_tool(&mut a), 0);
    assert_eq!(count_lines(&count), ["P"], "A's pin resolves and runs");

    // B flushes ITS OWN table. GNU: this is B's phash_flush
    // (builtins/hash.def:150) — it cannot reach A's shell.
    let outcome = b.run("hash -r", &opts()).expect("run");
    assert_eq!(outcome.status, 0);

    assert_eq!(
        run_tool(&mut a),
        0,
        "B's hash -r must not drop A's remembered location"
    );
    assert_eq!(count_lines(&count), ["P", "P"]);

    let _ = fs::remove_dir_all(root);
}

/// Pin 3 — interleaved resolution across two sessions with different PATH
/// fingerprints stays per-session (the thrash face of the single-fingerprint
/// slot: each alternating lookup used to wipe the other session's scan
/// scaffold). Resolution results were never served cross-fingerprint, so
/// this pin holds before AND after; it guards the slice redesign.
#[test]
fn issue2078_interleaved_resolution_stays_per_session() {
    let root = temp_dir("interleave");
    let count = root.join("count.txt");
    let dir_a = root.join("a");
    let dir_b = root.join("b");
    make_tool(&dir_a, "A", &count);
    make_tool(&dir_b, "B", &count);

    let mut a = session(&dir_a.join("bin").to_string_lossy());
    let mut b = session(&dir_b.join("bin").to_string_lossy());

    for round in 0..2 {
        assert_eq!(run_tool(&mut a), 0, "round {round}: A resolves A's");
        assert_eq!(
            count_lines(&count).last(),
            Some(&"A".to_string()),
            "round {round}: A keeps running A's tool"
        );
        assert_eq!(run_tool(&mut b), 0, "round {round}: B resolves B's");
        assert_eq!(
            count_lines(&count).last(),
            Some(&"B".to_string()),
            "round {round}: B keeps running B's tool"
        );
    }
    assert_eq!(count_lines(&count), ["A", "B", "A", "B"]);

    let _ = fs::remove_dir_all(root);
}

/// Pin 4 — single-session semantics are unchanged (contract Q5): within
/// ONE session, `hash -p` resolves, `hash -r` forgets, and the name no
/// longer resolves (GNU: findcmd.c:623 re-walks $PATH and finds nothing).
#[test]
fn issue2078_single_session_hash_r_still_forgets() {
    let root = temp_dir("single");
    let count = root.join("count.txt");
    let dir_a = root.join("a");
    fs::create_dir_all(&dir_a).expect("dir a");
    let pinned = make_tool(&root, "P", &count);

    let mut a = session(&dir_a.to_string_lossy());

    let script = format!(
        "hash -p \"{}\" {TOOL}",
        pinned.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);

    assert_eq!(run_tool(&mut a), 0);
    assert_eq!(count_lines(&count), ["P"]);

    let outcome = a.run("hash -r", &opts()).expect("run");
    assert_eq!(outcome.status, 0);

    assert_eq!(
        run_tool(&mut a),
        127,
        "after hash -r the name is off PATH and unresolved"
    );
    assert_eq!(count_lines(&count), ["P"]);

    let _ = fs::remove_dir_all(root);
}

/// Pin 5 — churning scopes past the resolution-slice cap must not disturb
/// a LIVE session's hash table. The slices are capped pure caches (LRU),
/// but the mirror is state: when the cap still cleared the whole store,
/// the 17th scope's creation wiped every live session's mirror with it,
/// and a pinned command died with 127 — pin 2's failure shape re-entering
/// through the cap instead of a sibling's `hash -r`. GNU's guarantee is
/// that a shell's hash table lives as long as the shell (entries die only
/// by that shell's own PATH assignment / `hash -r`).
#[test]
fn issue2078_slice_cap_rollover_keeps_live_sessions_hash_pin() {
    let root = temp_dir("cap");
    let count = root.join("count.txt");
    let dir_a = root.join("a");
    fs::create_dir_all(&dir_a).expect("dir a");
    // The pinned tool lives OUTSIDE A's PATH: only the hash table can
    // resolve it, so losing the mirror entry is observable as a 127.
    let pinned = make_tool(&root, "P", &count);

    let mut a = session(&dir_a.to_string_lossy());
    let script = format!(
        "hash -p \"{}\" {TOOL}",
        pinned.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);

    assert_eq!(run_tool(&mut a), 0);
    assert_eq!(count_lines(&count), ["P"], "A's pin resolves and runs");

    // A's `hash` listing shows the pin before the churn.
    let hash_before = root.join("hash-before.txt");
    let script = format!(
        "hash > \"{}\"",
        hash_before.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);
    let before = fs::read_to_string(&hash_before).unwrap_or_default();
    assert!(
        before.contains(TOOL),
        "hash listing must show the pin: {before}"
    );

    // Churn: 20 further sessions, each resolving the tool through its own
    // PATH — every one creates (and keeps touching) its own resolution
    // slice, rolling the 16-slice cap over repeatedly.
    let dir_b = root.join("b");
    make_tool(&dir_b, "B", &count);
    for _ in 0..20 {
        let mut churn = session(&dir_b.join("bin").to_string_lossy());
        assert_eq!(run_tool(&mut churn), 0, "churn session runs its own tool");
    }

    // A's listing is byte-identical after the rollover (no A lookup has
    // run since, so even the hit counts must not have moved)...
    let hash_after = root.join("hash-after.txt");
    let script = format!(
        "hash > \"{}\"",
        hash_after.to_string_lossy().replace('\\', "/")
    );
    let outcome = a.run(&script, &opts()).expect("run");
    assert_eq!(outcome.status, 0);
    let after = fs::read_to_string(&hash_after).unwrap_or_default();
    assert_eq!(after, before, "A's hash listing must survive the rollover");

    // ...and the pin still resolves: the mirror entry itself survived.
    assert_eq!(
        run_tool(&mut a),
        0,
        "cap rollover must not drop A's hash -p pin"
    );
    assert_eq!(count_lines(&count).last(), Some(&"P".to_string()));

    let _ = fs::remove_dir_all(root);
}
