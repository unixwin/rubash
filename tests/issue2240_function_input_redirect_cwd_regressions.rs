//! Issue unixwin/rubash#2240: an input redirect on a FUNCTION CALL
//! resolved a relative operand against the process (startup) cwd
//! instead of the shell's current logical cwd — after any `cd`,
//! `g <f.txt` failed ENOENT for a file sitting in the current
//! directory. A #2211 sibling on a different code path: #2211 anchored
//! the comsub shortcuts, this one is the function-call stdin bind.
//!
//! GNU keeps exactly one cwd: `cd` moves the process's
//! (builtins/cd.def change_to_directory -> chdir), and
//! execute_function applies the call's redirections with
//! do_redirections (execute_cmd.c:5181 execute_function; redir.c
//! redir_open performs a plain open of the expanded word), so a
//! relative operand resolves against the directory the shell most
//! recently cd'd to. Rubash's G-02 split makes the SESSION cwd field
//! the physical truth (the process cwd never moves after startup);
//! every other input path anchors the mapped operand with
//! `session_path` before opening (open_input_redirect,
//! open_compound_numbered_input_redirects, compound_fd0_stream_file),
//! but function_call_stdin opened the bare shell_path_to_windows
//! result, i.e. against the startup directory.
//!
//! Expectations measured against GNU bash 5.3.20 (owner-compiled
//! oracle) with script files, 2026-10-30, master 1d5d62d1 (probes
//! probeV.sh / probeW.sh / probeX.sh in the cluster workdir). Control
//! pins (startup cwd, absolute path, numbered fd, output side) pass on
//! master too; the pins named for the defect shapes are red on master.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Fixture {
    root: PathBuf,
    launch: PathBuf,
}

fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("issue2240_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let launch = root.join("launch");
    let sub = launch.join("sub");
    std::fs::create_dir_all(&sub).expect("create fixture dirs");
    // The same file NAME carries different bytes in the launch dir and
    // in sub/, so a read anchored at the wrong cwd is visible as wrong
    // CONTENT, not only as ENOENT.
    std::fs::write(launch.join("f.txt"), "launch-data\n").expect("write launch f.txt");
    std::fs::write(sub.join("f.txt"), "sub-data\n").expect("write sub f.txt");
    std::fs::write(launch.join("launch.txt"), "launch-only\n").expect("write launch.txt");
    let root = std::fs::canonicalize(&root).expect("canonicalize fixture root");
    let launch = root.join("launch");
    Fixture { root, launch }
}

