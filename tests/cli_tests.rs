use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use std::{fs, path::Path};

use regex::Regex;

#[path = "cli_tests/bashdb_compat.rs"]
mod bashdb_compat;
#[path = "cli_tests/compat_issue_regressions.rs"]
mod compat_issue_regressions;
#[path = "cli_tests/compat_issue78_multiline_arrays.rs"]
mod compat_issue78_multiline_arrays;
#[path = "cli_tests/declare_output.rs"]
mod declare_output;
#[path = "cli_tests/examples.rs"]
mod examples;
#[path = "cli_tests/fd_redirects.rs"]
mod fd_redirects;
#[path = "cli_tests/process_substitution.rs"]
mod process_substitution;
#[path = "cli_tests/scripts.rs"]
mod scripts;

fn shell_test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn assert_stderr_matches(stderr: &str, pattern: &str) {
    let regex = Regex::new(pattern).expect("valid regex");
    assert!(
        regex.is_match(stderr),
        "stderr {stderr:?} did not match {pattern:?}"
    );
}

#[test]
fn c_command_reads_named_coproc_stdout_through_array_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("coproc MY { printf 'coproc-ok\\n'; }; read -r out <&\"${MY[0]}\"; echo \"$out\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "coproc-ok\n");
}

#[test]
fn c_command_reads_coproc_raw_bytes_without_utf8_loss() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("coproc C { printf '\\377\n'; }; read -r value <&${C[0]}; printf '%s' \"$value\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(output.stdout, [0xff]);
    assert_eq!(output.stderr, b"");
}

#[test]
fn scalar_command_substitution_assignment_preserves_c0_and_raw_bytes() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"x=$(printf '\377'); y=$(printf '\035'); printf '%s%s' "$x" "$y""#)
        .output()
        .expect("run scalar assignment raw-byte probe");

    assert!(output.status.success());
    assert_eq!(output.stdout, [0xff, 0x1d]);
    assert_eq!(output.stderr, b"");
}

#[test]
fn backtick_assignment_preserves_c0_bytes() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"value=`printf '\035'`; printf '%s' "$value""#)
        .output()
        .expect("run backtick assignment raw-byte probe");

    assert!(output.status.success());
    assert_eq!(output.stdout, [0x1d]);
    assert_eq!(output.stderr, b"");
}

#[test]
fn scalar_assignment_preserves_bytes_across_local_subshell_and_export() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"x=$(printf '\377'); y=$(printf '\035'); foo() { local x=$(printf '\377'); printf '%s' "$x"; }; foo; (x=$(printf '\377'); printf '%s' "$x"); x=$(printf '\377'); export x; env | grep '^x='"#)
        .output()
        .expect("run scalar assignment scope probe");

    assert!(output.status.success());
    assert_eq!(output.stdout, [0xff, 0xff, b'x', b'=', 0xff, b'\n']);
    assert_eq!(output.stderr, b"");
}

#[test]
fn c_command_keeps_unread_coproc_records_for_later_reads() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { printf 'one\\ntwo\\n'; }; \
             read -r first <&${C[0]}; read -r second <&${C[0]}; \
             printf '%s:%s\\n' \"$first\" \"$second\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "one:two\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_keeps_named_coproc_output_fds_distinct() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc FIRST { printf 'first\\n'; }; coproc SECOND { printf 'second\\n'; }; \
             read -r first <&\"${FIRST[0]}\"; read -r second <&\"${SECOND[0]}\"; \
             printf '%s:%s\\n' \"$first\" \"$second\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "first:second\n");
}

#[test]
fn c_command_exposes_distinct_virtual_fds_for_named_coproc() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { sleep 1; }; printf '%s %s\\n' \"${C[0]}\" \"${C[1]}\"; \
             wait \"$C_PID\"",
        )
        .output()
        .expect("run coproc virtual fd probe");

    assert!(output.status.success());
    let values = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(|value| value.parse::<u32>().expect("coproc fd is numeric"))
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_ne!(values[0], values[1]);
    assert!(values.iter().all(|value| *value < 1024));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_reuses_closed_dynamic_fds_and_resolves_nameref_targets() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "exec {first}<&0; exec {second}>&1; printf 'first:%s %s\\n' \"$first\" \"$second\"; \
             exec {first}<&-; exec {second}>&-; \
             declare -n input_fd=input; declare -n output_fd=output; \
             exec {input_fd}<&0; exec {output_fd}>&1; \
             printf 'second:%s %s %s %s\\n' \"$input\" \"$output\" \"$input_fd\" \"$output_fd\"",
        )
        .output()
        .expect("run dynamic fd reuse and nameref probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "first:10 11\nsecond:10 11 10 11\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_dynamic_varredir_covers_read_write_dup_and_auto_close() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            ": {fd}<>/dev/null; printf 'rw=%s fd=%s\\n' \"$?\" \"$fd\"; \
             : {dup}>&1; printf 'dup=%s fd=%s\\n' \"$?\" \"$dup\"; \
             shopt -s varredir_close; : {auto}>&1; \
             printf 'auto=%s fd=%s\\n' \"$?\" \"$auto\"; \
             printf 'after-auto=%s\\n' \"$?\"",
        )
        .output()
        .expect("run dynamic varredir probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rw=0 fd=10\ndup=0 fd=11\nauto=0 fd=12\nafter-auto=0\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_rejects_unbalanced_arithmetic_command_as_parse_error() {
    // GNU bash 5.3 rejects `bash -c '"'"'((X=([))]'"'"'' with a parse error and
    // status 2 ("syntax error near unexpected token `('"); rubash parses the
    // same input through the GNU parse_dparen lexical rule and rejects it as
    // a syntax error from the subshell path.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("((X=([))]")
        .output()
        .expect("run malformed arithmetic command");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
}

#[test]
fn c_command_err_trap_preserves_failed_bash_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("trap 'echo err:$BASH_COMMAND' ERR; false; echo after")
        .output()
        .expect("run ERR trap command probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "err:false\nafter\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_failed_dynamic_varredir_continues_and_does_not_set_variable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "unset fd; : {fd}<>target/rubash-varredir-missing-parent/file; \
             open_status=$?; printf 'open=%s fd=%s\\n' \"$open_status\" \"${fd-unset}\"; \
             printf 'continued\\n'",
        )
        .output()
        .expect("run failed dynamic varredir probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "open=1 fd=unset\ncontinued\n"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn c_command_closes_read_write_dynamic_fd_before_reusing_slot_after_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            ": {fd}<>/dev/null; exec {fd}>&-; \
             : {fd}<>target/rubash-varredir-missing-parent/file; \
             failed=$?; : {next}>&1; \
             printf 'failed=%s fd=%s next=%s\\n' \"$failed\" \"$fd\" \"$next\"",
        )
        .output()
        .expect("run dynamic read-write fd close/reuse probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "failed=1 fd=10 next=10\n"
    );
}

