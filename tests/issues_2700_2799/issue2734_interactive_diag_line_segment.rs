//! rubash#2734 (B5-17) — diagnostics on the interactive routes
//! must not carry the file-style `line N:` segment (nor the `-c:`
//! input-name segment) that GNU omits there. GNU error.c:300-316
//! (`parser_error`): `if (interactive)` the prolog is the error name
//! alone; the `iname: line N:` form appears only for
//! `interactive_shell` non-current input (a script file) or
//! non-interactive input. GNU error.c:80: the runtime-diagnostic
//! line segment prints only when `interactive_shell == 0`, and
//! error.c:90-117 (`get_name_for_error`) gives an interactive shell
//! just `base_pathname (shell_name)`.
//!
//! The two GNU predicates differ, and the pin keeps them apart:
//! an `-i script` PARSE error keeps `script: line N:` (its
//! `interactive` is 0), while an `-i script` RUNTIME error is
//! name-only like every interactive route.
//!
//! Measured against GNU bash 5.3.20 (setsid, no controlling tty;
//! `{name}` = basename of argv[0]):
//! | form | GNU stderr |
//! |---|---|
//! | piped `-i`, `if then` | `{name}: syntax error near unexpected token \`then'` |
//! | `-i -c 'if then'` (rc 2) | same name-only form |
//! | `-i script`, error line 2 | `{name}: {script}: line 2: syntax error ...` |
//! | `-c 'if then'` (control) | `{argv0}: -c: line 1: syntax error ...` |
//! | script (control) | `{script}: line 2: syntax error ...` |
//! | piped `-i`, unknown cmd | `{name}: nosuchcmd_zz9: command not found` |
//! | `-i -c 'return 5'` (rc 2) | `{name}: return: can only \`return' ...` |
//! | `-i script`, top `return` | name-only form; script CONTINUES, rc 0 |
//!
//! Harness shape: same setsid/no-ctty + cleared-env + scratch-HOME
//! shape as issue2731.

use std::io::Write;
use std::process::{Command, Stdio};

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i2734-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn rubash_no_tty(
    args: &[&str],
    stdin_bytes: &[u8],
    home: &std::path::Path,
) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home)
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn rubash");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(stdin_bytes);
    }
    let out = child.wait_with_output().expect("wait for rubash");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// GNU's interactive error name: `base_pathname (shell_name)` — the
/// basename of the name the shell was invoked under.
fn shell_name() -> String {
    std::path::Path::new(env!("CARGO_BIN_EXE_rubash"))
        .file_name()
        .expect("binary file name")
        .to_string_lossy()
        .into_owned()
}

/// The stderr line carrying `needle`, if any (reader prompts and
/// echoed input share the stream; the diagnostic is its own line).
fn diag_line<'a>(stderr: &'a str, needle: &str) -> Option<&'a str> {
    stderr.lines().find(|line| line.contains(needle))
}

#[test]
fn piped_interactive_syntax_error_has_no_line_segment() {
    let home = scratch_dir("pipe");
    let (stdout, stderr, code) = rubash_no_tty(&["-i"], b"if then\necho AFTERMARK\nexit\n", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "syntax error")
        .unwrap_or_else(|| panic!("no syntax diagnostic in stderr: {stderr:?}"));
    assert_eq!(
        line.trim(),
        format!(
            "{}: syntax error near unexpected token `then'",
            shell_name()
        ),
        "GNU: interactive parse diagnostics are name-only, got stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("AFTERMARK"),
        "GNU: the interactive shell survives a syntax error, got stdout: {stdout:?}"
    );
    assert_eq!(code, Some(0), "fed `exit` after the error, got {code:?}");
}

