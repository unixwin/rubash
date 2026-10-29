//! rubash#1960 — `${v=x:=y}` expanded the value `x:=y` but never bound
//! `v`: the assignment pre-scan split the `${...}` body on `:=` FIRST,
//! claimed the fragment under the invalid composite name `v=x`, and
//! returned, so the `=` arm — the body's actual first operator — never
//! ran. GNU parameter_brace_expand (subst.c:9777) extracts the parameter
//! name in one string_extract pass (subst.c:9807, SX_VARNAME delimiter
//! set `#%^,:-=?+/@}`) and dispatches on the FIRST operator character
//! (subst.c:9886-9917); for `v=x:=y` that operator is `=` with word
//! `x:=y`, bound through parameter_brace_expand_rhs (subst.c:7966,
//! `case '='` at subst.c:10346). The pre-scan now lets a `:=` split
//! whose left side is not an assignment target fall through to the `=`
//! arm — the same fall-through parse_parameter_assignment_operator
//! (parameter_ops.rs) already used — while the rubash#1953 guard is
//! preserved: an `=`/`:=` inside another operator's word (`${a:-x=1}`,
//! `${a:-x:=1}`) is still word text and binds nothing.
//!
//! All expectations measured against GNU bash 5.3.20 with script files
//! (LC_ALL=C.UTF-8), 2026-10-29.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
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
        let status = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .env("LC_ALL", "C.UTF-8")
            .current_dir(&self.dir)
            .stdout(Stdio::from(out_file))
            .stderr(Stdio::from(err_file))
            .status()
            .unwrap();
        let read = |p: &PathBuf| {
            let mut s = String::new();
            fs::File::open(p).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        (
            read(&out_path),
            read(&err_path),
            status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn issue1960_equal_before_colon_equal_binds_the_whole_word() {
    let s = Scratch::new("main");
    // GNU: operator is the first `=`; the word is `x:=y` verbatim.
    let script = "unset v\necho \"1:[${v=x:=y}] v=[${v-unset}]\"\n\
                  unset v\necho \"9:[${v=x:=y} ${v}]\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "1:[x:=y] v=[x:=y]\n9:[x:=y x:=y]\n");
    assert_eq!(err, "");
}

#[test]
fn issue1960_adjacent_operator_matrix_matches_gnu() {
    let s = Scratch::new("matrix");
    // GNU 5.3.20 (m1960.sh): `:=` with an `=` in its word binds the
    // whole word; a set variable is left alone by `=`; an array element
    // target binds through the `=` arm too.
    let script = "unset v\necho \"2:[${v:=x=y}] v=[${v-unset}]\"\n\
                  unset v; v=set\necho \"3:[${v=x:=y}] v=[$v]\"\n\
                  unset v\necho \"4:[${v:=y}] v=[${v-unset}]\"\n\
                  unset v\necho \"5:[${v=x}] v=[${v-unset}]\"\n\
                  unset arr\necho \"8:[${arr[1]=x:=y}] a1=[${arr[1]-unset}]\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "2:[x=y] v=[x=y]\n3:[set] v=[set]\n4:[y] v=[y]\n5:[x] v=[x]\n\
         8:[x:=y] a1=[x:=y]\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue1960_operator_word_text_still_binds_nothing() {
    let s = Scratch::new("guard");
    // rubash#1953 guard preserved: `=`/`:=` inside a `:-` word are
    // literal default-word text — the value is the word, `a` and `x`
    // both stay unset (GNU).
    let script = "unset a\necho \"6:[${a:-x=1}] a=[${a-unset}]\"\n\
                  unset a\necho \"7:[${a:-x:=1}] a=[${a-unset}]\"\n\
                  echo \"x=[${x-unset}]\"\n";
    let (out, err, rc) = s.run(script);
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "6:[x=1] a=[unset]\n7:[x:=1] a=[unset]\nx=[unset]\n");
    assert_eq!(err, "");
}
