//! rubash#3201 — command substitution must deliver the shell's
//! ambient descriptors to external children, on EVERY comsub path.
//!
//! Repeatability measurement first (the issue reports an unstable
//! shape; the lane measured before touching any fix, base tree,
//! N=20 per shape, enumerator = external bash listing /proc/$$/fd):
//!   trivial body     `x=$(/usr/bin/bash enum.sh)`        0/20 delivered
//!   non-trivial body `x=$(/usr/bin/bash enum.sh; echo e)` 10/10 delivered
//!     (with extra alias numbers {6,8} — the #2688 face, not this one)
//! So the "instability" is PATH SELECTION, not timing: a trivial body
//! takes the word-level shortcut spawner
//! (command_substitution_values::run_external_command_substitution),
//! which registered no ambient fd pins at all, while a non-trivial
//! body falls through to the full executor, whose spawn delivers the
//! ambient set. GNU has one path: subst.c:7143 command_substitute
//! forks, and the forked child inherits the shell's descriptors at
//! their numbers (oracle 5.3.20: both shapes see exactly 3->f3.txt,
//! 5->f5.txt).
//!
//! The fix registers the ambient pin set on the shortcut spawner too
//! (ambient_fd_pins_unbound + pin_ambient_fds_in_child, the pipeline
//! faces' existing transport). Pin sources are the fd table's own
//! handles, so no inheritable duplicate is left behind in the child:
//! the target files must appear at EXACTLY their slot numbers (the
//! #2688 alias set must not appear on this face).
//!
//! Assertion style follows the F3 pins: per target file, the set of
//! fd numbers naming it, plus presence — internal pipe descriptors
//! (the capture machinery) vary and are never asserted.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-3201-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

fn run_rubash(dir: &Path, script: &str) -> (String, String, Option<i32>) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn rubash");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait().expect("poll rubash") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rubash did not exit within 30s");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    use std::io::Read;
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr, status.code())
}

/// Write the fd enumerator (an external GNU bash script listing its
/// own /proc/$$/fd as `N basename` lines) into the scratch dir and
/// return its path.
fn write_enum(dir: &Path) -> String {
    let path = dir.join("enum.sh");
    std::fs::write(
        &path,
        "#!/usr/bin/bash\nfor d in /proc/$$/fd/*; do\n  n=${d##*/}\n  t=$(readlink \"$d\")\n  echo \"$n ${t##*/}\"\ndone\n",
    )
    .expect("write enum");
    path.to_string_lossy().into_owned()
}

/// The set of fd numbers in `enum_output` whose target basename is
/// `base`.
fn fd_set_for(enum_output: &str, base: &str) -> BTreeSet<u32> {
    enum_output
        .lines()
        .filter_map(|line| {
            let (num, name) = line.split_once(' ')?;
            if name == base {
                num.parse::<u32>().ok()
            } else {
                None
            }
        })
        .collect()
}

fn set_of(values: &[u32]) -> BTreeSet<u32> {
    values.iter().copied().collect()
}

/// Stage: f3.txt/f5.txt written, enum written; returns the script
/// that opens 3 and 5 and runs the given comsub body expression.
fn staged(dir: &Path, body: &str) -> String {
    std::fs::write(dir.join("f3.txt"), "F3\n").expect("write f3");
    std::fs::write(dir.join("f5.txt"), "F5\n").expect("write f5");
    let enum_path = write_enum(dir);
    format!(
        "exec 3<f3.txt 5<f5.txt\nx=$({body})\necho \"BEGIN\"\necho \"$x\"\necho \"END\"\n",
        body = body.replacen("ENUM", &enum_path, 1)
    )
}

#[test]
fn issue3201_trivial_comsub_delivers_ambient_fds() {
    let dir = scratch("trivial");
    let script = staged(&dir.0, "/usr/bin/bash ENUM");
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        fd_set_for(&stdout, "f3.txt"),
        set_of(&[3]),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert_eq!(
        fd_set_for(&stdout, "f5.txt"),
        set_of(&[5]),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
}

#[test]
fn issue3201_trivial_comsub_write_fd_delivered() {
    let dir = scratch("writefd");
    std::fs::write(dir.0.join("f3.txt"), "F3\n").expect("write f3");
    let enum_path = write_enum(&dir.0);
    let script = format!("exec 3<f3.txt 9>f9.txt\nx=$(/usr/bin/bash {enum_path})\necho \"$x\"\n");
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(
        fd_set_for(&stdout, "f3.txt"),
        set_of(&[3]),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert_eq!(
        fd_set_for(&stdout, "f9.txt"),
        set_of(&[9]),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
}

#[test]
fn issue3201_trivial_comsub_delivery_is_stable_across_repeats() {
    // The issue's core symptom is instability: the same shape must
    // deliver on every run, not intermittently. Five consecutive
    // runs of the trivial shape in one shell session.
    let dir = scratch("stable");
    std::fs::write(dir.0.join("f3.txt"), "F3\n").expect("write f3");
    std::fs::write(dir.0.join("f5.txt"), "F5\n").expect("write f5");
    let enum_path = write_enum(&dir.0);
    let mut script = String::from("exec 3<f3.txt 5<f5.txt\n");
    for i in 0..5 {
        script.push_str(&format!("x{i}=$(/usr/bin/bash {enum_path})\n"));
        script.push_str(&format!("echo \"RUN{i}\"\necho \"$x{i}\"\n"));
    }
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    // Every one of the five runs must contribute exactly one
    // `3 f3.txt` and one `5 f5.txt` line, and no other fd number may
    // name either file in any run.
    assert_eq!(
        stdout.lines().filter(|line| *line == "3 f3.txt").count(),
        5,
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert_eq!(
        stdout.lines().filter(|line| *line == "5 f5.txt").count(),
        5,
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert_eq!(fd_set_for(&stdout, "f3.txt"), set_of(&[3]));
    assert_eq!(fd_set_for(&stdout, "f5.txt"), set_of(&[5]));
}

#[test]
fn issue3201_nontrivial_comsub_still_delivers() {
    // Regression lock for the full-executor path: presence is
    // asserted, exactness is not — that path's inheritable-duplicate
    // transport can add alias numbers, which is #2688's face (the
    // W-F3 lane), not this issue's.
    let dir = scratch("nontrivial");
    let script = staged(&dir.0, "/usr/bin/bash ENUM; echo extra");
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        fd_set_for(&stdout, "f3.txt").contains(&3),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert!(
        fd_set_for(&stdout, "f5.txt").contains(&5),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
    assert!(stdout.contains("extra"), "stdout:\n{stdout}");
}

#[test]
fn issue3201_direct_external_still_delivers() {
    // Regression lock: the plain foreground face (the F3 pins' home)
    // keeps delivering — presence only, for the same #2688 reason.
    let dir = scratch("direct");
    std::fs::write(dir.0.join("f3.txt"), "F3\n").expect("write f3");
    let enum_path = write_enum(&dir.0);
    let script = format!("exec 3<f3.txt\n/usr/bin/bash {enum_path}\n");
    let (stdout, stderr, code) = run_rubash(&dir.0, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert!(
        fd_set_for(&stdout, "f3.txt").contains(&3),
        "stdout:\n{stdout} stderr:\n{stderr}"
    );
}
