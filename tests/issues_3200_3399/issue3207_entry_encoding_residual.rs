//! rubash#3207 — entry-encoding contract gap, residual of #3105:
//! two raw-data host write paths land in storage unencoded, so a
//! carrier byte (\\x1c) in the value is stripped as IFS_GLUE protocol
//! when the value is finalized into a word, and a marker-zone char
//! (U+E314) is read as live protocol downstream.
//!
//! * `SessionBuilder::env` / `environment` seeds
//!   (`src/api/session.rs`, build-time `executor.set_env` loop);
//! * `VariablesView::set` (`src/api/views.rs`, direct `set_env`);
//! * `VariablesView::set_array` / `set_assoc` (typed-cell stores,
//!   `view_set_indexed_array` / `view_set_assoc_array`) — same face,
//!   decided by these cells.
//!
//! The contract (rubash#2884 env import, rubash#3105 argv/positional):
//! every RAW-data entry channel encodes at its boundary with the data
//! encoding (`entry_encode_value_text`, the `bytes_to_shell_text`
//! parity), so user carrier bytes ride the raw-byte marker pair
//! through storage and decode back at consumption boundaries. The
//! shared setter `Executor::set_env` is NOT the boundary: shell
//! internals hand it cooked (already-encoded) words, so encoding
//! there would double-encode — the plain-value cell guards that the
//! fix stays at the API entry faces.
//!
//! Read face: `VariablesView::resolved_value` returns the stored
//! (storage-domain) spelling, per the #3105 read-face precedent
//! (`positional()` returns storage-domain spelling); the probe
//! records it to `resolved.txt` for the report without asserting
//! on it.
//!
//! Harness: in-process host API only (`rubash::api`), the way an
//! embedding host drives a session; byte-level output assertions.

use std::path::PathBuf;

const CARRIER: &str = "f\u{1c}g";
const CARRIER_BYTES: &[u8] = b"f\x1cg";

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3207-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run_to_file(
    session: &mut rubash::api::Session,
    script: &str,
    dir: &PathBuf,
    name: &str,
) -> Vec<u8> {
    use rubash::api::RunOptions;
    let outcome = session
        .run(script, &RunOptions::default())
        .expect("run succeeds");
    assert_eq!(outcome.status, 0, "script {script:?} status");
    std::fs::read(dir.join(name)).expect("output file")
}

fn base_session(dir: &PathBuf) -> rubash::api::Session {
    rubash::api::Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .build()
        .expect("session builds")
}

#[test]
fn builder_env_seed_finalization() {
    let dir = tmpdir("builderenv");
    let mut session = rubash::api::Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .env("PROBE", CARRIER)
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    // Record the declaration-domain read face for the report.
    let rv = session.variables().resolved_value("PROBE");
    std::fs::write(dir.join("resolved.txt"), format!("{rv:?}")).expect("record resolved");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"$PROBE\" > \"$OUT/a.bin\"",
        &dir,
        "a.bin",
    );
    assert_eq!(
        bytes, CARRIER_BYTES,
        "builder env seed loses carrier at finalization"
    );
}

#[test]
fn builder_env_seed_export_child() {
    let dir = tmpdir("builderenvx");
    let mut session = rubash::api::Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .env("PROBE", CARRIER)
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    let bytes = run_to_file(
        &mut session,
        "export PROBE; /usr/bin/printf %s \"$PROBE\" > \"$OUT/b.bin\"",
        &dir,
        "b.bin",
    );
    assert_eq!(
        bytes, CARRIER_BYTES,
        "builder env seed loses carrier at execve funnel"
    );
}

#[test]
fn builder_env_marker_zone() {
    let dir = tmpdir("builderenvmz");
    let arg = "a\u{e314}b";
    let mut session = rubash::api::Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .env("PROBE", arg)
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"$PROBE\" > \"$OUT/c.bin\"",
        &dir,
        "c.bin",
    );
    assert_eq!(
        bytes,
        arg.as_bytes(),
        "builder env seed mangles marker-zone char"
    );
}

#[test]
fn variables_view_set_finalization() {
    let dir = tmpdir("viewset");
    let mut session = base_session(&dir);
    session.variables().set("PROBE", CARRIER);
    let rv = session.variables().resolved_value("PROBE");
    std::fs::write(dir.join("resolved.txt"), format!("{rv:?}")).expect("record resolved");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"$PROBE\" > \"$OUT/a.bin\"",
        &dir,
        "a.bin",
    );
    assert_eq!(
        bytes, CARRIER_BYTES,
        "VariablesView::set loses carrier at finalization"
    );
}

#[test]
fn variables_view_set_array_finalization() {
    let dir = tmpdir("viewarr");
    let mut session = base_session(&dir);
    let ok = session
        .variables()
        .set_array("ARR", &[(0, CARRIER.to_string())]);
    assert!(ok, "set_array accepted");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"${ARR[0]}\" > \"$OUT/a.bin\"",
        &dir,
        "a.bin",
    );
    assert_eq!(
        bytes, CARRIER_BYTES,
        "VariablesView::set_array loses carrier at finalization"
    );
}

#[test]
fn variables_view_set_assoc_finalization() {
    let dir = tmpdir("viewassoc");
    let mut session = base_session(&dir);
    let ok = session
        .variables()
        .set_assoc("MAP", &[("k".to_string(), CARRIER.to_string())]);
    assert!(ok, "set_assoc accepted");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"${MAP[k]}\" > \"$OUT/a.bin\"",
        &dir,
        "a.bin",
    );
    assert_eq!(
        bytes, CARRIER_BYTES,
        "VariablesView::set_assoc loses carrier at finalization"
    );
}

#[test]
fn builder_env_seed_plain_value_round_trips() {
    let dir = tmpdir("builderplain");
    let mut session = rubash::api::Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .env("PROBE", "hello world")
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    let bytes = run_to_file(
        &mut session,
        "printf %s \"$PROBE\" > \"$OUT/a.bin\"",
        &dir,
        "a.bin",
    );
    assert_eq!(
        bytes, b"hello world",
        "plain builder env seed must round-trip byte-identical"
    );
}
