//! rubash#3970 — `declare` / `typeset` / `local` / `readonly` compound array
//! assignments must brace-expand their elements like GNU Bash 5.3.20:
//! - `subst.c:12727-12732 brace_expand_word_list`: `W_COMPASSIGN|W_ASSIGNARG`
//!   skips outer command-word brace expansion (previously `declare e=({1..3})`
//!   outer-expanded into `e=(1) e=(2) e=(3)`, leaving only `3`).
//! - `arrayfunc.c:557-610 expand_compound_array_assignment` ->
//!   `expand_words_no_vars` -> `subst.c:13250 brace_expand_word_list`:
//!   indexed compound elements undergo brace expansion before parameter and
//!   pathname expansion (while `assoc` returns early at `arrayfunc.c:594-599`).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn issue3970_declare_compound_assignment_brace_expands() {
    let (stdout, stderr, code) = rubash(
        "declare e=({1..3}); echo \"e:${#e[@]}:[${e[*]}]\"\n\
         declare x=({a..A}); echo \"x:${#x[@]}:${x[0]}:${x[32]}\"\n\
         typeset t=(a{1,2}b); echo \"t:${#t[@]}:[${t[*]}]\"\n\
         f() { local l=({1..3}); echo \"l:${#l[@]}:[${l[*]}]\"; }; f\n\
         readonly r=({1..3}); echo \"r:${#r[@]}:[${r[*]}]\"\n\
         declare -a sq='({1..3})'; echo \"sq:${#sq[@]}:[${sq[*]}]\"\n\
         declare -a dq=\"({1..3})\"; echo \"dq:${#dq[@]}:[${dq[*]}]\"",
    );
    assert_eq!(
        stdout,
        "e:3:[1 2 3]\nx:33:a:A\nt:2:[a1b a2b]\nl:3:[1 2 3]\nr:3:[1 2 3]\nsq:3:[1 2 3]\ndq:3:[1 2 3]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}

#[test]
fn issue3970_declare_compound_brace_controls() {
    let (stdout, stderr, code) = rubash(
        "declare -a q=('{1..3}'); echo \"q:${#q[@]}:[${q[*]}]\"\n\
         declare -A h=({1..4}); echo \"h:${#h[@]}:[${!h[*]}]\"\n\
         set +B; declare -a nb=({1..3}); echo \"nb:${#nb[@]}:[${nb[*]}]\"\n\
         set -B; declare -a emp=({a,}); echo \"emp:${#emp[@]}:[${emp[*]}]\"",
    );
    assert_eq!(
        stdout,
        "q:1:[{1..3}]\nh:1:[{1..4}]\nnb:1:[{1..3}]\nemp:1:[a]\n"
    );
    assert_eq!(stderr, "");
    assert_eq!(code, Some(0));
}