#[test]
fn c_command_writes_to_named_coproc_stdin_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc WORKER { read -r value; printf 'got:%s\\n' \"$value\"; }; \
             printf 'hello\\n' >&\"${WORKER[1]}\"; \
             read -r result <&\"${WORKER[0]}\"; echo \"$result\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "got:hello\n");
}

#[test]
fn c_command_closing_named_coproc_stdin_fd_produces_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { while read -r value; do printf 'got:%s\\n' \"$value\"; done; }; \
             printf 'hello\\n' >&\"${C[1]}\"; read -r value <&\"${C[0]}\"; \
             exec {C[1]}>&-; wait \"$C_PID\"; printf 'read:%s\\n' \"$value\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash coproc close probe");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child
            .try_wait()
            .expect("poll rubash coproc close probe")
            .is_some()
        {
            let output = child
                .wait_with_output()
                .expect("collect rubash coproc close probe");
            assert!(
                output.status.success(),
                "stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8_lossy(&output.stdout), "read:got:hello\n");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let output = child.wait_with_output().expect("collect timed out probe");
    panic!(
        "rubash coproc stdin close did not produce EOF within 5 seconds; stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn c_command_retires_finished_coproc_endpoints_before_later_redirects() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg("-c")
        .arg(
            "coproc C { :; }; sleep 1; \
             printf 'c0=<%s> pid=<%s>\\n' \"${C[0]-unset}\" \"${C_PID-unset}\"; \
             exec 4<&${C[0]}-; read value <&4; \
             printf 'status=%s value=<%s>\\n' \"$?\" \"$value\"",
        )
        .output()
        .expect("run finished coproc endpoint probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "c0=<unset> pid=<unset>\nstatus=1 value=<>\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: 4: Bad file descriptor\n"
    );
}

#[test]
fn c_command_materializes_persistent_stderr_to_stdout_for_external_children() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg("-c")
        .arg(
            "exec 2>&1; /usr/bin/cat /definitely-missing-rubash-fd2-path; \
             printf 'status=%s\\n' \"$?\"",
        )
        .output()
        .expect("run persistent fd2 external child probe");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/definitely-missing-rubash-fd2-path"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("status=1"));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_pipeline_stages_inherit_persistent_stderr_to_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg("-c")
        .arg(
            "exec 2>&1; cat /definitely-missing-rubash-pipeline-fd2-path | cat; \
             printf 'status=%s\\n' \"$?\"",
        )
        .output()
        .expect("run persistent fd2 pipeline probe");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("/definitely-missing-rubash-pipeline-fd2-path"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("status=0"));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_coproc_child_inherits_persistent_stderr_to_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg("-c")
        .arg(
            "exec 2>&1; coproc xcase -n -u; wait \"$COPROC_PID\"; \
             printf 'done=%s\\n' \"$?\"",
        )
        .output()
        .expect("run persistent fd2 coproc child probe");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("xcase: command not found"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("done=127"));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_echo_reports_persistent_closed_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("--")
        .arg("-c")
        .arg(
            "coproc C { :; }; sleep 1; exec 2>&1; exec >&${C[1]}-; \
             echo ${C[@]}",
        )
        .output()
        .expect("run persistent closed stdout probe");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rubash: echo: write error: Bad file descriptor\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_external_cat_receives_coproc_data_before_writer_close() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { cat; }; printf 'hello\\n' >&\"${C[1]}\"; \
             read -r value <&\"${C[0]}\"; exec {C[1]}>&-; \
             wait \"$C_PID\"; printf 'read:%s\\n' \"$value\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn external coproc cat probe");

    if !wait_for_child_exit(&mut child, Duration::from_secs(5)) {
        let output = child
            .wait_with_output()
            .expect("collect timed-out external coproc cat probe");
        panic!(
            "external coproc cat did not finish; stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = child
        .wait_with_output()
        .expect("collect external coproc cat probe");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "read:hello\n");
}

#[test]
fn c_command_external_cat_dash_receives_coproc_data_before_writer_close() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { cat -; }; printf 'hello\\n' >&\"${C[1]}\"; \
             read -r value <&\"${C[0]}\"; exec {C[1]}>&-; \
             wait \"$C_PID\"; printf 'read:%s\\n' \"$value\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn external cat dash coproc probe");

    if !wait_for_child_exit(&mut child, Duration::from_secs(5)) {
        let output = child
            .wait_with_output()
            .expect("collect timed-out external cat dash coproc probe");
        panic!(
            "external cat dash coproc did not finish; stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = child
        .wait_with_output()
        .expect("collect external cat dash coproc probe");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "read:hello\n");
}

#[test]
fn c_command_starts_cat_dash_coproc_after_waiting_for_previous_coproc() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc FIRST { echo a b c; sleep 2; }; read line <&${FIRST[0]}; \
             wait $FIRST_PID; coproc REFLECT { cat -; }; \
             echo flop >&${REFLECT[1]}; read line <&${REFLECT[0]}; \
             { sleep 1; kill $REFLECT_PID; } & wait $REFLECT_PID >/dev/null 2>&1; \
             printf '%s\\n' \"$line\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sequential coproc probe");

    if !wait_for_child_exit(&mut child, Duration::from_secs(8)) {
        let output = child
            .wait_with_output()
            .expect("collect timed-out sequential coproc probe");
        panic!(
            "sequential coproc probe did not finish; stdout={:?} stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = child
        .wait_with_output()
        .expect("collect sequential coproc probe");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "flop\n");
}

