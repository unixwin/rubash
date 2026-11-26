//! rubash#1708 — `[[ -O /dev/null ]]` and `[[ -G /dev/null ]]` were
//! false for the owning user; `/dev/stdin` diverged the same way.
//!
//! GNU ground truth: test.c:561-568 — `-O` is `sh_stat(arg) == 0 &&
//! current_user.euid == stat_buf.st_uid`, `-G` the same with egid /
//! st_gid. Expectations are derived from the live filesystem (the
//! file's real uid/gid vs this process's euid/egid, read off
//! /proc/self's metadata) so the test pins the stat comparison itself
//! on any host; verified against GNU bash 5.3.20 as root (euid/egid 0,
//! /dev/null owned by 0:0 -> both true).

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::process::{Command, Stdio};

#[cfg(unix)]
fn run_with_stdin(script: &str, stdin_file: &std::fs::File) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .stdin(Stdio::from(
            stdin_file.try_clone().expect("clone stdin file"),
        ))
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[cfg(unix)]
fn verdict(want_true: bool) -> &'static str {
    if want_true {
        "0"
    } else {
        "1"
    }
}

// Expectations are derived from /proc/self's uid/gid and /dev/null's
// ownership — probes that only exist on unix — so the test is unix-only.
#[cfg(unix)]
#[test]
fn issue1708_dash_o_g_dev_null_and_stdin() {
    let dir = std::env::temp_dir().join(format!("rubash1708-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let owned = dir.join("owned-file");
    std::fs::write(&owned, b"data").expect("write owned file");

    let self_meta = std::fs::metadata("/proc/self").expect("stat /proc/self");
    let (euid, egid) = (self_meta.uid(), self_meta.gid());
    let null_meta = std::fs::metadata("/dev/null").expect("stat /dev/null");
    let owned_meta = std::fs::metadata(&owned).expect("stat owned file");

    let stdin_file = std::fs::File::open(&owned).expect("open owned file as stdin");
    let script = format!(
        "[[ -O /dev/null ]]; echo \"O-null=$?\"\n\
         [[ -G /dev/null ]]; echo \"G-null=$?\"\n\
         [ -O /dev/null ]; echo \"bO-null=$?\"\n\
         [ -G /dev/null ]; echo \"bG-null=$?\"\n\
         [[ -O /dev/stdin ]]; echo \"O-stdin=$?\"\n\
         [[ -G /dev/stdin ]]; echo \"G-stdin=$?\"\n\
         [[ -O {owned} ]]; echo \"O-own=$?\"\n\
         [[ -G {owned} ]]; echo \"G-own=$?\"\n",
        owned = owned.display()
    );
    let (stdout, stderr, _) = run_with_stdin(&script, &stdin_file);
    let expected = format!(
        "O-null={}\nG-null={}\nbO-null={}\nbG-null={}\nO-stdin={}\nG-stdin={}\nO-own={}\nG-own={}\n",
        verdict(euid == null_meta.uid()),
        verdict(egid == null_meta.gid()),
        verdict(euid == null_meta.uid()),
        verdict(egid == null_meta.gid()),
        verdict(euid == owned_meta.uid()),
        verdict(egid == owned_meta.gid()),
        verdict(euid == owned_meta.uid()),
        verdict(egid == owned_meta.gid()),
    );
    assert_eq!(stdout, expected, "stderr: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
