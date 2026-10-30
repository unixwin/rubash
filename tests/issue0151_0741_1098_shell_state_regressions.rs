//! W4 backfill slice 1 — shell option / job-state faces.
//!
//! * rust#151 — `set -m` in a script turns the `m` letter on in `$-`
//!   (GNU script value: `hmB`; after `set +m`: `hB`). On the invocation
//!   side, `-m` is an option (not a script operand): without a tty GNU
//!   warns on stderr and runs the `-c` string with $- = `hBc` (no `m`);
//!   `-i -c` includes the `H` letter (GNU: `hiBHc`, or `himBHc` when
//!   the runner grants the shell a controlling terminal and job
//!   control comes up — the `m` letter follows the environment).
//! * rust#741 — a script that kills itself with a fatal signal must die
//!   by that signal (a waiting parent observes signal death, not a
//!   synthetic exit code), and its post-kill commands never run.
//! * rust#1098 — `${PS1@P}` renders `\j` as the live background-job
//!   count.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-bfw4-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn base_cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_rubash"));
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir);
        c
    }

    fn norm(&self, b: &[u8]) -> String {
        String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S")
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        (
            self.norm(&out.stdout),
            self.norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = self.base_cmd().arg(&case).output().unwrap();
        self.finish(out)
    }

    fn run_args(&self, args: &[&str]) -> (String, String, i32) {
        let out = self.base_cmd().args(args).output().unwrap();
        self.finish(out)
    }

    /// Run a script and report (stdout, stderr, terminating signal).
    /// Death-by-signal is a unix wait-status concept (`ExitStatusExt`);
    /// a Windows exit status carries only an exit code.
    #[cfg(unix)]
    fn run_signal(&self, script: &str) -> (String, String, Option<i32>) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = self.base_cmd().arg(&case).output().unwrap();
        (
            self.norm(&out.stdout),
            self.norm(&out.stderr),
            out.status.signal(),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue151_set_m_toggles_m_letter_in_dollar_dash() {
    let s = Scratch::new("i151a");
    let (out, err, rc) = s.run("set -m\necho \"flags=$-\"\nset +m\necho \"flags2=$-\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "flags=hmB\nflags2=hB\n");
}

#[test]
fn issue151_dash_m_invocation_is_an_option_not_a_script() {
    let s = Scratch::new("i151b");
    let (out, _err, rc) = s.run_args(&["-m", "-c", "echo \"flags=$-\""]);
    assert_eq!(rc, 0);
    assert_eq!(out, "flags=hBc\n");
}

#[test]
fn issue151_interactive_dash_c_includes_h_letter() {
    let s = Scratch::new("i151c");
    let (out, _err, rc) = s.run_args(&["-i", "-c", "echo \"flags=$-\""]);
    assert_eq!(rc, 0);
    // The guarded shape is the `H` letter. GNU 5.3.20 measured two
    // environment-dependent values: `hiBHc` when the shell cannot
    // claim a terminal (piped/null stdio), and `himBHc` when the
    // runner grants a controlling terminal (session leader + ctty),
    // where job control legitimately turns `m` on. Accept exactly
    // those two; any other letter drift still fails (rubash#2175).
    let flags = out.trim().strip_prefix("flags=").unwrap_or("");
    assert!(
        flags == "hiBHc" || flags == "himBHc",
        "expected $- to be hiBHc (or himBHc with job control), got stdout: {out}"
    );
}

#[cfg(unix)]
#[test]
fn issue741_fatal_signal_kills_the_shell_itself() {
    let s = Scratch::new("i741");
    let (out, _err, sig) = s.run_signal("kill -TERM $$\necho after\n");
    assert_eq!(out, "");
    assert_eq!(sig, Some(15), "shell must die by SIGTERM, not an exit code");
}

#[test]
fn issue1098_prompt_jobs_escape_counts_background_jobs() {
    let s = Scratch::new("i1098");
    let (out, err, rc) = s.run(
        "sleep 0.3 &\nPS1='\\j'\necho \"jobs=[${PS1@P}]\"\nkill %1 2>/dev/null\nwait 2>/dev/null\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "jobs=[1]\n");
}
