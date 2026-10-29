//! Oils-spec vendor: spec/arith.test.sh.
//!
//! Vendored from the Oils shell specification suite (Apache License 2.0):
//!   https://github.com/oils-for-unix/oils/blob/master/spec/arith.test.sh
//!
//! Transcription rules (see task): the `#### case` / `## STDOUT` / `## OK`
//! blocks were rewritten as `Command::new(env!("CARGO_BIN_EXE_rubash"))`
//! assertions in this file's style. `argv.py` prints became
//! `printf '<%s>\n'` lines and `$TMP` fixtures became sandbox-local files.
//! Expected values were measured with the local GNU bash 5.3.20
//! (`bash -c`, file-free sandbox, `LC_ALL=C`) — never copied from an oils
//! `## OK` line, which may record oils behavior instead of bash behavior.
//! Only cases where rubash already matches bash byte-for-byte are kept;
//! the divergences found while probing are listed in the task report.

use std::fs;
use std::process::Command;

fn sandbox(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-oils-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos(),
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create sandbox dir");
    dir
}

fn run(script: &str) -> (i32, String, String) {
    let dir = sandbox("case");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env("HOME", &dir)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("TMPDIR", &dir)
        .output()
        .expect("spawn rubash");
    let _ = fs::remove_dir_all(&dir);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn arith_64bit() {
    // Oils `####` case: arith/64bit
    let (status, stdout, stderr) = run(r#"a=$(( 1 << 31 ))
echo $a
b=$(( a + a ))
echo $b
c=$(( b + a ))
echo $c
x=$(( 1 << 62 ))
y=$(( x - 1 ))
echo "max positive = $(( x + y ))""#);
    assert_eq!(status, 0, "arith/64bit status");
    assert_eq!(
        stdout,
        r"2147483648
4294967296
6442450944
max positive = 9223372036854775807
",
        "arith/64bit stdout"
    );
    assert_eq!(stderr, r"", "arith/64bit stderr");
}

#[test]
fn arith_add_one() {
    // Oils `####` case: arith/add-one
    let (status, stdout, stderr) = run(r"i=1
echo $(($i+1))");
    assert_eq!(status, 0, "arith/add-one status");
    assert_eq!(
        stdout,
        r"2
",
        "arith/add-one stdout"
    );
    assert_eq!(stderr, r"", "arith/add-one stderr");
}

#[test]
fn arith_arithsub_array_decay() {
    // Oils `####` case: arith/arithsub-array-decay
    let (status, stdout, stderr) = run(r"a=(4 5 6)
echo declared
b=$(( a ))
echo $b");
    assert_eq!(status, 0, "arith/arithsub-array-decay status");
    assert_eq!(
        stdout,
        r"declared
4
",
        "arith/arithsub-array-decay stdout"
    );
    assert_eq!(stderr, r"", "arith/arithsub-array-decay stderr");
}

#[test]
fn arith_array_decay_add() {
    // Oils `####` case: arith/array-decay-add
    let (status, stdout, stderr) = run(r"declare -a array=(1 2 3)
echo $((array + 5))");
    assert_eq!(status, 0, "arith/array-decay-add status");
    assert_eq!(
        stdout,
        r"6
",
        "arith/array-decay-add stdout"
    );
    assert_eq!(stderr, r"", "arith/array-decay-add stderr");
}

#[test]
fn arith_array_indexing() {
    // Oils `####` case: arith/array-indexing
    let (status, stdout, stderr) = run(r"array=(1 2 3 4)
echo $((array[1] + array[2]*3))");
    assert_eq!(status, 0, "arith/array-indexing status");
    assert_eq!(
        stdout,
        r"11
",
        "arith/array-indexing stdout"
    );
    assert_eq!(stderr, r"", "arith/array-indexing stderr");
}

#[test]
fn arith_aug_assign() {
    // Oils `####` case: arith/aug-assign
    let (status, stdout, stderr) = run(r"a=4
echo $((a+=1))
echo $a");
    assert_eq!(status, 0, "arith/aug-assign status");
    assert_eq!(
        stdout,
        r"5
5
",
        "arith/aug-assign stdout"
    );
    assert_eq!(stderr, r"", "arith/aug-assign stderr");
}

#[test]
fn arith_backticks() {
    // Oils `####` case: arith/backticks
    let (status, stdout, stderr) = run(r"echo $((`echo 1` + 2))");
    assert_eq!(status, 0, "arith/backticks status");
    assert_eq!(
        stdout,
        r"3
",
        "arith/backticks stdout"
    );
    assert_eq!(stderr, r"", "arith/backticks stderr");
}

#[test]
fn arith_base_2_to_64() {
    // Oils `####` case: arith/base-2-to-64
    let (status, stdout, stderr) =
        run(r"echo $((64#a))-$((64#z)), $((64#A))-$((64#Z)), $((64#@)), $(( 64#_ ))");
    assert_eq!(status, 0, "arith/base-2-to-64 status");
    assert_eq!(
        stdout,
        r"10-35, 36-61, 62, 63
",
        "arith/base-2-to-64 stdout"
    );
    assert_eq!(stderr, r"", "arith/base-2-to-64 stderr");
}

#[test]
fn arith_base36() {
    // Oils `####` case: arith/base36
    let (status, stdout, stderr) = run(r"echo $((36#a))-$((36#z))");
    assert_eq!(status, 0, "arith/base36 status");
    assert_eq!(
        stdout,
        r"10-35
",
        "arith/base36 stdout"
    );
    assert_eq!(stderr, r"", "arith/base36 stderr");
}

#[test]
fn arith_bit_shift_basic() {
    // Oils `####` case: arith/bit-shift-basic
    let (status, stdout, stderr) = run(r"echo $(( 5 << 1 ))
echo $(( 5 << 0 ))
echo ---
echo $(( 16 >> 1 ))
echo $(( 16 >> 0 ))
echo ---");
    assert_eq!(status, 0, "arith/bit-shift-basic status");
    assert_eq!(
        stdout,
        r"10
5
---
8
16
---
",
        "arith/bit-shift-basic stdout"
    );
    assert_eq!(stderr, r"", "arith/bit-shift-basic stderr");
}

#[test]
fn arith_bitwise() {
    // Oils `####` case: arith/bitwise
    let (status, stdout, stderr) = run(r"echo $((1|2))
echo $((1&2))
echo $((1^2))
echo $((~(1|2)))");
    assert_eq!(status, 0, "arith/bitwise status");
    assert_eq!(
        stdout,
        r"3
0
3
-4
",
        "arith/bitwise stdout"
    );
    assert_eq!(stderr, r"", "arith/bitwise stderr");
}

#[test]
fn arith_braced_varsub() {
    // Oils `####` case: arith/braced-varsub
    let (status, stdout, stderr) = run(r"echo $((${j:-5} + 1))");
    assert_eq!(status, 0, "arith/braced-varsub status");
    assert_eq!(
        stdout,
        r"6
",
        "arith/braced-varsub stdout"
    );
    assert_eq!(stderr, r"", "arith/braced-varsub stderr");
}

#[test]
fn arith_bracket_64bit_compare() {
    // Oils `####` case: arith/bracket-64bit-compare
    let (status, stdout, stderr) = run(r"[[ 12345678909 = $(( 1 << 30 )) ]]
echo eq=$?
[[ 12345678909 = 12345678909 ]]
echo eq=$?
[ 12345678909 -gt $(( 1 << 30 )) ]
echo greater=$?
[[ 12345678909 -gt $(( 1 << 30 )) ]]
echo greater=$?
[[ 12345678909 -ge $(( 1 << 30 )) ]]
echo ge=$?
[[ 12345678909 -ge 12345678909 ]]
echo ge=$?
[[ 12345678909 -le $(( 1 << 30 )) ]]
echo le=$?
[[ 12345678909 -le 12345678909 ]]
echo le=$?");
    assert_eq!(status, 0, "arith/bracket-64bit-compare status");
    assert_eq!(
        stdout,
        r"eq=1
eq=0
greater=0
greater=0
ge=0
ge=0
le=1
le=0
",
        "arith/bracket-64bit-compare stdout"
    );
    assert_eq!(stderr, r"", "arith/bracket-64bit-compare stderr");
}

#[test]
fn arith_comma_op() {
    // Oils `####` case: arith/comma-op
    let (status, stdout, stderr) = run(r"a=1
b=2
echo $((a,(b+1)))");
    assert_eq!(status, 0, "arith/comma-op status");
    assert_eq!(
        stdout,
        r"3
",
        "arith/comma-op stdout"
    );
    assert_eq!(stderr, r"", "arith/comma-op stderr");
}

#[test]
fn arith_comma_with_array() {
    // Oils `####` case: arith/comma-with-array
    let (status, stdout, stderr) = run(r"a=(4 5 6)
echo $(( a, last = a[2], 42 ))
echo last=$last");
    assert_eq!(status, 0, "arith/comma-with-array status");
    assert_eq!(
        stdout,
        r"42
last=6
",
        "arith/comma-with-array stdout"
    );
    assert_eq!(stderr, r"", "arith/comma-with-array stderr");
}

#[test]
fn arith_comparison_ops() {
    // Oils `####` case: arith/comparison-ops
    let (status, stdout, stderr) = run(r"echo $(( 1 == 1 ))
echo $(( 1 != 1 ))
echo $(( 1 < 1 ))
echo $(( 1 <= 1 ))
echo $(( 1 > 1 ))
echo $(( 1 >= 1 ))");
    assert_eq!(status, 0, "arith/comparison-ops status");
    assert_eq!(
        stdout,
        r"1
0
0
1
0
1
",
        "arith/comparison-ops stdout"
    );
    assert_eq!(stderr, r"", "arith/comparison-ops stderr");
}

#[test]
fn arith_dollar_optional() {
    // Oils `####` case: arith/dollar-optional
    let (status, stdout, stderr) = run(r"i=1
echo $((i+1))");
    assert_eq!(status, 0, "arith/dollar-optional status");
    assert_eq!(
        stdout,
        r"2
",
        "arith/dollar-optional stdout"
    );
    assert_eq!(stderr, r"", "arith/dollar-optional stderr");
}

#[test]
fn arith_double_subscript() {
    // Oils `####` case: arith/double-subscript
    let (status, stdout, stderr) = run(r"a=(1 2 3)
echo $(( a[1] ))
echo $(( a[1][1] ))");
    assert_eq!(status, 1, "arith/double-subscript status");
    assert_eq!(
        stdout,
        r"2
",
        "arith/double-subscript stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 3: a[1][1] : arithmetic syntax error: invalid arithmetic operator (error token is "[1] ")
"#,
        "arith/double-subscript stderr"
    );
}

#[test]
fn arith_dynamic_array_name() {
    // Oils `####` case: arith/dynamic-array-name
    let (status, stdout, stderr) = run(r"foo=bar
echo $(( x$foo[5] = 42 ))
echo 'xbar[5]='${xbar[5]}");
    assert_eq!(status, 0, "arith/dynamic-array-name status");
    assert_eq!(
        stdout,
        r"42
xbar[5]=42
",
        "arith/dynamic-array-name stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-array-name stderr");
}

#[test]
fn arith_dynamic_assign_name() {
    // Oils `####` case: arith/dynamic-assign-name
    let (status, stdout, stderr) = run(r"foo=bar
echo $(( x$foo = 42 ))
echo xbar=$xbar");
    assert_eq!(status, 0, "arith/dynamic-assign-name status");
    assert_eq!(
        stdout,
        r"42
xbar=42
",
        "arith/dynamic-assign-name stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-assign-name stderr");
}

#[test]
fn arith_dynamic_base() {
    // Oils `####` case: arith/dynamic-base
    let (status, stdout, stderr) = run(r"base=16
echo $(( ${base}#a ))");
    assert_eq!(status, 0, "arith/dynamic-base status");
    assert_eq!(
        stdout,
        r"10
",
        "arith/dynamic-base stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-base stderr");
}

#[test]
fn arith_dynamic_empty_string() {
    // Oils `####` case: arith/dynamic-empty-string
    let (status, stdout, stderr) = run(r"a=''
echo $(( a ))
a2=' '
echo $(( a2 ))");
    assert_eq!(status, 0, "arith/dynamic-empty-string status");
    assert_eq!(
        stdout,
        r"0
0
",
        "arith/dynamic-empty-string stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-empty-string stderr");
}

#[test]
fn arith_dynamic_hex() {
    // Oils `####` case: arith/dynamic-hex
    let (status, stdout, stderr) = run(r"zero=0
echo $(( ${zero}xAB ))");
    assert_eq!(status, 0, "arith/dynamic-hex status");
    assert_eq!(
        stdout,
        r"171
",
        "arith/dynamic-hex stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-hex stderr");
}

#[test]
fn arith_dynamic_octal() {
    // Oils `####` case: arith/dynamic-octal
    let (status, stdout, stderr) = run(r"zero=0
echo $(( ${zero}11 ))");
    assert_eq!(status, 0, "arith/dynamic-octal status");
    assert_eq!(
        stdout,
        r"9
",
        "arith/dynamic-octal stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-octal stderr");
}

#[test]
fn arith_dynamic_parsing() {
    // Oils `####` case: arith/dynamic-parsing
    let (status, stdout, stderr) = run(r"e=1+2
echo $(( e + 3 ))
[[ e -eq 3 ]] && echo true
[ e -eq 3 ]
echo status=$?");
    assert_eq!(status, 0, "arith/dynamic-parsing status");
    assert_eq!(
        stdout,
        r"6
true
status=2
",
        "arith/dynamic-parsing stdout"
    );
    assert_eq!(
        stderr,
        r"bash: line 4: [: e: integer expected
",
        "arith/dynamic-parsing stderr"
    );
}

#[test]
fn arith_dynamic_var_names() {
    // Oils `####` case: arith/dynamic-var-names
    let (status, stdout, stderr) = run(r"foo=5
x=oo
echo $(( foo + f$x + 1 ))");
    assert_eq!(status, 0, "arith/dynamic-var-names status");
    assert_eq!(
        stdout,
        r"11
",
        "arith/dynamic-var-names stdout"
    );
    assert_eq!(stderr, r"", "arith/dynamic-var-names stderr");
}

#[test]
fn arith_exponent() {
    // Oils `####` case: arith/exponent
    let (status, stdout, stderr) = run(r"echo $(( 3 ** 0 ))
echo $(( 3 ** 1 ))
echo $(( 3 ** 2 ))");
    assert_eq!(status, 0, "arith/exponent status");
    assert_eq!(
        stdout,
        r"1
3
9
",
        "arith/exponent stdout"
    );
    assert_eq!(stderr, r"", "arith/exponent stderr");
}

#[test]
fn arith_exponent_precedence() {
    // Oils `####` case: arith/exponent-precedence
    let (status, stdout, stderr) = run(r"echo $(( -3 ** 2 ))");
    assert_eq!(status, 0, "arith/exponent-precedence status");
    assert_eq!(
        stdout,
        r"9
",
        "arith/exponent-precedence stdout"
    );
    assert_eq!(stderr, r"", "arith/exponent-precedence stderr");
}

#[test]
fn arith_hex_capital() {
    // Oils `####` case: arith/hex-capital
    let (status, stdout, stderr) = run(r"echo $(( 0XAA ))");
    assert_eq!(status, 0, "arith/hex-capital status");
    assert_eq!(
        stdout,
        r"170
",
        "arith/hex-capital stdout"
    );
    assert_eq!(stderr, r"", "arith/hex-capital stderr");
}

#[test]
fn arith_incr_array_elems() {
    // Oils `####` case: arith/incr-array-elems
    let (status, stdout, stderr) = run(r#"a=(5 6 7 8)
(( a[0]++, ++a[1], a[2]--, --a[3] ))
(( undef[0]++, ++undef[1], undef[2]--, --undef[3] ))
echo "${a[@]} - ${undef[@]}""#);
    assert_eq!(status, 0, "arith/incr-array-elems status");
    assert_eq!(
        stdout,
        r"6 7 6 7 - 1 1 -1 -1
",
        "arith/incr-array-elems stdout"
    );
    assert_eq!(stderr, r"", "arith/incr-array-elems stderr");
}

#[test]
fn arith_incr_undef() {
    // Oils `####` case: arith/incr-undef
    let (status, stdout, stderr) = run(r#"(( undef1++ ))
(( ++undef2 ))
echo "[$undef1][$undef2]""#);
    assert_eq!(status, 0, "arith/incr-undef status");
    assert_eq!(
        stdout,
        r"[1][1]
",
        "arith/incr-undef stdout"
    );
    assert_eq!(stderr, r"", "arith/incr-undef stderr");
}

#[test]
fn arith_int_consts() {
    // Oils `####` case: arith/int-consts
    let (status, stdout, stderr) = run(r"echo $(( 0x12A ))
echo $(( 0x0A ))
echo $(( 0777 ))
echo $(( 0010 ))
echo $(( 24#ag7 ))");
    assert_eq!(status, 0, "arith/int-consts status");
    assert_eq!(
        stdout,
        r"298
10
511
8
6151
",
        "arith/int-consts stdout"
    );
    assert_eq!(stderr, r"", "arith/int-consts stderr");
}

#[test]
#[allow(non_snake_case)] // name mirrors the Oils spec case id `arith/invalid-const-A`
fn arith_invalid_const_A() {
    // Oils `####` case: arith/invalid-const-A
    let (status, stdout, stderr) = run(r"echo $((a + x42))
echo status=$?");
    assert_eq!(status, 0, "arith/invalid-const-A status");
    assert_eq!(
        stdout,
        r"0
status=0
",
        "arith/invalid-const-A stdout"
    );
    assert_eq!(stderr, r"", "arith/invalid-const-A stderr");
}

#[test]
#[allow(non_snake_case)] // name mirrors the Oils spec case id `arith/invalid-const-B-fresh`
fn arith_invalid_const_B_fresh() {
    // Oils `####` case: arith/invalid-const-B-fresh
    let (status, stdout, stderr) = run(r"echo $((a + 42x))
echo status=$?");
    assert_eq!(status, 0, "arith/invalid-const-B-fresh status");
    assert_eq!(
        stdout,
        r"status=1
",
        "arith/invalid-const-B-fresh stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 1: a + 42x: value too great for base (error token is "42x")
"#,
        "arith/invalid-const-B-fresh stderr"
    );
}

#[test]
fn arith_invalid_lvalue_array() {
    // Oils `####` case: arith/invalid-lvalue-array
    let (status, stdout, stderr) = run(r#"(( 1[2] = 3 ))
echo "status=$?""#);
    assert_eq!(status, 0, "arith/invalid-lvalue-array status");
    assert_eq!(
        stdout,
        r"status=1
",
        "arith/invalid-lvalue-array stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 1: ((: 1[2] = 3 : arithmetic syntax error: invalid arithmetic operator (error token is "[2] = 3 ")
"#,
        "arith/invalid-lvalue-array stderr"
    );
}

#[test]
fn arith_invalid_lvalue_paren() {
    // Oils `####` case: arith/invalid-lvalue-paren
    let (status, stdout, stderr) = run(r"a=9
(( (a + 2) = 3 ))
echo $a");
    assert_eq!(status, 0, "arith/invalid-lvalue-paren status");
    assert_eq!(
        stdout,
        r"9
",
        "arith/invalid-lvalue-paren stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 2: ((: (a + 2) = 3 : attempted assignment to non-variable (error token is "= 3 ")
"#,
        "arith/invalid-lvalue-paren stderr"
    );
}

#[test]
fn arith_invalid_lvalue_twice() {
    // Oils `####` case: arith/invalid-lvalue-twice
    let (status, stdout, stderr) = run(r#"(( a[1][2] = 3 ))
echo "status=$?""#);
    assert_eq!(status, 0, "arith/invalid-lvalue-twice status");
    assert_eq!(
        stdout,
        r"status=1
",
        "arith/invalid-lvalue-twice stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 1: ((: a[1][2] = 3 : arithmetic syntax error: invalid arithmetic operator (error token is "[2] = 3 ")
"#,
        "arith/invalid-lvalue-twice stderr"
    );
}

#[test]
fn arith_invalid_string_to_int() {
    // Oils `####` case: arith/invalid-string-to-int
    let (status, stdout, stderr) = run(r"s=foo
echo $((s+5))");
    assert_eq!(status, 0, "arith/invalid-string-to-int status");
    assert_eq!(
        stdout,
        r"5
",
        "arith/invalid-string-to-int stdout"
    );
    assert_eq!(stderr, r"", "arith/invalid-string-to-int stderr");
}

#[test]
fn arith_logical_ops() {
    // Oils `####` case: arith/logical-ops
    let (status, stdout, stderr) = run(r"echo $((1 || 2))
echo $((1 && 2))
echo $((!(1 || 2)))");
    assert_eq!(status, 0, "arith/logical-ops status");
    assert_eq!(
        stdout,
        r"1
1
0
",
        "arith/logical-ops stdout"
    );
    assert_eq!(stderr, r"", "arith/logical-ops stderr");
}

#[test]
fn arith_logical_shortcircuit() {
    // Oils `####` case: arith/logical-shortcircuit
    let (status, stdout, stderr) = run(r"x=11
(( 1 || (x = 22) ))
echo $x
(( 0 || (x = 33) ))
echo $x
(( 0 && (x = 44) ))
echo $x
(( 1 && (x = 55) ))
echo $x");
    assert_eq!(status, 0, "arith/logical-shortcircuit status");
    assert_eq!(
        stdout,
        r"11
33
33
55
",
        "arith/logical-shortcircuit stdout"
    );
    assert_eq!(stderr, r"", "arith/logical-shortcircuit stderr");
}

#[test]
fn arith_multidigit_base() {
    // Oils `####` case: arith/multidigit-base
    let (status, stdout, stderr) = run(r"echo $((10#0123)), $((16#1b))");
    assert_eq!(status, 0, "arith/multidigit-base status");
    assert_eq!(
        stdout,
        r"123, 27
",
        "arith/multidigit-base stdout"
    );
    assert_eq!(stderr, r"", "arith/multidigit-base stderr");
}

#[test]
fn arith_neg_division() {
    // Oils `####` case: arith/neg-division
    let (status, stdout, stderr) = run(r"echo $(( 10 / 3))
echo $((-10 / 3))
echo $(( 10 / -3))
echo $((-10 / -3))
echo ---
a=20
: $(( a /= 3 ))
echo $a
a=-20
: $(( a /= 3 ))
echo $a
a=20
: $(( a /= -3 ))
echo $a
a=-20
: $(( a /= -3 ))
echo $a");
    assert_eq!(status, 0, "arith/neg-division status");
    assert_eq!(
        stdout,
        r"3
-3
-3
3
---
6
-6
-6
6
",
        "arith/neg-division stdout"
    );
    assert_eq!(stderr, r"", "arith/neg-division stderr");
}

#[test]
fn arith_neg_modulo() {
    // Oils `####` case: arith/neg-modulo
    let (status, stdout, stderr) = run(r"echo $(( 10 % 3))
echo $((-10 % 3))
echo $(( 10 % -3))
echo $((-10 % -3))");
    assert_eq!(status, 0, "arith/neg-modulo status");
    assert_eq!(
        stdout,
        r"1
-1
1
-1
",
        "arith/neg-modulo stdout"
    );
    assert_eq!(stderr, r"", "arith/neg-modulo stderr");
}

#[test]
fn arith_nested_sub() {
    // Oils `####` case: arith/nested-sub
    let (status, stdout, stderr) = run(r"echo $((1 + $((2 + 3)) + 4))");
    assert_eq!(status, 0, "arith/nested-sub status");
    assert_eq!(
        stdout,
        r"10
",
        "arith/nested-sub stdout"
    );
    assert_eq!(stderr, r"", "arith/nested-sub stderr");
}

#[test]
fn arith_nested_ternary() {
    // Oils `####` case: arith/nested-ternary
    let (status, stdout, stderr) = run(r"echo $((1?2?3:4:5))");
    assert_eq!(status, 0, "arith/nested-ternary status");
    assert_eq!(
        stdout,
        r"3
",
        "arith/nested-ternary stdout"
    );
    assert_eq!(stderr, r"", "arith/nested-ternary stderr");
}

#[test]
fn arith_newline_in_expr() {
    // Oils `####` case: arith/newline-in-expr
    let (status, stdout, stderr) = run(r"echo $((1
+ 2))");
    assert_eq!(status, 0, "arith/newline-in-expr status");
    assert_eq!(
        stdout,
        r"3
",
        "arith/newline-in-expr stdout"
    );
    assert_eq!(stderr, r"", "arith/newline-in-expr stderr");
}

#[test]
fn arith_no_float() {
    // Oils `####` case: arith/no-float
    let (status, stdout, stderr) = run(r"echo $((1 + 2.3))");
    assert_eq!(status, 1, "arith/no-float status");
    assert_eq!(stdout, r"", "arith/no-float stdout");
    assert_eq!(
        stderr,
        r#"bash: line 1: 1 + 2.3: arithmetic syntax error: invalid arithmetic operator (error token is ".3")
"#,
        "arith/no-float stderr"
    );
}

#[test]
fn arith_nounset() {
    // Oils `####` case: arith/nounset
    let (status, stdout, stderr) = run(r#"set -o nounset
x=$(( y + 5 ))
echo "should not get here: x=${x:-<unset>}""#);
    assert_eq!(status, 127, "arith/nounset status");
    assert_eq!(stdout, r"", "arith/nounset stdout");
    assert_eq!(
        stderr,
        r"bash: line 2: y: unbound variable
",
        "arith/nounset stderr"
    );
}

#[test]
fn arith_octal() {
    // Oils `####` case: arith/octal
    let (status, stdout, stderr) = run(r"echo $(( 011 ))");
    assert_eq!(status, 0, "arith/octal status");
    assert_eq!(
        stdout,
        r"9
",
        "arith/octal stdout"
    );
    assert_eq!(stderr, r"", "arith/octal stderr");
}

#[test]
fn arith_postincr() {
    // Oils `####` case: arith/postincr
    let (status, stdout, stderr) = run(r"a=4
echo $((a++))
echo $a");
    assert_eq!(status, 0, "arith/postincr status");
    assert_eq!(
        stdout,
        r"4
5
",
        "arith/postincr stdout"
    );
    assert_eq!(stderr, r"", "arith/postincr stderr");
}

#[test]
fn arith_precedence() {
    // Oils `####` case: arith/precedence
    let (status, stdout, stderr) = run(r"echo $(( 1 + 2*3 - 8/2 ))");
    assert_eq!(status, 0, "arith/precedence status");
    assert_eq!(
        stdout,
        r"3
",
        "arith/precedence stdout"
    );
    assert_eq!(stderr, r"", "arith/precedence stderr");
}

#[test]
fn arith_preincr() {
    // Oils `####` case: arith/preincr
    let (status, stdout, stderr) = run(r"a=4
echo $((++a))
echo $a");
    assert_eq!(status, 0, "arith/preincr status");
    assert_eq!(
        stdout,
        r"5
5
",
        "arith/preincr stdout"
    );
    assert_eq!(stderr, r"", "arith/preincr stderr");
}

#[test]
fn arith_quoted_const() {
    // Oils `####` case: arith/quoted-const
    let (status, stdout, stderr) = run(r"echo $(('1' + 2))");
    assert_eq!(status, 1, "arith/quoted-const status");
    assert_eq!(stdout, r"", "arith/quoted-const stdout");
    assert_eq!(
        stderr,
        r#"bash: line 1: '1' + 2: arithmetic syntax error: operand expected (error token is "'1' + 2")
"#,
        "arith/quoted-const stderr"
    );
}

#[test]
fn arith_recursive_names() {
    // Oils `####` case: arith/recursive-names
    let (status, stdout, stderr) = run(r"foo=5
bar=foo
spam=bar
eggs=spam
echo $((foo+1)) $((bar+1)) $((spam+1)) $((eggs+1))");
    assert_eq!(status, 0, "arith/recursive-names status");
    assert_eq!(
        stdout,
        r"6 6 6 6
",
        "arith/recursive-names stdout"
    );
    assert_eq!(stderr, r"", "arith/recursive-names stderr");
}

#[test]
fn arith_side_effect_in_array_index() {
    // Oils `####` case: arith/side-effect-in-array-index
    let (status, stdout, stderr) = run(r#"a=(4 5 6)
echo "${a[b=2]} b=$b""#);
    assert_eq!(status, 0, "arith/side-effect-in-array-index status");
    assert_eq!(
        stdout,
        r"6 b=2
",
        "arith/side-effect-in-array-index stdout"
    );
    assert_eq!(stderr, r"", "arith/side-effect-in-array-index stderr");
}

#[test]
fn arith_simple_varsub() {
    // Oils `####` case: arith/simple-varsub
    let (status, stdout, stderr) = run(r"j=0
echo $(($j + 42))");
    assert_eq!(status, 0, "arith/simple-varsub status");
    assert_eq!(
        stdout,
        r"42
",
        "arith/simple-varsub stdout"
    );
    assert_eq!(stderr, r"", "arith/simple-varsub stderr");
}

#[test]
fn arith_str_elem_42() {
    // Oils `####` case: arith/str-elem-42
    let (status, stdout, stderr) = run(r"s='42'
echo 42 $(( s[0] )) $(( s[1] ))
echo status=$?");
    assert_eq!(status, 0, "arith/str-elem-42 status");
    assert_eq!(
        stdout,
        r"42 42 0
status=0
",
        "arith/str-elem-42 stdout"
    );
    assert_eq!(stderr, r"", "arith/str-elem-42 stderr");
}

#[test]
fn arith_str_elem_abc() {
    // Oils `####` case: arith/str-elem-abc
    let (status, stdout, stderr) = run(r"s='abc'
echo abc $(( s[0] )) $(( s[1] ))
echo status=$?
echo
(( s[0] ))
echo status=$?
echo");
    assert_eq!(status, 0, "arith/str-elem-abc status");
    assert_eq!(
        stdout,
        r"abc 0 0
status=0

status=1

",
        "arith/str-elem-abc stdout"
    );
    assert_eq!(stderr, r"", "arith/str-elem-abc stderr");
}

#[test]
fn arith_str_elem_space() {
    // Oils `####` case: arith/str-elem-space
    let (status, stdout, stderr) = run(r"s='12 34'
echo '12 34' $(( s[0] )) $(( s[1] ))
echo status=$?");
    assert_eq!(status, 0, "arith/str-elem-space status");
    assert_eq!(
        stdout,
        r"status=1
",
        "arith/str-elem-space stdout"
    );
    assert_eq!(
        stderr,
        r#"bash: line 2: 12 34: arithmetic syntax error in expression (error token is "34")
"#,
        "arith/str-elem-space stderr"
    );
}

#[test]
fn arith_ternary() {
    // Oils `####` case: arith/ternary
    let (status, stdout, stderr) = run(r"a=1
b=2
echo $((a>b?5:10))");
    assert_eq!(status, 0, "arith/ternary status");
    assert_eq!(
        stdout,
        r"10
",
        "arith/ternary stdout"
    );
    assert_eq!(stderr, r"", "arith/ternary stderr");
}

#[test]
fn arith_ternary_assign() {
    // Oils `####` case: arith/ternary-assign
    let (status, stdout, stderr) = run(r"echo $((1 ? a=1 : 42 ))
echo a=$a");
    assert_eq!(status, 0, "arith/ternary-assign status");
    assert_eq!(
        stdout,
        r"1
a=1
",
        "arith/ternary-assign stdout"
    );
    assert_eq!(stderr, r"", "arith/ternary-assign stderr");
}

#[test]
fn arith_unary_array_dynamic_name() {
    // Oils `####` case: arith/unary-array-dynamic-name
    let (status, stdout, stderr) = run(r"foo=bar
xbar[5]=42
echo $(( x$foo[5]++ ))
echo 'xbar[5]='${xbar[5]}");
    assert_eq!(status, 0, "arith/unary-array-dynamic-name status");
    assert_eq!(
        stdout,
        r"42
xbar[5]=43
",
        "arith/unary-array-dynamic-name stdout"
    );
    assert_eq!(stderr, r"", "arith/unary-array-dynamic-name stderr");
}

#[test]
fn arith_unary_dynamic_name() {
    // Oils `####` case: arith/unary-dynamic-name
    let (status, stdout, stderr) = run(r"foo=bar
xbar=42
echo $(( x$foo++ ))
echo xbar=$xbar");
    assert_eq!(status, 0, "arith/unary-dynamic-name status");
    assert_eq!(
        stdout,
        r"42
xbar=43
",
        "arith/unary-dynamic-name stdout"
    );
    assert_eq!(stderr, r"", "arith/unary-dynamic-name stderr");
}

#[test]
fn arith_unary_minus_plus() {
    // Oils `####` case: arith/unary-minus-plus
    let (status, stdout, stderr) = run(r"a=1
b=3
echo $((- a + + b))");
    assert_eq!(status, 0, "arith/unary-minus-plus status");
    assert_eq!(
        stdout,
        r"2
",
        "arith/unary-minus-plus stdout"
    );
    assert_eq!(stderr, r"", "arith/unary-minus-plus stderr");
}

#[test]
fn arith_undef_elem() {
    // Oils `####` case: arith/undef-elem
    let (status, stdout, stderr) = run(r"echo ARITH $(( undef[0] ))
echo status=$?
echo
(( undef[0] ))
echo status=$?
echo
echo UNDEF ${undef[0]}
echo status=$?");
    assert_eq!(status, 0, "arith/undef-elem status");
    assert_eq!(
        stdout,
        r"ARITH 0
status=0

status=1

UNDEF
status=0
",
        "arith/undef-elem stdout"
    );
    assert_eq!(stderr, r"", "arith/undef-elem stderr");
}

#[test]
fn arith_undef_elem_nounset() {
    // Oils `####` case: arith/undef-elem-nounset
    let (status, stdout, stderr) = run(r"set -o nounset
echo UNSET $(( undef[0] ))
echo status=$?");
    assert_eq!(status, 127, "arith/undef-elem-nounset status");
    assert_eq!(stdout, r"", "arith/undef-elem-nounset stdout");
    assert_eq!(
        stderr,
        r"bash: line 2: undef: unbound variable
",
        "arith/undef-elem-nounset stderr"
    );
}

#[test]
fn arith_word_part() {
    // Oils `####` case: arith/word-part
    let (status, stdout, stderr) = run(r"foo=1; echo $((foo+1))bar$(($foo+1))");
    assert_eq!(status, 0, "arith/word-part status");
    assert_eq!(
        stdout,
        r"2bar2
",
        "arith/word-part stdout"
    );
    assert_eq!(stderr, r"", "arith/word-part stderr");
}

#[test]
fn arith_word_parts_sub() {
    // Oils `####` case: arith/word-parts-sub
    let (status, stdout, stderr) = run(r"echo $((1 + $(echo 1)${undefined:-3}))");
    assert_eq!(status, 0, "arith/word-parts-sub status");
    assert_eq!(
        stdout,
        r"14
",
        "arith/word-parts-sub stdout"
    );
    assert_eq!(stderr, r"", "arith/word-parts-sub stderr");
}
