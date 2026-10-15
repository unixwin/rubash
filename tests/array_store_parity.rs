//! Issue rubash#616 Phase 1a: the indexed-array structured store.
//!
//! `${#name[@]}` counts through `ShellValue::IndexedArray(BTreeMap)`
//! (GNU reads the ARRAY `num_elements` count field directly,
//! subst.c:7533 `array_num_elements` / array.h:121) instead of
//! re-parsing the rendered `env_vars` storage projection.
//!
//! The invariant these tests pin: the structured cell is either a
//! COMPLETE mirror of the rendered storage or absent, so the count can
//! never disagree with the projection. Every expectation below is the
//! literal GNU bash 5.3 output for the same source; each case is also
//! re-run with `RUBASH_ARRAY_STORE=0`, which forces the mirror-derived
//! count fallback, and must produce the identical bytes.

use std::process::{Command, Output};

fn run_with_env(source: &str, arrays: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["-c", source]).env_remove("BASH_ENV");
    match arrays {
        Some(value) => {
            command.env("RUBASH_ARRAY_STORE", value);
        }
        None => {
            command.env_remove("RUBASH_ARRAY_STORE");
        }
    }
    command.output().expect("run rubash")
}

fn run(source: &str) -> Output {
    run_with_env(source, None)
}

/// (case, source, GNU bash 5.3 stdout)
const CASES: &[(&str, &str, &str)] = &[
    (
        "compound_element_append",
        r##"a=(1 2 3 4); a[6]=x; a+=(8 9); echo "${#a[@]}:${a[@]}:${!a[@]}""##,
        r##"7:1 2 3 4 x 8 9:0 1 2 3 6 7 8
"##,
    ),
    (
        "sparse_keys",
        r##"a=([5]=x [2]=y); echo "${#a[@]}:${a[@]}:${!a[@]}""##,
        r##"2:y x:2 5
"##,
    ),
    (
        "negative_subscript",
        r##"a=(1 2 3); a[-1]=z; echo "${#a[@]}:${a[-1]}:${a[0]}""##,
        r##"3:z:1
"##,
    ),
    (
        "unset_whole",
        r##"a=(1 2 3); unset a; echo "${#a[@]}""##,
        r##"0
"##,
    ),
    (
        "unset_element",
        r##"a=(1 2 3); unset 'a[1]'; echo "${#a[@]}:${a[@]}:${!a[@]}""##,
        r##"2:1 3:0 2
"##,
    ),
    (
        "scalar_over_array",
        r##"a=(1 2 3); a=zz; echo "${#a[@]}:${a[@]}""##,
        r##"3:zz 2 3
"##,
    ),
    (
        "declare_replaces",
        r##"a=(1); a[0]=x; declare -a a=(p q); echo "${#a[@]}:${a[@]}""##,
        r##"2:p q
"##,
    ),
    (
        "declare_int_printf_elem",
        r##"declare -i a=(3); printf -v a[5] %s 9; echo "${#a[@]}:${a[@]}:${!a[@]}""##,
        r##"2:3 9:0 5
"##,
    ),
    (
        "read_array_replaces",
        r##"a=(9 9 9); read -r -a a <<< 'q w e'; echo "${#a[@]}:${a[@]}""##,
        r##"3:q w e
"##,
    ),
    (
        "read_ro_array_refused",
        r##"a=(3); readonly a; read -a a <<< 'a' 2>/dev/null; echo "${#a[@]}""##,
        r##"1
"##,
    ),
    (
        "mapfile_replaces",
        r##"mapfile -t a <<< $'m1\nm2\nm3'; echo "${#a[@]}:${a[@]}""##,
        r##"3:m1 m2 m3
"##,
    ),
    (
        "mapfile_offset_keys",
        r##"mapfile -t -O 2 a <<< $'x\ny'; echo "${#a[@]}:${!a[@]}""##,
        r##"2:2 3
"##,
    ),
    (
        "mapfile_offset_append",
        r##"a=([4]=keep); mapfile -t -O 1 a <<< $'x\ny'; echo "${#a[@]}:${!a[@]}""##,
        r##"3:1 2 4
"##,
    ),
    (
        "compgen_V_replaces",
        r##"a=(1); compgen -V a -W 'p q r'; echo "${#a[@]}:${a[@]}""##,
        r##"3:p q r
"##,
    ),
    (
        "arith_then_printf",
        r##"a=(3 4); ((a[1]=9)); printf -v a[0] %s ab; echo "${#a[@]}:${a[@]}:${!a[@]}""##,
        r##"2:ab 9:0 1
"##,
    ),
    (
        "temp_assign_then_element",
        r##"a=(1 7) true; printf -v a[0] %s ab; echo "${#a[@]}:${a[@]}""##,
        r##"1:ab
"##,
    ),
    (
        "temp_assign_keeps_value",
        r##"a=(5 6); a=(1 2 3) true; echo "${#a[@]}:${a[@]}""##,
        r##"2:5 6
"##,
    ),
    (
        "local_array_scope",
        r##"a=(9); f(){ local -a a=(1 2); echo "in=${#a[@]}"; }; f; echo "out=${#a[@]}""##,
        r##"in=2
out=1
"##,
    ),
    (
        "nameref_element",
        r##"a=(1 2 3); declare -n r=a; r[1]=x; echo "${#r[@]}:${a[@]}""##,
        r##"3:1 x 3
"##,
    ),
    (
        "nameref_negative",
        r##"a=(1 2 3); declare -n r=a; r[-2]=9; echo "${#a[@]}:${a[@]}""##,
        r##"3:1 9 3
"##,
    ),
    (
        "bash_rematch",
        r##"if [[ abc123 =~ ([a-z]+)([0-9]+) ]]; then echo "${#BASH_REMATCH[@]}:${BASH_REMATCH[1]}"; fi"##,
        r##"3:abc
"##,
    ),
    (
        "printf_element_then_len",
        r##"printf -v a[2] %s hi; printf -v a[0] %s lo; echo "${#a[@]}:${!a[@]}""##,
        r##"2:0 2
"##,
    ),
    (
        "export_a_replaces",
        r##"a=(1 2 3); export -a a=(9); echo "${#a[@]}:${a[@]}""##,
        r##"1:9
"##,
    ),
    (
        "readonly_a_replaces",
        r##"a=(7 8); readonly -a a=(1); echo "${#a[@]}""##,
        r##"1
"##,
    ),
    (
        "empty_reset",
        r##"a=(1 2 3); a=(); echo "${#a[@]}""##,
        r##"0
"##,
    ),
    (
        "declare_unset_len",
        r##"declare -a a; echo "${#a[@]}""##,
        r##"0
"##,
    ),
    (
        "eval_compound",
        r##"eval 'a=(1 2 3)'; a[5]=z; echo "${#a[@]}:${!a[@]}""##,
        r##"4:0 1 2 5
"##,
    ),
    (
        "func_scope_restore",
        r##"f(){ a=(1 2 3); echo "in=${#a[@]}"; }; f; echo "out=${#a[@]}""##,
        r##"in=3
out=3
"##,
    ),
    (
        "append_scalar_element",
        r##"a=(1 2 3); a[1]+=5; echo "${#a[@]}:${a[@]}""##,
        r##"3:1 25 3
"##,
    ),
    (
        "subshell_isolated",
        r##"( a=(9 9) ); a=(1 2); ( a[5]=x ); echo "${#a[@]}:${!a[@]}""##,
        r##"2:0 1
"##,
    ),
    (
        "sparse_after_unset",
        r##"a=([2]=x [4]=y [7]=z); unset 'a[4]'; echo "${#a[@]}:${!a[@]}""##,
        r##"2:2 7
"##,
    ),
    (
        "gate_zero_equiv",
        r##"a=(1 2 3); RUBASH_ARRAY_STORE=0; echo "${#a[@]}"; a[5]=q; echo "${#a[@]}""##,
        r##"3
4
"##,
    ),
    (
        "braced_indices_len",
        r##"a=([3]=a [9]=b [12]=c); echo "${#a[@]}"; b=(x); echo "${#b[@]}""##,
        r##"3
1
"##,
    ),
    (
        "element_append_sparse",
        r##"a=([4]=keep); a+=(x y); echo "${#a[@]}:${!a[@]}:${a[@]}""##,
        r##"3:4 5 6:keep x y
"##,
    ),
    // rubash#616 (1b): the in-place mirror extension must be byte-identical to
    // a full re-render, so `declare -p` (which prints the render's parse) pins
    // it. The first case is the empty-array first element write, which a wrong
    // "is the body empty?" test renders as `( [0]=...)`; the second is a value
    // whose own last byte is `(`/`)`, which a `ends_with('(')` test mishandles.
    (
        "element_append_from_empty_declare_p",
        r##"a=(); a[0]=x; a[1]=y; declare -p a"##,
        r##"declare -a a=([0]="x" [1]="y")
"##,
    ),
    (
        "element_append_paren_edges_declare_p",
        r##"b=(); b[0]='p('; b[1]='q)'; declare -p b"##,
        r##"declare -a b=([0]="p(" [1]="q)")
"##,
    ),
    (
        "element_append_sparse_gap_declare_p",
        r##"e=(); e[0]=a; e[2]=c; e[3]=d; declare -p e"##,
        r##"declare -a e=([0]="a" [2]="c" [3]="d")
"##,
    ),
    (
        "compound_then_element_append_declare_p",
        r##"f=(p q); f[2]='r('; declare -p f"##,
        r##"declare -a f=([0]="p" [1]="q" [2]="r(")
"##,
    ),
    // rubash#616 (1b): the compound-append cell refresh copies only the
    // appended tail, so these pin the shapes where the retained prefix must
    // survive an element write, a sparse gap, or a negative subscript, and
    // where a later append must still land after the max index.
    (
        "compound_append_after_element_edit",
        r##"a=(p q r); a[1]=Q; a+=(s); declare -p a"##,
        r##"declare -a a=([0]="p" [1]="Q" [2]="r" [3]="s")
"##,
    ),
    (
        "compound_append_sparse_after_edit",
        r##"b=([2]=x); b[0]=y; b+=(z); declare -p b"##,
        r##"declare -a b=([0]="y" [2]="x" [3]="z")
"##,
    ),
    (
        "compound_append_edit_append",
        r##"e=(a b c); e+=(d); e[1]=B; e+=(f); declare -p e"##,
        r##"declare -a e=([0]="a" [1]="B" [2]="c" [3]="d" [4]="f")
"##,
    ),
    (
        "negative_then_append",
        r##"f=(10 20 30); f[-1]=X; f+=(40); declare -p f"##,
        r##"declare -a f=([0]="10" [1]="20" [2]="X" [3]="40")
"##,
    ),
];

#[test]
fn indexed_array_length_matches_gnu_on_every_write_route() {
    for (name, source, expected) in CASES {
        let output = run(source);
        assert!(output.status.success(), "{name}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            *expected,
            "case {name} diverged from GNU bash"
        );
    }
}

#[test]
fn indexed_array_length_is_identical_with_the_gate_disabled() {
    for (name, source, expected) in CASES {
        let output = run_with_env(source, Some("0"));
        assert!(output.status.success(), "{name}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            *expected,
            "case {name} diverged with RUBASH_ARRAY_STORE=0"
        );
    }
}
