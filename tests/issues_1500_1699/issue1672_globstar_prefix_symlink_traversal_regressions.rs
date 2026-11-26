//! rubash#1672 — globstar `./**/*.txt` did not traverse symlinked
//! directories, while GNU includes the through-link hits.
//!
//! GNU ground truth (lib/glob/glob.c glob_filename): the symlink-skip
//! special case applies only when the directory part consists solely
//! of `**` segments (all_starstar). With any literal prefix component
//! (`./`, `sub/`, an absolute path), a symlinked directory enumerated
//! by `**` is treated as an ordinary directory for the REMAINING
//! pattern: the suffix is applied one level inside it. The `**`
//! enumeration itself still never descends into a symlinked directory,
//! and a bare `PREFIX/**` yields the enumeration only (link names, no
//! link contents). The plain `**/*.txt` form (no literal prefix) keeps
//! the skip. Expectations verified against GNU bash 5.3.20 (script
//! files).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1672-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn run(dir: &std::path::Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("run rubash");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn globstar_dot_prefix_applies_suffix_inside_symlinked_dir() {
    let dir = tempdir("dot");
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("d1")).expect("fixture dirs");
    std::fs::write(fix.join("d1/f1.txt"), "").expect("fixture file");
    std::fs::write(fix.join("top.txt"), "").expect("fixture file");
    std::os::unix::fs::symlink("d1", fix.join("linkd")).expect("symlink fixture");
    let stdout = run(
        &fix,
        "shopt -s globstar\n\
         echo \"dot:\"; for x in ./**/*.txt; do printf \"<%s>\\n\" \"$x\"; done\n\
         echo \"plain:\"; for x in **/*.txt; do printf \"<%s>\\n\" \"$x\"; done\n",
    );
    assert_eq!(
        stdout,
        "dot:\n<./d1/f1.txt>\n<./linkd/f1.txt>\n<./top.txt>\n\
         plain:\n<d1/f1.txt>\n<top.txt>\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn globstar_literal_prefix_symlink_one_level_only() {
    let dir = tempdir("sub");
    let fix = dir.join("fix");
    std::fs::create_dir_all(fix.join("sub/real/deep")).expect("fixture dirs");
    std::fs::write(fix.join("sub/real/f.txt"), "").expect("fixture file");
    std::fs::write(fix.join("sub/real/deep/g.txt"), "").expect("fixture file");
    std::fs::write(fix.join("sub/top.txt"), "").expect("fixture file");
    std::os::unix::fs::symlink("real", fix.join("sub/linkd")).expect("symlink fixture");
    let stdout = run(
        &fix,
        "shopt -s globstar\n\
         echo \"txt:\"; for x in sub/**/*.txt; do printf \"<%s>\\n\" \"$x\"; done\n\
         echo \"g:\"; for x in sub/**/g.txt; do printf \"<%s>\\n\" \"$x\"; done\n\
         echo \"bare:\"; for x in sub/**; do printf \"<%s>\\n\" \"$x\"; done\n",
    );
    // sub/linkd/f.txt: the suffix applies one level inside the link.
    // sub/**/g.txt must NOT reach sub/linkd/deep/g.txt: the `**`
    // enumeration never descends into the link. Bare sub/** is the
    // enumeration only: linkd appears as a name, its contents do not.
    assert_eq!(
        stdout,
        "txt:\n<sub/linkd/f.txt>\n<sub/real/deep/g.txt>\n<sub/real/f.txt>\n<sub/top.txt>\n\
         g:\n<sub/real/deep/g.txt>\n\
         bare:\n<sub/>\n<sub/linkd>\n<sub/real>\n<sub/real/deep>\n<sub/real/deep/g.txt>\n<sub/real/f.txt>\n<sub/top.txt>\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
