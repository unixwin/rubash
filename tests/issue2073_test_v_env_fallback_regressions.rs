//! rubash#2073: `test -v` / `[[ -v ]]` used to fall back to raw
//! process-environment presence (`env::var_os`) after the variable-table
//! lookup missed. GNU test.c:650-677 (unary `-v`) is `find_variable(arg)`
//! over the shell variable table only: an environment entry the shell
//! never imported is unset, even though the raw environ still carries
//! it. GNU's importer (variables.c:368 initialize_shell_variables)
//! routes entries it cannot bind into a separate `invalid_env` table
//! (variables.c:126, bind_invalid_envvar at :3307) that `find_variable`
//! never sees.
//!
//! This engine's ingest layer (rubash#2064) drops undecodable entries
//! the same way, so a poisoned entry is absent from the variable table
//! while the host process environment retains it verbatim — exactly the
//! shape the fallback misreported as set. The pins bootstrap a child
//! process with such an environment (std::env::set_var cannot express
//! these entries) on both faces: the standalone binary and a library
//! Session built through the public API, the scenario the issue names.
//! The Session face is the discriminating one: on the binary face the
//! fallback was already masked by main()'s rubash#2064 startup scrub
//! (the entry leaves the process environment before the first command
//! runs), so that pin is a no-over-removal anchor; in a library Session
//! nothing scrubs the embedding process's environment, and the fallback
//! read it raw — baseline verdicts there were bad_v/bad_cond/bad_at=set.
//!
//! GNU expectations, oracle-verified against bash 5.3.20: an imported
//! decodable entry is set; a never-imported entry is unset for `test
//! -v`, for `[[ -v ]]`, and for the `name[@]` operand shape; a
//! script-assigned variable is set and unset again after `unset`.

use std::ffi::OsString;
use std::process::Command;

const GOOD_VALUE: &str = "kept verbatim / with space";

const PROBE_SCRIPT: &str = "\
if test -v RUBASH_2073_GOOD; then echo good_v=set; else echo good_v=unset; fi
if test -v RUBASH_2073_BADVAL; then echo bad_v=set; else echo bad_v=unset; fi
if [[ -v RUBASH_2073_BADVAL ]]; then echo bad_cond=set; else echo bad_cond=unset; fi
if test -v 'RUBASH_2073_BADVAL[@]'; then echo bad_at=set; else echo bad_at=unset; fi
if [ -n \"${RUBASH_2073_BADVAL+set}\" ]; then echo bad_plus=present; else echo bad_plus=absent; fi
RUBASH_2073_LOCAL=1
if test -v RUBASH_2073_LOCAL; then echo local_v=set; else echo local_v=unset; fi
unset RUBASH_2073_LOCAL
if test -v RUBASH_2073_LOCAL; then echo local_after_unset=set; else echo local_after_unset=unset; fi
echo alive
";

/// The verdicts the fixed engine must print, one per probe line. The
/// first is the no-over-removal anchor (an imported entry stays set);
/// the next three are the shapes the process-env fallback poisoned.
const EXPECTED_LINES: [&str; 8] = [
    "good_v=set",
    "bad_v=unset",
    "bad_cond=unset",
    "bad_at=unset",
    "bad_plus=absent",
    "local_v=set",
    "local_after_unset=unset",
    "alive",
];

/// One entry with an undecodable value under a decodable name: dropped
/// by ingest (rubash#2064) yet still present in the raw process
/// environment. `None` on platforms where the shape cannot be built.
fn undecodable_entry() -> Option<(OsString, OsString)> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some((
            OsString::from("RUBASH_2073_BADVAL"),
            OsString::from_vec(vec![b'b', b'a', b'd', 0xFF]),
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        Some((
            OsString::from("RUBASH_2073_BADVAL"),
            OsString::from_wide(&[b'b' as u16, b'a' as u16, b'd' as u16, 0xD800]),
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn poisoned(command: &mut Command) -> Option<&mut Command> {
    let (name, value) = undecodable_entry()?;
    command.env("RUBASH_2073_GOOD", GOOD_VALUE);
    command.env(name, value);
    Some(command)
}

fn assert_probe_output(text: &str, face: &str) {
    for line in EXPECTED_LINES {
        assert!(
            text.lines().any(|actual| actual == line),
            "{face}: expected verdict line `{line}`, got:\n{text}"
        );
    }
}

#[test]
fn issue2073_binary_test_v_ignores_unimported_env_entry() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.arg("-c").arg(PROBE_SCRIPT);
    let Some(command) = poisoned(&mut command) else {
        return;
    };
    let output = command.output().expect("spawn the shell binary");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "binary face: run must not panic, stderr: {stderr}"
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
fn issue2073_session_test_v_ignores_unimported_env_entry() {
    if std::env::var_os("RUBASH_2073_CHILD").is_some() {
        return;
    }
    let out_path =
        std::env::temp_dir().join(format!("rubash-2073-session-probe-{}", std::process::id()));
    let mut command = Command::new(std::env::current_exe().expect("current exe path"));
    command
        .arg("--exact")
        .arg("issue2073_session_child_entry")
        .arg("--nocapture")
        .env("RUBASH_2073_CHILD", "1")
        .env("RUBASH_2073_OUT", &out_path);
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

/// Child-mode entry for `issue2073_session_test_v_ignores_unimported_env_entry`:
/// inert unless the parent spawned this binary with RUBASH_2073_CHILD.
#[test]
fn issue2073_session_child_entry() {
    if std::env::var_os("RUBASH_2073_CHILD").is_none() {
        return;
    }
    let out = std::env::var("RUBASH_2073_OUT").expect("parent passes the probe output path");
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
