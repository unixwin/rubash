//! Issue rubash#1044 regression: `compgen -A service` was always empty.
//!
//! GNU bashline.c:2668 bash_servicename_completion_function walks the
//! system services database in getservent() order — /etc/services on
//! unix, %SystemRoot%\System32\drivers\etc\services on Windows (the
//! file the OS resolver consults). An entry contributes its primary
//! name when it matches the word prefix, otherwise a matching alias —
//! never both — and tcp/udp duplicates are kept.

use std::path::PathBuf;
use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(unix)]
fn services_db_path() -> PathBuf {
    PathBuf::from("/etc/services")
}

#[cfg(windows)]
fn services_db_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("windir"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join(r"System32\drivers\etc\services")
}

/// Re-derive the expected candidates from the on-disk database under
/// the getservent rule so the assertion survives any DB contents.
fn expected_candidates(word: &str) -> Option<Vec<String>> {
    let contents = std::fs::read_to_string(services_db_path()).ok()?;
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = raw.split('#').next().unwrap_or("");
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let _ = fields.next(); // port/proto
        if name.starts_with(word) {
            out.push(name.to_string());
        } else if let Some(alias) = fields.find(|alias| alias.starts_with(word)) {
            out.push(alias.to_string());
        }
    }
    Some(out)
}

#[test]
fn service_candidates_come_from_the_system_database() {
    let Some(expected) = expected_candidates("") else {
        eprintln!("no services database on this host; skipping");
        return;
    };
    let (stdout, _, rc) = rubash("compgen -A service");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(rc, Some(0));
    assert_eq!(got, expected, "candidate list must follow database order");
}

#[test]
fn service_prefix_and_alias_matching() {
    let Some(expected) = expected_candidates("ftp") else {
        eprintln!("no services database on this host; skipping");
        return;
    };
    let (stdout, _, _) = rubash("compgen -A service ftp");
    let got: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(
        got, expected,
        "prefix/alias matches must follow getservent rules"
    );
    // Every well-known services DB defines `ftp`; the unfiltered count
    // must exceed the filtered one.
    let (all, _, _) = rubash("compgen -A service");
    assert!(all.lines().count() > got.len());
}
