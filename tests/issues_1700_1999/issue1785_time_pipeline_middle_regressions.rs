//! Issue rubash#1785: a `time` word in a non-head pipeline position
//! was folded into a per-stage TimeCommand by the parser, so
//! `echo hi | time cat` ran rubash's own timing instead of resolving
//! the command name `time` through PATH.
//!
//! GNU: the timespec production belongs to pipeline_command
//! (parse.y:1416 `timespec pipeline_command`); a pipeline's later
//! elements derive from `command` alone, and
//! `time_command_acceptable` (parse.y:3286) has no `|' case — after
//! a pipe connector `time` is an ordinary WORD. Verified against
//! GNU bash 5.3.20 (self-compiled oracle), script-file mode.

use std::process::Command;

fn rubash_file(script: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1785-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("case.sh"), script).expect("write case.sh");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("case.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash file");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// The shell-format time report (a line starting `real`/`user`/`sys`
/// in POSIX form, or the bare `user`/`sys` block) must not appear:
/// the middle stage is not timed at all.
fn assert_no_time_report(stderr: &str) {
    assert!(
        !stderr.lines().any(|line| {
            line.starts_with("real\t")
                || line.starts_with("real ")
                || line.starts_with("user\t")
                || line.starts_with("sys\t")
        }),
        "unexpected time report in stderr: {stderr}"
    );
}

#[test]
fn issue1785_pipeline_middle_time_is_a_command_name() {
    // PATH pinned empty inside the script so no external `time` can
    // mask the verdict (the #1520 harness idiom).
    let (stdout, stderr, code) =
        rubash_file("PATH=/nonexistent\necho hi | time cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_no_time_report(&stderr);
    assert_eq!(code, Some(0));
}

#[test]
fn issue1785_pipeline_middle_time_posix_option_is_a_command_name() {
    let (stdout, stderr, _) =
        rubash_file("PATH=/nonexistent\necho hi | time -p cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_no_time_report(&stderr);
}

#[test]
fn issue1785_pipeline_last_stage_time_is_a_command_name() {
    let (stdout, stderr, _) =
        rubash_file("PATH=/nonexistent\necho hi | cat | time cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_no_time_report(&stderr);
}

#[test]
fn issue1785_pipeline_head_time_still_times_the_pipeline() {
    // Control: the pipeline HEAD keeps the timespec — GNU times the
    // whole pipeline and the command runs.
    let (stdout, stderr, code) = rubash_file("time echo hi | cat\necho \"S=$?\"\n");
    assert_eq!(stdout, "hi\nS=0\n");
    assert!(
        stderr.lines().any(|line| line.starts_with("user\t")),
        "head time report missing: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue1785_group_stage_time_still_times_inside_the_group() {
    // Control: a brace group in a later pipeline position starts a
    // fresh command list, whose head `time` IS the keyword (GNU
    // `echo hi | { time echo x; }` times the echo).
    let (stdout, _, code) = rubash_file("echo hi | { time echo x; }\necho \"S=$?\"\n");
    assert_eq!(stdout, "x\nS=0\n");
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue1785_pipeline_middle_time_resolves_an_external_program() {
    // With an external `time` on PATH, the middle stage runs THAT
    // program (with the stage's arguments), not the shell keyword.
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue1785-stub-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("stub dir");
    let stub = dir.join("time");
    // The script pins PATH to the stub dir alone, so the stub's own
    // stdin relay must not resolve through PATH either: use the
    // absolute /bin/cat (the pin is unix-gated).
    std::fs::write(
        &stub,
        "#!/bin/sh\necho \"EXTERNAL_TIME_RAN args=$*\"\nexec /bin/cat\n",
    )
    .expect("write stub");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod stub");
    let script = format!(
        "PATH={}\necho hi | time cat\necho \"S=$?\"\n",
        dir.display()
    );
    let (stdout, stderr, code) = rubash_file(&script);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(stdout, "EXTERNAL_TIME_RAN args=cat\nhi\nS=0\n");
    assert_no_time_report(&stderr);
    assert_eq!(code, Some(0));
}

#[test]
fn issue1785_alias_introduced_time_in_middle_stage_is_a_command_name() {
    // Alias expansion is lexical: text an alias substitutes into a
    // non-head pipeline position is re-read in that position, so a
    // substituted `time` is a command name there too (GNU: 127).
    let (stdout, stderr, _) = rubash_file(
        "shopt -s expand_aliases\nalias tc='time cat'\nPATH=/nonexistent\necho hi | tc\necho \"S=$?\"\n",
    );
    assert_eq!(stdout, "S=127\n");
    assert!(
        stderr.contains("time: command not found"),
        "stderr: {stderr}"
    );
    assert_no_time_report(&stderr);
}
