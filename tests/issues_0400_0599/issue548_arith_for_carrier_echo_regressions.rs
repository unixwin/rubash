//! Issue rubash#548: `for (( i="1;2" ; ... ))` leaked the internal 0x1c
//! carrier byte into the evalerror echo (`i=\x1c1;2` where GNU bash 5.3.0
//! echoes the dequoted section `i=1;2 `). The section text fed to the
//! arithmetic evaluator must be the verbatim (( )) capture — quotes intact,
//! carriers stripped — because the evaluator removes quotes in place and
//! evalerror echoes the result verbatim (GNU make_cmd.c:254-261 section
//! split; execute_cmd.c:3171 quote removal before evalexp). The same root
//! cause failed the success path: `for (( i="3" ; ... ))` aborted the whole
//! loop on the carrier byte where GNU runs three iterations.

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, Vec<u8>, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("rubash-issue548-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue548 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (output.status.code(), output.stdout, output.stderr)
}

/// The 0x1c carrier (IFS_GLUE, the quoted-glue position marker) and the rest
/// of the C0 registry must never surface in the user-visible diagnostic.
fn assert_no_carrier_bytes(stderr: &[u8], context: &str) {
    let carriers: Vec<u8> = stderr
        .iter()
        .copied()
        .filter(|b| (0x01..=0x1f).contains(b) && *b != b'\t' && *b != b'\n')
        .collect();
    assert!(
        carriers.is_empty(),
        "carrier bytes {carriers:?} in {context}"
    );
}

#[test]
fn quoted_semicolon_error_echo_is_gnu_verbatim() {
    // WSL GNU bash 5.3.0, stdin oracle (byte-exact):
    //   bash: line 1: ((: i=1;2 : arithmetic syntax error: invalid
    //   arithmetic operator (error token is ";2 ")
    // The echo carries the dequoted section with its trailing blank, and
    // the error token is the raw `;2 ` remainder — never a 0x1c carrier.
    let (code, stdout, stderr) = run_script("for (( i=\"1;2\" ; i<3 ; i++ )); do :; done\n");
    // The failed init leaves the arith-for at status 1 and the script exits
    // with it (WSL GNU bash 5.3.0 rc 1).
    assert_eq!(
        code,
        Some(1),
        "stderr={:?}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(stdout.is_empty(), "stdout={stdout:?}");
    assert_no_carrier_bytes(&stderr, "quoted-semicolon arith-for error");
    let text = String::from_utf8_lossy(&stderr).into_owned();
    let body =
        "((: i=1;2 : arithmetic syntax error: invalid arithmetic operator (error token is \";2 \")";
    assert!(
        text.contains(body),
        "missing GNU verbatim body {body:?} in {text:?}"
    );
    // GNU keeps the section's trailing blank in the echo (make_cmd.c
    // substring keeps everything before the `;`): `i=1;2 :` with the space.
    assert!(text.contains("i=1;2 :"), "trailing blank lost: {text:?}");
}

#[test]
fn quoted_value_init_runs_the_loop() {
    // WSL GNU bash 5.3.0 prints hi three times, rc 0. Before the fix the
    // carrier byte in the section text aborted the loop with
    // `((: i=\x1c3 : arithmetic syntax error: operand expected`.
    let (code, stdout, stderr) = run_script("for (( i=\"3\" ; i>0 ; i-- )); do echo hi; done\n");
    assert_eq!(
        code,
        Some(0),
        "stderr={:?}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"hi\nhi\nhi\n", "loop must run three times");
    assert_no_carrier_bytes(&stderr, "quoted-value arith-for success path");
}

#[test]
fn quoted_operator_inside_section_matches_gnu_body() {
    // WSL GNU bash 5.3.0: the quoted `;` does not split the section (GNU
    // make_cmd.c skip_to_delim is quote-aware), the dequoted text then
    // fails at the `;`:
    //   bash: line 1: ((: i=a;b : arithmetic syntax error: invalid
    //   arithmetic operator (error token is ";b ")
    let (code, _stdout, stderr) = run_script("for (( i=\"a;b\" ; 0 ; 1 )); do echo q; done\n");
    assert_no_carrier_bytes(&stderr, "quoted-operator arith-for error");
    let text = String::from_utf8_lossy(&stderr).into_owned();
    let body =
        "((: i=a;b : arithmetic syntax error: invalid arithmetic operator (error token is \";b \")";
    assert!(
        text.contains(body),
        "missing GNU verbatim body {body:?} in {text:?}"
    );
    let _ = code;
}

#[test]
fn plain_sections_keep_running_normally() {
    // The capture-based section text must not disturb the unquoted common
    // path: GNU prints 0 1 2, rc 0.
    let (code, stdout, stderr) = run_script("for (( i=0 ; i<3 ; i++ )); do echo $i; done\n");
    assert_eq!(
        code,
        Some(0),
        "stderr={:?}",
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(stdout, b"0\n1\n2\n");
}
