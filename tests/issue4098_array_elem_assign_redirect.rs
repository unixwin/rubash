use std::{env, process::Command};

// rubash#4098: array element assignment statements (`a[0]=1 < /nonexistent`,
// `a[0]=1 > out`) and mixed assignment statements (`x=1 a[0]=2 > out`,
// `a[0]=3 b=4 > out`) must apply their redirections via the null-command
// path after binding the assignments, matching GNU `execute_cmd.c:4628-4642`
// (`execute_simple_command` -> `execute_null_command` at `execute_cmd.c:4203`).

#[test]
fn array_element_assignment_applies_redirections() {
    let dir = env::temp_dir().join(format!(
        "rubash_issue4098_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let script = r#"
a[0]=1 < /nonexistent3994
echo "1:rc=$? a0=${a[0]}"

a[0]=1 > out20
echo "2:rc=$? a0=${a[0]} exists=$(test -e out20 && echo yes || echo no)"

x=0
x=1 a[0]=2 > out21
echo "3:rc=$? x=$x a0=${a[0]} exists=$(test -e out21 && echo yes || echo no)"

a[0]=3 b=4 > out22
echo "4:rc=$? a0=${a[0]} b=$b exists=$(test -e out22 && echo yes || echo no)"

a[0]=$(false) > out23
echo "5:rc=$? exists=$(test -e out23 && echo yes || echo no)"

a[0]=$(false) </dev/null
echo "6:rc=$?"

a[0]=newname > ${a[0]}.txt
echo "7:rc=$? exists=$(test -e newname.txt && echo yes || echo no)"

a[0]=1 {v}>out24
echo "8:rc=$? v=$v exists=$(test -e out24 && echo yes || echo no)"

(
  a[]=1 > out25
  echo "after_bad_sub rc=$?"
)
echo "9:sub_rc=$? exists=$(test -e out25 && echo yes || echo no)"

(
  readonly ro_arr=(x)
  ro_arr[0]=1 > out26
  echo "after_ro rc=$?"
)
echo "10:sub_rc=$? exists=$(test -e out26 && echo yes || echo no)"

(
  set -e
  trap 'echo ERR' ERR
  a[0]=1 < /nonexistent3994
  echo "after_errexit"
)
echo "11:errexit_rc=$?"
"#;

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .current_dir(&dir)
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash -c");

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        output.status.success(),
        "rubash failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let stderr = String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n");

    let expected_stdout = "\
1:rc=1 a0=1
2:rc=0 a0=1 exists=yes
3:rc=0 x=1 a0=2 exists=yes
4:rc=0 a0=3 b=4 exists=yes
5:rc=1 exists=yes
6:rc=0
7:rc=0 exists=yes
8:rc=0 v= exists=yes
9:sub_rc=1 exists=no
10:sub_rc=1 exists=no
ERR
11:errexit_rc=1
";
    assert_eq!(stdout, expected_stdout);
    assert!(
        stderr.contains("/nonexistent3994: No such file or directory"),
        "expected nonexistent file diagnostic in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("bad array subscript"),
        "expected bad array subscript diagnostic in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("readonly variable"),
        "expected readonly variable diagnostic in stderr, got: {stderr}"
    );
}
