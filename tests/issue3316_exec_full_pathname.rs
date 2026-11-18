//! Regression pins for rubash#3316 — `exec` of a relative-path script
//! left a `/./` segment in the script's `$0`.
//!
//! GNU ownership: builtins/exec.def:151 picks the target word (the
//! typed word for a slash-bearing name, the PATH search's as-written
//! `<elem>/<name>` join otherwise), and exec.def:172 rebinds it with
//! `full_pathname` (general.c:931-943) before shell_execve
//! (exec.def:231). full_pathname returns an absolute word verbatim and
//! otherwise calls `sh_makepath (NULL, file, MP_DOCWD|MP_RMDOT)`
//! (lib/sh/makepath.c): the cwd, a `/`, then the word with exactly ONE
//! leading `./` removed — no other canonicalization (`././s`, `a/./s`,
//! `a/../s`, `.//s` keep their remaining components). That string is
//! the execve filename, so a `#!` script sees it as `$0`, the ENOEXEC
//! re-entry binds it as the new `$0` (execute_cmd.c:6253 `args[1] =
//! command`), and shell_execve's failure reports name it.
//!
//! Expectations are GNU bash 5.3.20 outputs for the same probe script
//! (WSL /usr/local/bin/bash, 2026-11-14), with `$PWD` folded to `D`.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-3316-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).expect("scratch dir");
    dir
}

fn run(root: &Path, body: &str) -> (String, String) {
    std::fs::write(root.join("case.sh"), body).expect("case script");
    let mut command = Command::new(BIN);
    command
        .arg("case.sh")
        .current_dir(root)
        .stdin(std::process::Stdio::null());
    // A real parent shell exports a PWD matching the cwd it chdir'd into;
    // Command::current_dir only chdirs the child, so without this the
    // child inherits a stale PWD and these pins would exercise the
    // startup PWD-rebind path instead of the exec semantics under test.
    #[cfg(unix)]
    command.env("PWD", root);
    let out = command.output().expect("run rubash");
    let _ = std::fs::remove_dir_all(root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Fixture prelude: a shebang-less script (the ENOEXEC re-entry face,
/// reachable on every platform) at the root and under `sub/`.
const PRELUDE: &str = r#"printf 'echo "0=$0"\n' > n.sh; chmod +x n.sh; cp n.sh sub/m.sh
show() { o=$( ( exec "$@" ) ); printf '%s\n' "${o/"$PWD"/D}"; }
"#;

#[test]
fn exec_relative_script_dollar0_strips_one_leading_dot_slash() {
    let root = scratch("enoexec");
    let script = format!(
        "{PRELUDE}show ./n.sh\nshow ././n.sh\nshow .//n.sh\nshow sub/../n.sh\nshow ./sub/../n.sh\nshow sub/./m.sh\nshow sub//m.sh\n"
    );
    let (stdout, stderr) = run(&root, &script);
    assert_eq!(
        stdout,
        "0=D/n.sh\n0=D/./n.sh\n0=D//n.sh\n0=D/sub/../n.sh\n0=D/sub/../n.sh\n0=D/sub/./m.sh\n0=D/sub//m.sh\n",
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
mod unix_faces {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// The `#!` face: the execve filename is what the kernel hands the
    /// interpreter as the script name.
    #[test]
    fn exec_shebang_script_dollar0_is_full_pathname() {
        let root = scratch("shebang");
        let script = format!(
            "{PRELUDE}printf '#!/bin/sh\\necho \"0=$0\"\\n' > s.sh; chmod +x s.sh\nshow ./s.sh\nshow ././s.sh\nshow sub/../s.sh\n"
        );
        let (stdout, stderr) = run(&root, &script);
        assert_eq!(
            stdout, "0=D/s.sh\n0=D/./s.sh\n0=D/sub/../s.sh\n",
            "stderr: {stderr}"
        );
    }

    /// A PATH-found target: full_pathname applies to the search's
    /// as-written join (`.` -> `./n.sh` -> `D/n.sh`; `./sub/` ->
    /// `./sub/m.sh` -> `D/sub/m.sh`).
    #[test]
    fn exec_path_relative_element_dollar0_is_full_pathname() {
        let root = scratch("path");
        let script = format!(
            "{PRELUDE}(PATH=.:$PATH; show n.sh)\n(PATH=./sub/:$PATH; show m.sh)\n(PATH=sub:$PATH; show m.sh)\n"
        );
        let (stdout, stderr) = run(&root, &script);
        assert_eq!(
            stdout, "0=D/n.sh\n0=D/sub/m.sh\n0=D/sub/m.sh\n",
            "stderr: {stderr}"
        );
    }

    /// The failure reports name the same full_pathname string: one
    /// leading `./` removed, `/./` deeper in the word kept.
    #[test]
    fn exec_missing_target_report_is_full_pathname() {
        let root = scratch("missing");
        let script = "e() { ( exec \"$@\" ) 2>&1 | sed \"s#$PWD#D#; s#^.*line [0-9]*: ##\"; }\ne ./nope\ne ./sub/../nope\ne sub/./nope\ne ././nope\n";
        let (stdout, stderr) = run(&root, script);
        assert_eq!(
            stdout,
            "D/nope: No such file or directory\nD/sub/../nope: No such file or directory\nD/sub/./nope: No such file or directory\nD/./nope: No such file or directory\n",
            "stderr: {stderr}"
        );
    }

    /// Unix faces for the Windows-only path syntax: `\` is an ordinary
    /// filename byte and `D:/x` is a relative path there, so neither the
    /// backslash fold nor the `X:/` drive check may fire off Windows.
    #[test]
    fn exec_literal_backslash_filename_runs_verbatim_on_unix() {
        let root = scratch("literal-backslash");
        fs::copy("/bin/echo", root.join("back\\slash")).expect("copy /bin/echo");
        let (stdout, stderr) = run(&root, "exec './back\\slash' literal-ok\n");
        assert_eq!(stdout, "literal-ok\n", "stderr: {stderr}");
        assert!(stderr.is_empty(), "stderr: {stderr}");
    }

    #[test]
    fn exec_cwd_with_backslash_byte_joins_verbatim_on_unix() {
        let root = scratch("cwd\\backslash");
        fs::copy("/bin/echo", root.join("echo")).expect("copy /bin/echo");
        let (stdout, stderr) = run(&root, "exec ./echo cwd-ok\n");
        assert_eq!(stdout, "cwd-ok\n", "stderr: {stderr}");
        assert!(stderr.is_empty(), "stderr: {stderr}");
    }

    #[test]
    fn exec_drive_colon_word_is_relative_and_binds_full_pathname_on_unix() {
        let root = scratch("drive-relative");
        let dir = root.join("D:");
        fs::create_dir(&dir).expect("D: dir");
        let target = dir.join("target.sh");
        fs::write(&target, "#!/bin/sh\nprintf 'zero=%s\\n' \"$0\"\n").expect("target.sh");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).expect("chmod +x");
        let (stdout, stderr) = run(&root, "exec 'D:/target.sh'\n");
        assert_eq!(
            stdout,
            format!("zero={}/D:/target.sh\n", root.display()),
            "stderr: {stderr}"
        );
        assert!(stderr.is_empty(), "stderr: {stderr}");
    }
}
