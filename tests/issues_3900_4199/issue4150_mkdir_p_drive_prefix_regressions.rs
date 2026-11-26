//! rubash#4150 — `mkdir -p` on a drive-absolute operand (`C:/…`)
//! left a stray directory named `C` + U+F03A in the current
//! directory, and `mkdir -p x:y/z` failed outright.
//!
//! Mechanism (captain adjudication on the issue, code-proven):
//! `mkdir_parent_prefixes` cuts the first ancestor prefix of
//! `C:/Windows` to the bare `C:`; the shell-name funnel has no
//! drive branch for that form, so `map_ntfs_colon_final_component`
//! encodes it as the RELATIVE name `C<U+F03A>`, which the session
//! path then joins onto the cwd and creates. GNU coreutils
//! mkdir.c `make_ancestors` never creates the root. The `x:y/z`
//! face is the same root one level down: the prefix `x:y` is
//! created under its encoded name `x<U+F03A>y`, while the full
//! operand keeps the colon literal in a non-final component, so
//! its parent is never the directory the walk just created.
//!
//! The fix lives in the mkdir ancestor-prefix walk (the funnel is
//! untouched — its encoding of a bare `C:` OPERAND is exactly the
//! Git Bash agreement pinned by the control test below): a bare
//! drive prefix of a drive-absolute operand resolves as the drive
//! root, and an operand carrying a colon in a non-final component
//! resolves its prefixes as a chain, each component under the
//! previous prefix's resolved path.
//!
//! The `/c/…` arm is a guard pin: per the adjudication the bare
//! `/c` prefix already resolves to the drive root (the #566 arm),
//! so the single-operand form must not create a stray either.
//!
//! Windows only: the shapes are drive semantics. On unix `C:` is
//! an ordinary relative name and GNU/Linux creates `./C:` — a
//! different, correct behavior. Execution on a real Windows host
//! is owed (the captain's verification host was unreachable when
//! this landed); CI carries the stem compile on the Windows MSVC
//! check leg.

#![cfg(windows)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(case: &str) -> ScratchDir {
    let dir = std::env::temp_dir().join(format!("rubash-4150-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch");
    ScratchDir(dir)
}

/// Names of the entries directly inside the scratch cwd, sorted.
fn entries(dir: &ScratchDir) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(&dir.0)
        .expect("read scratch")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn run_rubash(dir: &ScratchDir, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir.0)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
    )
}

#[test]
fn issue4150_mkdir_p_drive_absolute_existing_target_no_stray() {
    let dir = scratch("drive-existing");
    let (stdout, stderr, code) = run_rubash(&dir, "mkdir -p C:/Windows; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        Vec::<String>::new(),
        "no stray C: directory may appear in the cwd"
    );
}

#[test]
fn issue4150_mkdir_p_drive_root_operand_no_stray() {
    let dir = scratch("drive-root");
    let (stdout, stderr, code) = run_rubash(&dir, "mkdir -p C:/; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        Vec::<String>::new(),
        "the drive root operand itself must not materialize in the cwd"
    );
}

#[test]
fn issue4150_mkdir_p_reported_combination_no_stray() {
    let dir = scratch("reported-combo");
    let (stdout, stderr, code) =
        run_rubash(&dir, "mkdir -p C:/Windows /c/Windows C:/; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        Vec::<String>::new(),
        "the reported three-operand run leaves the cwd empty"
    );
}

#[test]
fn issue4150_mkdir_p_drive_mount_single_operand_no_stray() {
    // Guard pin (captain adjudication): the bare `/c` prefix
    // already resolves to the drive root via the #566 arm, so the
    // single-operand /c form must not create a stray either. The
    // reported counts accumulated three operands and could not
    // separate the arms; this isolates the /c arm.
    let dir = scratch("mount-single");
    let (stdout, stderr, code) = run_rubash(&dir, "mkdir -p /c/Windows; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        Vec::<String>::new(),
        "the /c arm must not create a stray directory"
    );
}

#[test]
fn issue4150_mkdir_p_drive_absolute_creates_target_chain() {
    let dir = scratch("drive-create");
    // The scratch directory's own drive-absolute spelling, in
    // shell form — creating the chain must land there and only
    // there, on whatever drive the host's temp dir lives.
    let base = dir.0.to_string_lossy().replace('\\', "/");
    let script = format!("mkdir -p '{base}/new/deep'; echo rc=$?\n");
    let (stdout, stderr, code) = run_rubash(&dir, &script);
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert!(
        dir.0.join("new").join("deep").is_dir(),
        "the target chain must be created"
    );
    assert_eq!(
        entries(&dir),
        vec!["new".to_string()],
        "only the target chain may appear in the cwd"
    );
}

#[test]
fn issue4150_mkdir_p_colon_parent_chain_creates() {
    // Git Bash creates x:y/z (on disk: x<U+F03A>y/z). The walk
    // must create the operand inside the parent it just created
    // under the encoded name, not under the literal colon name.
    let dir = scratch("colon-chain");
    let (stdout, stderr, code) = run_rubash(&dir, "mkdir -p x:y/z; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        vec!["x\u{F03A}y".to_string()],
        "the parent lands under its encoded name only"
    );
    assert!(
        dir.0.join("x\u{F03A}y").join("z").is_dir(),
        "the operand must be created inside the encoded parent"
    );
}

#[test]
fn issue4150_mkdir_p_bare_drive_operand_keeps_colon_name() {
    // Control (the report's example 3): a bare `C:` OPERAND is a
    // relative colon name in both niu and Git Bash — it creates
    // the encoded name in the cwd. The funnel behavior this rides
    // on must not change; only ancestor prefixes are roots.
    let dir = scratch("bare-drive");
    let (stdout, stderr, code) = run_rubash(&dir, "mkdir -p C:; echo rc=$?\n");
    assert_eq!(code, Some(0), "stderr:\n{stderr}");
    assert_eq!(stdout, "rc=0\n", "stderr:\n{stderr}");
    assert_eq!(stderr, "", "stderr:\n{stderr}");
    assert_eq!(
        entries(&dir),
        vec!["C\u{F03A}".to_string()],
        "a bare C: operand keeps the Git Bash colon-name behavior"
    );
}
