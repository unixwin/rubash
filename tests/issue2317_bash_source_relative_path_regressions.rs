//! rubash#2317 — the #2302 residual: for a script found through a
//! RELATIVE PATH element, GNU records the as-written `<elem>/<name>`
//! join; rubash recorded the session-cwd-anchored absolute form.
//!
//! GNU ground: the PATH walk builds one string per element,
//! `full_path = sh_makepath (xpath, name, 0)` in find_in_path_element
//! (findcmd.c, called from find_user_command_in_path, findcmd.c:623),
//! with the element verbatim as written — and that single string is
//! what is recorded:
//! - Command face: shell_execve's ENOEXEC re-exec replaces the typed
//!   word with it (execute_cmd.c:6237-6260), so $0, the BASH_SOURCE
//!   push (variables.c:5718 push_source via shell.c:1640) and the
//!   in-script diagnostic prefix are all the join.
//! - Program face: open_shell_script replaces filename with the
//!   find_path_file result (shell.c:1595-1597) and push_source
//!   records it (shell.c:1640), while dollar_vars[0] keeps the
//!   spelled name (shell.c:1613). The search only fires when the
//!   as-given open fails, so the program-face fixtures keep the
//!   script OUT of the shell's cwd.
//!
//! Expectations byte-verified against GNU bash 5.3.20 (oracle) with
//! the cluster probe table (probes/probe-all.sh): every relative
//! cell is red on master d758fd05 (anchored `$PWD/<elem>/<name>`)
//! and green with the fix; the absolute-element and as-given
//! controls match on both.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

struct Fixture {
    root: PathBuf,
    sub: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2317_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).expect("create fixture sub dir");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let sub = root.join("sub");
    // Command-face fixtures: the bare word is always PATH-searched,
    // so copies live in the cwd (for the `.` element) and in sub/.
    write_script(
        &root,
        "relscript",
        "printf \"ZERO=%s\\nBS0=%s\\n\" \"$0\" \"${BASH_SOURCE[0]}\"\n",
    );
    write_script(
        &root,
        "errscript",
        "echo first\nnosuchcmd_zzz\necho after\n",
    );
    write_script(
        &sub,
        "relscript",
        "printf \"ZERO=%s\\nBS0=%s\\n\" \"$0\" \"${BASH_SOURCE[0]}\"\n",
    );
    write_script(&sub, "errscript", "echo first\nnosuchcmd_zzz\necho after\n");
    // Program-face fixtures: the search only fires when the as-given
    // open fails, so these names exist ONLY under sub/.
    write_script(
        &sub,
        "subscript",
        "printf \"ZERO=%s\\nBS0=%s\\n\" \"$0\" \"${BASH_SOURCE[0]}\"\n",
    );
    write_script(&sub, "suberr", "echo first\nnosuchcmd_zzz\necho after\n");
    Fixture { root, sub }
}

fn write_script(dir: &PathBuf, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture script");
    }
}

impl Fixture {
    fn path_with(&self, elem: &str) -> String {
        format!("{elem}:/usr/bin:/bin")
    }

