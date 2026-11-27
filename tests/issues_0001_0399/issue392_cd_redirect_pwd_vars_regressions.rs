//! Issue rubash#392 regression: `cd` under any redirection lost its
//! PWD/OLDPWD maintenance, and combined redirects were bound
//! first-match-wins so only the first channel took effect.
//!
//! GNU spec: builtins/cd.def:136-175 bindpwd() binds OLDPWD and PWD in the
//! current shell for EVERY successful cd path — including the cdable_vars
//! branch (cd.def:395-402, `LCD_DOVARS`: `change_to_directory(temp)` then
//! `printf` then `return bindpwd(no_symlinks)`). execute_cmd.c's builtin
//! path applies a builtin's redirections in the current process
//! (redir.c do_redirections / undo_redirections, no fork), so
//! `cd dest > f` must update the live shell's PWD/OLDPWD exactly like an
//! unredirected `cd`, and `cd nosuch > f 2> g` must route the diagnostic
//! to g while stdout's file f is still created.
//!
//! Root cause in rubash: Executor::execute_cd
//! (src/executor/printf_path_builtins.rs) called sync_cd_variables() only
//! on the unredirected tail path; every redirect branch returned early.
//! The redirect binding was also first-match-wins, so a second redirect
//! slot was dropped (`>f 2>g` wrote the diagnostic to real stderr and
//! never created g) and `>&2` fell into File::create("&2").
//!
//! Verified byte-for-byte against WSL GNU Bash 5.3.0 script-file probes
//! (target/gapfix2/mtx/{matrix,combo,fdprobe}.sh double-shell matrices).

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

struct RunOutcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_rubash_script(script: &str) -> RunOutcome {
    let dir = std::env::temp_dir().join(format!("rubash-issue392-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue392 scratch dir");
    let script_path = dir.join("probe.sh");
    let mut file = std::fs::File::create(&script_path).expect("write probe script");
    // LF line endings: probes run identically under WSL GNU Bash for
    // baseline comparison.
    file.write_all(script.as_bytes())
        .expect("write probe script");
    drop(file);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash probe");
    RunOutcome {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// cdable_vars `cd dest > file` updates PWD (and a sane OLDPWD) in the
/// live shell, and the resolved directory is printed into the redirect
/// file (GNU cd.def:395-402 printf + bindpwd).
#[test]
fn cdable_vars_cd_with_redirect_updates_pwd_and_oldpwd() {
    let outcome = run_rubash_script(
        // The variable name must NOT collide with a relative directory:
        // `dest` only takes the cdable_vars branch (cd.def:395-402) when
        // ./dest does not exist; then GNU prints the variable VALUE into
        // the redirect file.
        "mkdir -p real\n\
         shopt -s cdable_vars\n\
         save=$PWD\n\
         dest=\"$PWD/real\"\n\
         cd dest > \"$save/out.txt\"\n\
         echo \"pwdvar=${PWD##*/}\"\n\
         [ \"$OLDPWD\" = \"$save\" ] && echo oldpwd_ok || echo oldpwd_bad\n\
         line=$(cat \"$save/out.txt\")\n\
         echo \"printed=${line##*/}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "pwdvar=real\noldpwd_ok\nprinted=real\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// Plain `cd sub > /dev/null` keeps PWD/OLDPWD in the live shell for
/// every redirect shape (GNU bindpwd runs in the current process).
#[test]
fn plain_cd_redirect_shapes_maintain_pwd_vars() {
    for shape in [
        "> /dev/null",
        "> f.out",
        ">> f.out",
        "2> f.out",
        "2>> f.out",
    ] {
        let script = format!(
            "mkdir -p sub\n\
             cd sub {shape}\n\
             echo \"P=${{PWD##*/}}\"\n\
             cd .. > /dev/null\n"
        );
        let outcome = run_rubash_script(&script);
        assert_eq!(outcome.code, Some(0), "shape {shape} rc");
        assert_eq!(outcome.stdout, "P=sub\n", "shape {shape} stdout");
    }
}

/// Combined redirects bind both channels (GNU redir.c do_redirections):
/// `cd nosuch > f 2> g` puts the diagnostic in g, creates f, keeps the
/// shell's PWD, and leaves real stderr empty.
#[test]
fn cd_combined_redirects_bind_both_channels() {
    let outcome = run_rubash_script(
        "cd nosuchdir > A.f 2> A.g\n\
         echo \"rc=$? P=${PWD##*/}\"\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
    assert!(
        outcome.stdout.starts_with("rc=1 "),
        "stdout: {}",
        outcome.stdout
    );
}

/// `2>&1` with a stdout file shares fd 1's destination: the cd diagnostic
/// lands in the file, not on real stderr.
#[test]
fn cd_stderr_dup_follows_rebound_stdout_file() {
    let outcome = run_rubash_script(
        "cd nosuchdir > B.f 2>&1\n\
         echo rc=$?\n\
         grep -c \"cd: nosuchdir\" B.f\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\n1\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}

/// `cd - >&2` routes the printed directory to stderr via the fd-dup
/// target instead of creating a junk file named `&2` (old behavior).
#[test]
fn cd_stdout_dup_to_stderr_prints_directory() {
    let outcome = run_rubash_script(
        "mkdir -p sub\n\
         cd sub > /dev/null\n\
         cd - >&2\n\
         echo rc=$?\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=0\n");
    // `cd -` prints the DESTINATION (the pre-cd scratch dir), on stderr
    // because of `>&2` (GNU fdprobe shape E).
    let stderr = outcome.stderr.trim_end();
    assert!(
        stderr.contains("rubash-issue392-") && !stderr.ends_with("/sub"),
        "stderr should carry the destination dir: {}",
        outcome.stderr
    );
}

/// `cd - >&file` is r_err_and_out (redir.c:832-838): both the printed
/// directory (fd 1) and any diagnostic (fd 2) land in file.
#[test]
fn cd_err_and_out_word_binds_both_fds_to_file() {
    let outcome = run_rubash_script(
        "cd nosuchdir >& C.f\n\
         echo rc=$?\n\
         grep -c \"cd: nosuchdir\" C.f\n",
    );
    assert_eq!(outcome.code, Some(0));
    assert_eq!(outcome.stdout, "rc=1\n1\n");
    assert!(outcome.stderr.is_empty(), "stderr: {}", outcome.stderr);
}
