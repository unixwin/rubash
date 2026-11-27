//! Issue rubash#754 regressions: tilde expansion and the cd/pushd/popd path
//! model must share ONE $HOME baseline — the CURRENT variable cell at every
//! evaluation, never a startup capture, with GNU's missing-vs-empty
//! distinction.
//!
//! GNU specification (vendored tree, bash.git @b4608166, Bash-5.3 patch 15):
//! - lib/tilde/tilde.c:344-356 `tilde_expand_word()`: a leading `~`/`~/` is
//!   ALWAYS translated to `sh_get_env_value("HOME")` read at expansion time
//!   (variables.c get_string_value — the live cell). A set-but-EMPTY value is
//!   still a value: the glue yields the bare suffix, so `HOME=; cd ~` is
//!   `cd ""` and reports "null directory" (builtins/cd.def:335). Only a
//!   MISSING variable reaches the fallback — tilde.c:349-350 APPDATA on
//!   _WIN32, tilde.c:354-356 sh_get_home_dir (variables.c:736 getpwuid),
//!   which is fixed for the uid and never tracks a reassigned $HOME. Rubash's
//!   stand-in for that fallback on Windows is USERPROFILE.
//! - builtins/cd.def:316-322: operand-less `cd` reads the same live cell and
//!   refuses only a MISSING variable (`dirname == 0`, POSIX cd step 1 — cd
//!   has no passwd fallback). A set-but-EMPTY HOME flows on through
//!   make_absolute, glues the cwd onto the empty name, and the chdir is a
//!   same-directory success: rc 0, no diagnostic, PWD unchanged. An empty
//!   OPERAND (the `cd ~` path) still trips cd.def:331-336's "null directory".
//! - builtins/pushd.def:296-302: the Dir operand delegates to
//!   `cd_builtin (list)`, so an operand that expands to "" inherits cd's
//!   "null directory" failure (prolog names pushd); with -n, cd_builtin is
//!   skipped and pushd.def:301 pushes the RAW operand word unconditionally —
//!   an empty expansion stays "" on the stack (`<cwd> ""`). popd lives in
//!   the same file: builtins/pushd.def:318 `popd_builtin()` breaks its
//!   option walk on an empty word (pushd.def:356-363), so `popd ~` with an
//!   empty expansion performs the default top-of-stack pop.
//!
//! Every case below was probed against WSL GNU bash 5.3.0(1)-release via the
//! stdin method before being pinned here (source-built rubash only — the
//! installed niu 1.4.0 binary is unverifiable provenance, see #827); the
//! matrix also covers the issue's headline pair (`mkdir ~/x` succeeds, then
//! `cd ~/x` must enter the SAME directory the expansion created).

use std::io::Write;
use std::process::{Command, Stdio};

fn run_script_stdin(script: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--norc", "--noprofile", "-s"])
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
        .expect("write script");
    let output = child.wait_with_output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_command_string(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--norc", "--noprofile", "-c", script])
        .output()
        .expect("run rubash -c");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fresh_dir(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "rubash-i754-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.to_string_lossy().replace('\\', "/")
}

/// The issue headline: `cd ~` must land in the CURRENT $HOME, and the
/// `mkdir ~/x` / `cd ~/x` pair must agree on one baseline (the directory the
/// word expansion created is the directory cd enters). GNU T02/T03.
#[test]
fn cd_tilde_and_mkdir_tilde_share_current_home() {
    let dir = fresh_dir("pair");
    let (stdout, stderr, rc) = run_command_string(&format!(
        "cd \"{dir}\" || exit 99; mkdir -p newhome; HOME=\"{dir}/newhome\"; mkdir ~/x && cd ~/x && echo \"in:$PWD\""
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        stdout.starts_with("in:") && stdout.trim_end().ends_with("/newhome/x"),
        "stdout: {stdout}"
    );
}

/// Same baseline from the stdin/script form, `cd ~` operand form (GNU T02).
#[test]
fn cd_tilde_follows_reassigned_home_from_stdin() {
    let dir = fresh_dir("stdin");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nmkdir -p newhome\nHOME=\"{dir}/newhome\"; cd ~; echo \"$PWD\"\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(stdout.trim_end().ends_with("/newhome"), "stdout: {stdout}");
}

/// Sequential reassignments: each evaluation re-reads the cell (GNU T15).
#[test]
fn tilde_tracks_sequential_home_changes() {
    let dir = fresh_dir("seq");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nmkdir -p h1 h2\nHOME=\"{dir}/h1\"; a=~\nHOME=\"{dir}/h2\"; b=~\necho \"$a|$b\"\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, format!("{dir}/h1|{dir}/h2\n"), "stderr: {stderr}");
}

/// The other tilde consumers stay on the same baseline: assignment values,
/// redirections, `[[ -d ]]`, `case` words (GNU T08-T11).
#[test]
fn tilde_contexts_all_use_current_home() {
    let dir = fresh_dir("ctx");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nmkdir -p newhome/x\nHOME=\"{dir}/newhome\"\nd=~/x; echo \"d=$d\"\necho hi > ~/f; echo \"f=$(ls newhome/f | wc -l)\"\n[[ -d ~/x ]]; echo \"test=$?\"\ncase ~/x in \"{dir}/newhome/x\") echo case=yes;; *) echo case=no;; esac\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        format!("d={dir}/newhome/x\nf=1\ntest=0\ncase=yes\n"),
        "stderr: {stderr}"
    );
}

/// command cd / subshell / function / pushd share the baseline (GNU T05-T07).
/// All four must land in the SAME directory the current $HOME names; the
/// $PWD spelling each binder chooses (POSIX slash-drive vs native) is the
/// separate #537 form policy and is not pinned here — the tail is.
#[test]
fn command_cd_subshell_function_pushd_follow_current_home() {
    let dir = fresh_dir("forms");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nmkdir -p newhome\nHOME=\"{dir}/newhome\"\ncommand cd ~; echo \"command=$PWD\"\n(cd ~; echo \"sub=$PWD\")\nf() {{ cd ~; }}; f; echo \"fn=$PWD\"\npushd ~ >/dev/null; echo \"pushd=$PWD\"; popd >/dev/null\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    let tail = format!(
        "/{}/newhome",
        std::path::Path::new(&dir)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("temp dir basename")
    );
    let lines: Vec<String> = stdout.lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 4, "stdout: {stdout}");
    for line in &lines {
        assert!(
            line.ends_with(&tail),
            "line {line} must end with {tail} (stderr: {stderr})"
        );
    }
}

