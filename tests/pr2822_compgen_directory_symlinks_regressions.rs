//! Regression pin for rubash PR #2822 (external contributor):
//! directory completion dropped symlinks that point at directories.
//! `path_completion_candidates` filtered on the dirent type, which is
//! false for a symlink, so `compgen -d` (and `cd` completion) never
//! offered a symlinked folder.
//!
//! Oracle: GNU bash 5.3.20 `compgen -d` in a fixture directory holding
//! realdir/, realfile, linkdir -> realdir, linkfile -> realfile and
//! brokenlink -> nonexistent prints exactly:
//!   linkdir
//!   realdir
//! A link to a file and a broken link are not directories and stay out.

#![cfg(unix)]

use std::process::Command;

struct Fixture {
    dir: std::path::PathBuf,
}

impl Fixture {
    fn run(&self, script: &str) -> (Vec<String>, Option<i32>) {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .current_dir(&self.dir)
            .output()
            .expect("run rubash");
        let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        (lines, output.status.code())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("rubash-2822-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::create_dir_all(dir.join("realdir")).expect("fixture realdir");
    std::fs::write(dir.join("realfile"), "x\n").expect("fixture realfile");
    std::os::unix::fs::symlink("realdir", dir.join("linkdir")).expect("symlink linkdir");
    std::os::unix::fs::symlink("realfile", dir.join("linkfile")).expect("symlink linkfile");
    std::os::unix::fs::symlink("nonexistent", dir.join("brokenlink")).expect("symlink brokenlink");
    Fixture { dir }
}

#[test]
fn pr2822_compgen_d_lists_symlink_to_directory() {
    let fx = fixture("all");
    let (lines, code) = fx.run("compgen -d");
    assert_eq!(
        lines,
        vec!["linkdir".to_string(), "realdir".to_string()],
        "compgen -d must offer the symlink to a directory alongside the real one"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn pr2822_compgen_d_prefix_filters_symlink_to_directory() {
    let fx = fixture("prefix");
    let (lines, code) = fx.run("compgen -d li");
    assert_eq!(lines, vec!["linkdir".to_string()]);
    assert_eq!(code, Some(0));

    // A link to a file is not a directory: no match, rc 1.
    let (lines, code) = fx.run("compgen -d linkf");
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(code, Some(1));

    // A broken link resolves to nothing: no match, rc 1.
    let (lines, code) = fx.run("compgen -d broken");
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(code, Some(1));
}
