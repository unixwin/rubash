//! Issues rubash#1331 (suspend operand handling), rubash#1332 (EXIT trap
//! runs when an untrapped fatal signal kills the shell), rubash#1333
//! (SIGPIPE starts at SIG_DFL and is trappable), rubash#1334 (IFS is
//! never imported from the environment).
//!
//! Expected outputs verified against GNU bash 5.3.20.

use std::io::Write;
use std::process::Command;

fn run_script(source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-k7sig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("case.sh");
    let mut file = std::fs::File::create(&path).expect("write script");
    file.write_all(source.as_bytes()).expect("write script");
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn suspend_with_operand_abandons_list() {
    // GNU builtins/common.c no_args class: diagnostic, list abandoned,
    // `-c` shell exits 1; `||` cannot rescue it.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("suspend foo || echo survived")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("suspend: too many arguments"));
}

#[test]
fn suspend_bad_option_is_plain_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("suspend -x; echo \"after rc=$?\"")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after rc=2\n");
}

#[cfg(unix)]
#[test]
fn exit_trap_runs_on_untrapped_fatal_signal() {
    use std::os::unix::process::ExitStatusExt;
    let dir = std::env::temp_dir().join(format!("rubash-1332term-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("term.sh");
    std::fs::write(&path, "trap 'echo EXIT-TRAP-RAN' EXIT\nkill -TERM $$\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    // rubash#1332's half: the EXIT trap runs. rubash#2346's half: the
    // death is BY SIGTERM (sig.c:649-667 kill_shell), not a plain 143
    // exit -- the earlier Some(143) expectation encoded the form the
    // #2346 oracle evidence (waitpid: signaled 15) overturned.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "EXIT-TRAP-RAN\n");
    assert_eq!(output.status.signal(), Some(15));
    assert_eq!(output.status.code(), None);
}

#[cfg(unix)]
#[test]
fn untrapped_sigpipe_kills_shell() {
    use std::os::unix::process::ExitStatusExt;
    let dir = std::env::temp_dir().join(format!("rubash-k7sig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("pipe.sh");
    std::fs::write(&path, "kill -PIPE $$\necho alive\n").expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&path)
        .output()
        .expect("run rubash");
    // Killed BY SIGPIPE (13), not a shell-reported 141 exit: the
    // process status carries the signal itself.
    assert_eq!(output.status.signal(), Some(13));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[cfg(unix)]
#[test]
fn sigpipe_trap_fires() {
    let (stdout, _, code) =
        run_script("trap 'echo PIPE-TRAPPED' PIPE\nkill -PIPE $$\necho after\n");
    assert_eq!(stdout, "PIPE-TRAPPED\nafter\n");
    assert_eq!(code, Some(0));
}

#[test]
fn ifs_is_not_imported_from_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("IFS", ":")
        .arg("-c")
        .arg("v='a:b c'; printf '<%s>\\n' $v")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<a:b>\n<c>\n");
}

// niubash#525: the EMPTY imported IFS is discarded the same way —
// GNU variables.c never imports IFS from the environment, whatever
// its value; word splitting uses the default IFS. (The `IFS=:`
// sibling above is the form the niu side originally pinned.)
#[test]
fn empty_ifs_is_not_imported_from_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("IFS", "")
        .arg("-c")
        .arg("v='a:b c'; printf '<%s>\\n' $v")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<a:b>\n<c>\n");
}
