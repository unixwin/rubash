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
