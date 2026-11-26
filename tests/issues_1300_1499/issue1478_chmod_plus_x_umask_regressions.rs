//! rubash#1478 — the builtin `chmod +x` ignored the umask (granted
//! the full 0111); GNU applies 0111 & ~umask to files that had no
//! execute bits, like coreutils: with umask 007 a 660 file becomes
//! 770, not 771.
//!
//! Probe: clusters/x3/repro/1478.sh.
//! Expectations verified against GNU bash 5.3.20 (script files).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1478-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
fn chmod_plus_x_respects_umask() {
    let dir = tempdir("umask");
    let script = dir.join("probe.sh");
    std::fs::write(
        &script,
        "umask 007\nprintf 'x\\n' > f\nstat -c %a f\nchmod +x f\nstat -c %a f\n",
    )
    .expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).into_owned(),
        "660\n770\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