#[test]
fn c_command_moves_coproc_reader_to_numbered_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { printf 'moved\\n'; }; exec 4<&\"${C[0]}\"-; \
             read -r value <&4; printf 'read:%s\\n' \"$value\"",
        )
        .output()
        .expect("run coproc reader move probe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "read:moved\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_moves_coproc_writer_to_numbered_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { read -r value; printf 'got:%s\\n' \"$value\"; }; \
             exec 4>&\"${C[1]}\"-; printf 'payload\\n' >&4; exec 4>&-; \
             read -r value <&\"${C[0]}\"; printf '%s\\n' \"$value\"",
        )
        .output()
        .expect("run coproc writer move probe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "got:payload\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_external_reads_named_coproc_stdout_through_array_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc PRODUCER { printf 'external-read\\n'; }; \
             cat <&\"${PRODUCER[0]}\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "external-read\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_external_writes_named_coproc_stdin_through_array_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc CONSUMER { read -r value; printf 'external-write:%s\\n' \"$value\"; }; \
             printf 'payload\\n' >&\"${CONSUMER[1]}\"; \
             read -r result <&\"${CONSUMER[0]}\"; printf '%s\\n' \"$result\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "external-write:payload\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_closing_coproc_stdin_fd_unblocks_reader() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { cat; }; fd=${C[1]}; printf 'hi\\n' >&$fd; \
             eval \"exec ${fd}>&-\"; read -r x <&${C[0]}; \
             printf 'x=%s status=%s\\n' \"$x\" \"$?\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    assert!(wait_for_child_exit(&mut child, Duration::from_secs(3)));
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x=hi status=0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_duplicates_coproc_stdin_fd_for_writes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "coproc C { cat; }; exec 7>&${C[1]}; printf 'hi\\n' >&7; \
             eval \"exec ${C[1]}>&-\"; exec 7>&-; read -r x <&${C[0]}; \
             printf 'x=%s status=%s\\n' \"$x\" \"$?\"",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    assert!(wait_for_child_exit(&mut child, Duration::from_secs(3)));
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x=hi status=0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn shopt_print_pipeline_is_captured_before_external_stage() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("shopt -p -o | head -n 3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 3);
}

#[test]
fn option_and_stack_builtin_pipelines_are_captured() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("umask -S | head -n 1; kill -l | head -n 1; dirs -p | head -n 1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 3);
}

#[test]
fn times_limits_and_enable_pipeline_output_is_captured() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("times | head -n 1; ulimit -a | head -n 1; enable -a | head -n 1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 3);
}

#[test]
fn declare_pipeline_output_is_captured() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("declare -p | head -n 1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 1);
}

#[test]
fn prefix_assignments_reach_env_builtin_pipeline() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("x=1 y=2 env | grep '^x='; x=1 y=2 env | grep '^y='")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("x=1"));
    assert!(stdout.contains("y=2"));
}

#[test]
fn pipeline_pipefail_triggers_errexit_on_failure() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -e -o pipefail; false | true; echo after")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn nonfinal_brace_pipeline_stage_keeps_errexit_inside_group() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -e; { false; echo group; } | cat; echo end")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "end\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn nonfinal_function_pipeline_stage_keeps_errexit_inside_body() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -e; f() { false; echo function; }; f | cat; echo end")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "end\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn final_brace_pipeline_stage_triggers_errexit() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -e; true | { false; echo group; }; echo end")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn pipeline_statuses_match_bash_for_pipefail() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -o pipefail; false | true; printf 'rc=%s ps=%s len=%s\\n' \"$?\" \"${PIPESTATUS[*]}\" \"${#PIPESTATUS[@]}\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "rc=1 ps=1 0 len=2\n"
    );
}

#[test]
fn stderr_pipeline_preserves_output_and_statuses() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf 'pipe-stderr\\n' |& cat; printf 'ps=%s len=%s\\n' \"${PIPESTATUS[*]}\" \"${#PIPESTATUS[@]}\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "pipe-stderr\nps=0 0 len=2\n"
    );
}

#[test]
fn heredoc_removes_unquoted_backslash_newline_like_bash() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat <<EOF\none\\\ntwo\nEOF")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "onetwo\n");
}

#[test]
fn unquoted_at_preserves_empty_positional_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=,:; set -- a \"\" b; printf '<%s>\\n' $@")
        .output()
        .expect("run empty positional field probe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<a>\n<>\n<b>\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn unquoted_at_preserves_ifs_boundary_fields() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=,:; set -- :a: \"\" b; printf '<%s>\\n' $@")
        .output()
        .expect("run positional IFS boundary probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "<>\n<a>\n<>\n<>\n<b>\n",
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn parameter_replacement_matches_escaped_slash_pattern() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("x=a/b/c; printf '<%s>\\n' \"${x//\\//-}\"")
        .output()
        .expect("run escaped slash replacement probe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<a-b-c>\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn large_heredoc_preserves_all_lines() {
    let body = "payload\n".repeat(20_000);
    let script = format!("mapfile -t lines <<'EOF'\n{body}EOF\nprintf '%s\n' \"${{#lines[@]}}\"");
    let script_path = "target/rubash-large-heredoc-script.sh";
    fs::write(script_path, script).expect("write large heredoc probe");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script_path)
        .output()
        .expect("run large heredoc probe");
    let _ = fs::remove_file(script_path);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "20000\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn large_pipeline_heredoc_does_not_block_before_downstream_spawn() {
    let body = "pipeline-payload\n".repeat(100_000);
    let script = format!("cat <<EOF | wc -l\n{body}EOF\n");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run large heredoc pipeline probe");
    child
        .stdin
        .as_mut()
        .expect("rubash stdin")
        .write_all(script.as_bytes())
        .expect("write large heredoc pipeline");
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .expect("wait for large heredoc pipeline");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "100000");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn quoted_heredoc_preserves_backslash_newline() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat <<'EOF'\none\\\ntwo\nEOF")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "one\\\ntwo\n");
}

#[cfg(windows)]
#[test]
fn quoted_environment_paths_keep_native_windows_form() {
    let expected = std::env::var("LOCALAPPDATA").expect("LOCALAPPDATA is set");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf '%s\\n' \"${LOCALAPPDATA}\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{expected}\n")
    );
}

#[test]
fn nested_parameter_expansion_can_supply_pattern_removal() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "v=a\n echo \"hash=${v#?}\"\n echo \"pct=${v%\"${v#?}\"}\"\n \
             v=ab\n echo \"hash2=${v#?}\"\n echo \"pct2=${v%\"${v#?}\"}\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "hash=\npct=a\nhash2=b\npct2=a\n"
    );
}

#[test]
fn arithmetic_errors_in_assignment_abort_the_script() {
    for (prefix, assignment) in [
        ("", "x=$((1.5))"),
        ("", "x=$(( '1' ))"),
        ("set -u;", "x=$((missing))"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(format!("{prefix}{assignment}; echo should-not-run"))
            .output()
            .expect("run rubash");

        let expected_status = if prefix == "set -u;" { 127 } else { 1 };
        assert_eq!(
            output.status.code(),
            Some(expected_status),
            "assignment: {prefix}{assignment}"
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "");
        assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
    }
}

#[test]
fn quoted_ps0_assignment_preserves_prompt_arithmetic_literal() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            r#"starship_preexec_ps0() { echo 123; }
PS0='${STARSHIP_START_TIME:$((STARSHIP_START_TIME="$(starship_preexec_ps0)",STARSHIP_PREEXEC_READY=0,0)):0}'"${PS0-}"
printf '<%s>\n' "$PS0""#,
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "<${STARSHIP_START_TIME:$((STARSHIP_START_TIME=\"$(starship_preexec_ps0)\",STARSHIP_PREEXEC_READY=0,0)):0}>\n"
    );
}

#[test]
fn command_substitution_preserves_quoted_array_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"args=(a b c); out=$(printf '<%s>' "${args[@]}"); echo "$out""#)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<a><b><c>\n");
}

#[cfg(windows)]
#[test]
fn env_assignment_runs_command_in_materialized_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("env FOO=bar cmd.exe /c set FOO")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "FOO=bar\r\n");
}

#[cfg(windows)]
#[test]
fn env_short_option_cluster_runs_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("env -vuFOO FOO=bar cmd.exe /c set FOO")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "FOO=bar\r\n");
}

#[test]
fn env_null_prints_materialized_environment_without_newlines() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("env -i0 A=1 B=two")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"A=1\0B=two\0");
}

