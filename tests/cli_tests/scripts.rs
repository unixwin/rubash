use std::io::Write;
use std::process::{Command, Stdio};
use std::{fs, path::Path};

#[path = "errexit.rs"]
mod errexit;

#[test]
fn stdin_script_uses_s_positional_arguments() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .arg("alpha")
        .arg("beta")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "rubash:alpha:2\n");
}

#[test]
fn stdin_script_child_shell_inherits_unread_input() {
    let child_script = Path::new("target/rubash-cli-input-line-child.sh");
    let _ = fs::remove_file(child_script);
    fs::write(child_script, "read line\nprintf 'child:%s\\n' \"$line\"\n").unwrap();

    let shell = env!("CARGO_BIN_EXE_rubash").replace('\\', "/");
    let child_script_arg = child_script.to_string_lossy().replace('\\', "/");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("THIS_SH", &shell)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            format!(
                "echo before\n${{THIS_SH}} {child_script_arg}\nthis line is child input\necho after\n"
            )
            .as_bytes(),
        )
        .unwrap();

    let output = child.wait_with_output().unwrap();

    let _ = fs::remove_file(child_script);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "before\nchild:this line is child input\nafter\n"
    );
}

#[test]
fn stdin_script_runs_multiline_case_command() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"case x in\n  x) echo one ;&\n  y) echo two ;;\n  *) echo star ;;\nesac\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "one\ntwo\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn stdin_script_runs_multiline_function_definition() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"f() {\n  echo body\n}\nf\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "body\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn stdin_script_runs_split_function_definition() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"f()\n{\n  echo split\n}\nf\nfunction g\n{\n  echo keyword\n}\ng\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "split\nkeyword\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_multiline_quoted_assignment_is_one_command() {
    let script_path = Path::new("target").join("rubash-cli-multiline-assignment.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        concat!(
            "usage=\"\\\n",
            "Usage: $0 [OPTION]\n",
            "Options:\n",
            "  -h, --help print help\"\n",
            "printf '%s\\n' \"$usage\"\n",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(&script_path);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Usage: {} [OPTION]\nOptions:\n  -h, --help print help\n",
            script_path.to_string_lossy()
        )
    );
}

#[test]
fn script_nested_case_stays_inside_outer_case_body() {
    let script_path = Path::new("target").join("rubash-cli-nested-case.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "case \"$1\" in\n\
         a)\n\
           case \"$2\" in\n\
             b) echo inner ;;\n\
           esac\n\
           echo outer\n\
           ;;\n\
         *) echo other ;;\n\
         esac\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .arg("a")
        .arg("b")
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "inner\nouter\n");
}

#[test]
fn script_backtick_basename_expands_script_name() {
    let script_path = Path::new("target").join("rubash-cli-backtick-basename.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "prog=`basename $0`\nprintf '%s\\n' \"$prog\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rubash-cli-backtick-basename.sh\n"
    );
}

#[test]
fn script_nested_if_keeps_outer_fi_pairing() {
    let script_path = Path::new("target").join("rubash-cli-nested-if.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "patch_level=\n\
         if [ -z \"$patch_level\" ]; then\n\
           patchlevel_h=target/no-such-patchlevel.h\n\
           if [ -s $patchlevel_h ]; then\n\
             echo bad-inner\n\
           fi\n\
         fi\n\
         if [ -z \"$patch_level\" ]; then\n\
           patch_level=0\n\
         fi\n\
         printf '%s\\n' \"$patch_level\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_backtick_echo_sed_pipeline_splits_version() {
    let script_path = Path::new("target").join("rubash-cli-backtick-sed-pipeline.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "dist_version=5.3\n\
         dist_major=`echo $dist_version | sed 's:\\..*$::'`\n\
         dist_minor=`echo $dist_version | sed 's:^.*\\.::'`\n\
         printf '%s:%s\\n' \"$dist_major\" \"$dist_minor\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5:3\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_escaped_backtick_in_quotes_is_literal() {
    let script_path = Path::new("target").join("rubash-cli-escaped-backtick.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "echo \"   \\`make version.h' to the Makefile.  It is created by mkversion. */\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "   `make version.h' to the Makefile.  It is created by mkversion. */\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_bash_source_pattern_removal_uses_first_element() {
    let script_path = Path::new("target").join("rubash-cli-bash-source-pattern.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s\\n' \"${BASH_SOURCE##*/}\"\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script_path.to_string_lossy().replace('\\', "/"))
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(&script_path);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rubash-cli-bash-source-pattern.sh\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_bash_source_index_locates_sibling_source_file() {
    let script_dir = Path::new("target").join("rubash-cli-script-dir");
    let script_path = script_dir.join("main.sh");
    let lib_path = script_dir.join("lib.sh");
    fs::create_dir_all(&script_dir).unwrap();
    fs::write(&lib_path, "helper() { printf 'lib:%s\\n' \"$1\"; }\n").unwrap();
    fs::write(
        &script_path,
        "SCRIPT_DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"\n\
         source \"$SCRIPT_DIR/lib.sh\"\n\
         helper ok\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script_path.to_string_lossy().replace('\\', "/"))
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(&script_path);
    let _ = fs::remove_file(&lib_path);
    let _ = fs::remove_dir(&script_dir);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "lib:ok\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn script_assignment_command_substitution_captures_function_output() {
    let script_path = Path::new("target").join("rubash-cli-function-comsub.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "greet() { echo \"hello $1\"; }\nvalue=$(greet world)\nprintf '%s\\n' \"$value\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello world\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}
