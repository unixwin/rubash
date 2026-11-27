//! Regression tests for rubash#2826: bounded compiled-form cache at the
//! arithmetic parser boundary.

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn flood_of_one_off_expressions_and_hot_loop_expressions_produce_exact_results() {
    let mut script = String::from("sum=0\n");
    for i in 0..600 {
        script.push_str(&format!("(( sum += (1 + {i}) * 2 - {i} ))\n"));
    }
    script.push_str(
        "acc=0\n\
         for (( i = 0; i < 50; i++ )); do\n\
           (( acc += (i & 1) ? i * 3 + 1 : i * 2 - 1 ))\n\
         done\n\
         echo \"sum=$sum acc=$acc\"\n",
    );

    let (stdout, stderr, code) = rubash(&script);
    assert_eq!(stderr, "", "{stderr:?}");
    assert_eq!(code, Some(0));

    let expected_sum: i64 = (0..600).map(|i| (1 + i) * 2 - i).sum();
    let expected_acc: i64 = (0..50)
        .map(|i| if (i & 1) != 0 { i * 3 + 1 } else { i * 2 - 1 })
        .sum();
    assert_eq!(stdout, format!("sum={expected_sum} acc={expected_acc}\n"));
}

#[test]
fn same_expression_with_different_bindings_and_side_effects() {
    let script = r#"
a=10; b=3
echo $(( (a += b) + (b++) + (a > 12 ? a * 2 : b * 5) ))
echo "a=$a b=$b"

a=1; b=2
echo $(( (a += b) + (b++) + (a > 12 ? a * 2 : b * 5) ))
echo "a=$a b=$b"

# Short-circuiting must suppress side effects in untaken branches.
x=5; y=7
echo $(( 1 || (x++) )) $(( 0 && (y++) )) $(( x > 0 ? ++x : ++y ))
echo "x=$x y=$y"

# Comma operator, bitwise shifts, power, and unary operators.
k=2
echo $(( k = (k << 3) + 1, ~k & 15, 2 ** 5 ))
echo "k=$k"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(stderr, "", "{stderr:?}");
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "\
42
a=13 b=4
20
a=3 b=3
1 0 6
x=6 y=7
32
k=17
"
    );
}

#[test]
fn dynamic_and_eval_style_cases_and_runtime_errors_preserve_semantics() {
    let script = r#"
# Variable containing an arithmetic expression (eval-style recursion).
expr='x + y * 2'
x=3; y=4
echo $(( expr ))
x=10; y=5
echo $(( expr ))

# Division by zero in a compiled expression still reports the exact error.
(( 10 / 0 ))
echo "rc=$?"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert!(
        stderr.contains("division by 0"),
        "expected division by 0 diagnostic, got: {stderr:?}"
    );
    assert_eq!(stdout, "11\n20\nrc=1\n");
}

#[test]
fn gnu53_precedence_dead_branch_wrap_diagnostics_and_nounset_match() {
    let script = r#"
echo "p1=$(( -2**2 )) p2=$(( (-2)**2 )) p3=$(( - 2 ** 2 )) p4=$(( +2**3 )) p5=$(( ~1**2 )) p6=$(( !0**2 ))"
echo "d1=$(( 0 ? 1/0 : 2 )) d2=$(( 1 || (1/0) )) d3=$(( 0 && (1/0) )) d4=$(( 1 ? 2 : 2**-1 )) d5=$(( 0 ? 2**-1 : 3 )) d6=$(( 1 || (2**-1) ))"
echo "w1=$(( 9223372036854775807 + 1 ))"
echo "w2=$(( -9223372036854775807 - 1 - 1 ))"
echo "w3=$(( -(-9223372036854775807 - 1) ))"
echo "w4=$(( (-9223372036854775807 - 1) / -1 ))"
echo "w5=$(( (-9223372036854775807 - 1) % -1 ))"

(( 10 / 0 )); echo "div0=$?"
(( 10 % 0 )); echo "mod0=$?"
(( 2 ** -1 )); echo "negexp=$?"
a=5; (( a /= 0 )); echo "diva=$? a=$a"
a=5; (( a %= 0 )); echo "moda=$? a=$a"

( set -u; echo "u1=$(( 1 || unset_var ))"; echo "u1rc=$?" )
( set -u; echo "u2=$(( 0 ? unset_var : 42 ))"; echo "u2rc=$?" )
( set -u; echo $(( unset_var )); echo "u3rc=$?" )
echo "u3sub=$?"
( set -u; x=1; (( x = unset_var )); echo "u4rc=$?" )
echo "u4sub=$?"
"#;
    let (stdout, stderr, code) = rubash(script);
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "\
p1=4 p2=4 p3=4 p4=8 p5=4 p6=1
d1=2 d2=1 d3=0 d4=2 d5=3 d6=1
w1=-9223372036854775808
w2=9223372036854775807
w3=-9223372036854775808
w4=-9223372036854775808
w5=0
div0=1
mod0=1
negexp=1
diva=1 a=5
moda=1 a=5
u1=1
u1rc=0
u2=42
u2rc=0
u3sub=1
u4sub=1
"
    );
    assert!(
        stderr.contains("10 / 0 : division by 0 (error token is \"0 \")"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("10 % 0 : division by 0 (error token is \"0 \")"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("2 ** -1 : exponent less than 0 (error token is \"1 \")"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("a /= 0 : division by 0 (error token is \"0 \")"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("a %= 0 : division by 0 (error token is \"0 \")"),
        "stderr={stderr:?}"
    );
    assert!(
        stderr.contains("unset_var: unbound variable"),
        "stderr={stderr:?}"
    );
}
