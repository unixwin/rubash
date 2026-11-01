//! rubash#2417 — glued compound-assignment elements: a quoted list
//! span (`"${arr[@]}"`, `"${!foo_@}"`, `"$@"`) with literal text glued
//! on either side collapsed to ONE element in rubash, because the
//! plain-element path expands the whole token to a single joined
//! string. GNU expand_word_internal returns a word LIST for the
//! element (subst.c -> string_list_pos_params / the array value list;
//! arrayfunc.c:557 expand_compound_array_assignment expands each
//! resulting word), so the span contributes one element per member
//! with the glued text attached to the first/last member.
//!
//! All expectations below were measured against GNU bash 5.3.20 (the
//! project oracle) with these exact script files. Tests only pin the
//! behavior; the fix lives in glued_quoted_list_element_fields
//! (src/executor/assignment_expansion.rs).

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
            std::env::temp_dir().join(format!("rubash-2417-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Scratch { dir }
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let case = self.dir.join("case.sh");
        fs::write(&case, script).unwrap();
        let out: Output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg(&case)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.dir)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn assert_script(tag: &str, script: &str, expected_stdout: &str) {
    let scratch = Scratch::new(tag);
    let (stdout, stderr, code) = scratch.run(script);
    assert_eq!(code, 0, "{tag}: exit code (stderr: {stderr})");
    assert_eq!(stdout, expected_stdout, "{tag}: stdout (stderr: {stderr})");
}

/// The filed seed: quoted ${arr[@]} glued on both sides fans out per
/// member, prefix on the first, suffix on the last.
#[test]
fn seed_glued_quoted_array_fans_out() {
    assert_script(
        "seed",
        "arr=(x y z)\na=(pre\"${arr[@]}\"post)\ndeclare -p a\n",
        "declare -a a=([0]=\"prex\" [1]=\"y\" [2]=\"zpost\")\n",
    );
}

/// The filed indirect seed: ${!foo_@} names glued the same way.
#[test]
fn seed_glued_indirect_name_list_fans_out() {
    assert_script(
        "indirect",
        "foo_1=1\nfoo_2=2\nc=(pre\"${!foo_@}\"post)\ndeclare -p c\n",
        "declare -a c=([0]=\"prefoo_1\" [1]=\"foo_2post\")\n",
    );
}

/// One-sided glue, an empty array (glued text alone survives as one
/// element), a scalar under [@], and glued positional parameters.
#[test]
fn glue_variants() {
    assert_script(
        "variants",
        r#"arr=(x y z)
a=("${arr[@]}"post)
declare -p a
e=()
f=(pre"${e[@]}"post)
declare -p f
s=one
n=(pre"${s[@]}"post)
declare -p n
set -- aa bb
g=(pre"$@"post)
declare -p g
"#,
        "declare -a a=([0]=\"x\" [1]=\"y\" [2]=\"zpost\")\n\
         declare -a f=([0]=\"prepost\")\n\
         declare -a n=([0]=\"preonepost\")\n\
         declare -a g=([0]=\"preaa\" [1]=\"bbpost\")\n",
    );
}

/// Two list spans in one element word fold sequentially: the text
/// between them glues the first span's last member to the second
/// span's first member.
#[test]
fn two_spans_fold_sequentially() {
    assert_script(
        "twospan",
        "arr=(x y z)\nh=(\"${arr[@]}\"_\"${arr[@]}\")\ndeclare -p h\n",
        "declare -a h=([0]=\"x\" [1]=\"y\" [2]=\"z_x\" [3]=\"y\" [4]=\"z\")\n",
    );
}

/// Guards: the unglued quoted form (already correct) and the quoted
/// [*] join (ONE element by design) are unchanged.
#[test]
fn unglued_and_star_forms_unchanged() {
    assert_script(
        "guards",
        r#"arr=(x y z)
b=("${arr[@]}")
declare -p b
d=(pre"${arr[*]}"post)
declare -p d
"#,
        "declare -a b=([0]=\"x\" [1]=\"y\" [2]=\"z\")\n\
         declare -a d=([0]=\"prex y zpost\")\n",
    );
}
