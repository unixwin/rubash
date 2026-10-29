//! rubash#2046 (S6 of the rubash#2036 sync line): the four cells S5
//! registered unruled — B2 env PATH-unset resolution, B3 stty
//! validation order, C1 `uname --help`, C2 arch diagnostic quotes —
//! adjudicated 2026-10-29 (DECISIONS: all four align GNU/WinuxCmd)
//! and fixed in S6. Each pin names its cell; expectations are the
//! GNU measurements (bash 5.3.20 + coreutils 9.4, dual-run anchored
//! in the S5 corpus, whose four registered cells flipped to hard
//! `gnu` expectations in the same batch).

use std::process::Command;

fn rubash() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    path.pop();
    path.push("rubash");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

struct Output {
    stdout: String,
    stderr: String,
}

fn run(script: &str, dir: &std::path::Path) -> Output {
    run_with_path(script, dir, "/usr/bin:/bin")
}

fn run_with_path(script: &str, dir: &std::path::Path, path: &str) -> Output {
    let output = Command::new(rubash())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("PATH", path)
        .output()
        .expect("spawn rubash");
    Output {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-2046-s6-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

// ---------------------------------------------------------------- B2

#[test]
fn issue2046_b2_env_unset_path_resolves_via_execvp_default() {
    let dir = scratch("b2");
    // The corpus shape: PATH stripped from the CHILD's environment —
    // execvp's default path still finds printenv, which then reports
    // PATH absent from its own environment (rc 1, no output). The
    // pre-fix engine answered env's own 127 here.
    let out = run("env -u PATH printenv PATH; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(out.stderr, "");
    // Same absence reached through -i: the composed env is honoured
    // (FOO visible) while resolution still succeeds.
    let out = run("env -i FOO=bar printenv FOO; echo rc=$?", &dir);
    assert_eq!(out.stdout, "bar\nrc=0\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2046_b2_env_unset_path_missing_command_stays_127() {
    let dir = scratch("b2miss");
    // The fallback is a search, not an invention: a name the default
    // path does not hold still fails with env's 127 face (the message
    // text is the engine's WinuxCmd-aligned form, unchanged by B2).
    let out = run("env -u PATH s6_no_such_cmd_xyz; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=127\n");
    assert!(
        out.stderr.contains("s6_no_such_cmd_xyz"),
        "stderr: {:?}",
        out.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn issue2046_b2_env_set_but_empty_path_is_out_of_scope() {
    let dir = scratch("b2empty");
    // Ruling scope: ONLY the unset form falls back. A set-but-empty
    // PATH keeps the plain (failing) lookup — GNU agrees (rc 127 in
    // an empty directory: the empty element is the cwd, which holds
    // no printenv here).
    let out = run("env PATH= printenv PATH; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=127\n");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- B3

#[test]
fn issue2046_b3_stty_validates_all_operands_before_touching_the_device() {
    let dir = scratch("b3");
    // The corpus shape: stdin is not a terminal (piped), yet the bad
    // operand is what gets reported — validation precedes the device
    // probe, exactly like GNU (the pre-fix engine answered the ioctl
    // diagnostic here).
    let out = run("stty -Z; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "stty: invalid argument '-Z'\nTry 'stty --help' for more information.\n"
    );
    // First invalid operand in argv order wins, whichever side of a
    // valid operand it sits on.
    let out = run("stty size -Z; echo rc=$?", &dir);
    assert_eq!(
        out.stderr,
        "stty: invalid argument '-Z'\nTry 'stty --help' for more information.\n"
    );
    let out = run("stty -Y -Z; echo rc=$?", &dir);
    assert_eq!(
        out.stderr,
        "stty: invalid argument '-Y'\nTry 'stty --help' for more information.\n"
    );
    // An all-valid list still reaches the device probe: on this
    // non-terminal stdin the answer is the ioctl diagnostic.
    let out = run("stty size; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "stty: 'standard input': Inappropriate ioctl for device\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- C1

#[test]
fn issue2046_c1_uname_help_is_the_full_coreutils_text() {
    let dir = scratch("c1");
    // GNU coreutils 9.4 `uname --help`, byte for byte (the pre-fix
    // engine answered a two-line short form). The same text is the
    // S5 corpus uname-help cell's hard expectation.
    let out = run("uname --help", &dir);
    assert_eq!(
        out.stdout,
        "Usage: uname [OPTION]...\n\
         Print certain system information.  With no OPTION, same as -s.\n\
         \n\
         \x20 -a, --all                print all information, in the following order,\n\
         \x20                            except omit -p and -i if unknown:\n\
         \x20 -s, --kernel-name        print the kernel name\n\
         \x20 -n, --nodename           print the network node hostname\n\
         \x20 -r, --kernel-release     print the kernel release\n\
         \x20 -v, --kernel-version     print the kernel version\n\
         \x20 -m, --machine            print the machine hardware name\n\
         \x20 -p, --processor          print the processor type (non-portable)\n\
         \x20 -i, --hardware-platform  print the hardware platform (non-portable)\n\
         \x20 -o, --operating-system   print the operating system\n\
         \x20     --help        display this help and exit\n\
         \x20     --version     output version information and exit\n\
         \n\
         GNU coreutils online help: <https://www.gnu.org/software/coreutils/>\n\
         Report any translation bugs to <https://translationproject.org/team/>\n\
         Full documentation <https://www.gnu.org/software/coreutils/uname>\n\
         or available locally via: info '(coreutils) uname invocation'\n"
    );
    assert_eq!(out.stderr, "");
    // arch shares the usage_text owner; its --help is the full
    // coreutils page too (body line is GNU's "Print machine
    // architecture.", not the paraphrase the short form carried).
    let out = run("arch --help", &dir);
    assert_eq!(
        out.stdout,
        "Usage: arch [OPTION]...\n\
         Print machine architecture.\n\
         \n\
         \x20     --help        display this help and exit\n\
         \x20     --version     output version information and exit\n\
         \n\
         GNU coreutils online help: <https://www.gnu.org/software/coreutils/>\n\
         Report any translation bugs to <https://translationproject.org/team/>\n\
         Full documentation <https://www.gnu.org/software/coreutils/arch>\n\
         or available locally via: info '(coreutils) arch invocation'\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- C2

#[test]
fn issue2046_c2_arch_extra_operand_quotes_are_ascii() {
    let dir = scratch("c2");
    // The corpus shape: ASCII quotes, not the Unicode ‘’ pair.
    let out = run("arch extra; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "arch: extra operand 'extra'\nTry 'arch --help' for more information.\n"
    );
    // The shared extra-operand owner serves uname identically.
    let out = run("uname extra; echo rc=$?", &dir);
    assert_eq!(out.stdout, "rc=1\n");
    assert_eq!(
        out.stderr,
        "uname: extra operand 'extra'\nTry 'uname --help' for more information.\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