#[cfg(windows)]
#[test]
fn env_command_supplies_windows_profile_vars_from_home() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("USERPROFILE")
        .env_remove("HOMEDRIVE")
        .env_remove("HOMEPATH")
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA")
        .env("HOME", "C:/rubash-home-test")
        .arg("-c")
        .arg("env cmd.exe /c set USERPROFILE")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "USERPROFILE=C:\\rubash-home-test\r\n"
    );
}

#[test]
fn env_file_assignments_are_printed_when_no_command_is_given() {
    let env_file = std::env::temp_dir().join(format!("rubash-env-file-{}.env", std::process::id()));
    fs::write(&env_file, "A=1\n# ignored\nB=two\n").expect("write env file");
    let script = format!("env -f {}", shell_test_path(&env_file));

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("A=1\n"), "{stdout}");
    assert!(stdout.contains("B=two\n"), "{stdout}");
    let _ = fs::remove_file(env_file);
}

// GNU Bash 5.2.37 (2026-08-24): $((1/0)) as a command word ends the
// noninteractive run with status 1 before any later command.
#[test]
fn arithmetic_word_division_error_exits_noninteractive_shell() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $((1/0)); echo after")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("division by 0"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn arithmetic_commands_reject_single_quoted_operands() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("(( '1' )); echo after")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "after\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("operand expected"));
}

#[test]
fn escaped_quote_array_subscript_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("a[\\\" \\\"]=15; echo after")
        .output()
        .expect("run escaped array-subscript probe");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("operand expected"));
}

#[test]
fn function_call_stack_reports_multiline_source_and_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "t2() { printf '%s|%s|%s\\n' \"${FUNCNAME[*]}\" \"${BASH_SOURCE[*]}\" \"${BASH_LINENO[*]}\"; }\nt2\n",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "t2|environment|2\n"
    );
}

#[test]
fn function_call_stack_omits_internal_main_frame() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "outer(){ inner; }; inner(){ printf '%s|%s|%s\\n' \"${FUNCNAME[*]}\" \"${BASH_SOURCE[*]}\" \"${BASH_LINENO[*]}\"; }; outer",
        )
        .output()
        .expect("run nested function stack probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "inner outer|environment environment|1 1\n"
    );
}

#[test]
fn function_call_stack_includes_main_for_script_files() {
    let script_path = Path::new("target/rubash-funcname-main-script.sh");
    fs::write(
        script_path,
        "show() { printf '%s\\n' \"${FUNCNAME[*]}\"; }; show\n",
    )
    .expect("write function stack script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(script_path)
        .output()
        .expect("run function stack script");
    let _ = fs::remove_file(script_path);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "show main\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn umask_symbolic_mode_prints_after_setting_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("umask -S 0002")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "u=rwx,g=rwx,o=rx\n"
    );
}

#[test]
fn malformed_pipeline_and_if_are_syntax_errors() {
    for command in [
        "echo hi |",
        "echo hi &&",
        "echo hi & && echo x",
        "if then; fi; echo after",
        "<<EOF; then <W",
        "while; do :; done",
        "for (( i=0; i<1; i++ ); do :; done",
        "case x in x) ;;",
        "case x in ) echo x;; esac",
        "case x in |) echo x;; esac",
        "( echo hi",
        "{ echo hi;",
        "echo @(",
        "echo !(",
        "echo +(",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {command}"
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
    }
}

#[test]
fn brace_groups_require_a_command_terminator_before_close() {
    for command in ["{ : }", "{ }"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "command: {command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
    }

    let valid = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("{ :; }")
        .output()
        .expect("run rubash");
    assert!(valid.status.success());
}

#[test]
fn brace_groups_allow_completed_compound_commands_before_close() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("(exit 2); { { true; } }; echo Zero:$?; (exit 2); {(true)}; echo Zero:$?; (exit 2); { true | { true; } }; echo Zero:$?; (exit 2); { while false; do :; done }; echo Zero:$?; (exit 2); { case a in b) ;; esac }; echo Zero:$?")
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "Zero:0\nZero:0\nZero:0\nZero:0\nZero:0\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn malformed_case_reserved_word_boundaries_are_syntax_errors() {
    for command in [
        "case x in esac) echo done; esac",
        "case in do do) echo in; esac",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "command: {command}");
        assert!(output.stdout.is_empty(), "command: {command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
    }
}

#[test]
fn sequential_compact_case_subshells_preserve_closing_esac() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("(case m in (m) echo first;; esac)\n(case m in (m) echo second;; esac)\ncase w in `echo case-stderr >&2`) echo skip;; `echo`w) echo redirected;; esac")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "first\nsecond\nredirected\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "case-stderr\n");
}

#[test]
fn loop_numbered_heredoc_expands_variables_in_its_receiver_context() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "value=expanded; while IFS= read -r line <&3; do printf '%s\\n' \"$line\"; done 3<<EOF\n$value\nEOF",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "expanded\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn pipeline_missing_external_command_reports_command_not_found() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf 'input\\n' | rubash_missing_pipeline_command")
        .output()
        .expect("run rubash");

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("rubash_missing_pipeline_command: command not found"));
}

#[test]
fn unterminated_complete_command_strings_are_syntax_errors() {
    for command in [
        "echo $(echo hi",
        "echo \"hi",
        "echo `echo hi",
        "echo $((1+2)",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "command: {command}");
        assert!(output.stdout.is_empty(), "command: {command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
    }
}

#[test]
fn newline_for_header_inside_case_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("case x in x)\nfor x\nin x\ndo echo bad; done\nesac")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("syntax error near unexpected token `do'")
    );
}

#[test]
fn malformed_parameter_expansion_in_arithmetic_for_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("for (( ${ case x in x) esac; };; )); do break; done; echo after")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
}

#[test]
fn stdin_script_stops_after_arithmetic_for_syntax_error_without_errexit() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(b"for (( ${ case x in x) esac; };; )); do break; done\necho after\n")
        .expect("write script");
    let output = child.wait_with_output().expect("wait for rubash");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("syntax error"));
}

#[test]
fn invalid_for_and_select_names_fail_at_execution_like_bash() {
    for command in [
        "for invalid-name in a b; do echo bad; done",
        "for 1 in a b; do echo bad; done",
        "select invalid-name in a b; do echo bad; done",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(1), "command: {command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("not a valid identifier"));
        assert!(!String::from_utf8_lossy(&output.stdout).contains("bad"));
    }
}

#[test]
fn invalid_for_name_is_fatal_in_posix_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -o posix; for invalid-name in a b; do echo bad; done; echo after")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a valid identifier"));
}

#[test]
fn malformed_conditional_operator_is_a_syntax_error() {
    for command in ["[[ -n & ]]", "[[ 4 & ]]", "[[ -n < ]]"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "command: {command}");
    }
}

#[test]
fn unterminated_conditional_is_a_syntax_error() {
    for command in ["[[ -n foo", "[[ ( -t X ) ]", "[[ -n foo ]"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(2), "command: {command}");
    }
}

