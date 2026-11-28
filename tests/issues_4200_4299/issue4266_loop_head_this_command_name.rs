//! Issue unixwin/rubash#4266 regression: `for` and `select` loop heads own
//! part of GNU's `this_command_name` lifecycle.
//!
//! GNU execute_for_command clears this_command_name on every iteration after
//! the head DEBUG/xtrace work and before binding the loop variable
//! (execute_cmd.c:3064). execute_select_command clears it once after the head
//! DEBUG/xtrace work and before expanding the select word list
//! (execute_cmd.c:3539); unlike `for`, that clear is not repeated per
//! iteration. execute_arith_for_command installs `((` at entry
//! (execute_cmd.c:3231), and eval_arith_for_expr reinstalls it immediately
//! before every init/test/step evalexp (execute_cmd.c:3204).
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
    let dir = std::env::temp_dir().join(format!("issue4266-{name}-{}", std::process::id()));
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
fn for_head_first_iteration_expansion_layer_is_bare() {
    let run = run_script(
        "for-first",
        "declare -r r\nf() {\n  for x in 1; do\n    local r=(9)\n  done\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn for_head_clears_again_on_each_iteration() {
    // Iteration 1 ends with `((1))`, whose sticky `((` label would reach
    // iteration 2 if execute_cmd.c:3064 did not clear it at the next head.
    let run = run_script(
        "for-each",
        "declare -r r\nf() {\n  for x in 1 2; do\n    local r=(9)\n    ((1))\n  done\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n\
         probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn for_zero_iterations_preserves_the_invocation_label() {
    // execute_cmd.c:3064 is inside the iteration loop. With an empty word
    // list it never runs, so the following expansion still sees `f`.
    let run = run_script(
        "for-zero",
        "declare -r r\nf() {\n  for x in ; do\n    :\n  done\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 6: f: r: readonly variable\n\
         probe.sh: line 6: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn select_head_expansion_layer_is_bare() {
    let run = run_script(
        "select-head",
        "declare -r r\nf() {\n  select x in a; do\n    local r=(9)\n    break\n  done\n}\nf <<< '1'\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "1) a\n\
         #? probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn select_head_clears_once_not_per_iteration() {
    // Iteration 1 is bare because execute_cmd.c:3539 cleared the head once.
    // Its trailing `((1))` then remains live into iteration 2: select does
    // not repeat the `for` head's per-iteration clear.
    let run = run_script(
        "select-each",
        "declare -r r\nf() {\n  select x in a; do\n    local r=(9)\n    ((1))\n    if ((++n == 2)); then\n      break\n    fi\n  done\n}\nf <<< $'1\\n1'\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "1) a\n\
         #? probe.sh: line 4: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n\
         #? probe.sh: line 4: ((: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn select_empty_word_list_still_clears_the_head_label() {
    // execute_cmd.c:3539 runs before expand_words_no_vars at :3546 and the
    // empty-list return at :3548-3553.
    let run = run_script(
        "select-empty",
        "declare -r r\nf() {\n  select x in ; do\n    :\n  done\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 6: r: readonly variable\n\
         probe.sh: line 6: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn arithmetic_for_installs_operator_label_for_every_iteration() {
    let run = run_script(
        "arith-for",
        "declare -r r\nf() {\n  for ((i=0; i<2; i++)); do\n    local r=(9)\n  done\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 4: ((: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n\
         probe.sh: line 4: ((: r: readonly variable\n\
         probe.sh: line 4: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn arithmetic_for_empty_test_and_step_still_reinstall_operator_label() {
    // GNU calls eval_arith_for_expr even for empty header words, so
    // execute_cmd.c:3204 reinstalls `((` at every iteration boundary.
    let run = run_script(
        "arith-empty",
        "declare -r r\nf() {\n  i=0\n  for ((; ; )); do\n    local r=(9)\n    i=$((i+1))\n    [ \"$i\" -ge 2 ] && break\n  done\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 5: ((: r: readonly variable\n\
         probe.sh: line 5: local: r: readonly variable\n\
         probe.sh: line 5: ((: r: readonly variable\n\
         probe.sh: line 5: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(0));
}

#[test]
fn arithmetic_for_operator_label_survives_after_the_loop() {
    let run = run_script(
        "arith-after",
        "declare -r r\nf() {\n  for ((i=0; i<1; i++)); do\n    :\n  done\n  local r=(9)\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr,
        "probe.sh: line 6: ((: r: readonly variable\n\
         probe.sh: line 6: local: r: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}

#[test]
fn if_while_until_and_case_do_not_own_a_loop_head_clear() {
    // `if`/`while`/`until` do not clear at their compound heads. Their
    // `true`/`false` conditions are simple commands, whose execute_cmd.c:4952
    // completion clear is what leaves these expansion layers bare. `case` has
    // no condition command at all, so the invocation label survives into the
    // first branch command.
    for (name, script, first_line) in [
        (
            "if",
            "declare -r r\nf() {\n  if true; then\n    local r=(9)\n  fi\n}\nf\n",
            "probe.sh: line 4: r: readonly variable\n",
        ),
        (
            "while",
            "declare -r r\nf() {\n  while true; do\n    local r=(9)\n    break\n  done\n}\nf\n",
            "probe.sh: line 4: r: readonly variable\n",
        ),
        (
            "until",
            "declare -r r\nf() {\n  until false; do\n    local r=(9)\n    break\n  done\n}\nf\n",
            "probe.sh: line 4: r: readonly variable\n",
        ),
        (
            "case",
            "declare -r r\nf() {\n  case 1 in\n    1) local r=(9) ;;\n  esac\n}\nf\n",
            "probe.sh: line 4: f: r: readonly variable\n",
        ),
    ] {
        let run = run_script(name, script);
        assert_eq!(run.stdout, "", "{name} stdout");
        assert_eq!(
            run.stderr,
            format!("{first_line}probe.sh: line 4: local: r: readonly variable\n"),
            "{name} stderr: {}",
            run.stderr
        );
    }
}

#[test]
fn compound_conditions_that_run_no_simple_command_preserve_invocation_label() {
    // Strong negative controls: a no-match `case` condition runs no simple
    // command at all. If `if`/`while`/`until` erroneously owned a loop-head
    // clear, the first branch/body expansion would be bare; GNU instead keeps
    // the invocation label `f` across all three compound boundaries. The
    // `until` shape negates the compound condition so its body runs once.
    for (name, script, code) in [
        (
            "if-noexec",
            "declare -r r\nf() {\n  if case 1 in\n    2) : ;;\n  esac; then\n    local r=(9)\n  fi\n}\nf\n",
            Some(1),
        ),
        (
            "while-noexec",
            "declare -r r\nf() {\n  while case 1 in\n    2) : ;;\n  esac; do\n    local r=(9)\n    break\n  done\n}\nf\n",
            Some(0),
        ),
        (
            "until-noexec",
            "declare -r r\nf() {\n  until ! case 1 in\n    2) : ;;\n  esac; do\n    local r=(9)\n    break\n  done\n}\nf\n",
            Some(0),
        ),
    ] {
        let run = run_script(name, script);
        assert_eq!(run.stdout, "", "{name} stdout");
        assert_eq!(
            run.stderr,
            "probe.sh: line 6: f: r: readonly variable\n\
             probe.sh: line 6: local: r: readonly variable\n",
            "{name} stderr: {}",
            run.stderr
        );
        assert_eq!(run.code, code, "{name} code");
    }
}

#[test]
fn readonly_for_loop_variable_diagnostic_has_no_command_label() {
    // execute_cmd.c:3064 runs before the loop-variable bind at :3066-3080;
    // GNU's err_readonly output therefore has no `f:' command segment.
    let run = run_script(
        "for-readonly",
        "declare -r x\nf() {\n  for x in 1; do\n    :\n  done\n}\nf\n",
    );
    assert_eq!(run.stdout, "");
    assert_eq!(
        run.stderr, "probe.sh: line 3: x: readonly variable\n",
        "stderr: {}",
        run.stderr
    );
    assert_eq!(run.code, Some(1));
}
