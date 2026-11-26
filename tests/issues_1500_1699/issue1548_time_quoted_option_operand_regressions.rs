//! Regression pins for rubash#1548: a QUOTED dash word after `time`
//! is the timed command's NAME, never time's own option.
//!
//! GNU decides the operand tokens in the lexer (parse.y
//! `special_case_tokens`): `-p` is TIMEOPT only when the immediately
//! preceding token was TIME and the token's RAW spelling is exactly
//! `-p`; `--` is TIMEIGN only after TIME or TIMEOPT; the timespec
//! grammar admits at most one of each, in that order, and TIMEIGN
//! also selects the POSIX report format. A quoted or escaped spelling
//! ('-p', "-p", \-p, -'p') is an ordinary word. Every expectation
//! below was measured against GNU bash 5.3.20 (the vendored oracle):
//! `-p: command not found` + a timing report, status 127 — and when a
//! real executable named `-p` is on PATH, it RUNS.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn rubash() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rubash"))
}

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rubash-issue1548-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }

    fn write(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, body).expect("write scratch file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Output {
    stdout: String,
    stderr: String,
}

fn run_script(path: &Path) -> Output {
    let output = Command::new(rubash())
        .arg(path)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run rubash script");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_stdin(script: &str) -> Output {
    let mut child = Command::new(rubash())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Default format prints `real\t0m...`; the POSIX format prints
/// `real 0.00`.
fn assert_default_format(stderr: &str) {
    assert!(
        stderr.contains("real\t"),
        "expected default timing format: {stderr}"
    );
    assert!(
        !stderr.contains("real 0"),
        "unexpected posix timing format: {stderr}"
    );
}

fn assert_posix_format(stderr: &str) {
    assert!(
        stderr.contains("real 0"),
        "expected posix timing format: {stderr}"
    );
    assert!(
        !stderr.contains("real\t"),
        "unexpected default timing format: {stderr}"
    );
}

#[test]
fn issue1548_single_quoted_dash_p_is_the_command_name() {
    let scratch = Scratch::new("sq");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent; time '-p' true; echo \"rc=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "rc=127\n");
    assert!(
        out.stderr.contains("-p: command not found"),
        "stderr: {}",
        out.stderr
    );
    assert_default_format(&out.stderr);
}

#[test]
fn issue1548_double_quoted_and_escaped_dash_p_are_command_names() {
    let scratch = Scratch::new("dq");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent\n\
         time \"-p\" true; echo \"dq=$?\"\n\
         time \\-p true; echo \"esc=$?\"\n\
         time -'p' true; echo \"part=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "dq=127\nesc=127\npart=127\n");
    assert_eq!(out.stderr.matches("-p: command not found").count(), 3);
}

#[test]
fn issue1548_unquoted_dash_p_stays_the_option() {
    let scratch = Scratch::new("plain");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent; time -p true; echo \"rc=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "rc=0\n");
    assert!(
        !out.stderr.contains("command not found"),
        "stderr: {}",
        out.stderr
    );
    assert_posix_format(&out.stderr);
}

#[test]
fn issue1548_second_dash_p_after_option_is_the_command_name() {
    let scratch = Scratch::new("second");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent\n\
         time -p '-p' true; echo \"quoted=$?\"\n\
         time -p -p true; echo \"repeated=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "quoted=127\nrepeated=127\n");
    assert_eq!(out.stderr.matches("-p: command not found").count(), 2);
    // The first (real) -p still selects the POSIX report format.
    assert_posix_format(&out.stderr);
}

#[test]
fn issue1548_timeign_window_and_posix_format() {
    let scratch = Scratch::new("ign");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent\n\
         time -- '-p'; echo \"quoted=$?\"\n\
         time -- -p true; echo \"after=$?\"\n\
         time -- true; echo \"plain=$?\"\n\
         time -p -- true; echo \"both=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "quoted=127\nafter=127\nplain=0\nboth=0\n");
    assert_eq!(out.stderr.matches("-p: command not found").count(), 2);
    // TIMEIGN selects the POSIX format too (timespec: TIME TIMEIGN
    // -> CMD_TIME_POSIX); every line above is POSIX-format.
    assert_posix_format(&out.stderr);
}

#[test]
fn issue1548_dash_p_after_bang_is_the_command_name() {
    let scratch = Scratch::new("bang");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent\n\
         time ! -p true; echo \"bang=$?\"\n\
         time -p ! -p true; echo \"optbang=$?\"\n",
    );
    let out = run_script(&script);
    // The command `-p` fails 127 and the pipeline `!` negates it.
    assert_eq!(out.stdout, "bang=0\noptbang=0\n");
    assert_eq!(out.stderr.matches("-p: command not found").count(), 2);
}