#[test]
fn unterminated_command_substitution_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $(printf foo")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn command_substitution_still_executes_when_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo \"$(printf '%s' ok)\"")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok\n");
}

#[test]
fn malformed_compound_inside_command_substitution_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $( if x; then echo foo )")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn arithmetic_for_errors_preserve_failure_status() {
    for (command, expected_stdout) in [
        (
            "for (( 7=1; i<4; i++ )); do echo body; done; echo status:$?",
            "status:1\n",
        ),
        (
            "for (( i=1; 7++; i++ )); do echo body; done; echo status:$?",
            "status:1\n",
        ),
        (
            "for (( i=1; i<4; 7++ )); do echo body; done; echo status:$?",
            "body\nstatus:1\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert!(
            output.status.success(),
            "script should reach status echo: {command}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected_stdout,
            "command: {command}"
        );
        assert!(
            !output.stderr.is_empty(),
            "missing arithmetic diagnostic: {command}"
        );
    }
}

#[test]
fn arithmetic_builtin_errors_report_their_owner() {
    for (command, marker) in [
        ("let", "let: expression expected"),
        ("let '4 +'", "let: 4 +: syntax error: operand expected"),
        (
            "let '7=4'",
            "let: 7=4: attempted assignment to non-variable",
        ),
        ("[[ 1/0 -eq 1 ]]", "[[: 1/0: division by 0"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(command)
            .output()
            .expect("run rubash");
        assert_eq!(output.status.code(), Some(1), "command: {command}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(marker),
            "command: {command}, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn arithmetic_expansion_preserves_non_lvalue_increment_operand() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $((--7)); echo $((++ 7)); (( -- ))")
        .output()
        .expect("run rubash");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7\n7\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("((: -- : syntax error: operand expected (error token is \"- \")"));
}

#[test]
fn empty_arithmetic_contexts_match_bash_zero_semantics() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo A:$(( )); echo B:$(( \"\" )); (( )); echo command:$?; [[ 0 -eq \"\" ]]; echo cond:$?")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "A:0\nB:0\ncommand:1\ncond:0\n"
    );
}

#[test]
fn arithmetic_array_subscript_quote_removal_targets_index_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("declare -a a; a[0]=0; (( a[\" \"]=11 )); declare -p a")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a a=([0]=\"11\")\n"
    );
}

#[test]
fn array_assignment_quoted_empty_arithmetic_subscripts_use_index_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"declare -a a; a[" " ]=10; a[""]=23; declare -p a"#)
        .output()
        .expect("run quoted empty array subscript probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a a=([0]=\"23\")\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn declare_quoted_array_element_assignment_targets_index_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"declare -a a; declare 'a[" "]=14'; declare -p a"#)
        .output()
        .expect("run declare quoted array subscript probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a a=([0]=\"14\")\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn declare_print_unmarked_indexed_array_is_reparseable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"typeset array=("a b" c); out=$(typeset -p array); eval "$out"; typeset -p array; printf '<%s><%s>\n' "${array[0]}" "${array[1]}""#)
        .output()
        .expect("run unmarked indexed array serialization probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a array=([0]=\"a b\" [1]=\"c\")\n<a b><c>\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn malformed_parameter_expansions_return_status_two() {
    for script in ["echo ${x", "echo ${x/foo", "echo ${x:?"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
            .arg("-c")
            .arg(script)
            .output()
            .expect("run malformed parameter expansion");

        assert_eq!(output.status.code(), Some(2), "script: {script}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected EOF"));
    }
}

#[test]
fn array_subscript_diagnostics_match_bash_for_assignment_and_expansion() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("target/issue-suites/results/arith-array-probes-20220822/array-probe.sh")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "b=<this is a test> b0=<this>\ncneg=<>\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("b[]=bcde: bad array subscript"));
    assert!(stderr.contains("b[*]=aaa: bad array subscript"));
    assert!(stderr.contains("c[-2]=4: bad array subscript"));
    assert!(stderr.contains("c: bad array subscript"));
}

#[test]
fn readonly_array_element_argument_matches_bash_identifier_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("unset a; a=(zero one); readonly a[1]")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("readonly: `a[1]`: not a valid identifier"));
}

#[test]
fn declare_assigns_an_element_of_an_existing_associative_array() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "declare -A map=([one]=first); declare map[two]=second; printf '%s:%s\n' \"${map[one]}\" \"${map[two]}\"",
        )
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "first:second\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn readonly_arithmetic_case_pattern_aborts_without_mutating() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("readonly xx=1; case 1 in $((xx++))) echo unexpected ;; *) : ;; esac; echo $xx.$?")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("xx: readonly variable"));
}

#[test]
fn compound_inside_command_substitution_still_executes_when_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $( if true; then echo foo; fi )")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "foo\n");
}

#[test]
fn invalid_case_terminator_inside_command_substitution_is_a_syntax_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo $(case x in x) ;; x) done ;; esac)")
        .output()
        .expect("run rubash");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn escaped_ifs_space_in_parameter_assignment_stays_one_field() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"unset a; printf '%s\n' ${a:=a\ b}; printf '<%s>\n' "$a""#)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a\nb\n<a b>\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn subshell_resets_nonempty_signal_traps_but_preserves_ignored_traps() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("trap 'echo bad' TERM; trap '' HUP; (trap)")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "trap -- '' SIGHUP\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn help_and_trap_pipeline_output_is_captured() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("help | head -n 1; trap -l | head -n 1")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 2);
}

#[test]
#[cfg(windows)]
fn external_pipeline_does_not_buffer_an_unbounded_producer() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("yes pipeline | head -n 3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "pipeline\npipeline\npipeline\n"
    );
}

#[test]
#[cfg(windows)]
fn external_pipeline_waits_for_all_members_of_a_multi_stage_pipeline() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("yes pipeline | head -n 3 | wc -l")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
}

#[test]
#[cfg(windows)]
fn external_pipeline_writes_large_output_redirect_without_blocking() {
    let output_path = std::env::temp_dir().join("rubash-large-pipeline-output.txt");
    let _ = fs::remove_file(&output_path);
    let command = format!(
        "yes '123456789 123456789 123456789 123456789' | head -3000 >> '{}'",
        shell_test_path(&output_path)
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(command)
        .output()
        .expect("run rubash");

    let contents = fs::read(&output_path).expect("read pipeline output");
    let _ = fs::remove_file(&output_path);
    assert!(output.status.success());
    assert_eq!(contents.len(), 120_000);
}

#[test]
#[cfg(windows)]
fn windows_userprofile_supplies_home_when_home_is_absent() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env_remove("HOME")
        .env("USERPROFILE", r"C:\rubash-home-test")
        .arg("-c")
        .arg("printf '%s\\n' \"$HOME\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "C:\\rubash-home-test\n"
    );
}

