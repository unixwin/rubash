//! rubash#2302 — a script invoked by bare name and found through
//! PATH recorded the bare name in BASH_SOURCE[0]; GNU records the
//! path actually opened (the PATH-search result) on both invocation
//! faces:
//!
//! - Command face: execute_disk_command resolves the word through
//!   find_user_command (findcmd.c:247; find_user_command_in_path
//!   returns the PATH-joined full_path, findcmd.c:623) and
//!   shell_execve's ENOEXEC re-exec replaces the typed word with
//!   that resolved name (execute_cmd.c:6237-6260), so $0
//!   (shell.c:1613), the BASH_SOURCE push (variables.c:5718
//!   push_source via shell.c:1640) and the in-script diagnostic
//!   prefix are all the resolved path.
//! - Program face (`rubash myscript`): open_shell_script replaces
//!   filename with the PATH find (shell.c:1595-1597) and push_source
//!   records it (shell.c:1640), while dollar_vars[0] keeps the
//!   spelled name (shell.c:1613) — $0 stays bare, BASH_SOURCE[0]
//!   resolves.
//!
//! A word carrying a slash is never PATH-searched (findcmd.c:385)
//! and keeps its typed spelling on both faces.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the cluster probe table (probes/probe-all.sh): the bare-name
//! cells are red on master 1cdace4c (BS0=myscript) and green with
//! the fix; the direct-path, ./ and not-in-PATH controls match on
//! both.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2302_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("create fixture bin dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let bin = root.join("bin");
    write_script(
        &bin,
        "myscript",
        "echo \"ZERO=$0\"\necho \"BS0=${BASH_SOURCE[0]}\"\n",
    );
    write_script(
        &bin,
        "errscript",
        "echo before\nnosuchcmd_zzz\necho after\n",
    );
    Fixture { root, bin }
}

fn write_script(bin: &PathBuf, name: &str, body: &str) {
    let path = bin.join(name);
    std::fs::write(&path, body).expect("write fixture script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture script");
    }
}

impl Fixture {
    fn path_env(&self) -> String {
        format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn run(
        &self,
        args: &[&str],
        path: Option<String>,
        cwd: Option<&PathBuf>,
    ) -> (String, String, Option<i32>) {
        let mut cmd = Command::new(RUBASH);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = path {
            cmd.env("PATH", path);
        }
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        let out = cmd
            .spawn()
            .expect("spawn rubash")
            .wait_with_output()
            .expect("wait rubash");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }

    fn script_abs(&self, name: &str) -> String {
        self.bin.join(name).display().to_string()
    }
}

/// Command face, bare name via PATH: BASH_SOURCE[0] (and $0, per the
/// re-exec word replacement) are the resolved path. The issue's
/// reproducer.
#[cfg(unix)]
#[test]
fn issue2302_command_face_bare_name_records_resolved_path() {
    let fx = fixture("cmdf");
    let (stdout, _stderr, code) = fx.run(&["-c", "myscript"], Some(fx.path_env()), None);
    let abs = fx.script_abs("myscript");
    assert_eq!(stdout, format!("ZERO={abs}\nBS0={abs}\n"));
    assert_eq!(code, Some(0));
}

/// Command face, direct absolute path: unchanged control.
#[cfg(unix)]
#[test]
fn issue2302_command_face_direct_path_control() {
    let fx = fixture("direct");
    let abs = fx.script_abs("myscript");
    let (stdout, _stderr, code) = fx.run(&["-c", &abs], None, None);
    assert_eq!(stdout, format!("ZERO={abs}\nBS0={abs}\n"));
    assert_eq!(code, Some(0));
}

/// Command face, ./ spelling: a slash word is never searched, the
/// typed spelling stands on both faces.
#[cfg(unix)]
#[test]
fn issue2302_slash_spelling_kept_control() {
    let fx = fixture("slash");
    let (stdout, _stderr, code) = fx.run(&["-c", "./myscript"], None, Some(&fx.bin));
    assert_eq!(stdout, "ZERO=./myscript\nBS0=./myscript\n");
    assert_eq!(code, Some(0));
}

/// Command face: the in-script diagnostic prefix names the resolved
/// file, not the bare word.
#[cfg(unix)]
#[test]
fn issue2302_command_face_diagnostic_prefix_resolved() {
    let fx = fixture("diagc");
    let (stdout, stderr, _code) = fx.run(&["-c", "errscript"], Some(fx.path_env()), None);
    let abs = fx.script_abs("errscript");
    assert_eq!(stdout, "before\nafter\n");
    assert!(
        stderr.contains(&format!("{abs}: line 2: nosuchcmd_zzz: command not found")),
        "stderr: {stderr}"
    );
}

/// Program face, bare name via PATH: BASH_SOURCE[0] resolves, $0
/// keeps the spelled name (shell.c:1640 vs shell.c:1613).
#[cfg(unix)]
#[test]
fn issue2302_program_face_bare_name_records_resolved_path() {
    let fx = fixture("progf");
    let (stdout, _stderr, code) = fx.run(&["myscript"], Some(fx.path_env()), None);
    let abs = fx.script_abs("myscript");
    assert_eq!(stdout, format!("ZERO=myscript\nBS0={abs}\n"));
    assert_eq!(code, Some(0));
}

/// Program face: the in-script diagnostic prefix names the resolved
/// file too.
#[cfg(unix)]
#[test]
fn issue2302_program_face_diagnostic_prefix_resolved() {
    let fx = fixture("diagp");
    let (stdout, stderr, _code) = fx.run(&["errscript"], Some(fx.path_env()), None);
    let abs = fx.script_abs("errscript");
    assert_eq!(stdout, "before\nafter\n");
    assert!(
        stderr.contains(&format!("{abs}: line 2: nosuchcmd_zzz: command not found")),
        "stderr: {stderr}"
    );
}

/// Control: a bare name PATH cannot resolve is still refused with
/// 127 / command not found (the parent shell's diagnostic keeps the
/// bare word — the name was never resolved to anything).
#[cfg(unix)]
#[test]
fn issue2302_not_in_path_refusal_control() {
    let fx = fixture("notinpath");
    let (_stdout, stderr, code) = fx.run(&["-c", "myscript"], None, None);
    assert_eq!(code, Some(127));
    assert!(
        stderr.contains("myscript: command not found"),
        "stderr: {stderr}"
    );
    let _ = fx.root;
}