    fn run(&self, args: &[&str], path: String) -> (String, String, Option<i32>) {
        let out = Command::new(RUBASH)
            .args(args)
            .env("PATH", path)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
}

/// Command face, element `.`: $0 and BASH_SOURCE[0] are the
/// as-written join `./relscript`, not the anchored absolute form.
#[cfg(unix)]
#[test]
fn issue2317_command_face_dot_element_records_as_written_join() {
    let fx = fixture("cmddot");
    let (stdout, _stderr, code) = fx.run(&["-c", "relscript"], fx.path_with("."));
    assert_eq!(stdout, "ZERO=./relscript\nBS0=./relscript\n");
    assert_eq!(code, Some(0));
}

/// Command face, relative subdir element: `sub/relscript` on both.
#[cfg(unix)]
#[test]
fn issue2317_command_face_subdir_element_records_as_written_join() {
    let fx = fixture("cmdsub");
    let (stdout, _stderr, code) = fx.run(&["-c", "relscript"], fx.path_with("sub"));
    assert_eq!(stdout, "ZERO=sub/relscript\nBS0=sub/relscript\n");
    assert_eq!(code, Some(0));
}

/// Command face, element `./sub`: the element's own `./` survives.
#[cfg(unix)]
#[test]
fn issue2317_command_face_dotted_subdir_element_records_as_written_join() {
    let fx = fixture("cmddotsub");
    let (stdout, _stderr, code) = fx.run(&["-c", "relscript"], fx.path_with("./sub"));
    assert_eq!(stdout, "ZERO=./sub/relscript\nBS0=./sub/relscript\n");
    assert_eq!(code, Some(0));
}

/// Command face, trailing-slash element: sh_makepath collapses the
/// boundary, recording `sub/relscript` (no doubled slash).
#[cfg(unix)]
#[test]
fn issue2317_command_face_trailing_slash_element_collapses() {
    let fx = fixture("cmdts");
    let (stdout, _stderr, code) = fx.run(&["-c", "relscript"], fx.path_with("sub/"));
    assert_eq!(stdout, "ZERO=sub/relscript\nBS0=sub/relscript\n");
    assert_eq!(code, Some(0));
}

/// Command face, absolute element (control): unchanged from #2302 —
/// the join and the anchor coincide.
#[cfg(unix)]
#[test]
fn issue2317_command_face_absolute_element_control() {
    let fx = fixture("cmdabs");
    let abs = fx.sub.join("relscript").display().to_string();
    let (stdout, _stderr, code) = fx.run(
        &["-c", "relscript"],
        fx.path_with(&fx.sub.display().to_string()),
    );
    assert_eq!(stdout, format!("ZERO={abs}\nBS0={abs}\n"));
    assert_eq!(code, Some(0));
}

/// Command face: the in-script diagnostic prefix is the join too.
#[cfg(unix)]
#[test]
fn issue2317_command_face_diagnostic_prefix_is_the_join() {
    let fx = fixture("cmddiag");
    let (stdout, stderr, _code) = fx.run(&["-c", "errscript"], fx.path_with("sub"));
    assert_eq!(stdout, "first\nafter\n");
    assert!(
        stderr.contains("sub/errscript: line 2: nosuchcmd_zzz: command not found"),
        "stderr: {stderr}"
    );
}

/// Program face, relative subdir element: $0 keeps the spelled name
/// (shell.c:1613), BASH_SOURCE[0] is the join (shell.c:1640).
#[cfg(unix)]
#[test]
fn issue2317_program_face_subdir_element_records_as_written_join() {
    let fx = fixture("progsub");
    let (stdout, _stderr, code) = fx.run(&["subscript"], fx.path_with("sub"));
    assert_eq!(stdout, "ZERO=subscript\nBS0=sub/subscript\n");
    assert_eq!(code, Some(0));
}

/// Program face, element `./sub`.
#[cfg(unix)]
#[test]
fn issue2317_program_face_dotted_subdir_element_records_as_written_join() {
    let fx = fixture("progdotsub");
    let (stdout, _stderr, code) = fx.run(&["subscript"], fx.path_with("./sub"));
    assert_eq!(stdout, "ZERO=subscript\nBS0=./sub/subscript\n");
    assert_eq!(code, Some(0));
}

/// Program face, trailing-slash element collapses like the
/// command face's.
#[cfg(unix)]
#[test]
fn issue2317_program_face_trailing_slash_element_collapses() {
    let fx = fixture("progts");
    let (stdout, _stderr, code) = fx.run(&["subscript"], fx.path_with("sub/"));
    assert_eq!(stdout, "ZERO=subscript\nBS0=sub/subscript\n");
    assert_eq!(code, Some(0));
}

/// Program face, absolute element (control): unchanged from #2302.
#[cfg(unix)]
#[test]
fn issue2317_program_face_absolute_element_control() {
    let fx = fixture("progabs");
    let abs = fx.sub.join("subscript").display().to_string();
    let (stdout, _stderr, code) =
        fx.run(&["subscript"], fx.path_with(&fx.sub.display().to_string()));
    assert_eq!(stdout, format!("ZERO=subscript\nBS0={abs}\n"));
    assert_eq!(code, Some(0));
}

/// Program face: the in-script diagnostic prefix is the join too.
#[cfg(unix)]
#[test]
fn issue2317_program_face_diagnostic_prefix_is_the_join() {
    let fx = fixture("progdiag");
    let (stdout, stderr, _code) = fx.run(&["suberr"], fx.path_with("sub"));
    assert_eq!(stdout, "first\nafter\n");
    assert!(
        stderr.contains("sub/suberr: line 2: nosuchcmd_zzz: command not found"),
        "stderr: {stderr}"
    );
}

/// Program face control: a script present in the cwd is opened
/// as given — no search fires and both names keep the spelling.
#[cfg(unix)]
#[test]
fn issue2317_program_face_as_given_in_cwd_control() {
    let fx = fixture("progcwd");
    let (stdout, _stderr, code) = fx.run(&["relscript"], fx.path_with("sub"));
    assert_eq!(stdout, "ZERO=relscript\nBS0=relscript\n");
    assert_eq!(code, Some(0));
}
