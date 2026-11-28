//! Issue unixwin/rubash#4161 regression: inside a function, the first
//! diagnostic layer of a readonly compound-declaration operand must follow
//! GNU's `this_command_name` lifecycle, not the function-scope stack.
//!
//! GNU execute_simple_command expands the word list BEFORE installing the
//! command word as this_command_name (execute_cmd.c:4617 expand_words vs
//! :4799-4800 run_builtin), so the expansion-time make_internal_declare
//! diagnostic (subst.c:12949 expand_declaration_argument -> variables.c:2658
//! make_local_variable -> sh_readonly -> builtin_error prolog,
//! builtins/common.c:83-94) still sees the enclosing invocation's name for
//! the FIRST simple command of a function body. Completion of every simple
//! command clears the label to NULL (execute_cmd.c:4952 — the :4796 unwind
//! protect is discarded, never restored), so later commands' expansion-time
//! diagnostics are bare. Null commands NULL the label immediately
//! (execute_cmd.c:4629-4631); `((`/`[[` install their operator spellings
//! (execute_cmd.c:3231/3931/4164) and never clear; pipelines and subshells
//! run in forked children and leave the parent's label untouched.
//!
//! rubash emits the nested-declare diagnostic from the builtin body — after
//! dispatch — so the expansion-time label is captured at the install point
//! and the clearing guard reproduces :4952 on every exit path.
//!
//! All expectations below were probed byte-for-byte against WSL GNU Bash
//! 5.3.20 (/usr/local/bin/bash) with the same LF script files.

use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run_script(name: &str, script: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("issue4161-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create probe dir");
    std::fs::write(dir.join("probe.sh"), script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("probe.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

#[test]
fn issue_repro_second_command_expansion_layer_is_bare() {
    // The exact #4161 repro: line 3's first layer carries `f:` (the first
    // command expands while this_command_name is still the invocation), but
    // line 4's first layer must be BARE — `local` completed and :4952
    // cleared the label.
    let run = run_script(
        "repro",
        "readonly r=1\nf() {\n  local r=(9)\n  declare r=(4)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 3: f: r: readonly variable\n\
         probe.sh: line 3: local: r: readonly variable\n\
         probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: declare: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn first_command_in_function_carries_invocation_name() {
    // `declare` as the very first command of the body: expansion still sees
    // the invocation label `f` (execute_cmd.c:4617 before :4800).
    let run = run_script("firstcmd", "readonly r=1\nf() {\n  declare r=(4)\n}\nf\n");
    assert_eq!(
        run.stderr,
        "probe.sh: line 3: f: r: readonly variable\n\
         probe.sh: line 3: declare: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn prior_simple_command_nulls_the_label() {
    // Any completed simple command (`true`, even `:`) clears the label:
    // the next expansion-time layer is bare.
    let run = run_script(
        "priorcmd",
        "readonly r=1\nf() {\n  true\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn assignment_only_command_nulls_the_label() {
    // execute_cmd.c:4629-4631: a simple command whose expansion leaves no
    // words NULLs the label — even though it never installs a command word.
    let run = run_script(
        "nullcmd",
        "readonly r=1\nf() {\n  x=1\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn nested_function_installs_its_own_invocation_name() {
    // Inside `g`, the first failing `local` sees `g`; after `g` returns the
    // label is NULL, so `declare` back in `f` is bare.
    let run = run_script(
        "nested",
        "readonly r=1\ng() {\n  local r=(9)\n}\nf() {\n  g\n  declare r=(4)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 3: g: r: readonly variable\n\
         probe.sh: line 3: local: r: readonly variable\n\
         probe.sh: line 7: r: readonly variable\n\
         probe.sh: line 7: declare: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn pipeline_and_subshell_do_not_clear_the_parent_label() {
    // Pipeline elements and subshell bodies execute in forked children in
    // GNU — the parent's this_command_name survives, so the next expansion
    // still carries `f`.
    let run = run_script(
        "pipeline",
        "readonly r=1\nf() {\n  true | cat\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: f: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    let run = run_script(
        "subshell",
        "readonly r=1\nf() {\n  (true)\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: f: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn arith_and_cond_commands_install_operator_labels() {
    // execute_cmd.c:3231/4164: `((`/`[[` install their operator spelling as
    // this_command_name and never clear it.
    let run = run_script(
        "arith",
        "readonly r=1\nf() {\n  ((1))\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: ((: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    let run = run_script(
        "cond",
        "readonly r=1\nf() {\n  [[ -n x ]]\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: [[: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn rejected_copy_creates_no_local_and_function_continues() {
    // make_local_variable refused the copy: `declare -p r` shows the GLOBAL
    // readonly binding, and execution continued past the failure.
    let run = run_script(
        "nocopy",
        "readonly r=1\nf() {\n  local r=(9)\n  declare -p r\n}\nf\n",
    );
    assert_eq!(run.stdout, "declare -r r=\"1\"\n");
    assert_eq!(
        run.stderr,
        "probe.sh: line 3: f: r: readonly variable\n\
         probe.sh: line 3: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}
