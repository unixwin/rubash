use std::io::Write;
use std::process::{Command, Stdio};
use std::{fs, path::Path};

fn shell_test_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) && value.len() >= 3 && value.as_bytes()[1] == b':' {
        let drive = value.as_bytes()[0] as char;
        format!("/{}{}", drive.to_ascii_lowercase(), &value[2..])
    } else {
        value
    }
}

#[test]
fn bash_execution_string_reflects_c_command() {
    let command = "printf '%s\\n' \"$BASH_EXECUTION_STRING\"";
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(command)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{command}\n")
    );
}

#[test]
fn c_command_uses_command_name_and_positional_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"")
        .arg("arg0")
        .arg("alpha")
        .arg("beta")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "arg0:alpha:2\n");
}

#[test]
fn script_file_uses_script_name_and_positional_arguments() {
    let script_path = Path::new("target").join("rubash-cli-script-args.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"\n").unwrap();
    let script = script_path.to_string_lossy().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .arg("alpha")
        .arg("beta")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{script}:alpha:2\n")
    );
    let _ = fs::remove_file(script_path);
}

#[test]
fn script_file_accepts_shell_style_drive_path() {
    let script_path = Path::new("target").join("rubash-cli-shell-drive-path.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s\\n' \"$0\"\n").unwrap();
    let shell_path = shell_test_path(&std::env::current_dir().unwrap().join(&script_path));
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&shell_path)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{shell_path}\n")
    );
    let _ = fs::remove_file(script_path);
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
fn double_dash_allows_script_file_after_options() {
    let script_path = Path::new("target").join("rubash-cli-double-dash-script.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "printf '%s:%s\\n' \"$0\" \"$1\"\n").unwrap();
    let script = script_path.to_string_lossy().to_string();
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg(&script)
        .arg("alpha")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{script}:alpha\n")
    );
    let _ = fs::remove_file(script_path);
}

#[test]
fn posix_long_option_enables_posix_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--posix")
        .arg("-c")
        .arg("type break")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "break is a special shell builtin\n"
    );
}

#[test]
fn profile_startup_options_are_accepted_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--noprofile")
        .arg("--norc")
        .arg("-c")
        .arg("printf '%s\\n' ok")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[test]
fn login_startup_options_are_accepted_before_command_string() {
    let long_output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--login")
        .arg("-c")
        .arg("printf '%s\\n' long")
        .output()
        .expect("run rubash");
    let short_output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-l")
        .arg("-c")
        .arg("printf '%s\\n' short")
        .output()
        .expect("run rubash");

    assert!(long_output.status.success());
    assert_eq!(String::from_utf8_lossy(&long_output.stdout), "long\n");
    assert!(short_output.status.success());
    assert_eq!(String::from_utf8_lossy(&short_output.stdout), "short\n");
}

#[test]
fn cli_shell_flags_apply_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-u")
        .arg("-c")
        .arg("printf '%s\\n' \"$-\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .contains('u'));
}

#[test]
fn cli_plus_shell_flags_disable_previous_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-u")
        .arg("+u")
        .arg("-c")
        .arg("printf '%s\\n' \"$-\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .contains('u'));
}

#[test]
fn cli_shopt_options_apply_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("nullglob")
        .arg("-c")
        .arg("shopt -q nullglob; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
}

#[test]
fn cli_plus_shopt_options_disable_previous_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("nullglob")
        .arg("+O")
        .arg("nullglob")
        .arg("-c")
        .arg("shopt -q nullglob; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
}

#[test]
fn invalid_cli_shopt_option_fails_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-O")
        .arg("no_such_shopt")
        .arg("-c")
        .arg("echo should-not-run")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid shell option name"));
}

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
fn script_errexit_allows_common_conditional_failures() {
    let script_path = Path::new("target").join("rubash-cli-errexit-common.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "set -e\n\
         if false; then echo bad-if; fi\n\
         false || echo recovered\n\
         ! false\n\
         echo after-invert\n\
         while false; do echo bad-while; done\n\
         echo before-final-failure\n\
         true && false\n\
         echo bad-after\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "recovered\nafter-invert\nbefore-final-failure\n"
    );
}

#[test]
fn script_errexit_exits_on_assignment_command_substitution_failure() {
    let script_path = Path::new("target").join("rubash-cli-errexit-assignment-comsub.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(&script_path, "set -e\nv=$(false)\necho bad\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn script_assignment_command_substitution_success_keeps_running() {
    let script_path = Path::new("target").join("rubash-cli-assignment-comsub-success.sh");
    fs::create_dir_all("target").unwrap();
    fs::write(
        &script_path,
        "set -e\nv=$(printf ok)\nprintf '%s\\n' \"$v\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .output()
        .expect("run rubash");

    let _ = fs::remove_file(script_path);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
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
