//! Startup / environment / option-mode regression pins (backfill wave bf-c).
//!
//! rubash#1039 — `set -P` (physical) ignored: `cd` into a symlink then
//! `pwd` printed the logical path in both modes.
//! rubash#1055 — runtime `set -o posix` back-propagated the STARTUP-only
//! option bundle into BASHOPTS (expand_aliases / inherit_errexit /
//! shift_verbose flipped on; GNU grants those only at `--posix` startup).
//! rubash#1056 — bare `set` omitted function definitions (GNU lists
//! variables AND functions outside posix mode).
//! rubash#1228 — startup `$0` did not honor the process argv[0]
//! (`exec -a hello <shell> -c 'echo $0'` must print `hello`).
//! rubash#1260 — in posix mode bare `set` must list variables ONLY
//! (GNU posix `set` suppresses function definitions; non-posix lists them).
//! rubash#1277 — `set -e` did not fire after `time <function>` /
//! `time <pipeline>` (the timing report must still print, then the shell
//! exits with the timed command's status).
//! rubash#1306 — `-c` mode pre-populated BASH_SOURCE with the `$0` name;
//! GNU leaves BASH_SOURCE UNSET in `-c` mode.
//!
//! Expectations hand-measured against GNU bash 5.3.20 via script files
//! and `-c` probes in this environment (byte-level stdout/stderr/rc).

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "rubash-bfc-startup-{}-{tag}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_args(args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(args)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn run_c(script: &str) -> (String, String, Option<i32>) {
    run_args(&["-c", script])
}

fn run_script(script: &str) -> (String, String, Option<i32>) {
    let dir = scratch("script");
    let path = dir.join("probe.sh");
    std::fs::write(&path, script).expect("write script");
    let out = run_args(&[path.to_str().expect("utf8 path")]);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

const BASHOPTS_PROBE: &str = "case \":$BASHOPTS:\" in *\":expand_aliases:\"*) echo ea=ON;; *) echo ea=off;; esac; case \":$BASHOPTS:\" in *\":inherit_errexit:\"*) echo ie=ON;; *) echo ie=off;; esac; case \":$BASHOPTS:\" in *\":shift_verbose:\"*) echo sv=ON;; *) echo sv=off;; esac";

#[cfg(unix)]
#[test]
fn issue_1039_set_p_cd_through_symlink_prints_physical_path() {
    use std::os::unix::fs::symlink;
    let dir = scratch("lnk");
    let base = std::fs::canonicalize(&dir).expect("canonicalize scratch");
    std::fs::create_dir(base.join("rdir")).expect("mkdir rdir");
    symlink("rdir", base.join("ldir")).expect("symlink ldir -> rdir");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", "set -P; cd ldir; pwd; cd ..; set +P; cd ldir; pwd"])
        .current_dir(&base)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let base = base.to_string_lossy();
    // GNU: physical path under -P, logical (as-typed) path without it.
    assert_eq!(stdout, format!("{base}/rdir\n{base}/ldir\n"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue_1055_runtime_set_o_posix_does_not_grant_startup_bundle() {
    let (stdout, stderr, code) = run_c(&format!("set -o posix; {BASHOPTS_PROBE}"));
    assert_eq!(stdout, "ea=off\nie=off\nsv=off\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1055_startup_posix_flag_grants_the_bundle() {
    // Control: the SAME three options ARE present when posix mode comes
    // from the command line — only the runtime path must not grant them.
    let (stdout, stderr, code) = run_args(&["--posix", "-c", BASHOPTS_PROBE]);
    assert_eq!(stdout, "ea=ON\nie=ON\nsv=ON\n", "stderr: {stderr}");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1056_bare_set_lists_function_definitions() {
    let (stdout, stderr, code) = run_script(
        "SETVAR=1\nmyf() { echo INBODY; }\nout=$(set)\ncase \"$out\" in *\"myf ()\"*) echo HAS_MYF_DEF;; *) echo NO_MYF_DEF;; esac\ncase \"$out\" in *INBODY*) echo HAS_BODY;; *) echo NO_BODY;; esac\ncase \"$out\" in *\"SETVAR=1\"*) echo HAS_SETVAR;; *) echo NO_SETVAR;; esac\n",
    );
    assert_eq!(
        stdout, "HAS_MYF_DEF\nHAS_BODY\nHAS_SETVAR\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[cfg(unix)]
#[test]
fn issue_1228_startup_dollar_zero_honors_argv0() {
    use std::os::unix::process::CommandExt;
    for argv0 in ["hello", "-hello"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg0(argv0)
            .args(["-c", "echo zero=[$0]"])
            .output()
            .expect("run rubash");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert_eq!(stdout, format!("zero=[{argv0}]\n"), "argv0 {argv0}");
        assert_eq!(output.status.code(), Some(0));
    }
}

#[test]
fn issue_1260_posix_bare_set_lists_variables_only() {
    let (stdout, stderr, code) = run_script(
        "myf() { echo INBODY; }\nset -o posix\nout=$(set)\ncase \"$out\" in *\"myf ()\"*) echo POSIX_HAS_MYF;; *) echo POSIX_NO_MYF;; esac\ncase \"$out\" in *INBODY*) echo POSIX_HAS_BODY;; *) echo POSIX_NO_BODY;; esac\nset +o posix\nout=$(set)\ncase \"$out\" in *\"myf ()\"*) echo NORMAL_HAS_MYF;; *) echo NORMAL_NO_MYF;; esac\n",
    );
    assert_eq!(
        stdout, "POSIX_NO_MYF\nPOSIX_NO_BODY\nNORMAL_HAS_MYF\n",
        "stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1277_errexit_fires_after_time_function() {
    let (stdout, stderr, code) = run_script("set -e\nf() { return 3; }\ntime f\necho lived\n");
    assert_eq!(stdout, "", "shell must exit under -e; stderr: {stderr}");
    assert!(
        stderr.contains("real"),
        "timing block still printed; stderr: {stderr}"
    );
    assert_eq!(code, Some(3));
}

#[test]
fn issue_1277_errexit_fires_after_time_pipeline_with_pipefail() {
    let (stdout, stderr, code) =
        run_script("set -e\nset -o pipefail\ntime true | false\necho lived\n");
    assert_eq!(stdout, "", "shell must exit under -e; stderr: {stderr}");
    assert!(
        stderr.contains("real"),
        "timing block still printed; stderr: {stderr}"
    );
    assert_eq!(code, Some(1));
}

#[test]
fn issue_1306_dash_c_mode_leaves_bash_source_unset() {
    let (stdout, _stderr, code) = run_c("echo src=[${BASH_SOURCE-UNSET}]");
    assert_eq!(stdout, "src=[UNSET]\n");
    assert_eq!(code, Some(0));
    // Also with an explicit $0 name argument: BASH_SOURCE stays unset.
    let (stdout, _stderr, code) =
        run_args(&["-c", "echo src=[${BASH_SOURCE-UNSET}] zero=[$0]", "myname"]);
    assert_eq!(stdout, "src=[UNSET] zero=[myname]\n");
    assert_eq!(code, Some(0));
}

#[test]
fn issue_1306_script_mode_bash_source_is_the_script() {
    // Control: script-file mode DOES populate BASH_SOURCE (with the path).
    let (stdout, _stderr, code) = run_script("echo src=[${BASH_SOURCE-UNSET}]\n");
    assert!(stdout.starts_with("src=[/"), "stdout: {stdout}");
    assert!(stdout.contains("probe.sh]"), "stdout: {stdout}");
    assert_eq!(code, Some(0));
}