/// GNU tilde.c:344-356 — a set-but-empty HOME is a VALUE: `~` expands to ""
/// and `~/x` to "/x"; the USERPROFILE/passwd fallback must NOT fire.
#[test]
fn empty_home_expands_to_empty_not_fallback() {
    let (stdout, stderr, rc) = run_script_stdin(
        "HOME=\nt=~; echo \"t=[$t]\"\nu=~/x; echo \"u=[$u]\"\na=~/x; echo \"a=[$a]\"\n",
    );
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "t=[]\nu=[/x]\na=[/x]\n", "stderr: {stderr}");
}

/// cd.def:335 — `cd ~` with a set-but-empty HOME is `cd ""`: "null
/// directory", status 1.
#[test]
fn empty_home_cd_tilde_is_null_directory() {
    let (stdout, stderr, rc) =
        run_script_stdin("HOME=\ncd ~; echo \"rc=$?\"\ncd ''; echo \"rc=$?\"\n");
    assert_eq!(rc, Some(0));
    assert!(stderr.contains("cd: null directory"), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\nrc=1\n", "stdout: {stdout}");
}

/// pushd.def:296-302 delegates the Dir operand to cd_builtin, so an empty
/// expansion inherits "null directory" with pushd in the prolog; -n skips
/// cd_builtin, and pushd.def:301 pushes the RAW word — the stack becomes
/// `<cwd> ""` (dirs prints the trailing blank element), not the cwd twice.
#[test]
fn empty_home_pushd_tilde_is_null_directory() {
    let dir = fresh_dir("pushdn");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nHOME=\npushd ~; echo \"rc=$?\"\npushd -n ~ >/dev/null; echo \"rc=$?\"\nd=$(dirs); p=$PWD; [ \"$d\" = \"$p \" ] && echo stack=blank-tail || echo stack=[$d]\n"
    ));
    assert_eq!(rc, Some(0));
    assert!(stderr.contains("pushd: null directory"), "stderr: {stderr}");
    assert_eq!(stdout, "rc=1\nrc=0\nstack=blank-tail\n", "stdout: {stdout}");
}

/// pushd.def:318 `popd_builtin()` breaks its option walk on an empty word
/// (pushd.def:356-363): `popd ~` with an empty expansion is the default
/// top-of-stack pop (or "directory stack empty" on a fresh stack), not
/// "invalid argument".
#[test]
fn empty_home_popd_tilde_uses_default_operand() {
    let (stdout, stderr, rc) = run_script_stdin("HOME=\npopd ~; echo \"rc=$?\"\n");
    assert_eq!(rc, Some(0));
    assert!(
        stderr.contains("popd: directory stack empty"),
        "stderr: {stderr}"
    );
    assert_eq!(stdout, "rc=1\n", "stdout: {stdout}");
}

