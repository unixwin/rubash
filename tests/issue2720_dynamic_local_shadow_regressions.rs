//! rubash#2720 — local (and tempenv) bindings of dynamic variables
//! never shadowed the provider: reads consulted the dynamic getter BY
//! NAME before the variable table, so `local SECONDS=42` was
//! unreachable across expansion, arithmetic, `[[ ]]`, command
//! substitution and `declare -p`, and the dynamic assignment hooks
//! (SECONDS offset, RANDOM seed, the provider swallows) fired for
//! binds that never touched the dynamic cell — `SECONDS=99 true`
//! permanently shifted the session clock and `local RANDOM=42`
//! reseeded the global generator.
//!
//! GNU ground: the dynamic variables are ordinary SHELL_VAR cells in
//! the global table carrying get/set hooks (variables.c:1844
//! initialize_dynamic_variables, INIT_DYNAMIC_VAR at variables.c:1202;
//! assign_random at variables.c:1393-1408, unbind_variable at
//! variables.c:3839). find_variable walks the variable-context stack
//! first, so a `local`/tempenv binding is a separate plain cell that
//! owns the name while visible; the hooks live on the global cell and
//! answer only for binds that reach it. The fix is that one discipline
//! applied to every face: Executor::dynamic_name_shadowed /
//! dynamic_bind_targets_plain_cell gate the provider reads, the
//! declare view, the arithmetic context and evaluator arms, and the
//! apply_shell_assignment arms (the DIRSTACK gate, rubash#2424, is the
//! in-tree precedent); tempenv binds store plain cells, and a KEPT
//! tempenv bind of SECONDS/RANDOM converts to the real dynamic bind.
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file probes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2720-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> (String, String) {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn check(tag: &str, script: &str, expected_stdout: &str) {
    let dir = scratch(tag);
    let (stdout, stderr) = run_script(&dir, script);
    assert_eq!(
        stdout, expected_stdout,
        "stdout for {tag} (stderr: {stderr})"
    );
    assert_eq!(stderr, "", "stderr for {tag}");
}

#[test]
fn issue2720_local_binds_own_reads_on_every_face() {
    // The filing oracle case: every dynamic name in the local bind
    // reads back the bound value, not the provider's.
    check(
        "main",
        "f() { local SECONDS=42 BASH_SUBSHELL=9 EPOCHSECONDS=111 RANDOM=7 SRANDOM=5 BASHPID=1 LINENO=999 BASH_COMMAND=x; echo \"$SECONDS $BASH_SUBSHELL $EPOCHSECONDS $RANDOM $SRANDOM $BASHPID $LINENO $BASH_COMMAND\"; }\nf\n",
        "42 9 111 7 5 1 999 x\n",
    );
    // Static reads: a bound RANDOM/SRANDOM cell does not draw.
    check(
        "static",
        "f() { local RANDOM=7 SRANDOM=5; echo \"r=$RANDOM,$RANDOM s=$SRANDOM,$SRANDOM\"; }\nf\n",
        "r=7,7 s=5,5\n",
    );
    // Arithmetic evaluation (direct and through the context snapshot).
    check(
        "arith",
        "f() { local SECONDS=42 EPOCHSECONDS=111 BASHPID=4242 BASH_SUBSHELL=3; echo $((SECONDS + 1)) $((EPOCHSECONDS + 1)) $((BASHPID + BASH_SUBSHELL)); }\nf\n",
        "43 112 4245\n",
    );
    // [[ ]] and [ ] conditions, and command substitution (the comsub
    // executor inherits the shadow).
    check(
        "cond",
        "f() { local SECONDS=42; [[ $SECONDS == 42 ]] && echo cond-ok; [ \"$SECONDS\" = 42 ] && echo test-ok; echo \"in=$(echo $SECONDS)\"; }\nf\n",
        "cond-ok\ntest-ok\nin=42\n",
    );
    // declare -p renders the plain cell, not the provider's -i view.
    check(
        "declarep",
        "f() { local SECONDS=42 RANDOM=7; declare -p SECONDS RANDOM; }\nf\n",
        "declare -- SECONDS=\"42\"\ndeclare -- RANDOM=\"7\"\n",
    );
    // A plain assignment after the local bind writes the local cell.
    check(
        "plainassign",
        "f() { local BASHPID=777; BASHPID=555; echo $BASHPID; }\nf\n",
        "555\n",
    );
    // A valueless local reads as unset on every operator face.
    check(
        "valueless",
        "f() { local SECONDS; echo \"a=${SECONDS-unset} b=${SECONDS:-def} p=${SECONDS+plus}\"; }\nf\n",
        "a=unset b=def p=\n",
    );
    // Dynamic scoping: an inner function sees the outer local.
    check(
        "dynscope",
        "g() { echo \"g sees $SECONDS\"; }\nf() { local SECONDS=42; g; }\nf\n",
        "g sees 42\n",
    );
    // Unsetting a local dynamic name leaves a valueless local for the
    // rest of the frame (GNU: `declare -p SECONDS` inside shows a
    // valueless plain declaration), and the global dynamic variable
    // survives the frame intact.
    check(
        "unsetlocal",
        "f() { local SECONDS=42; unset SECONDS; echo \"in: [${SECONDS-unset}]\"; declare -p SECONDS; }\nf\n[ $SECONDS -lt 5 ] && echo dyn-survives\nSECONDS=1000\n[ $SECONDS -ge 1000 ] && echo dyn\n",
        "in: [unset]\ndeclare -- SECONDS\ndyn-survives\ndyn\n",
    );
    check(
        "unsetlocalrand",
        "f() { local RANDOM=7; unset RANDOM; echo \"rand-in: [${RANDOM-unset}]\"; }\nf\n[ -n \"$RANDOM\" ] && echo draws\n",
        "rand-in: [unset]\ndraws\n",
    );
    check(
        "histcmd",
        "f() { local HISTCMD=55; echo $HISTCMD; }\nf\n",
        "55\n",
    );
}

#[test]
fn issue2720_hooks_fire_only_for_the_dynamic_cell() {
    // A local SECONDS bind must not move the global offset.
    check(
        "localnoleak",
        "f() { local SECONDS=1000; }\nf\n[ $SECONDS -lt 5 ] && echo noleak\n",
        "noleak\n",
    );
    // A plain assignment to a visible local must not move it either.
    check(
        "plainnoleak",
        "f() { local SECONDS; SECONDS=5; echo \"in=$SECONDS\"; }\nf\n[ $SECONDS -lt 5 ] && echo after-ok\n",
        "in=5\nafter-ok\n",
    );
    // A local RANDOM bind (at declaration or by later assignment) must
    // not reseed the global generator: after seeding 42 and drawing
    // once, the sequence must ADVANCE past the same point, not restart.
    check(
        "localnoreseed",
        "f() { local RANDOM=42; }\nRANDOM=42\nx=$RANDOM\nf\ny=$RANDOM\n[ \"$x\" == \"$y\" ] && echo RESEEDED || echo advanced\n",
        "advanced\n",
    );
    check(
        "plainlocalnoreseed",
        "f() { local RANDOM; RANDOM=42; echo \"r=$RANDOM\"; }\nf\n",
        "r=42\n",
    );
    // declare inside a function binds a local in GNU: no reseed.
    check(
        "declareinfunc",
        "f() { declare RANDOM=42; }\nRANDOM=42\nx=$RANDOM\nf\ny=$RANDOM\n[ \"$x\" == \"$y\" ] && echo RESEEDED || echo advanced\n",
        "advanced\n",
    );
    // ... while declare -g binds the global cell: the seed fires.
    check(
        "declareg",
        "f() { declare -g RANDOM=42; }\nRANDOM=42\nx=$RANDOM\nf\ny=$RANDOM\n[ \"$x\" == \"$y\" ] && echo RESEEDED || echo advanced\n",
        "RESEEDED\n",
    );
    // A declare bind of SECONDS at the top level (or with -g) DOES move
    // the offset — the hook belongs to the dynamic cell's bind.
    check(
        "declareseconds",
        "declare SECONDS=1000\n[ $SECONDS -ge 1000 ] && echo off-ok\n",
        "off-ok\n",
    );
    check(
        "declaregseconds",
        "f() { declare -g SECONDS=1000; }\nf\n[ $SECONDS -ge 1000 ] && echo off-ok\n",
        "off-ok\n",
    );
    // The global plain-assignment controls are unchanged.
    check(
        "globalseconds",
        "SECONDS=1000\n[ $SECONDS -ge 1000 ] && echo off-ok\n",
        "off-ok\n",
    );
}

#[test]
fn issue2720_tempenv_binds_are_plain_cells() {
    // A tempenv SECONDS owns reads for the command's duration and the
    // global offset is untouched afterwards.
    check(
        "tempfunc",
        "f() { echo \"in=$SECONDS\"; }\nSECONDS=99 f\n[ $SECONDS -lt 5 ] && echo after-ok\n",
        "in=99\nafter-ok\n",
    );
    check(
        "tempbuiltin",
        "SECONDS=99 true\n[ $SECONDS -lt 5 ] && echo after-ok\n",
        "after-ok\n",
    );
    check(
        "tempspecial",
        "SECONDS=99 :\n[ $SECONDS -lt 5 ] && echo gone\n",
        "gone\n",
    );
    // A tempenv RANDOM reads as the bound value inside the command and
    // does not reseed the global generator.
    check(
        "temprandom",
        "f() { echo \"r=$RANDOM,$RANDOM\"; }\nRANDOM=42 f\n",
        "r=42,42\n",
    );
    check(
        "temprandomseed",
        "RANDOM=42\na=$RANDOM\nRANDOM=42 true\nb=$RANDOM\n[ \"$a\" == \"$b\" ] && echo RESEEDED || echo advanced\n",
        "advanced\n",
    );
    // Kept tempenv binds convert to the real dynamic bind: posix-mode
    // persistence and attribute-builtin absorption both run the hook.
    check(
        "posixkeep",
        "set -o posix\nSECONDS=99 :\n[ $SECONDS -ge 99 ] && echo kept-dynamic\n",
        "kept-dynamic\n",
    );
    check(
        "absorb",
        "SECONDS=99 export SECONDS\n[ $SECONDS -ge 99 ] && echo kept-dynamic\n",
        "kept-dynamic\n",
    );
}
