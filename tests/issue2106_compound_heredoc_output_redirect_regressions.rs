//! Issue rubash#2106: a compound command (brace group / function body)
//! carrying BOTH a heredoc input and an output redirect lost the heredoc
//! input for the pipeline stages inside it — the first stage's output
//! never reached the next stage (`{ awk | while read …; } <<EOF > /dev/null`
//! iterated zero times; GNU runs the loop over the heredoc lines).
//!
//! Root cause: a pipeline element runs in a stage subshell cloned from
//! the enclosing executor. The compound's output redirect binds fd 1 in
//! the fd table (open_compound_output_redirects / the group splice), and
//! the clone carried that File binding verbatim, so an external or
//! emulated leaf inside a BUILTIN stage resolved its stdout to the
//! ambient binding instead of the stage capture — the stage materialized
//! as empty and downstream stages starved. GNU execute_cmd.c:2620
//! execute_pipeline forks every element with fd 1 = the inter-stage
//! pipe (execute_cmd.c:6373 do_piping), replacing whatever the enclosing
//! context bound; the compound's binding applies ONCE, to the pipeline
//! as a whole, when the driver routes the final stage's output. The
//! function and compound stage executors already rebind the clone's
//! fd 1 to the default Stdout endpoint (which resolves to the active
//! capture); the builtin stage executor missed that rebind. Fix: the
//! same rebind in execute_builtin_pipeline_stage, for final stages too —
//! the sequential driver materializes every stage's output, and a final
//! stage carrying its own output redirect (or sitting under a
//! pipeline-level redirect) lost its output the same way.
//!
//! Expected outputs are byte-verified against GNU Bash 5.3.20
//! (self-compiled oracle, script-file probes 2026-10-29). Every run is
//! deadline-bounded: sibling shapes of this defect HANG (a starved
//! `command cat` final stage falls back to the ambient fd 0), and a
//! hanging pin must fail, not eat the CI job.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2106-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, body: &str) -> (String, String, Option<i32>) {
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write probe script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let mut out_pipe = child.stdout.take().expect("stdout pipe");
    let mut err_pipe = child.stderr.take().expect("stderr pipe");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rubash hung (>30s) running probe script:\n{body}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_thread.join().expect("stdout drain")).into_owned();
    let stderr = String::from_utf8_lossy(&err_thread.join().expect("stderr drain")).into_owned();
    (stdout, stderr, status.code())
}

fn file_text(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok()
}

/// The issue's minimal form d1: brace group with heredoc + output
/// redirect; the external first stage feeds an internal read loop.
#[test]
fn brace_group_heredoc_and_output_redirect_feeds_pipeline() {
    let dir = scratch("d1");
    let (stdout, stderr, _) = run_script(
        &dir,
        "{ command awk '{print}' | while read -r A B; do printf '%s\\n' \"$B\" > \"t-$A\"; done; } << HD1 > /dev/null\na 1\nb 2\nHD1\n",
    );
    assert_eq!(
        file_text(&dir, "t-a").as_deref(),
        Some("1\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        file_text(&dir, "t-b").as_deref(),
        Some("2\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The issue's form d2: the heredoc rides a brace group inside a
/// function; the CALL carries the output redirect.
#[test]
fn function_call_output_redirect_keeps_inner_group_heredoc() {
    let dir = scratch("d2");
    let (stdout, stderr, _) = run_script(
        &dir,
        "f() { { command awk '{print}' | while read -r A B; do printf '%s\\n' \"$B\" > \"u-$A\"; done; } << HD2\na 1\nb 2\nHD2\n}\nf > /dev/null\n",
    );
    assert_eq!(
        file_text(&dir, "u-a").as_deref(),
        Some("1\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        file_text(&dir, "u-b").as_deref(),
        Some("2\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Same family, `< file` input instead of a heredoc: the compound's
/// fd-0 stream binding must feed the pipeline exactly like the heredoc.
#[test]
fn brace_group_file_input_and_output_redirect_feeds_pipeline() {
    let dir = scratch("file-input");
    std::fs::write(dir.join("input.txt"), "a 1\nb 2\n").expect("write input");
    let (stdout, stderr, _) = run_script(
        &dir,
        "{ command awk '{print}' | while read -r A B; do printf '%s\\n' \"$B\" > \"t-$A\"; done; } < input.txt > /dev/null\n",
    );
    assert_eq!(
        file_text(&dir, "t-a").as_deref(),
        Some("1\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(
        file_text(&dir, "t-b").as_deref(),
        Some("2\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A builtin first stage must reach the stage capture, not the group's
/// fd-1 target: `echo` inside the redirected group feeds the next stage.
#[test]
fn builtin_first_stage_output_reaches_next_stage_in_redirected_group() {
    let dir = scratch("builtin-first");
    let (stdout, stderr, _) = run_script(
        &dir,
        "{ echo hi | command cat > o1.txt; } << HD > /dev/null\nx\nHD\n",
    );
    assert_eq!(
        file_text(&dir, "o1.txt").as_deref(),
        Some("hi\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A function first stage (already rebound) feeding a builtin final
/// stage that carries its OWN output redirect inside the group.
#[test]
fn function_stage_feeds_builtin_final_stage_with_own_redirect() {
    let dir = scratch("func-then-builtin");
    let (stdout, stderr, _) = run_script(
        &dir,
        "g() { command awk '{print}'; }\n{ g | command cat > o2.txt; } << HD > /dev/null\na 1\nb 2\nHD\n",
    );
    assert_eq!(
        file_text(&dir, "o2.txt").as_deref(),
        Some("a 1\nb 2\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A final builtin stage carrying its own output redirect inside a
/// redirected group: the stage's own target wins for its output.
#[test]
fn final_builtin_stage_own_redirect_wins_over_group_binding() {
    let dir = scratch("final-own-redirect");
    let (stdout, stderr, _) = run_script(
        &dir,
        "{ printf 'a 1\\n' | command awk '{print}' > own.txt; } > /dev/null\n",
    );
    assert_eq!(
        file_text(&dir, "own.txt").as_deref(),
        Some("a 1\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard (passed before the fix): a command substitution directly
/// inside a redirected group already rebinds fd 1 to its capture.
#[test]
fn comsub_inside_redirected_group_still_captures() {
    let dir = scratch("comsub-guard");
    let (stdout, stderr, code) = run_script(
        &dir,
        "{ x=$(command awk '{print}' << HD2\na 1\nb 2\nHD2\n); } > /dev/null\nprintf 'x=[%s]\\n' \"$x\"\n",
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "x=[a 1\nb 2]\n", "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard (passed before the fix): a function stage feeding a purely
/// external final stage inside a redirected group.
#[test]
fn function_stage_feeds_external_final_stage_in_redirected_group() {
    let dir = scratch("func-external-guard");
    let awk = "/usr/bin/awk";
    let cat = "/usr/bin/cat";
    if !Path::new(awk).exists() || !Path::new(cat).exists() {
        return; // external-stage guard is Unix-tooling shaped
    }
    let (stdout, stderr, _) = run_script(
        &dir,
        &format!(
            "g() {{ command awk '{{print}}'; }}\n{{ g | {cat} > o5.txt; }} << HD > /dev/null\na 1\nb 2\nHD\n"
        ),
    );
    assert_eq!(
        file_text(&dir, "o5.txt").as_deref(),
        Some("a 1\nb 2\n"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
