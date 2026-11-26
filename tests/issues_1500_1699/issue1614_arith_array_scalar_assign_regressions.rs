//! rubash#1614 — An arithmetic scalar assignment to an array name
//! (`$((a=7))`) destroyed the whole array: GNU treats an unsubscripted
//! array name in arithmetic as element 0 for writes as well as reads,
//! so the bind lands on element 0 and the remaining elements survive.
//!
//! Expectations verified against GNU bash 5.3.20 (`-c` strings). The
//! associative case asserts membership rather than an exact rendering:
//! GNU prints assoc elements in hash order, rubash in insertion order
//! (a pre-existing rendering-order difference outside this issue).

use std::process::Command;

fn rubash(script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn arith_scalar_assign_to_array_writes_element_zero_only() {
    let (stdout, _, code) = rubash("a=(10 20 30)\necho $((a=7, a[0]))\ndeclare -p a");
    assert_eq!(
        stdout, "7\ndeclare -a a=([0]=\"7\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn arith_compound_and_increment_forms_keep_the_rest_of_the_array() {
    let (stdout, _, _) = rubash("a=(10 20 30)\n((a+=100))\ndeclare -p a");
    assert_eq!(
        stdout, "declare -a a=([0]=\"110\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
    let (stdout, _, _) = rubash("a=(10 20 30)\n((a++))\ndeclare -p a");
    assert_eq!(
        stdout, "declare -a a=([0]=\"11\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
    let (stdout, _, _) = rubash("a=(10 20 30)\nlet a=7\ndeclare -p a");
    assert_eq!(
        stdout, "declare -a a=([0]=\"7\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
    let (stdout, _, _) = rubash("a=(10 20 30)\necho $((a-=4, a[1]))\ndeclare -p a");
    assert_eq!(
        stdout, "20\ndeclare -a a=([0]=\"6\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
}

#[test]
fn arith_scalar_assign_sparse_and_empty_arrays() {
    let (stdout, _, _) = rubash("a=([2]=5)\necho $((a=7))\ndeclare -p a");
    assert_eq!(
        stdout, "7\ndeclare -a a=([0]=\"7\" [2]=\"5\")\n",
        "stdout: {stdout}"
    );
    let (stdout, _, _) = rubash("declare -a a\necho $((a=3))\ndeclare -p a");
    assert_eq!(stdout, "3\ndeclare -a a=([0]=\"3\")\n", "stdout: {stdout}");
}

#[test]
fn arith_scalar_assign_to_assoc_writes_key_zero_only() {
    let (stdout, _, code) = rubash("declare -A m=([x]=1 [y]=2)\necho $((m=5))\ndeclare -p m");
    assert!(stdout.starts_with("5\n"), "stdout: {stdout}");
    for pair in ["[0]=\"5\"", "[x]=\"1\"", "[y]=\"2\""] {
        assert!(stdout.contains(pair), "missing {pair}; stdout: {stdout}");
    }
    assert_eq!(code, Some(0));
}

#[test]
fn arith_scalar_assign_through_nameref_and_for_loop() {
    let (stdout, _, _) = rubash("declare -n r=a\na=(10 20 30)\necho $((r=7))\ndeclare -p a");
    assert_eq!(
        stdout, "7\ndeclare -a a=([0]=\"7\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
    let (stdout, _, _) = rubash("a=(10 20 30)\nfor ((a=0; a<2; a++)); do :; done\ndeclare -p a");
    assert_eq!(
        stdout, "declare -a a=([0]=\"2\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
}

#[test]
fn arith_bare_array_read_still_reads_element_zero() {
    let (stdout, _, _) = rubash("a=(10 20 30)\necho $((a))\ndeclare -p a");
    assert_eq!(
        stdout, "10\ndeclare -a a=([0]=\"10\" [1]=\"20\" [2]=\"30\")\n",
        "stdout: {stdout}"
    );
}
