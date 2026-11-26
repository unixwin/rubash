//! rubash#1960 — `${v=x:=y}` never bound `v`. GNU subst.c:9777
//! parameter_brace_expand splits the braced body at the FIRST
//! operator: the body `v=x:=y` is parameter `v`, operator `=`, word
//! `x:=y`, so it assigns the literal text `x:=y` to `v`. The rubash
//! handler claimed the `:=` inside the word first — the assignment
//! target became the non-name fragment `v=x`, was rejected, and the
//! rejection returned without trying the `=` arm, so nothing bound.
//! The claimed target must pass the assignment-shape filter for EACH
//! operator candidate, and only a candidate whose base is a shell
//! name (or array element) may claim the expansion.
//!
//! All expectations measured against GNU bash 5.3.20.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
            std::env::temp_dir().join(format!("rubash-i1960-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out_path = self.dir.join("out.txt");
        let err_path = self.dir.join("err.txt");
        let out_file = fs::File::create(&out_path).unwrap();
        let err_file = fs::File::create(&err_path).unwrap();
        let start = Instant::now();
        let mut child: Child = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .spawn()
            .unwrap();
        let rc = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status.code().unwrap_or(-1),
                None => {
                    if start.elapsed() > Duration::from_secs(20) {
                        let _ = child.kill();
                        let _ = child.wait();
                        break -1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path).replace(&case.to_string_lossy().to_string(), "SH"),
            rc,
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1960_equal_word_containing_colon_equal_binds_the_parameter() {
    let s = Scratch::new("binds");
    // GNU: ${v=x:=y} assigns the literal `x:=y`; same for `p:=q`.
    let (out, err, rc) =
        s.run("unset v u\necho \"got=[${v=x:=y}] v=[$v]\"\necho \"got=[${u=p:=q}] u=[$u]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "got=[x:=y] v=[x:=y]\ngot=[p:=q] u=[p:=q]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1960_array_element_target_with_operator_word_binds() {
    let s = Scratch::new("array");
    // GNU: the claim filter accepts an array-element target too.
    let (out, err, rc) = s
        .run("declare -a a\necho \"got=[${a[0]=x:=y}]\"\ndeclare -p a | sed 's/.*a=(//; s/)$//'\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "got=[x:=y]\n[0]=\"x:=y\"\n");
    assert_eq!(err, "");
}

#[test]
fn issue1960_first_operator_still_wins_when_both_splits_are_names() {
    let s = Scratch::new("firstop");
    // GNU agreement guards: `${v:=x:=y}` binds via `:=` (value `x:=y`,
    // no second assignment); `${v=x=y}` binds via `=` (value `x=y`).
    let (out, err, rc) =
        s.run("unset v w\necho \"a=[${v:=x:=y}] v=[$v]\"\necho \"b=[${w=x=y}] w=[$w]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "a=[x:=y] v=[x:=y]\nb=[x=y] w=[x=y]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1960_operator_inside_dash_word_does_not_claim() {
    let s = Scratch::new("noclaim");
    // GNU agreement guard: `${a:-x=y:=z}` is the `:-` form — the
    // inner operators are word text; nothing is assigned and `a`
    // stays unset.
    let (out, err, rc) = s.run(
        "unset a\necho \"r=[${a:-x=y:=z}]\"\nif [[ -v a ]]; then echo bound; else echo unbound; fi\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "r=[x=y:=z]\nunbound\n");
    assert_eq!(err, "");
}