fn posix_real_seconds(stderr: &str) -> f64 {
    let real = stderr
        .lines()
        .find_map(|line| line.strip_prefix("real "))
        .expect("real time line");
    real.parse::<f64>().expect("numeric real seconds")
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
fn external_pipeline_preserves_limited_head_output() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf 'pipeline\\npipeline\\npipeline\\nextra\\n' | head -n 3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "pipeline\npipeline\npipeline\n"
    );
}

#[test]
fn declare_escaped_quote_array_element_assignment_targets_index_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"declare -a a; declare "a[\" \"]=14"; declare -p a"#)
        .output()
        .expect("run declare escaped array subscript probe");

    assert!(
        output.status.success(),
        "status={:?}, stdout={:?}, stderr={:?}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a a=([0]=\"14\")\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn embedded_arithmetic_escaped_quote_array_subscript_targets_index_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"declare -a a; : $(( a[\" \"]=17 )); declare -p a"#)
        .output()
        .expect("run embedded arithmetic escaped array subscript probe");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "declare -a a=([0]=\"17\")\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn builtin_pipeline_head_accepts_separate_line_count_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "printf 'x\\ny\\n' | head -n 1; \
             set -o 2>&1 | head -n 2; \
             export PIPE_HEAD_TEST=value; export | head -n 2",
        )
        .output()
        .expect("run builtin head pipeline probe");

    assert!(output.status.success());
    let lines = String::from_utf8_lossy(&output.stdout);
    let lines = lines.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], "x");
    assert!(lines[1].starts_with("allexport"));
    assert!(lines[2].starts_with("braceexpand"));
    assert!(lines.iter().any(|line| line.starts_with("declare -x ")));
}

#[test]
fn wait_without_operands_returns_success_after_failed_background_job() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("false & wait; printf 'wait=%s\\n' \"$?\"")
        .output()
        .expect("run no-operand wait probe");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "wait=0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn select_menu_uses_bash_stderr_format() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("select x; do printf '<%s>\\n' \"$x\"; break; done <<< 2")
        .arg("arg0")
        .arg("alpha")
        .arg("beta")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<beta>\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "1) alpha\n2) beta\n#? "
    );
}

#[test]
fn time_uses_timeformat_variable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("TIMEFORMAT='elapsed:%R cpu:%P percent:%%'; time true")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_stderr_matches(
        &String::from_utf8_lossy(&output.stderr),
        r"^elapsed:\d+\.\d{3} cpu:0\.00 percent:%\n$",
    );
}

#[test]
fn time_p_ignores_timeformat_variable() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("TIMEFORMAT='elapsed:%R'; time -p true")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_stderr_matches(
        &String::from_utf8_lossy(&output.stderr),
        r"^real \d+\.\d{2}\nuser 0\.00\nsys 0\.00\n$",
    );
}

#[test]
fn timeformat_supports_precision_and_long_modifiers() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("TIMEFORMAT='r:%3R u:%2U s:%0S long:%2lR p:%P'; time true")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_stderr_matches(
        &String::from_utf8_lossy(&output.stderr),
        r"^r:\d+\.\d{3} u:0\.00 s:0 long:\d+m\d+\.\d{2}s p:0\.00\n$",
    );
}

#[test]
fn time_reports_elapsed_wall_clock_for_slow_command() {
    let slow_command = if cfg!(windows) {
        "powershell.exe -NoProfile -Command Start-Sleep -Milliseconds 650"
    } else {
        "sh -c 'sleep 0.65'"
    };
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("time -p {slow_command}"))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_stderr_matches(&stderr, r"^real \d+\.\d{2}\nuser 0\.00\nsys 0\.00\n$");
    assert!(
        posix_real_seconds(&stderr) >= 0.50,
        "expected non-zero wall time, got stderr {stderr:?}"
    );
}

#[test]
fn timeformat_reports_invalid_format_character() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("TIMEFORMAT='bad:%Z'; time true; echo status:$?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:0\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: TIMEFORMAT: `Z': invalid format character\n"
    );
}

#[test]
fn timeformat_rejects_precision_on_percent_cpu() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("TIMEFORMAT='bad:%3P'; time true; echo status:$?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:0\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: TIMEFORMAT: `P': invalid format character\n"
    );
}

#[test]
fn c_command_redirects_stdout_to_stderr_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo -n '' 1>&2")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_redirects_stdout_with_default_fd_duplication() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo -n hi >&2")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "hi");
}

#[test]
fn c_command_exec_numeric_fd_copies_default_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3>&1; echo via-fd >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "via-fd\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_exec_numeric_fd_copies_default_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3>&2; echo via-fd >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "via-fd\n");
}

#[test]
fn c_command_printf_uses_persistent_fd_copied_from_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3>&1; printf '%s\\n' via-fd >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "via-fd\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_printf_uses_persistent_fd_copied_from_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3>&2; printf '%s\\n' via-fd >&3")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "via-fd\n");
}

#[test]
fn c_command_echo_applies_output_redirects_left_to_right() {
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_file(literal_fd_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo hi 3>&1 1>/dev/null >&3; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hi\nstatus:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(!literal_fd_path.exists());
}

#[test]
fn c_command_stderr_to_stdout_does_not_create_literal_ampersand_one() {
    let literal_fd_path = Path::new("&1");
    let _ = fs::remove_file(literal_fd_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("command-that-does-not-exist 2>&1 >/dev/null; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("command not found"), "{stdout}");
    assert!(stdout.contains("status:127"), "{stdout}");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(!literal_fd_path.exists());
}

#[test]
fn c_command_echo_reports_bad_fd_after_exec_close() {
    let literal_fd_path = Path::new("&3");
    let _ = fs::remove_file(literal_fd_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3>&1; echo hi >&3; exec 3>&-; echo fail >&3; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hi\nstatus:1\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: 3: Bad file descriptor\n"
    );
    assert!(!literal_fd_path.exists());
}

#[test]
fn c_command_echo_reports_write_error_for_closed_stdout() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo fail >&-; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:1\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: echo: write error: Bad file descriptor\n"
    );
}

#[test]
fn c_command_exec_numeric_fd_copies_default_stdin_for_read_u() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3<&0; read -u 3 value; printf '<%s>:%s\\n' \"$value\" \"$?\"")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"from-stdin\n")
        .unwrap();
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<from-stdin>:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_exec_streams_child_stdout_before_child_exit() {
    use std::io::BufRead;

    let rubash = shell_test_path(Path::new(env!("CARGO_BIN_EXE_rubash")));
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            r#"exec {rubash} -c "printf 'exec-ready\n'; sleep 2""#
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash exec streaming probe");

    let stdout = child.stdout.take().expect("capture rubash stdout");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
    });

    let line = match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => panic!("failed to read exec child stdout: {error}"),
        Err(error) => {
            let _ = wait_for_child_exit(&mut child, Duration::from_secs(4));
            panic!("exec child stdout was buffered until exit: {error}");
        }
    };

    assert_eq!(line, "exec-ready\n");
    assert!(
        child
            .try_wait()
            .expect("poll rubash exec streaming probe")
            .is_none(),
        "exec child exited before the streaming assertion"
    );
    assert!(
        wait_for_child_exit(&mut child, Duration::from_secs(4)),
        "exec streaming probe did not exit"
    );
    let output = child
        .wait_with_output()
        .expect("collect rubash exec streaming probe");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_read_dev_null_reports_eof() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("read value < /dev/null; printf '<%s>:%s\\n' \"$value\" \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<>:1\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_redirects_stdout_and_stderr_to_dev_null() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf out >/dev/null; command-that-does-not-exist 2>/dev/null; printf ok")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_external_command_reads_eof_from_null_device_alias() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("cat < nUl; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_appends_stdout_to_null_device_without_creating_path() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf first >> /dev/null; printf second >> /dev/null; printf ok")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "ok");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_read_closed_stdin_reports_bad_fd() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("read value <&-; printf '<%s>:%s\\n' \"$value\" \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<>:1\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: read: read error: 0: Bad file descriptor\n"
    );
}

