//! rubash#3030 (eco-2906 W3) — a command word whose EXPANDED text ends in
//! a bare `${` (e.g. the literal command name produced by `$'f+oo${'`)
//! earned a spurious `f+oo${: bad substitution` line on top of GNU's
//! `f+oo${: command not found`.
//!
//! GNU anchor: parse.y:5549-5568 (read_token_word) gathers a `$'...'`
//! span with parse_matched_pair — only the quote delimiter is special
//! inside, `${` is plain bytes — then ansiexpand + sh_single_quote rewrite
//! it as an ordinary single-quoted string, so the expansion engine never
//! sees a live `${`. The resulting word is looked up as a command name
//! verbatim (execute_cmd.c:6139 shell_execve receives the already-expanded
//! argv) and simply not found: one diagnostic, status 127.
//!
//! rubash root cause: the external-command dispatch receives the
//! materialized (already expanded) command node, but
//! execute_same_shell_script / enoexec_script_spelling ran expand_word
//! over the expanded name (and the script operand) a SECOND time as a
//! classification probe. Expansion DATA that looks like syntax — a
//! trailing `${` left literal by quoting — was then walked as a live
//! fragment: the empty unterminated body hit the bad-substitution
//! default (subst.c:10278's verdict, which in GNU only ever fires on
//! source text the parser actually handed to the expander). The fix
//! classifies on the materialized text directly, like shell_execve.
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash`, byte-level stdout
//! assertions plus exit code and stderr assertions. `LC_ALL=C.UTF-8` is
//! set on the child explicitly so the pin is hermetic under any ambient
//! locale.

use std::process::{Command, Stdio};

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: i32,
}

fn run(script: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code().unwrap_or(-1),
    }
}

/// A literal command name ending in `${`: exactly one diagnostic line,
/// GNU's command-not-found, status 127, no bad substitution anywhere.
fn expect_plain_not_found(script: &str, name: &str, ctx: &str) {
    let out = run(script);
    assert_eq!(
        out.code,
        127,
        "{ctx}: rc must be 127; stdout: {:?} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{ctx}: stdout must be empty; got {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !stderr.contains("bad substitution"),
        "{ctx}: stderr must not contain a bad substitution line; got {stderr}"
    );
    assert!(
        stderr
            .trim_end()
            .ends_with(&format!("{name}: command not found")),
        "{ctx}: stderr must end with the command-not-found line for {name:?}; got {stderr}"
    );
    assert_eq!(
        stderr.matches('\n').count(),
        1,
        "{ctx}: stderr must be exactly one line; got {stderr}"
    );
}

#[test]
fn dollar_quoted_unclosed_brace_command_word() {
    expect_plain_not_found("$'f+oo${'", "f+oo${", "repro: $'f+oo${' as command");
}

#[test]
fn dollar_quoted_bare_brace_command_word() {
    expect_plain_not_found("$'${'", "${", "repro: $'${' as command");
}

#[test]
fn plain_single_quoted_unclosed_brace_command_word() {
    // Same mechanism, no ANSI-C quoting involved: any command name whose
    // expanded text ends in a bare `${` was re-walked as live syntax.
    expect_plain_not_found("'f+oo${'", "f+oo${", "plain single quotes as command");
}

#[test]
fn command_builtin_unclosed_brace_word() {
    expect_plain_not_found("command $'f+oo${'", "f+oo${", "command builtin dispatch");
}

#[test]
fn argument_position_stays_literal() {
    let out = run("echo $'f+oo${'");
    assert_eq!(out.code, 0, "arg position rc");
    assert_eq!(out.stdout, b"f+oo${\n", "arg position stdout");
    assert!(
        out.stderr.is_empty(),
        "arg position stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn assignment_position_stays_literal() {
    let out = run("x=$'f+oo${'; echo \"$x\"");
    assert_eq!(out.code, 0, "assignment rc");
    assert_eq!(out.stdout, b"f+oo${\n", "assignment stdout");
    assert!(out.stderr.is_empty(), "assignment stderr must be empty");
}

#[test]
fn name_after_brace_is_not_a_defect() {
    // `${x` at the end of the name: the brace body has a name char, which
    // never raised the spurious verdict; keep it that way.
    expect_plain_not_found("$'f+oo${x'", "f+oo${x", "${x-terminated name");
}

#[test]
fn paren_after_dollar_is_not_a_defect() {
    expect_plain_not_found("$'f+oo$('", "f+oo$(", "$( in $'...' name");
}

#[test]
fn pipeline_member_stays_clean() {
    let out = run("$'f+oo${' | cat");
    assert_eq!(out.code, 0, "pipeline rc is the last stage's");
    assert!(out.stdout.is_empty(), "pipeline stdout must be empty");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !stderr.contains("bad substitution"),
        "pipeline stderr must not contain bad substitution; got {stderr}"
    );
    assert!(
        stderr.contains("f+oo${: command not found"),
        "pipeline stderr must carry the not-found line; got {stderr}"
    );
}

#[test]
fn this_sh_source_form_still_runs() {
    // The same-shell classification the re-expansion used to serve: the
    // SOURCE form ${THIS_SH} is expanded by the ordinary word pipeline
    // (materialization), so the classification must keep working from
    // the materialized name alone.
    let out = run("${THIS_SH} -c 'echo INNER-OK'");
    assert_eq!(out.code, 0, "THIS_SH -c rc");
    assert_eq!(out.stdout, b"INNER-OK\n", "THIS_SH -c stdout");
    assert!(
        out.stderr.is_empty(),
        "THIS_SH -c stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn quoted_this_sh_value_is_not_the_shell() {
    // A QUOTED '${THIS_SH}' value is data, not the shell path: GNU looks
    // the literal name up and reports it not found. The classification
    // must not re-expand data into a same-shell match.
    let out = run("p='${THIS_SH}'; \"$p\" -c 'echo SHOULD-NOT-RUN'");
    assert_eq!(out.code, 127, "quoted THIS_SH value rc");
    assert!(out.stdout.is_empty(), "quoted THIS_SH value stdout");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("${THIS_SH}: command not found"),
        "quoted THIS_SH value must be reported verbatim; got {stderr}"
    );
}

/// A scratch bin dir holding files whose names end in `${`, for the
/// PATH-found and ENOEXEC cells.
#[cfg(unix)]
fn scratch_bin() -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("rubash-issue3030-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch bin");
    let script = dir.join("f+oo${");
    std::fs::write(&script, "#!/bin/sh\necho SCRIPT-RAN\n").expect("write script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod script");
    let enoexec = dir.join("e+oo${");
    std::fs::write(&enoexec, "echo ENOEXEC-RAN\n").expect("write enoexec script");
    std::fs::set_permissions(&enoexec, std::fs::Permissions::from_mode(0o755))
        .expect("chmod enoexec script");
    dir
}

#[cfg(unix)]
#[test]
fn found_in_path_runs_without_bad_substitution() {
    let dir = scratch_bin();
    let script = format!("export PATH={}:$PATH; $'f+oo${{'", dir.display());
    let out = run(&script);
    assert_eq!(out.code, 0, "found-in-PATH rc");
    assert_eq!(out.stdout, b"SCRIPT-RAN\n", "found-in-PATH stdout");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !stderr.contains("bad substitution"),
        "found-in-PATH stderr must be clean; got {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "found-in-PATH stderr must be empty; got {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn enoexec_script_runs_without_bad_substitution() {
    // The shebangless shape reached BOTH re-expansion sites (same-shell
    // classification and the ENOEXEC $0 spelling), so the spurious line
    // appeared twice on the base tree.
    let dir = scratch_bin();
    let script = format!("export PATH={}:$PATH; $'e+oo${{'", dir.display());
    let out = run(&script);
    assert_eq!(out.code, 0, "ENOEXEC rc");
    assert_eq!(out.stdout, b"ENOEXEC-RAN\n", "ENOEXEC stdout");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        !stderr.contains("bad substitution"),
        "ENOEXEC stderr must be clean; got {stderr}"
    );
    assert!(
        stderr.is_empty(),
        "ENOEXEC stderr must be empty; got {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn this_sh_with_script_operand_still_runs() {
    let dir = scratch_bin();
    let operand = dir.join("operand.sh");
    std::fs::write(&operand, "echo ARGSCRIPT-RAN\n").expect("write operand script");
    let script = format!("${{THIS_SH}} {}", operand.display());
    let out = run(&script);
    assert_eq!(out.code, 0, "THIS_SH script operand rc");
    assert_eq!(
        out.stdout, b"ARGSCRIPT-RAN\n",
        "THIS_SH script operand stdout"
    );
    assert!(
        out.stderr.is_empty(),
        "THIS_SH script operand stderr must be empty; got {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
