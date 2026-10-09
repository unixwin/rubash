use std::process::Command;

#[test]
fn c_command_exec_persistent_output_process_substitution_receives_later_writes() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("exec 9> >(read -r line; printf 'line=%s\\n' \"$line\"); printf 'hello\\n' >&9; exec 9>&-; wait")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "line=hello\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn assignment_output_process_substitution_feeds_external_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(r#"f() { cat "$1" >"$x"; }; x=>(tr '[:lower:]' '[:upper:]') f <(printf 'hi there\n')"#)
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "HI THERE\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn exec_replacing_stderr_process_substitution_flushes_previous_target() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("rm -f __rubash_psubst_err.tmp; exec 4>&2; exec 2> >(tee __rubash_psubst_err.tmp); echo hello >&2; exec 2>&4; cat __rubash_psubst_err.tmp; rm -f __rubash_psubst_err.tmp")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "hello\nhello\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn coproc_input_move_marks_array_endpoint_closed() {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg("coproc C { echo hi; }; exec 4<&${C[0]}-; read -r x <&4; printf 'x=%s arr=%s\\n' \"$x\" \"${C[*]}\"")
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    // GNU coproc pairs land on high fds below 64 (jobs.c move_to_high_fd
    // policy; `coproc C { echo hi; }` binds 63 60, so after `exec 4<&${C[0]}-`
    // moves+closes the read end the array reads `-1 60`). WSL GNU Bash 5.3.0
    // probe 2026-09-27: `x=hi arr=-1 60`.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "x=hi arr=-1 60\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn coproc_child_inherits_function_table_for_debug_trap() {
    // unixwin/rubash#441: GNU coproc forks, so the coprocess shell inherits
    // the full function table. A DEBUG trap installed before the coproc
    // (bash-preexec's `__bp_preexec_invoke_exec`) must resolve inside the
    // child instead of falling through to a PATH lookup and reporting
    // `command not found`. GNU Bash 5.2.37 (msys) oracle probe
    // 2026-10-09: `PREEXEC: inside-coproc`, no diagnostics on stderr.
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "preexec() { echo \"PREEXEC: $1\" >&2; }; \
             trap 'preexec \"$BASH_COMMAND\"' DEBUG; \
             coproc C { preexec inside-coproc; echo coproc-ready; }; \
             read -r out <&${C[0]}; \
             echo \"got: $out\"",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    // The child executes the body's DEBUG trap successfully (the parent's
    // own DEBUG traps for the surrounding commands also log to stderr).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stdout, "got: coproc-ready\n", "stdout was: {stdout}");
    assert!(
        stderr.contains("PREEXEC: inside-coproc"),
        "stderr was: {stderr}"
    );
    assert!(
        !stderr.contains("command not found"),
        "stderr was: {stderr}"
    );
}

#[test]
fn coproc_function_snapshot_is_one_way_into_child() {
    // The function table rides to the coprocess as a BASH_FUNC snapshot;
    // the child sees it but the parent's table and exported set are
    // untouched by the spawn (fork snapshot semantics).
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(
            "parent_fn() { echo p; }; \
             coproc C { parent_fn; }; \
             read -r out <&${C[0]}; \
             echo \"child: $out\"; \
             declare -F parent_fn >/dev/null && echo parent-keeps-fn; \
             export -p | grep -q 'BASH_FUNC_parent_fn' || echo no-parent-export",
        )
        .output()
        .expect("run rubash");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout, "child: p\nparent-keeps-fn\nno-parent-export\n",
        "stdout was: {stdout}"
    );
}
