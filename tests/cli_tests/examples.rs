use super::shell_test_path;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{fs, path::Path};

#[test]
fn gnu_zprintf_usage_guard_exits_before_body() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("third_party/bash/examples/scripts/zprintf")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "zprintf: usage: zprintf format [args ...]\n"
    );
}

#[test]
fn gnu_dirstack_function_definitions_parse_comments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("third_party/bash/examples/functions/dirstack")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_aliasconv_example_converts_aliases() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("aliasconv.bash")
        .current_dir(Path::new("bash").join("examples").join("misc"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"ll\tls -l\nhi\techo hello\nstar\techo !*\narg\techo !:2 #tag\n")
        .unwrap();
    let output = child.wait_with_output().expect("wait for rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "alias ll='ls -l'\nalias hi='echo hello'\nstar () { command echo \"$@\" ; }\narg () { command echo \"$2\" #tag ; }\n"
    );
}

#[test]
fn script_arrayops_example_manipulates_arrays() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .current_dir(Path::new("bash").join("examples").join("functions"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"source arrayops.bash\n\
              arr=(a b)\n\
              apush arr c d\n\
              alen arr\n\
              aref arr 0 2 3\n\
              apop arr 2\n\
              declare -p arr\n\
              alen arr\n\
              aref arr 0 1\n",
        )
        .unwrap();
    let output = child.wait_with_output().expect("wait for rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "4\na\nc\nd\ndeclare -a arr=([0]=\"a\" [1]=\"b\")\n2\na\nb\n"
    );
}

#[test]
fn script_array_stuff_example_runs_array_workflows() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("array-stuff")
        .current_dir(Path::new("bash").join("examples").join("functions"))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1 2 3 4 5 6 7\n\
         7 6 5 4 3 2 1\n\
         1 2 3 4 5 6 7\n\
         1 2 3 4 5 6\n\
         3 4 5 6\n\
         4 5 6\n\
         \n\
         1 1 2 2 3 3 4 5 5 6 9\n\
         1 2 3 4 5 6 9\n"
    );
}

#[test]
fn script_array_to_string_example_joins_arrays() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .current_dir(Path::new("bash").join("examples").join("functions"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"source ./array-to-string\n\
              a=(x y z)\n\
              array_to_string a s ,\n\
              printf '<%s>\\n' \"$s\"\n\
              array_to_string a s\n\
              printf '<%s>\\n' \"$s\"\n",
        )
        .unwrap();
    let output = child.wait_with_output().expect("wait for rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "<x,y,z>\n<x y z>\n"
    );
}

#[test]
fn script_center_example_reads_file_in_nested_loop() {
    let input_path = Path::new("target").join("rubash-center-input.txt");
    fs::create_dir_all("target").unwrap();
    fs::write(&input_path, "alpha\nbeta\n").unwrap();
    let script_input = shell_test_path(&std::env::current_dir().unwrap().join(&input_path));
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("center")
        .arg(&script_input)
        .current_dir(Path::new("bash").join("examples").join("scripts"))
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(input_path);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "                                     alpha\n                                      beta\n"
    );
}

#[test]
fn script_sort_pos_params_example_handles_quoted_positional_args() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("sort-pos-params")
        .current_dir(Path::new("bash").join("examples").join("functions"))
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1\n1\n2\n2\n3\n3\n4\n5\n5\n6\n9\n1 1 2 2 3 3 4 5 5 6 9\n11\na b a c x z\n3\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("sort_posparams: argument expected"));
}

#[test]
fn script_kshenv_example_parses_multiline_awk_quote() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("kshenv")
        .current_dir(Path::new("bash").join("examples").join("functions"))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_xterm_title_example_reports_script_name_without_display() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("xterm_title")
        .current_dir(Path::new("bash").join("examples").join("scripts"))
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "xterm_title: not running X\n"
    );
}