#[test]
fn interactive_command_syntax_error_has_no_c_segment() {
    let home = scratch_dir("dashc");
    let (stdout, stderr, code) = rubash_no_tty(&["-i", "-c", "if then"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "syntax error")
        .unwrap_or_else(|| panic!("no syntax diagnostic in stderr: {stderr:?}"));
    assert_eq!(
        line.trim(),
        format!(
            "{}: syntax error near unexpected token `then'",
            shell_name()
        ),
        "GNU: -i -c drops both the `-c:` and the line segment, got stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("-c:"),
        "GNU: no `-c:` segment on the interactive -c route, got stderr: {stderr:?} stdout: {stdout:?}"
    );
    assert_eq!(code, Some(2), "GNU: parse failure exits 2, got {code:?}");
}

#[test]
fn interactive_script_syntax_error_keeps_file_line() {
    // Control: on the `-i script` route the input is a file, so
    // parser_error's interactive_shell branch keeps
    // `script: line N:` — a fix that strips segments wholesale
    // would break this.
    let home = scratch_dir("iscript");
    let script = home.join("bad.sh");
    std::fs::write(&script, "echo LINE1OK\nif then\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, code) = rubash_no_tty(&["-i", &script_arg], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "syntax error")
        .unwrap_or_else(|| panic!("no syntax diagnostic in stderr: {stderr:?}"));
    assert!(
        line.contains(&format!(
            "{script_arg}: line 2: syntax error near unexpected token `then'"
        )),
        "GNU: -i script parse errors keep the file+line form, got stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("LINE1OK"),
        "GNU: lines before the error still run, got stdout: {stdout:?}"
    );
    assert_eq!(code, Some(2), "GNU: parse failure exits 2, got {code:?}");
}

#[test]
fn noninteractive_syntax_error_controls() {
    let home = scratch_dir("controls");
    // Plain -c: `{argv0}: -c: line 1: ...`.
    let (_, stderr, code) = rubash_no_tty(&["-c", "if then"], b"", &home);
    let line = diag_line(&stderr, "syntax error")
        .unwrap_or_else(|| panic!("no syntax diagnostic in stderr: {stderr:?}"));
    assert!(
        line.contains("-c: line 1: syntax error near unexpected token `then'"),
        "GNU: non-interactive -c keeps `-c: line 1:`, got stderr: {stderr:?}"
    );
    assert_eq!(code, Some(2), "got {code:?}");
    // Plain script: `{script}: line 2: ...`.
    let script = home.join("bad.sh");
    std::fs::write(&script, "echo LINE1OK\nif then\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, code) = rubash_no_tty(&[&script_arg], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "syntax error")
        .unwrap_or_else(|| panic!("no syntax diagnostic in stderr: {stderr:?}"));
    assert!(
        line.contains(&format!(
            "{script_arg}: line 2: syntax error near unexpected token `then'"
        )),
        "GNU: non-interactive script keeps the file+line form, got stderr: {stderr:?}"
    );
    assert!(stdout.contains("LINE1OK"), "got stdout: {stdout:?}");
    assert_eq!(code, Some(2), "got {code:?}");
}

#[test]
fn piped_interactive_runtime_diag_has_no_line_segment() {
    let home = scratch_dir("notfound");
    let (stdout, stderr, _) = rubash_no_tty(&["-i"], b"nosuchcmd_zz9\necho RC=$?\nexit\n", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "command not found")
        .unwrap_or_else(|| panic!("no not-found diagnostic in stderr: {stderr:?}"));
    assert_eq!(
        line.trim(),
        format!("{}: nosuchcmd_zz9: command not found", shell_name()),
        "GNU: interactive runtime diagnostics are name-only, got stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("RC=127"),
        "GNU: not-found status is 127, got stdout: {stdout:?}"
    );
}

#[test]
fn interactive_command_return_diag() {
    let home = scratch_dir("retc");
    let (_, stderr, code) = rubash_no_tty(&["-i", "-c", "return 5"], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "return")
        .unwrap_or_else(|| panic!("no return diagnostic in stderr: {stderr:?}"));
    assert_eq!(
        line.trim(),
        format!(
            "{}: return: can only `return' from a function or sourced script",
            shell_name()
        ),
        "GNU: -i -c runtime diagnostics are name-only, got stderr: {stderr:?}"
    );
    assert_eq!(code, Some(2), "GNU: bad return on -c exits 2, got {code:?}");
}

#[test]
fn interactive_script_return_diag_and_continuation() {
    // Control: error.c:80 keys the line segment on
    // interactive_shell, so even the script route's RUNTIME
    // diagnostics are name-only — and an interactive script does
    // not die on the bad return.
    let home = scratch_dir("retscript");
    let script = home.join("ret.sh");
    std::fs::write(&script, "return 5\necho NEVER\n").expect("write script");
    let script_arg = script.to_string_lossy().into_owned();
    let (stdout, stderr, code) = rubash_no_tty(&["-i", &script_arg], b"", &home);
    let _ = std::fs::remove_dir_all(&home);
    let line = diag_line(&stderr, "return")
        .unwrap_or_else(|| panic!("no return diagnostic in stderr: {stderr:?}"));
    assert_eq!(
        line.trim(),
        format!(
            "{}: return: can only `return' from a function or sourced script",
            shell_name()
        ),
        "GNU: -i script runtime diagnostics are name-only, got stderr: {stderr:?}"
    );
    assert!(
        stdout.contains("NEVER"),
        "GNU: an interactive script continues past a bad return, got stdout: {stdout:?}"
    );
    assert_eq!(code, Some(0), "GNU: the run still exits 0, got {code:?}");
}
