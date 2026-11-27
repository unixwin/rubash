//! W4 backfill slice 1 — program-entry / invocation faces.
//!
//! * rust#171 — `$0` of a script run by path is the path as invoked.
//! * rust#172 — `exec env ASSIGN ./script` replaces the shell, applies
//!   the assignment, and runs the target script (rc 0).
//! * rust#253 — an exported function materializes in a child's
//!   environment as exactly one `BASH_FUNC_<name>%%=` entry.
//! * rust#599 — when the binary is invoked under the name `bash`
//!   (symlink), `-c` mode reports `$0` as `bash`.
//! * rust#1084 — `--pretty-print` combined with `-c` still executes the
//!   command string (prints its output, rc 0).
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
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

    fn base_cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir);
        c
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .output()
            .unwrap();
        self.finish(out)
    }

    fn run_args(&self, args: &[&str]) -> (String, String, i32) {
        let out = self
            .base_cmd(env!("CARGO_BIN_EXE_rubash"))
            .args(args)
            .output()
            .unwrap();
        self.finish(out)
    }

    /// Invoke the built binary through a symlink called `name`, with
    /// argv[0] set to exactly `name` (as a PATH lookup would produce).
    /// The probe is unix exec mechanics (symlink + explicit argv[0]
    /// override via `CommandExt::arg0`); std exposes no argv[0] override
    /// on Windows, where argv[0] follows the program path.
    #[cfg(unix)]
    fn run_as_name(&self, name: &str, args: &[&str]) -> (String, String, i32) {
        let bindir = self.dir.join("bin");
        fs::create_dir_all(&bindir).unwrap();
        let link = bindir.join(name);
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_rubash"), &link).unwrap();
        let out = self
            .base_cmd(link.to_str().unwrap())
            .arg0(name)
            .args(args)
            .output()
            .unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue171_dollar_zero_is_script_path_as_invoked() {
    let s = Scratch::new("i171");
    let (out, err, rc) = s.run("echo \"zero=$0\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "zero=$S/case.sh\n");
}

#[test]
fn issue172_exec_env_assignment_runs_target() {
    let s = Scratch::new("i172");
    let (out, err, rc) = s.run(
        "printf '#!/bin/sh\\necho inner-ran\\n' > inner.sh\nchmod +x inner.sh\nexec env MARKER=1 ./inner.sh\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "inner-ran\n");
}

#[test]
fn issue253_exported_function_materializes_once_in_env() {
    let s = Scratch::new("i253");
    let (out, err, rc) = s.run(
        "myf() { echo from-myf; }\nexport -f myf\n/usr/bin/env | grep -c '^BASH_FUNC_myf%%='\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "1\n");
}

#[cfg(unix)]
#[test]
fn issue599_invoked_as_bash_reports_bash_dollar_zero() {
    let s = Scratch::new("i599");
    let (out, err, rc) = s.run_as_name("bash", &["-c", "echo $0"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "bash\n");
}

#[test]
fn issue1084_pretty_print_with_dash_c_still_executes() {
    let s = Scratch::new("i1084");
    let (out, err, rc) = s.run_args(&["--pretty-print", "-c", "echo hi"]);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "hi\n");
}
