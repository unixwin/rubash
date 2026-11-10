//! rubash#3106 — the emulated chmod's parse loop only skipped `--`
//! while the mode was still unset, so a terminator AFTER the mode
//! (`chmod 600 -- file`) fell into the file list: the run reported
//! `chmod: cannot access '--': No such file or directory`, exited 1,
//! and only the real operands got their mode.
//!
//! GNU coreutils chmod parses argv with getopt_long, whose contract
//! is positional-independent: the FIRST `--` ends option scanning
//! wherever it appears, every later word is an operand (the first
//! operand is the mode, the rest are files), and a second `--` is an
//! ordinary operand. Measured against GNU coreutils 9.4:
//!   chmod 600 -- f        -> rc 0, silent, f is 0600
//!   chmod -- 600 f        -> rc 0, silent, f is 0600
//!   chmod 600 f1 -- f2    -> rc 0, silent, both 0600
//!   chmod 600 --          -> rc 1, "missing operand after '600'"
//!   chmod 600 -- --       -> rc 1, "cannot access '--'"
//!   chmod -- -v f         -> rc 1, "invalid mode: '-v'"
//!
//! Harness: `rubash -c` via `CARGO_BIN_EXE_rubash` in a scratch dir,
//! fixtures created with std::fs, modes read back with std::fs.
//! `LC_ALL=C.UTF-8` is pinned on the child so the pin is hermetic
//! under any ambient locale.

use std::process::{Command, Stdio};

fn run_full(dir: &std::path::Path, script: &str) -> (i32, Vec<u8>, Vec<u8>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["-c", script])
        .env("LC_ALL", "C.UTF-8")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash")
        .wait_with_output()
        .expect("wait rubash");
    (
        output.status.code().unwrap_or(-1),
        output.stdout,
        output.stderr,
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue3106-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn chmod_mode_then_dashdash_then_file() {
    // The issue's headline shape: rc, mode, and no diagnostic.
    let dir = scratch("post-mode");
    let file = dir.join("f");
    std::fs::write(&file, b"x").expect("fixture");
    let (rc, _stdout, stderr) = run_full(&dir, "chmod 600 -- f");
    assert_eq!(
        rc,
        0,
        "rc must be 0; stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(
        stderr.is_empty(),
        "no diagnostic expected, got: {}",
        String::from_utf8_lossy(&stderr)
    );
    #[cfg(unix)]
    assert_eq!(mode_of(&file), 0o600, "the file must take the mode");
}

#[test]
fn chmod_dashdash_before_mode_control() {
    // Control: the terminator before the mode already worked.
    let dir = scratch("pre-mode");
    let file = dir.join("f");
    std::fs::write(&file, b"x").expect("fixture");
    let (rc, _stdout, stderr) = run_full(&dir, "chmod -- 600 f");
    assert_eq!(
        rc,
        0,
        "rc must be 0; stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(stderr.is_empty());
    #[cfg(unix)]
    assert_eq!(mode_of(&file), 0o600);
}

#[test]
fn chmod_dashdash_between_operands() {
    // getopt permutation: the terminator may sit between operands;
    // the words after it are still file operands.
    let dir = scratch("between");
    let f1 = dir.join("f1");
    let f2 = dir.join("f2");
    std::fs::write(&f1, b"x").expect("fixture");
    std::fs::write(&f2, b"x").expect("fixture");
    let (rc, _stdout, stderr) = run_full(&dir, "chmod 600 f1 -- f2");
    assert_eq!(
        rc,
        0,
        "rc must be 0; stderr: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(stderr.is_empty());
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&f1), 0o600);
        assert_eq!(mode_of(&f2), 0o600);
    }
}

#[test]
fn chmod_mode_then_dashdash_no_file_is_not_a_phantom_operand() {
    // `chmod 600 --` has NO file operand: GNU reports a missing
    // operand. The defect's signature is the bogus diagnostic naming
    // '--' as an inaccessible file; whatever path answers after the
    // fix (the real binary via the decline gate, or the emulation),
    // it must fail nonzero and must not invent that operand.
    let dir = scratch("no-file");
    let (rc, _stdout, stderr) = run_full(&dir, "chmod 600 --");
    assert_ne!(rc, 0, "a missing operand must fail");
    assert!(
        !String::from_utf8_lossy(&stderr).contains("cannot access '--'"),
        "the terminator must not be reported as a file operand, got: {}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn chmod_dashdash_makes_option_shaped_word_the_mode() {
    // After the terminator every word is an operand, so `-v` here is
    // the MODE operand and GNU rejects it as an invalid mode — it is
    // not an option. Before the fix the emulation skipped `-v` as an
    // option and silently took `f` as the mode (rc 1, no diagnostic).
    let dir = scratch("mode-after-dd");
    std::fs::write(dir.join("f"), b"x").expect("fixture");
    let (rc, _stdout, stderr) = run_full(&dir, "chmod -- -v f");
    assert_eq!(rc, 1);
    assert!(
        String::from_utf8_lossy(&stderr).contains("invalid mode: '-v'"),
        "stderr must name '-v' as the invalid mode, got: {}",
        String::from_utf8_lossy(&stderr)
    );
}
