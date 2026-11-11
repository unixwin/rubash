//! Issue rubash#3185: bare `help` two-column listing layout.
//!
//! GNU contract (bash-5.3 builtins/help.def): show_builtin_command_help
//! computes width = default_columns()/2 (general.c default_columns:
//! the COLUMNS shell variable when it parses positive, else the window
//! size, else 80), capped at the 128-byte blurb, floored to 40 when
//! width <= 3; height = (num + 1) / 2 rows; row i pairs entry i (left)
//! with entry i + height (right) — column-major fill. The rendering
//! cell is dispcolumn (single-byte locale) or wdispcolumn
//! (MB_CUR_MAX > 1), whose truncation keep-counts differ by one
//! character at the boundary lengths.
//!
//! The expected rows below are the GNU algorithm applied to rubash's
//! own 78-entry table (which deliberately includes the extra `env`
//! entry, a ruled deviation that is NOT part of this issue): the
//! renderer used to generate them was first validated byte-for-byte
//! against the GNU bash 5.3.20 oracle's own 77-entry output at
//! COLUMNS=80 and COLUMNS=120 under both LC_ALL=C and LC_ALL=C.UTF-8
//! (all four grids matched exactly), then re-run over rubash's table.
//!
//! Base-tree behaviour pinned against: width was hardcoded to 40
//! (COLUMNS ignored) and the single-byte dispcolumn cell form was
//! never used (the multibyte form was applied in every locale).

use std::process::Command;

fn run_help(script: &str, env: &[(&str, Option<&str>)]) -> Vec<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg("-c").arg(script);
    cmd.env_remove("COLUMNS");
    for (name, value) in env {
        match value {
            Some(v) => {
                cmd.env(name, v);
            }
            None => {
                cmd.env_remove(name);
            }
        }
    }
    let output = cmd.output().expect("run rubash");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<String> = stdout.lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 8 + 39, "header + row count: {lines:?}");
    assert_eq!(
        lines[1],
        "These shell commands are defined internally.  Type `help' to see this list."
    );
    lines[8..].to_vec()
}

fn assert_rows(actual: &[String], expected: &[&str], grid: &str) {
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(actual, expected, "help listing rows differ at {grid}");
}