#[test]
fn c_command_read_closed_stdin_redirects_bad_fd_diagnostic() {
    let error_path = Path::new("target").join("rubash-cli-read-closed-stderr.txt");
    let _ = fs::remove_file(&error_path);
    let script_path = shell_test_path(&error_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "read value <&- 2> {script_path}; printf '<%s>:%s\\n' \"$value\" \"$?\""
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<>:1\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert_eq!(
        fs::read_to_string(&error_path)
            .unwrap()
            .replace("\r\n", "\n"),
        "rubash: read: read error: 0: Bad file descriptor\n"
    );
    let _ = fs::remove_file(error_path);
}

#[test]
fn c_command_read_uses_regular_stdin_redirect_file() {
    let input_path = Path::new("target").join("rubash-cli-read-redirect-input.txt");
    fs::write(&input_path, "from-file\n").unwrap();
    let script_path = shell_test_path(&input_path);

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "read value < {script_path}; printf '<%s>:%s\\n' \"$value\" \"$?\""
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<from-file>:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let _ = fs::remove_file(input_path);
}

#[test]
fn c_command_kill_zero_accepts_current_process() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("kill -0 {}", std::process::id()))
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_kill_zero_accepts_shell_pid() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("kill -0 $$; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_kill_zero_accepts_current_process_group() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("kill -0 0; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:0\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_background_subshell_preserves_shell_pid() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("echo parent:$$:$BASHPID; (echo bg:$$:$BASHPID) & wait")
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let parent = lines.next().expect("parent line");
    let child = lines.next().expect("child line");
    let parent_fields = parent.split(':').collect::<Vec<_>>();
    let child_fields = child.split(':').collect::<Vec<_>>();
    assert_eq!(parent_fields[1], child_fields[1], "$$ should stay stable");
    assert_ne!(
        parent_fields[2], child_fields[2],
        "BASHPID should identify the background child"
    );
}

#[test]
fn c_command_background_kill_shell_pid_runs_term_trap() {
    let delay = if cfg!(windows) {
        "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command Start-Sleep -Milliseconds 250; "
    } else {
        "sleep 0.25; "
    };
    let script = format!(
        "trap 'echo TERM; return' TERM; \
         f() {{ ({delay}kill $$) & until (exit 42); do (exit 42); done; }}; \
         f; printf 'status:%s\\n' \"$?\""
    );
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "TERM\nstatus:42\n");
}

#[test]
fn c_command_kill_rejects_invalid_pid_operand() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("kill abc")
        .output()
        .expect("run rubash");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("kill: abc: arguments must be process or job IDs"));
}

#[test]
fn c_command_kill_sigkill_terminates_windows_pid() {
    let mut child = spawn_long_child();
    let pid = child.id();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("kill -9 {pid}"))
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        wait_for_child_exit(&mut child, Duration::from_secs(5)),
        "child process {pid} did not exit after kill -9"
    );
}

#[test]
fn c_command_kill_sigkill_terminates_rubash_child_pid() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("while :; do :; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn long-running rubash child");
    let pid = child.id();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("kill -9 {pid}"))
        .output()
        .expect("run rubash");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        wait_for_child_exit(&mut child, Duration::from_secs(5)),
        "rubash child process {pid} did not exit after kill -9"
    );
}

#[cfg(windows)]
fn spawn_long_child() -> Child {
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Start-Sleep -Seconds 30",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn long-running child")
}

#[cfg(not(windows))]
fn spawn_long_child() -> Child {
    Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn long-running child")
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if child.try_wait().expect("poll child").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();
    false
}

#[test]
fn while_read_redirect_wins_over_inherited_process_stdin() {
    let input_path = Path::new("target/rubash-while-read-open-parent-stdin.txt");
    fs::create_dir_all("target").unwrap();
    fs::write(input_path, "alpha\nbeta\n").unwrap();
    let shell_input_path = shell_test_path(input_path);
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "while IFS= read -r file; do printf '<%s>\\n' \"$file\"; done < {shell_input_path}"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");
    let open_parent_stdin = child.stdin.take().unwrap();

    assert!(
        wait_for_child_exit(&mut child, Duration::from_secs(3)),
        "while/read consumed inherited process stdin instead of redirected file"
    );
    drop(open_parent_stdin);
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "<alpha>\n<beta>\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let _ = fs::remove_file(input_path);
}

#[test]
fn c_command_external_uses_persistent_fd_copied_from_stdin() {
    let rubash = shell_test_path(Path::new(env!("CARGO_BIN_EXE_rubash")));
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!("exec 3<&0; {rubash} -c 'cat' <&3"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"external-stdin\n")
        .unwrap();
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "external-stdin\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_mapfile_uses_persistent_fd_copied_from_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "exec 3<&0; mapfile -u 3 -t arr; printf '%s:%s:%s\\n' \"${#arr[@]}\" \"${arr[0]}\" \"${arr[1]}\"",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run rubash");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"alpha\nbeta\n")
        .unwrap();
    let output = child.wait_with_output().expect("wait rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "2:alpha:beta\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn c_command_mapfile_reports_bad_fd_after_exec_close() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 3<&-; mapfile -u3 arr; printf 'status:%s len:%s\\n' \"$?\" \"${#arr[@]}\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:1 len:0\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "rubash: mapfile: 3: invalid file descriptor: Bad file descriptor\n"
    );
}

