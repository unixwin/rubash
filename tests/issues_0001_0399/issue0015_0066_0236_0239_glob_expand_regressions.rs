//! W4 backfill slice 1 — glob / pathname expansion faces.
//!
//! * rust#15 — a glob pattern built from a variable prefix (`"$X"/*.txt`)
//!   must expand against the directory the variable names.
//! * rust#66 — adjacent brace groups expand as a cross product
//!   (`{a,b}{1,2}` -> `a1 a2 b1 b2`).
//! * rust#236 — an escaped star stays literal while an unescaped star in
//!   the same line still globs (`a\*b` vs `a*b`).
//! * rust#239 — `${!a[@]}` yields the index list of an indexed array.
//!
//! All expectations were measured against GNU bash 5.3.20 (the project
//! oracle). Tests only; no product behavior is changed here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("rubash-bfw4-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn finish(&self, out: Output) -> (String, String, i32) {
        let norm = |b: &[u8]| String::from_utf8_lossy(b).replace(self.dir.to_str().unwrap(), "$S");
        (
            norm(&out.stdout),
            norm(&out.stderr),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        self.finish(out)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue15_glob_with_variable_prefix() {
    let s = Scratch::new("i15");
    let (out, err, rc) = s.run(
        "X=./sub\nmkdir -p sub\ntouch sub/a.txt sub/b.txt\nfor p in \"$X\"/*.txt; do echo \"$p\"; done\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "./sub/a.txt\n./sub/b.txt\n");
    assert_eq!(s.path().join("sub/a.txt").exists(), true);
}

#[test]
fn issue66_adjacent_brace_groups_cross_product() {
    let s = Scratch::new("i66");
    let (out, err, rc) = s.run("echo {a,b}{1,2}\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "a1 a2 b1 b2\n");
}

#[test]
fn issue236_escaped_star_literal_unescaped_star_globs() {
    let s = Scratch::new("i236");
    let (out, err, rc) = s.run("touch 'a*b' axb\necho a\\*b\necho a*b\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "a*b\na*b axb\n");
}

#[test]
fn issue239_indirect_array_subscript_expands_indices() {
    let s = Scratch::new("i239");
    let (out, err, rc) =
        s.run("a=(1 2 3)\ns=\"${!a[@]}\"\necho \"sub=[$s]\"\necho \"vals=${a[@]}\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "sub=[0 1 2]\nvals=1 2 3\n");
}