impl Fixture {
    fn run(&self, script: &str) -> (String, String, Option<i32>) {
        let script_path = self.root.join("probe.sh");
        std::fs::write(&script_path, script).expect("write probe script");
        let child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&script_path)
            .current_dir(&self.launch)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rubash");
        let out = child.wait_with_output().expect("wait rubash");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn issue2240_function_input_redirect_relative_after_cd() {
    let fx = fixture("relative");
    let (stdout, stderr, code) = fx.run(
        "\
g () { cat; }
cd sub && g <f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue2240_function_input_redirect_dot_slash_after_cd() {
    let fx = fixture("dotslash");
    let (stdout, stderr, _code) = fx.run(
        "\
g () { cat; }
cd sub && g <./f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_builtin_body_function_input_redirect_after_cd() {
    let fx = fixture("builtinbody");
    let (stdout, stderr, _code) = fx.run(
        "\
h () { read l; echo \"h got: $l\"; }
cd sub && h <f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "h got: sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_input_redirect_inside_for_body_after_cd() {
    let fx = fixture("forbody");
    let (stdout, stderr, _code) = fx.run(
        "\
g () { cat; }
cd sub || exit 1
for i in 1; do g <f.txt; done
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_input_redirect_dotdot_after_cd() {
    let fx = fixture("dotdot");
    let (stdout, stderr, _code) = fx.run(
        "\
g () { cat; }
cd sub && g <../launch.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "launch-only\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_definition_redirect_resolves_at_call_time_cwd() {
    // GNU applies a definition redirect when the function is CALLED,
    // against the cwd at call time (do_redirections in
    // execute_function), not at definition time.
    let fx = fixture("defredir");
    let (stdout, stderr, _code) = fx.run(
        "\
m () { cat; } <f.txt
m
cd sub && m
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "launch-data\nsub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_readwrite_existing_file_after_cd() {
    let fx = fixture("rwexisting");
    let (stdout, stderr, _code) = fx.run(
        "\
k () { read l; echo \"k got: $l\"; }
cd sub && k <>f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "k got: sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_readwrite_missing_file_created_in_current_dir() {
    // GNU `<>` opens O_RDWR|O_CREAT (make_cmd.c:682): a missing name
    // is created in the CURRENT directory, and nothing may be
    // created in the startup directory.
    let fx = fixture("rwcreate");
    let sub_new = fx.launch.join("sub").join("new2.txt");
    let launch_new = fx.launch.join("new2.txt");
    let (stdout, _stderr, _code) = fx.run(
        "\
k () { read l; echo \"k got: [$l]\"; }
cd sub && k <>new2.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "k got: []\nrc=0\n");
    assert!(
        sub_new.is_file(),
        "missing <> target must be created in the current dir"
    );
    assert!(
        !launch_new.exists(),
        "no stray <> file may be created in the startup dir"
    );
}

#[test]
fn issue2240_simple_readwrite_missing_file_created_only_in_current_dir() {
    // The validation gate's `<>` probe is shared by every command
    // form: its create must also land in the current directory only.
    let fx = fixture("rwgate");
    let sub_new = fx.launch.join("sub").join("new1.txt");
    let launch_new = fx.launch.join("new1.txt");
    let (stdout, _stderr, _code) = fx.run(
        "\
cd sub && cat <>new1.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(
        sub_new.is_file(),
        "missing <> target must be created in the current dir"
    );
    assert!(
        !launch_new.exists(),
        "no stray <> file may be created in the startup dir"
    );
}

#[test]
fn issue2240_function_input_redirect_at_startup_cwd_control() {
    let fx = fixture("startup");
    let (stdout, stderr, _code) = fx.run(
        "\
g () { cat; }
g <f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "launch-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_input_redirect_absolute_path_control() {
    let fx = fixture("absolute");
    let script = format!(
        "\
g () {{ cat; }}
cd sub && g <{}/sub/f.txt
echo \"rc=$?\"
",
        display(&fx.launch)
    );
    let (stdout, stderr, _code) = fx.run(&script);
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_numbered_fd_input_after_cd_control() {
    let fx = fixture("numfd");
    let (stdout, stderr, _code) = fx.run(
        "\
h () { cat <&3; }
cd sub && h 3<f.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "sub-data\nrc=0\n");
    assert_eq!(stderr, "");
}

#[test]
fn issue2240_function_missing_operand_names_operand_as_spelled() {
    let fx = fixture("missing");
    let (stdout, stderr, _code) = fx.run(
        "\
g () { cat; }
cd sub && g <missing-xyz.txt
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("missing-xyz.txt: No such file or directory"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains(&display(&fx.launch)),
        "diagnostic must name the operand as spelled, not the anchored path: {stderr}"
    );
}

#[test]
fn issue2240_function_output_redirect_stays_in_current_dir() {
    // Output-side audit control: after cd, a function call's `>`
    // creates/truncates in the current directory only — a same-named
    // file in the startup directory keeps its bytes.
    let fx = fixture("outanchor");
    let launch_out = fx.launch.join("out.txt");
    std::fs::write(&launch_out, "launch-keep\n").expect("seed launch out.txt");
    let (stdout, _stderr, _code) = fx.run(
        "\
g () { cat; }
cd sub && g >out.txt </dev/null
echo \"rc=$?\"
",
    );
    assert_eq!(stdout, "rc=0\n");
    assert!(fx.launch.join("sub").join("out.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(&launch_out).expect("read launch out.txt"),
        "launch-keep\n"
    );
}
