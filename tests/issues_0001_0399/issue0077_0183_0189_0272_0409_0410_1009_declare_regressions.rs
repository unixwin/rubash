//! W4 backfill slice 1 — declare / variable-state faces.
//!
//! * rust#77 — `declare -p` reprints arrays in GNU's reusable form
//!   (`declare -A m=([k]="v" )`, `declare -a arr=([0]="one" [1]="two")`).
//! * rust#183 — inside a function, `declare -p FUNCNAME` reports the live
//!   call stack (`[0]="h" [1]="main"`).
//! * rust#189 — a `declare -p UNSET` diagnostic is capturable through
//!   `$(... 2>&1)`; it must not bypass redirection to the real stderr.
//! * rust#272 — `readonly -p NAME` with name arguments prints nothing
//!   (measured on the oracle: named printing is a `declare -p` /
//!   `typeset -p` behavior, `readonly -p` only dumps when no names are
//!   given); the bare `readonly -p` dump lists every readonly variable.
//! * rust#409 — a `declare -rx` variable that was never assigned still
//!   expands as unset for `${VAR-unset}`.
//! * rust#410 — in POSIX mode, `typeset +x foo` inside a function acts
//!   on a function-local shadow (a later `${foo-unset}` in the function
//!   reads as unset), while the global keeps its value AND its export
//!   attribute (`declare -x foo="abc"` after the call).
//! * rust#1009 — `unset 'a[$j]'` with an empty index unsets element 0.
//!
//! All expectations were measured against GNU bash 5.3.20. Tests only.

use std::fs;
use std::path::PathBuf;
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
fn issue77_declare_p_reprints_arrays_in_gnu_form() {
    let s = Scratch::new("i77");
    let (out, err, rc) =
        s.run("declare -A m=([k]=\"v\")\ndeclare -p m\ndeclare -a arr=(one two)\ndeclare -p arr\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "declare -A m=([k]=\"v\" )\ndeclare -a arr=([0]=\"one\" [1]=\"two\")\n"
    );
}

#[test]
fn issue183_declare_p_funcname_inside_function() {
    let s = Scratch::new("i183");
    let (out, err, rc) = s.run("h() { declare -p FUNCNAME; }\nh\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "declare -a FUNCNAME=([0]=\"h\" [1]=\"main\")\n");
}

#[test]
fn issue189_declare_p_unset_diagnostic_is_redirectable() {
    let s = Scratch::new("i189");
    let (out, err, rc) = s.run("unset ZZQ\nV=$(declare -p ZZQ 2>&1)\necho \"captured=[$V]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(
        out,
        "captured=[$S/case.sh: line 2: declare: ZZQ: not found]\n"
    );
    assert_eq!(err, "");
}

#[test]
fn issue272_readonly_p_with_names_prints_nothing() {
    let s = Scratch::new("i272");
    let (out, err, rc) = s.run(
        "readonly RA=1 RB=2\necho named-begin\nreadonly -p RA\necho named-end\nreadonly -p | grep -cE 'declare -r (RA|RB)='\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "named-begin\nnamed-end\n2\n");
}

#[test]
fn issue409_declared_unassigned_var_expands_as_unset() {
    let s = Scratch::new("i409");
    let (out, err, rc) = s.run("unset VAR\ndeclare -rx VAR\nprintf '<%s>\\n' \"${VAR-unset}\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "<unset>\n");
}

#[test]
fn issue410_posix_typeset_plus_x_shadows_locally_global_survives() {
    let s = Scratch::new("i410");
    let (out, err, rc) = s.run(
        "set -o posix\nf() { foo=abc; export foo; typeset +x foo; echo \"in:${foo-unset}\"; }\nf\necho \"out:${foo-unset}\"\ndeclare -p foo\n",
    );
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "in:unset\nout:abc\ndeclare -x foo=\"abc\"\n");
    assert_eq!(err, "");
}

#[test]
fn issue1009_unset_with_empty_index_removes_element_zero() {
    let s = Scratch::new("i1009");
    let (out, err, rc) =
        s.run("a=(x y z)\nj=\nunset 'a[$j]'\necho \"n=${#a[@]} e0=[${a[0]}] e1=[${a[1]}]\"\n");
    assert_eq!(rc, 0, "stderr: {err}");
    assert_eq!(out, "n=2 e0=[] e1=[y]\n");
}
