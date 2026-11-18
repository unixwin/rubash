//! rubash#3105 — OS-argv positional parameters enter storage
//! unencoded: a bare carrier byte (\x1c) in `$1` is visible to
//! storage-domain consumers (case matching, substring extraction)
//! but is stripped as IFS_GLUE protocol when the value is finalized
//! into a word, so `printf %s "$1"` prints `fg` where GNU prints
//! `f\x1cg`.
//!
//! The storage contract (markers.rs governance; env import was
//! aligned by rubash#2884): every RAW-data entry channel encodes at
//! its boundary with the data encoding
//! (`entry_encode_value_text`, the `bytes_to_shell_text` parity), so
//! user carrier bytes ride the raw-byte marker pair through storage
//! and decode back at consumption boundaries. The process
//! environment entry does this; the argv entries did not:
//!
//! * binary: `rubash -c cmd name args…` (main.rs -c face),
//!   `rubash script args…` (script-file face), `rubash -s args…`
//!   (stdin face);
//! * host API: `StateViews::set_positional` (api/views.rs,
//!   api/session.rs) and `ShellInvocation::positional_params`
//!   (invocation.rs apply) — hosts pass raw data, the engine
//!   encodes at the entry.
//!
//! The shared setter `Executor::set_positional_params` is NOT the
//! boundary: the `set` builtin and function calls hand it cooked
//! (already-encoded) words, so encoding there would double-encode —
//! the last cell guards that path.
//!
//! Harness: byte-level stdout assertions; `LC_ALL=C.UTF-8` pinned
//! on spawned children so the pin is hermetic under any locale.

use std::io::Write;
use std::process::{Command, Stdio};

const CARRIER_ARG: &str = "f\u{1c}g";
const CARRIER_BYTES: &[u8] = b"f\x1cg";