#[test]
fn issue1548_quoted_bang_is_the_command_name() {
    let scratch = Scratch::new("qbang");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent; time '!' true; echo \"rc=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "rc=127\n");
    assert!(
        out.stderr.contains("!: command not found"),
        "stderr: {}",
        out.stderr
    );
    assert_default_format(&out.stderr);
}

#[test]
fn issue1548_quoted_dash_p_alone_is_the_command_name() {
    let scratch = Scratch::new("alone");
    let script = scratch.write("case.sh", "PATH=/nonexistent; time '-p'; echo \"rc=$?\"\n");
    let out = run_script(&script);
    assert_eq!(out.stdout, "rc=127\n");
    assert!(out.stderr.contains("-p: command not found"));
}

#[test]
fn issue1548_executable_named_dash_p_runs() {
    let scratch = Scratch::new("realbin");
    let bin_dir = scratch.dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("create bin dir");
    let dash_p = bin_dir.join("-p");
    fs::write(&dash_p, "#!/bin/sh\necho dash-p-ran\n").expect("write -p file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dash_p, fs::Permissions::from_mode(0o755)).expect("chmod -p file");
    }
    let script = scratch.write(
        "case.sh",
        &format!(
            "PATH={}:/usr/bin:/bin; time '-p'; echo \"rc=$?\"\n",
            bin_dir.display()
        ),
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "dash-p-ran\nrc=0\n");
    assert_default_format(&out.stderr);
}

#[test]
fn issue1548_quoted_dash_p_in_pipeline_group_and_function() {
    let scratch = Scratch::new("shapes");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent\n\
         time '-p' true | cat; echo \"pipe=${PIPESTATUS[0]}\"\n\
         { time '-p' true; echo \"group=$?\"; }\n\
         f() { time '-p' true; echo \"fn=$?\"; }\n\
         f\n",
    );
    // The pipeline stage needs a working cat: /usr/bin:/bin was just
    // replaced, so run the pipeline line under the harness PATH in a
    // separate script below; here assert the group/function statuses.
    let out = run_script(&script);
    assert!(out.stdout.contains("group=127"), "stdout: {}", out.stdout);
    assert!(out.stdout.contains("fn=127"), "stdout: {}", out.stdout);

    let scratch2 = Scratch::new("pipe");
    let script2 = scratch2.write(
        "case.sh",
        "time '-p' true | cat; echo \"pipe=${PIPESTATUS[0]}\"\n",
    );
    let out2 = run_script(&script2);
    assert_eq!(out2.stdout, "pipe=127\n");
    assert!(out2.stderr.contains("-p: command not found"));
}

#[test]
fn issue1548_quoted_dash_p_posix_mode_and_eval_and_stdin() {
    let scratch = Scratch::new("entries");
    let script = scratch.write(
        "case.sh",
        "set -o posix\n\
         PATH=/nonexistent\n\
         time '-p' true; echo \"posix=$?\"\n\
         eval \"time '-p' true\"; echo \"eval=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "posix=127\neval=127\n");
    // In posix mode the KEYWORD verdict stands (the word after `time`
    // is quoted, so `time` stays reserved) and the failing name is
    // `-p`, not `time`.
    assert_eq!(out.stderr.matches("-p: command not found").count(), 2);

    let stdin_out = run_stdin("PATH=/nonexistent; time '-p' true; echo \"rc=$?\"\n");
    assert_eq!(stdin_out.stdout, "rc=127\n");
    assert!(stdin_out.stderr.contains("-p: command not found"));
}

#[test]
fn issue1548_quoted_ordinary_command_name_still_runs() {
    let scratch = Scratch::new("qtrue");
    let script = scratch.write(
        "case.sh",
        "PATH=/nonexistent; time 'true'; echo \"rc=$?\"\n",
    );
    let out = run_script(&script);
    assert_eq!(out.stdout, "rc=0\n");
    assert_default_format(&out.stderr);
}
