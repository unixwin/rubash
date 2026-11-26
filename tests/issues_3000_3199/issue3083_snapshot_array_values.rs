//! rubash#3083: the inspection snapshot must carry the array values
//! of the startup-seeded array/assoc variables, not their raw
//! mirror storage strings.
//!
//! BASH_VERSINFO, DIRSTACK, BASH_ALIASES and BASH_CMDS are seeded
//! into the env table as array/assoc storage strings at Executor
//! construction (init.rs), and the typed store is then imported
//! from that seeded table by `VariableStore::from_environment`,
//! which binds every entry as an exported Scalar — so each of these
//! names carries a typed Scalar cell holding the mirror storage
//! text verbatim. `capture_variable` trusted the typed cell first,
//! and the snapshot (and with it the DAP variables face) showed the
//! storage form — STORAGE_WORD_PREFIX and all — as a scalar value:
//! BASH_VERSINFO read as one string starting with U+001D instead of
//! a six-element array, DIRSTACK stayed the empty seed string even
//! after `pushd`, BASH_ALIASES/BASH_CMDS read as the scalar "()".
//!
//! GNU keeps these as real ARRAY cells from birth
//! (variables.c:1018 make_vers_array builds BASH_VERSINFO with
//! make_new_array_variable + array_insert per element;
//! variables.c:1882 registers DIRSTACK via init_dynamic_array_var
//! with get_dirstack at variables.c:1619 materializing the ARRAY
//! object), and initialize_shell_variables (variables.c:368)
//! imports only the process environment, whose entries are plain
//! scalars — the shell's own array cells are never imported as
//! scalars. The snapshot oracle is the shell's own `declare -p`
//! rendering, whose declaration text the snapshot already carries
//! correctly; the typed value must agree with it.
//!
//! These tests drive the public Session API only
//! (`Session::views().inspection()`), the same capture the debugger
//! consult consumes at a stop.

use rubash::api::views::VariableValue;
use rubash::api::{RunOptions, Session};

fn run(session: &mut Session, line: &str) {
    let outcome = session
        .run(line, &RunOptions::default())
        .expect("run succeeds");
    assert_eq!(outcome.status, 0, "run of {line:?} succeeds");
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3083-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The shell's own live composition of the directory stack, read
/// through element expansion into a file — the read the snapshot
/// must agree with.
fn shell_dirstack(session: &mut Session, dir: &std::path::Path) -> Vec<String> {
    run(
        session,
        "printf '%s\\n' \"${DIRSTACK[@]}\" > \"$OUT/dirstack.txt\"",
    );
    let text = std::fs::read_to_string(dir.join("dirstack.txt")).expect("dirstack dump");
    text.lines().map(str::to_string).collect()
}

fn snapshot_value(session: &mut Session, name: &str) -> VariableValue {
    let snap = session.views().inspection();
    snap.variable(name)
        .map(|variable| variable.value.clone())
        .unwrap_or_else(|| panic!("snapshot carries {name}"))
}

#[test]
fn seeded_indexed_arrays_snapshot_as_indexed_values() {
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "x=1");

    // BASH_VERSINFO: the six-element version array, not the storage
    // string. The first five elements are version constants; the
    // sixth is the platform MACHTYPE (non-empty, platform-shaped).
    match snapshot_value(&mut session, "BASH_VERSINFO") {
        VariableValue::Indexed(entries) => {
            assert_eq!(entries.len(), 6, "BASH_VERSINFO has six elements");
            assert_eq!(entries[0], (0, "5".to_string()));
            assert_eq!(entries[1], (1, "3".to_string()));
            assert_eq!(entries[2], (2, "20".to_string()));
            assert_eq!(entries[3], (3, "1".to_string()));
            assert_eq!(entries[4], (4, "release".to_string()));
            assert_eq!(entries[5].0, 5);
            assert!(!entries[5].1.is_empty(), "MACHTYPE element present");
        }
        other => panic!("BASH_VERSINFO snapshots as Indexed, got {other:?}"),
    }

    // DIRSTACK: a named capture resolves through the get_dirstack
    // composition (the mirror-absent capture arm already consults
    // parameter_array_storage for this name), so the snapshot is
    // the live stack — at birth its only member is the working
    // directory itself (builtins/pushd.def:669 get_directory_stack
    // appends it). Compare against the shell's own element read.
    let dir = temp_dir("seed");
    let mut session = Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    run(&mut session, "x=1");
    let live = shell_dirstack(&mut session, &dir);
    assert_eq!(live.len(), 1, "birth stack has only the cwd member");
    assert_eq!(
        snapshot_value(&mut session, "DIRSTACK"),
        VariableValue::Indexed(vec![(0, live[0].clone())]),
        "seeded DIRSTACK snapshots as the live stack"
    );
}

#[test]
fn dirstack_snapshot_tracks_the_live_stack() {
    let dir = temp_dir("pushd");
    let mut session = Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    run(&mut session, "pushd / >/dev/null");
    // The live two-member stack (new cwd over the pushed-from
    // directory), not the construction-time empty seed string.
    let live = shell_dirstack(&mut session, &dir);
    assert_eq!(live.len(), 2, "one push leaves two stack entries");
    assert_eq!(live[0], "/");
    let expected: Vec<(i64, String)> = live
        .iter()
        .enumerate()
        .map(|(index, value)| (index as i64, value.clone()))
        .collect();
    assert_eq!(
        snapshot_value(&mut session, "DIRSTACK"),
        VariableValue::Indexed(expected),
        "DIRSTACK snapshot matches the shell's own element read"
    );
}

#[test]
fn seeded_assoc_arrays_snapshot_as_associative_values() {
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "x=1");
    assert_eq!(
        snapshot_value(&mut session, "BASH_ALIASES"),
        VariableValue::Associative(Vec::new()),
        "seeded BASH_ALIASES snapshots as an empty associative array"
    );
    assert_eq!(
        snapshot_value(&mut session, "BASH_CMDS"),
        VariableValue::Associative(Vec::new()),
        "seeded BASH_CMDS snapshots as an empty associative array"
    );

    run(&mut session, "alias ll='ls -l'");
    assert_eq!(
        snapshot_value(&mut session, "BASH_ALIASES"),
        VariableValue::Associative(vec![("ll".to_string(), "ls -l".to_string())]),
        "BASH_ALIASES snapshots the live alias table"
    );
}

#[test]
fn user_bindings_snapshot_unchanged() {
    // Control group: bindings written through the shell's own write
    // paths carry synced structured typed cells and must snapshot
    // exactly as before.
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "a=(x y); declare -A h=([k]=v); s=plain");
    assert_eq!(
        snapshot_value(&mut session, "a"),
        VariableValue::Indexed(vec![(0, "x".to_string()), (1, "y".to_string())])
    );
    assert_eq!(
        snapshot_value(&mut session, "h"),
        VariableValue::Associative(vec![("k".to_string(), "v".to_string())])
    );
    assert_eq!(
        snapshot_value(&mut session, "s"),
        VariableValue::Scalar("plain".to_string())
    );
    // The dynamic-branch stack arrays keep their Indexed shape.
    match snapshot_value(&mut session, "BASH_ARGV") {
        VariableValue::Indexed(_) => {}
        other => panic!("BASH_ARGV snapshots as Indexed, got {other:?}"),
    }
}