const EXPECTED_C_80: &[&str] = &[
    " ! PIPELINE                              help [-dms] [pattern ...]",
    " job_spec [&]                            history [-c] [-d offset] [n] or hist>",
    " (( expression ))                        if COMMANDS; then COMMANDS; [ elif C>",
    " . [-p path] filename [arguments]        jobs [-lnprs] [jobspec ...] or jobs >",
    " :                                       kill [-s sigspec | -n signum | -sigs>",
    " [ arg... ]                              let arg [arg ...]",
    " [[ expression ]]                        local [option] name[=value] ...",
    " alias [-p] [name[=value] ... ]          logout [n]",
    " bg [job_spec ...]                       mapfile [-d delim] [-n count] [-O or>",
    " bind [-lpsvPSVX] [-m keymap] [-f file>  popd [-n] [+N | -N]",
    " break [n]                               printf [-v var] format [arguments]",
    " builtin [shell-builtin [arg ...]]       pushd [-n] [+N | -N | dir]",
    " caller [expr]                           pwd [-LP]",
    " case WORD in [PATTERN [| PATTERN]...)>  read [-Eers] [-a array] [-d delim] [>",
    " cd [-L|[-P [-e]]] [-@] [dir]            readarray [-d delim] [-n count] [-O >",
    " command [-pVv] command [arg ...]        readonly [-aAf] [name[=value] ...] o>",
    " compgen [-V varname] [-abcdefgjksuv] >  return [n]",
    " complete [-abcdefgjksuv] [-pr] [-DEI]>  select NAME [in WORDS ... ;] do COMM>",
    " compopt [-o|+o option] [-DEI] [name .>  set [-abefhkmnptuvxBCEHPT] [-o optio>",
    " continue [n]                            shift [n]",
    " coproc [NAME] command [redirections]    shopt [-pqsu] [-o] [optname ...]",
    " declare [-aAfFgiIlnrtux] [name[=value>  source [-p path] filename [arguments>",
    " dirs [-clpv] [+N] [-N]                  suspend [-f]",
    " disown [-h] [-ar] [jobspec ... | pid >  test [expr]",
    " echo [-neE] [arg ...]                   time [-p] pipeline",
    " enable [-a] [-dnps] [-f filename] [na>  times",
    " env [-i] [-u name] [name=value ...] [>  trap [-Plp] [[action] signal_spec ..>",
    " eval [arg ...]                          true",
    " exec [-cl] [-a name] [command [argume>  type [-afptP] name [name ...]",
    " exit [n]                                typeset [-aAfFgiIlnrtux] name[=value>",
    " export [-fn] [name[=value] ...] or ex>  ulimit [-SHabcdefiklmnpqrstuvxPRT] [>",
    " false                                   umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] o>  unalias [-a] name [name ...]",
    " fg [job_spec]                           unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMAND>  until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMAN>  variables - Names and meanings of so>",
    " function name { COMMANDS ; } or name >  wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]        while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name >  { COMMANDS ; }",
];
const EXPECTED_C_120: &[&str] = &[
    " ! PIPELINE                                                  help [-dms] [pattern ...]",
    " job_spec [&]                                                history [-c] [-d offset] [n] or history -anrw [filename]>",
    " (( expression ))                                            if COMMANDS; then COMMANDS; [ elif COMMANDS; then COMMAN>",
    " . [-p path] filename [arguments]                            jobs [-lnprs] [jobspec ...] or jobs -x command [args]",
    " :                                                           kill [-s sigspec | -n signum | -sigspec] pid | jobspec .>",
    " [ arg... ]                                                  let arg [arg ...]",
    " [[ expression ]]                                            local [option] name[=value] ...",
    " alias [-p] [name[=value] ... ]                              logout [n]",
    " bg [job_spec ...]                                           mapfile [-d delim] [-n count] [-O origin] [-s count] [-t>",
    " bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u >  popd [-n] [+N | -N]",
    " break [n]                                                   printf [-v var] format [arguments]",
    " builtin [shell-builtin [arg ...]]                           pushd [-n] [+N | -N | dir]",
    " caller [expr]                                               pwd [-LP]",
    " case WORD in [PATTERN [| PATTERN]...) COMMANDS ;;]... esa>  read [-Eers] [-a array] [-d delim] [-i text] [-n nchars]>",
    " cd [-L|[-P [-e]]] [-@] [dir]                                readarray [-d delim] [-n count] [-O origin] [-s count] [>",
    " command [-pVv] command [arg ...]                            readonly [-aAf] [name[=value] ...] or readonly -p",
    " compgen [-V varname] [-abcdefgjksuv] [-o option] [-A acti>  return [n]",
    " complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A act>  select NAME [in WORDS ... ;] do COMMANDS; done",
    " compopt [-o|+o option] [-DEI] [name ...]                    set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [ar>",
    " continue [n]                                                shift [n]",
    " coproc [NAME] command [redirections]                        shopt [-pqsu] [-o] [optname ...]",
    " declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p>  source [-p path] filename [arguments]",
    " dirs [-clpv] [+N] [-N]                                      suspend [-f]",
    " disown [-h] [-ar] [jobspec ... | pid ...]                   test [expr]",
    " echo [-neE] [arg ...]                                       time [-p] pipeline",
    " enable [-a] [-dnps] [-f filename] [name ...]                times",
    " env [-i] [-u name] [name=value ...] [command [argument ..>  trap [-Plp] [[action] signal_spec ...]",
    " eval [arg ...]                                              true",
    " exec [-cl] [-a name] [command [argument ...]] [redirectio>  type [-afptP] name [name ...]",
    " exit [n]                                                    typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p >",
    " export [-fn] [name[=value] ...] or export -p [-f]           ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]",
    " false                                                       umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [c>  unalias [-a] name [name ...]",
    " fg [job_spec]                                               unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMANDS; done                until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMANDS; done               variables - Names and meanings of some shell variables",
    " function name { COMMANDS ; } or name () { COMMANDS ; }      wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]                            while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name ...]                   { COMMANDS ; }",
];
const EXPECTED_UTF8_80: &[&str] = &[
    " ! PIPELINE                              help [-dms] [pattern ...]",
    " job_spec [&]                            history [-c] [-d offset] [n] or hist>",
    " (( expression ))                        if COMMANDS; then COMMANDS; [ elif C>",
    " . [-p path] filename [arguments]        jobs [-lnprs] [jobspec ...] or jobs >",
    " :                                       kill [-s sigspec | -n signum | -sigs>",
    " [ arg... ]                              let arg [arg ...]",
    " [[ expression ]]                        local [option] name[=value] ...",
    " alias [-p] [name[=value] ... ]          logout [n]",
    " bg [job_spec ...]                       mapfile [-d delim] [-n count] [-O or>",
    " bind [-lpsvPSVX] [-m keymap] [-f file>  popd [-n] [+N | -N]",
    " break [n]                               printf [-v var] format [arguments]",
    " builtin [shell-builtin [arg ...]]       pushd [-n] [+N | -N | dir]",
    " caller [expr]                           pwd [-LP]",
    " case WORD in [PATTERN [| PATTERN]...)>  read [-Eers] [-a array] [-d delim] [>",
    " cd [-L|[-P [-e]]] [-@] [dir]            readarray [-d delim] [-n count] [-O >",
    " command [-pVv] command [arg ...]        readonly [-aAf] [name[=value] ...] o>",
    " compgen [-V varname] [-abcdefgjksuv] >  return [n]",
    " complete [-abcdefgjksuv] [-pr] [-DEI]>  select NAME [in WORDS ... ;] do COMM>",
    " compopt [-o|+o option] [-DEI] [name .>  set [-abefhkmnptuvxBCEHPT] [-o optio>",
    " continue [n]                            shift [n]",
    " coproc [NAME] command [redirections]    shopt [-pqsu] [-o] [optname ...]",
    " declare [-aAfFgiIlnrtux] [name[=value>  source [-p path] filename [argument>",
    " dirs [-clpv] [+N] [-N]                  suspend [-f]",
    " disown [-h] [-ar] [jobspec ... | pid >  test [expr]",
    " echo [-neE] [arg ...]                   time [-p] pipeline",
    " enable [-a] [-dnps] [-f filename] [na>  times",
    " env [-i] [-u name] [name=value ...] [>  trap [-Plp] [[action] signal_spec ..>",
    " eval [arg ...]                          true",
    " exec [-cl] [-a name] [command [argume>  type [-afptP] name [name ...]",
    " exit [n]                                typeset [-aAfFgiIlnrtux] name[=value>",
    " export [-fn] [name[=value] ...] or ex>  ulimit [-SHabcdefiklmnpqrstuvxPRT] [>",
    " false                                   umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] o>  unalias [-a] name [name ...]",
    " fg [job_spec]                           unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMAND>  until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMAN>  variables - Names and meanings of so>",
    " function name { COMMANDS ; } or name >  wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]        while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name >  { COMMANDS ; }",
];
const EXPECTED_UTF8_120: &[&str] = &[
    " ! PIPELINE                                                  help [-dms] [pattern ...]",
    " job_spec [&]                                                history [-c] [-d offset] [n] or history -anrw [filename]>",
    " (( expression ))                                            if COMMANDS; then COMMANDS; [ elif COMMANDS; then COMMAN>",
    " . [-p path] filename [arguments]                            jobs [-lnprs] [jobspec ...] or jobs -x command [args]",
    " :                                                           kill [-s sigspec | -n signum | -sigspec] pid | jobspec .>",
    " [ arg... ]                                                  let arg [arg ...]",
    " [[ expression ]]                                            local [option] name[=value] ...",
    " alias [-p] [name[=value] ... ]                              logout [n]",
    " bg [job_spec ...]                                           mapfile [-d delim] [-n count] [-O origin] [-s count] [-t>",
    " bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u >  popd [-n] [+N | -N]",
    " break [n]                                                   printf [-v var] format [arguments]",
    " builtin [shell-builtin [arg ...]]                           pushd [-n] [+N | -N | dir]",
    " caller [expr]                                               pwd [-LP]",
    " case WORD in [PATTERN [| PATTERN]...) COMMANDS ;;]... esa>  read [-Eers] [-a array] [-d delim] [-i text] [-n nchars]>",
    " cd [-L|[-P [-e]]] [-@] [dir]                                readarray [-d delim] [-n count] [-O origin] [-s count] [>",
    " command [-pVv] command [arg ...]                            readonly [-aAf] [name[=value] ...] or readonly -p",
    " compgen [-V varname] [-abcdefgjksuv] [-o option] [-A acti>  return [n]",
    " complete [-abcdefgjksuv] [-pr] [-DEI] [-o option] [-A act>  select NAME [in WORDS ... ;] do COMMANDS; done",
    " compopt [-o|+o option] [-DEI] [name ...]                    set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [ar>",
    " continue [n]                                                shift [n]",
    " coproc [NAME] command [redirections]                        shopt [-pqsu] [-o] [optname ...]",
    " declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p>  source [-p path] filename [arguments]",
    " dirs [-clpv] [+N] [-N]                                      suspend [-f]",
    " disown [-h] [-ar] [jobspec ... | pid ...]                   test [expr]",
    " echo [-neE] [arg ...]                                       time [-p] pipeline",
    " enable [-a] [-dnps] [-f filename] [name ...]                times",
    " env [-i] [-u name] [name=value ...] [command [argument ..>  trap [-Plp] [[action] signal_spec ...]",
    " eval [arg ...]                                              true",
    " exec [-cl] [-a name] [command [argument ...]] [redirectio>  type [-afptP] name [name ...]",
    " exit [n]                                                    typeset [-aAfFgiIlnrtux] name[=value] ... or typeset -p >",
    " export [-fn] [name[=value] ...] or export -p [-f]           ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]",
    " false                                                       umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] or fc -s [pat=rep] [c>  unalias [-a] name [name ...]",
    " fg [job_spec]                                               unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMANDS; done                until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMANDS; done               variables - Names and meanings of some shell variables",
    " function name { COMMANDS ; } or name () { COMMANDS ; }      wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]                            while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name ...]                   { COMMANDS ; }",
];
#[test]
fn help_listing_columns_80_c_locale() {
    let rows = run_help("LC_ALL=C; COLUMNS=80; help", &[]);
    assert_rows(&rows, EXPECTED_C_80, "LC_ALL=C COLUMNS=80");
}