fn run_rubash(args: &[String], stdin_text: Option<&str>, extra_env: &[(&str, &str)]) -> Vec<u8> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env("LC_ALL", "C.UTF-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    if stdin_text.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(text) = stdin_text {
        child
            .stdin
            .as_mut()
            .expect("stdin piped")
            .write_all(text.as_bytes())
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait rubash");
    assert!(
        output.status.success(),
        "rubash {args:?} exited with {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn dash_c(script: &str, rest: &[&str]) -> Vec<u8> {
    let mut args: Vec<String> = vec!["-c".into(), script.into(), "_".into()];
    args.extend(rest.iter().map(|arg| arg.to_string()));
    run_rubash(&args, None, &[])
}

/// The headline defect: `$1` finalized into a word loses \x1c.
#[test]
fn argv_dollar1_direct_keeps_carrier_byte() {
    let out = dash_c("printf %s \"$1\"", &[CARRIER_ARG]);
    assert_eq!(out, CARRIER_BYTES);
}

/// Control (passes on the base): the byte IS in storage — a case
/// pattern over `"$1"` sees it. Proves the loss is at word
/// finalization, not at storage.
#[test]
fn argv_case_pattern_control_sees_carrier_byte() {
    let out = dash_c(
        "case \"$1\" in *\u{1c}*) echo HAS034;; *) echo NO034;; esac",
        &[CARRIER_ARG],
    );
    assert_eq!(out, b"HAS034\n");
}

/// Control (passes on the base): substring extraction also sees
/// the stored byte.
#[test]
fn argv_substring_control_sees_carrier_byte() {
    let out = dash_c("printf %s \"${1:1:1}\"", &[CARRIER_ARG]);
    assert_eq!(out, b"\x1c");
}

/// The byte must also survive export to a REAL child process
/// (the execve funnel decodes the pair; a bare byte was eaten
/// before the funnel ever saw it).
#[test]
fn argv_export_to_real_child_keeps_carrier_byte() {
    let out = dash_c("export E=\"$1\"; /usr/bin/printf %s \"$E\"", &[CARRIER_ARG]);
    assert_eq!(out, CARRIER_BYTES);
}

/// Control (passes on the base): the environment entry channel
/// already encodes — same value via env prints whole.
#[test]
fn env_entry_control_keeps_carrier_byte() {
    let out = run_rubash(
        &["-c".into(), "printf %s \"$LIT\"".into()],
        None,
        &[("LIT", CARRIER_ARG)],
    );
    assert_eq!(out, CARRIER_BYTES);
}

/// Script-file face: `rubash script args…`.
#[test]
fn script_file_arg_keeps_carrier_byte() {
    let dir = std::env::temp_dir().join(format!("rubash-3105-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, "printf %s \"$1\"\n").expect("write script");
    let out = run_rubash(
        &[
            script_path.to_string_lossy().into_owned(),
            CARRIER_ARG.into(),
        ],
        None,
        &[],
    );
    assert_eq!(out, CARRIER_BYTES);
}

/// Stdin face: `rubash -s args…`.
#[test]
fn stdin_dash_s_arg_keeps_carrier_byte() {
    let out = run_rubash(
        &["-s".into(), CARRIER_ARG.into()],
        Some("printf %s \"$1\""),
        &[],
    );
    assert_eq!(out, CARRIER_BYTES);
}

/// A literal marker-zone char in an OS argument is user DATA
/// (GNU keeps the PUA glyph verbatim); unencoded it is read as
/// live protocol downstream.
#[test]
fn argv_marker_zone_char_kept_verbatim() {
    let arg = "a\u{e314}b";
    let out = dash_c("printf %s \"$1\"", &[arg]);
    assert_eq!(out, arg.as_bytes());
}

/// Host API face: `StateViews::set_positional` takes raw data.
#[test]
fn session_set_positional_keeps_carrier_byte() {
    use rubash::api::{RunOptions, Session};
    let dir = std::env::temp_dir().join(format!("rubash-3105-api-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut session = Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    session
        .views()
        .set_positional(vec![CARRIER_ARG.to_string()]);
    let outcome = session
        .run(
            "printf '%s' \"$1\" > \"$OUT/p.bin\"",
            &RunOptions::default(),
        )
        .expect("run succeeds");
    assert_eq!(outcome.status, 0);
    let bytes = std::fs::read(dir.join("p.bin")).expect("output file");
    assert_eq!(bytes, CARRIER_BYTES);
}

/// Host API face, sibling method: `Session::set_positional`
/// (api/session.rs) takes the same raw-data contract.
#[test]
fn session_direct_set_positional_keeps_carrier_byte() {
    use rubash::api::{RunOptions, Session};
    let dir = std::env::temp_dir().join(format!("rubash-3105-api2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut session = Session::builder()
        .env("OUT", dir.to_string_lossy().into_owned())
        .working_directory(dir.clone())
        .build()
        .expect("session builds");
    session.set_positional(vec![CARRIER_ARG.to_string()]);
    let outcome = session
        .run(
            "printf '%s' \"$1\" > \"$OUT/p.bin\"",
            &RunOptions::default(),
        )
        .expect("run succeeds");
    assert_eq!(outcome.status, 0);
    let bytes = std::fs::read(dir.join("p.bin")).expect("output file");
    assert_eq!(bytes, CARRIER_BYTES);
}

/// Host invocation face: `ShellInvocation::positional_params`
/// carries raw host argv; the apply boundary encodes.
#[test]
fn invocation_positional_params_keep_carrier_byte() {
    use rubash::executor::Executor;
    use rubash::invocation::ShellInvocation;
    let dir = std::env::temp_dir().join(format!("rubash-3105-inv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let out_path = dir.join("p.bin");
    let invocation = ShellInvocation::parse(&[
        "-c".to_string(),
        "true".to_string(),
        "nm".to_string(),
        CARRIER_ARG.to_string(),
    ])
    .expect("invocation parses");
    let mut executor = Executor::new();
    invocation
        .apply_to_executor_before_startup(&mut executor)
        .expect("invocation applies");
    let script = format!(
        "printf '%s' \"$1\" > \"{}\"",
        out_path.to_string_lossy().replace('\\', "/")
    );
    let ast = rubash::parser::parse(&rubash::lexer::tokenize(&script));
    executor.execute_ast(&ast).expect("probe runs");
    let bytes = std::fs::read(&out_path).expect("output file");
    assert_eq!(bytes, CARRIER_BYTES);
}

/// Same-disease survey face (brief §survey): the `-c` name
/// operand seeds `BASH_ARGV0` — a shell-visible variable — from
/// raw argv; `$0` finalized into a word loses the byte the same
/// way. GNU shell.c:1505-1515 binds dollar_vars[0] verbatim.
#[test]
fn argv_dollar0_name_operand_keeps_carrier_byte() {
    let out = run_rubash(
        &["-c".into(), "printf %s \"$0\"".into(), CARRIER_ARG.into()],
        None,
        &[],
    );
    assert_eq!(out, CARRIER_BYTES);
}

/// Same-disease survey face: a script filename carrying the byte
/// seeds `BASH_ARGV0` at script entry (shell.c:1613-1614).
#[test]
fn script_name_dollar0_keeps_carrier_byte() {
    let dir = std::env::temp_dir().join(format!("rubash-3105-name-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let script_path = dir.join("scr\u{1c}ipt.sh");
    std::fs::write(&script_path, "printf %s \"$0\"\n").expect("write script");
    let path_text = script_path.to_string_lossy().into_owned();
    let out = run_rubash(&[path_text.clone()], None, &[]);
    assert_eq!(out, path_text.as_bytes());
}

/// Same-disease survey face: an environment-imported BASH_ARGV0
/// is re-seeded raw over the (encoded) table import in main, so
/// both `$0` and `$BASH_ARGV0` lose the byte at finalization.
/// GNU variables.c:1547-1555: the imported value is dollar_vars[0]
/// verbatim.
#[test]
fn env_bash_argv0_keeps_carrier_byte() {
    let out = run_rubash(
        &[
            "-c".into(),
            "printf %s \"$0\"; printf %s \"$BASH_ARGV0\"".into(),
        ],
        None,
        &[("BASH_ARGV0", CARRIER_ARG)],
    );
    let mut expected = CARRIER_BYTES.to_vec();
    expected.extend_from_slice(CARRIER_BYTES);
    assert_eq!(out, expected);
}

/// Guard (passes before and after): the `set` builtin hands the
/// shared setter COOKED words — entry encoding at the raw entries
/// must not double-encode this path.
#[test]
fn set_builtin_positional_not_double_encoded() {
    let out = dash_c("v=$'f\\034g'; set -- \"$v\"; printf %s \"$1\"", &[]);
    assert_eq!(out, CARRIER_BYTES);
}