/// cd.def:316-322 — the operand-less refusal is `dirname == 0`: only a
/// MISSING HOME reports "HOME not set". A set-but-EMPTY HOME flows on
/// through make_absolute and the chdir is a same-directory success (rc 0,
/// no diagnostic, PWD unchanged), while `cd ~` with HOME missing still
/// expands through the fallback chain.
#[test]
fn operandless_cd_refuses_missing_home_only() {
    // Set-but-empty: rc 0, in place, silent.
    let dir = fresh_dir("cdempty");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nHOME=\ncd; echo \"rc=$?\"\np=$PWD; echo \"same=$([ \"$PWD\" = \"$p\" ] && echo yes || echo no)\"\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert!(
        !stderr.contains("HOME not set"),
        "empty HOME must not refuse: {stderr}"
    );
    assert_eq!(
        stdout, "rc=0\nsame=yes\n",
        "stdout: {stdout} stderr: {stderr}"
    );

    // Missing entirely: `cd` refuses, `cd ~` still works (fallback).
    let (stdout, stderr, rc) =
        run_script_stdin("unset HOME\ncd; echo \"rc=$?\"\ncd ~; echo \"rc=$? PWD=$PWD\"\n");
    assert_eq!(rc, Some(0));
    assert!(stderr.contains("cd: HOME not set"), "stderr: {stderr}");
    let (first, second) = (
        stdout.lines().next().unwrap(),
        stdout.lines().nth(1).unwrap(),
    );
    assert_eq!(first, "rc=1");
    assert!(
        second.starts_with("rc=0 PWD=") && !second.ends_with("PWD=$PWD"),
        "second: {second}"
    );
}

/// The `$(cd ... && pwd)` fast path must agree with the builtin it stands in
/// for: with a set-but-empty HOME the real cd lands on the cwd, so the
/// substitution yields $PWD with status 0 (the fast path delegates); with a
/// MISSING HOME it reports "cd: HOME not set" with substitution status 1.
#[test]
fn comsub_cd_fast_path_matches_builtin_home_verdict() {
    // Set-but-empty: delegation — same directory, rc 0.
    let dir = fresh_dir("cdsub");
    let (stdout, stderr, rc) = run_script_stdin(&format!(
        "cd \"{dir}\" || exit 99\nHOME=\nout=$(cd && pwd); echo \"same=$([ \"$out\" = \"$PWD\" ] && echo yes || echo no:$out) rc=$?\"\n"
    ));
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout, "same=yes rc=0\n",
        "stdout: {stdout} stderr: {stderr}"
    );

    // Missing entirely: fast "HOME not set" verdict.
    let (stdout, stderr, rc) =
        run_script_stdin("unset HOME\nout=$(cd && pwd); echo \"out=[$out] rc=$?\"\n");
    assert_eq!(rc, Some(0));
    assert!(stderr.contains("cd: HOME not set"), "stderr: {stderr}");
    assert_eq!(stdout, "out=[] rc=1\n", "stdout: {stdout}");
}

/// Windows persona: with HOME missing entirely, `~` resolves through the
/// USERPROFILE stand-in for tilde.c:349-356's APPDATA/passwd chain (the
/// `cd ~` case from the issue's startup-HOME sweep).
#[cfg(windows)]
#[test]
fn missing_home_falls_back_to_userprofile_for_tilde_only() {
    let userprofile = std::env::var("USERPROFILE")
        .expect("USERPROFILE present on Windows test host")
        .replace('\\', "/");
    let tail = userprofile
        .split_once(':')
        .map(|(_, rest)| rest.trim_start_matches('/').to_string())
        .unwrap_or(userprofile.clone());
    let (stdout, stderr, rc) = run_script_stdin("unset HOME\ncd ~; echo \"rc=$? PWD=$PWD\"\n");
    assert_eq!(rc, Some(0), "stderr: {stderr}");
    let line = stdout.trim_end();
    assert!(line.starts_with("rc=0 PWD="), "line: {line}");
    let pwd = line.strip_prefix("rc=0 PWD=").expect("pwd value");
    let pwd = pwd.replace('\\', "/");
    let pwd_tail = pwd
        .split_once(':')
        .map(|(_, rest)| rest.trim_start_matches('/').to_string())
        .unwrap_or_else(|| pwd.trim_start_matches('/').to_string());
    // The drive prefix spelling varies (POSIX /c/... vs native C:/... — the
    // #537 form policy); the home tail must be the USERPROFILE stand-in.
    assert!(
        pwd_tail.ends_with(&tail),
        "cd ~ must land in the USERPROFILE stand-in: {pwd} vs {tail}"
    );
}