#[test]
fn help_listing_columns_120_c_locale() {
    let rows = run_help("LC_ALL=C; COLUMNS=120; help", &[]);
    assert_rows(&rows, EXPECTED_C_120, "LC_ALL=C COLUMNS=120");
}

#[test]
fn help_listing_columns_80_utf8_locale() {
    let rows = run_help("LC_ALL=C.UTF-8; COLUMNS=80; help", &[]);
    assert_rows(&rows, EXPECTED_UTF8_80, "LC_ALL=C.UTF-8 COLUMNS=80");
}

#[test]
fn help_listing_columns_120_utf8_locale() {
    let rows = run_help("LC_ALL=C.UTF-8; COLUMNS=120; help", &[]);
    assert_rows(&rows, EXPECTED_UTF8_120, "LC_ALL=C.UTF-8 COLUMNS=120");
}

#[test]
fn help_listing_columns_from_process_env() {
    // COLUMNS arriving through the process environment (imported into
    // the shell variable table at startup) drives the layout the same
    // way as an in-script assignment.
    let rows = run_help("LC_ALL=C.UTF-8; help", &[("COLUMNS", Some("120"))]);
    assert_rows(&rows, EXPECTED_UTF8_120, "env COLUMNS=120 LC_ALL=C.UTF-8");
}

#[test]
fn help_listing_without_columns_uses_80() {
    // No COLUMNS anywhere and no usable window size (piped io): GNU
    // default_columns falls back to 80.
    let rows = run_help("LC_ALL=C; help", &[]);
    assert_rows(&rows, EXPECTED_C_80, "fallback COLUMNS=80 LC_ALL=C");
}
