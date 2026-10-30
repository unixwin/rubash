//! rubash#2064: an inherited environment entry Unicode cannot represent
//! (an unpaired surrogate on Windows, arbitrary non-UTF-8 bytes on Unix)
//! used to panic `std::env::vars()` during environment ingest — the
//! standalone binary aborted during startup and an embedded Session
//! aborted during build, before the first command ran. GNU carries
//! environ as bytes (variables.c imports it verbatim); this engine's
//! variable table is String, so the ruled equivalent is to drop the
//! undecodable entry: it is absent for the shell and its children, every
//! decodable entry survives exactly, and startup/run proceed normally.
//!
//! Both pins bootstrap a child process with a poisoned environment:
//! `std::env::set_var` cannot express these entries (newer Rust refuses
//! some outright), so in-process injection is not an option. The second
//! pin re-executes this test binary in a child mode that builds a
//! Session through the public API only — the library ingest boundary
//! must hold on its own, without main()'s cleanup in front of it.
//!
//! Table absence is probed with `${NAME+set}`, which reads the shell
//! variable table (its process-environment fallback goes through the
//! same dropping ingest). `test -v` is deliberately not the probe: it
//! carries a separate pre-existing `env::var_os` presence fallback, so
//! in a library session it reports the host's raw entry even though the
//! shell never imported it.

use std::ffi::OsString;
use std::process::Command;

const GOOD_VALUE: &str = "kept verbatim / with space";

const PROBE_SCRIPT: &str = "\
echo \"good=[$RUBASH_2064_GOOD]\"
if [ -n \"${RUBASH_2064_BADVAL+set}\" ]; then echo badval=present; else echo badval=absent; fi
echo alive
";

/// One entry with an undecodable value under a decodable name, and one
/// entry whose name itself is undecodable. `None` on platforms where
/// neither shape can be constructed.
fn undecodable_entries() -> Option<Vec<(OsString, OsString)>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(vec![
            (
                OsString::from("RUBASH_2064_BADVAL"),
                OsString::from_vec(vec![b'b', b'a', b'd', 0xFF]),
            ),
            (
                OsString::from_vec(b"RUBASH_2064_BAD\xFFNAME".to_vec()),
                OsString::from("x"),
            ),
        ])
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let bad_value = OsString::from_wide(&[b'b' as u16, b'a' as u16, b'd' as u16, 0xD800]);
        let mut bad_name: Vec<u16> = "RUBASH_2064_BAD".encode_utf16().collect();
        bad_name.push(0xD800);
        bad_name.extend("NAME".encode_utf16());
        Some(vec![
            (OsString::from("RUBASH_2064_BADVAL"), bad_value),
            (OsString::from_wide(&bad_name), OsString::from("x")),
        ])
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn poisoned(command: &mut Command) -> Option<&mut Command> {
    let entries = undecodable_entries()?;
    command.env("RUBASH_2064_GOOD", GOOD_VALUE);
    for (name, value) in &entries {
        command.env(name, value);
    }
    Some(command)
}

fn assert_probe_output(text: &str, face: &str) {
    assert!(
        text.contains(&format!("good=[{GOOD_VALUE}]")),
        "{face}: decodable entries must survive verbatim, got: {text}"
    );
    assert!(
        text.contains("badval=absent"),
        "{face}: the undecodable entry must be dropped, not forged, got: {text}"
    );
    assert!(
        text.contains("alive"),
        "{face}: commands must run normally, got: {text}"
    );
}

#[test]
fn binary_starts_and_drops_undecodable_entries() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("-c").arg(PROBE_SCRIPT);
    let Some(command) = poisoned(&mut command) else {
        return;
    };
    let output = command.output().expect("spawn the shell binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "binary face: startup must not panic, stderr: {stderr}"
    );
    assert!(
        output.status.success(),
        "binary face: exit status {:?}, stderr: {stderr}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_probe_output(&stdout, "binary face");
}

#[test]
fn session_build_drops_undecodable_entries() {
    if std::env::var_os("RUBASH_2064_CHILD").is_some() {
        return;
    }
    let out_path =
        std::env::temp_dir().join(format!("rubash-2064-session-probe-{}", std::process::id()));
    let mut command = Command::new(std::env::current_exe().expect("current exe path"));
    command
        .arg("--exact")
        .arg("issue2064_session_child_entry")
        .arg("--nocapture")
        .env("RUBASH_2064_CHILD", "1")
        .env("RUBASH_2064_OUT", &out_path);
    let Some(command) = poisoned(&mut command) else {
        return;
    };
    let output = command.output().expect("spawn the child test process");
    assert!(
        output.status.success(),
        "session face: child process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&out_path).expect("child session wrote its probe output");
    let _ = std::fs::remove_file(&out_path);
    assert_probe_output(&text, "session face");
}

/// Child-mode entry for `session_build_drops_undecodable_entries`:
/// inert unless the parent spawned this binary with RUBASH_2064_CHILD.
#[test]
fn issue2064_session_child_entry() {
    if std::env::var_os("RUBASH_2064_CHILD").is_none() {
        return;
    }
    let out = std::env::var("RUBASH_2064_OUT").expect("parent passes the probe output path");
    let script = format!("{{ {PROBE_SCRIPT} }} > {} 2>&1", shell_quote(&out));
    let mut session = rubash::api::Session::builder()
        .build()
        .expect("session builds over a poisoned environment");
    let outcome = session
        .run(&script, &rubash::api::RunOptions::default())
        .expect("session runs over a poisoned environment");
    assert_eq!(outcome.status, 0, "probe script exit status");
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