#[test]
fn c_command_mapfile_u_accepts_expanded_persistent_fd() {
    let input_path = "target/rubash-cli-mapfile-fd-input.txt";
    let _ = fs::remove_file(input_path);
    fs::write(input_path, "alpha\nbeta\n").expect("write mapfile input");

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(format!(
            "exec 3<{input_path}; mapfile -u \"$((1+2))\" -t arr; \
             printf '%s:%s:%s:%s\\n' \"$?\" \"${{#arr[@]}}\" \"${{arr[0]}}\" \"${{arr[1]}}\""
        ))
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0:2:alpha:beta\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");

    let _ = fs::remove_file(input_path);
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

#[cfg(windows)]
#[test]
fn extensionless_shell_script_on_path_runs_without_external_sh() {
    let bin_dir = Path::new("target").join("rubash-cli-extensionless-bin");
    let _ = fs::remove_dir_all(&bin_dir);
    fs::create_dir_all(&bin_dir).unwrap();
    let script_path = bin_dir.join("tool");
    fs::write(
        &script_path,
        "#!/usr/bin/env sh\nprintf 'script:%s:%s:%s\\n' \"$0\" \"$1\" \"$#\"\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("tool alpha beta")
        .env("PATH", bin_dir.to_string_lossy().to_string())
        .output()
        .expect("run rubash");

    let _ = fs::remove_dir_all(&bin_dir);
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("script:{}:alpha:2\n", script_path.to_string_lossy())
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
fn cli_shell_option_name_applies_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-o")
        .arg("errexit")
        .arg("-c")
        .arg("[[ -o errexit ]]; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
}

#[test]
fn cli_plus_shell_option_name_disables_previous_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-e")
        .arg("+o")
        .arg("errexit")
        .arg("-c")
        .arg("[[ -o errexit ]]; echo $?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1\n");
}

#[test]
fn cli_o_posix_sets_posix_mode_and_shell_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-o")
        .arg("posix")
        .arg("-c")
        .arg("type break; [[ -o posix ]]; echo option:$?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "break is a special shell builtin\noption:0\n"
    );
}

#[test]
fn invalid_cli_shell_option_fails_before_command_string() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-o")
        .arg("no_such_shell_option")
        .arg("-c")
        .arg("echo should-not-run")
        .output()
        .expect("run rubash");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid shell option name"));
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
fn cli_noexec_flag_parses_without_executing_command_string() {
    let output_path = Path::new("target").join("rubash-cli-noexec-should-not-exist.txt");
    let _ = fs::remove_file(&output_path);
    let script = format!("printf should-not-run > {}", shell_test_path(&output_path));

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(!output_path.exists());
}

#[test]
fn cli_noexec_flag_parses_script_file_without_executing() {
    let script_path = Path::new("target").join("rubash-cli-noexec-script.sh");
    let output_path = Path::new("target").join("rubash-cli-noexec-script-should-not-exist.txt");
    let _ = fs::remove_file(&output_path);
    fs::write(
        &script_path,
        format!(
            "printf should-not-run > {}\n",
            shell_test_path(&output_path)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-n")
        .arg(&script_path)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    assert!(!output_path.exists());
    let _ = fs::remove_file(script_path);
}

#[test]
fn command_string_sets_c_shell_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("printf '%s\\n' \"$-\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .contains('c'));
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
fn c_command_set_o_invalid_name_returns_usage_status() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("set -o no_such; printf 'status:%s\\n' \"$?\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "status:2\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid option name"));
}

#[test]
fn oversized_continue_targets_outermost_active_loop() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("for v in a b c; do echo A:$v; continue 666; done; echo OK")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "A:a\nA:b\nA:c\nOK\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn subshell_break_does_not_escape_parent_loop() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("for v in a b c; do echo A:$v; (echo B; break; echo C); echo D; done; echo status:$?")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "A:a\nB\nC\nD\nA:b\nB\nC\nD\nA:c\nB\nC\nD\nstatus:0\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("break: only meaningful")
            .count(),
        3
    );
}

#[test]
fn command_substitution_break_uses_subshell_loop_context() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("for v in a b; do out=$(echo B; break; echo C); printf '%s:%s\\n' \"$v\" \"$out\"; done")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "a:B\nC\nb:B\nC\n");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .matches("break: only meaningful")
            .count(),
        2
    );
}

// GNU subst.c string_list_pos_params over arrayfunc.c array_keys: the
// ${!arr[@]}/${!arr[*]} key list joins with IFS[0] when the word is an
// unquoted command word (the joined string then field-splits normally),
// takes the dollar_at space-join inside double quotes and on assignment
// RHS (PF_ASSIGNRHS), and `*` always uses the IFS[0] join.
#[test]
fn assoc_key_list_at_unquoted_joins_with_ifs_first_char_then_splits() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; set -- ${!A[@]}; echo $#")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "2");
}

#[test]
fn assoc_key_list_star_unquoted_joins_with_ifs_first_char_then_splits() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; set -- ${!A[*]}; echo $#")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "2");
}

#[test]
fn assoc_key_list_at_double_quoted_stays_space_joined() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; echo \"[${!A[@]}]\"")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[c a b]");
}

#[test]
fn assoc_key_list_star_double_quoted_joins_with_ifs_first_char() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; echo \"[${!A[*]}]\"")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[c:a b]");
}

#[test]
fn assoc_key_list_at_assignment_rhs_stays_space_joined() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; x=${!A[@]}; echo [$x]")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[c a b]");
}

#[test]
fn assoc_key_list_star_assignment_rhs_joins_with_ifs_first_char() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("IFS=:; declare -A A; A[c]=1; A[\"a b\"]=2; x=${!A[*]}; echo \"[$x]\"")
        .output()
        .expect("run rubash");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[c:a b]");
}

fn tilde_user_passwd_fixture_root(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rubash-cli-tilde-user-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("etc")).expect("create fixture etc");
    std::fs::write(
        root.join("etc").join("passwd"),
        "root:x:0:0:root:/root:/bin/bash\nniu:x:1000:1000::/c/Users/niu-home:/bin/rubash\n",
    )
    .expect("write fixture passwd");
    root
}

#[test]
fn tilde_user_expands_passwd_home_via_rubash_root() {
    let root = tilde_user_passwd_fixture_root("word");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("RUBASH_ROOT", &root)
        .arg("-c")
        .arg("echo ~niu; echo ~niu/docs; echo ~missinguser; echo ~missinguser/x")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(root);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "/c/Users/niu-home\n/c/Users/niu-home/docs\n~missinguser\n~missinguser/x\n"
    );
}

#[test]
fn tilde_user_assignment_value_expands_and_missing_stays_literal() {
    let root = tilde_user_passwd_fixture_root("assign");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("RUBASH_ROOT", &root)
        .arg("-c")
        .arg("A=~niu/bin; B=bin:~niu/tools; C=~missing/x; echo \"$A|$B|$C\"")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(root);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "/c/Users/niu-home/bin|bin:/c/Users/niu-home/tools|~missing/x\n"
    );
}

#[test]
fn tilde_user_colon_terminated_word_glues_verbatim() {
    let root = tilde_user_passwd_fixture_root("colon");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .env("RUBASH_ROOT", &root)
        .arg("-c")
        .arg("echo ~niu:sub; echo ~niu:~niu2")
        .output()
        .expect("run rubash");
    let _ = std::fs::remove_dir_all(root);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "/c/Users/niu-home:sub\n/c/Users/niu-home:~niu2\n"
    );
}
