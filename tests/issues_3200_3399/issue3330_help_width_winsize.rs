//! Issue rubash#3330: unix `process_console_size` stub — `help`
//! width ignores the window size when COLUMNS is unset.
//!
//! GNU contract (bash 5.3.20): general.c:1461 default_columns takes
//! COLUMNS when it parses positive, else the window size from
//! get_new_window_size (lib/sh/winsize.c: tcgetwinsize on input_tty(),
//! the controlling terminal else stderr), else 80. The unix query was
//! a stub returning None, so every COLUMNS-unset consumer (bare `help`
//! listing, `select` menus, the interactive LINES/COLUMNS bind) fell
//! back to 80 at any window width.
//!
//! The nail runs bare `help` under a pty whose winsize is set before
//! exec (tests/interactive/support/pty_help_width_probe.py), COLUMNS
//! unset, at two widths (120 and 100 — both != 80), and pins the full
//! 39-row grids. The driver pins LC_ALL=C in the child environment so
//! the single-byte dispcolumn grids below apply on any leg locale: a
//! UTF-8 leg would select the wdispcolumn path instead, which keeps
//! one more character at the truncation boundary (the 100-column
//! `select` row loses its `>` there) — same product, different grid.
//! The grids below are rubash's own 78-entry table
//! rendered by the single-byte dispcolumn path (LC_ALL=C), produced by
//! the renderer in ~/workspace/fix-3330/gen_expected.py, which was
//! validated byte-for-byte against the GNU 5.3.20 oracle's own
//! 77-entry output at COLUMNS=100 and against the oracle-validated
//! #3185 grids at 80/120 (sole diff: the `pwd [-LPW]` row — rubash's
//! deliberate -W extension, pwd.rs / niubash#993 §6 — where the #3185
//! grids still carry GNU's `pwd [-LP]`, cd.def:436).
//!
//! Unix-only: the pty driver exits 2 where no POSIX pty exists and
//! the legs skip cleanly there (the Windows console face is covered
//! by the Windows process_console_size path, not this nail).

#![cfg(unix)]

use std::process::{Command, Stdio};

const DRIVER: &str = "tests/interactive/support/pty_help_width_probe.py";

/// Run bare `help` under a pty of the given width; COLUMNS/LINES are
/// unset inside the driver. Returns None when the leg must skip (no
/// python3, or the driver reports no usable pty).
fn run_help_under_pty(cols: u32) -> Option<Vec<String>> {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let mut cmd = Command::new("python3");
    cmd.arg(format!("{manifest}/{DRIVER}"))
        .arg(env!("CARGO_BIN_EXE_rubash"))
        .arg(cols.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = match cmd.output() {
        Ok(output) => output,
        Err(err) => {
            eprintln!("skip: cannot spawn python3 for the pty driver: {err}");
            return None;
        }
    };
    if output.status.code() == Some(2) {
        eprintln!(
            "skip: no POSIX pty for the width probe: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        return None;
    }
    assert_eq!(
        output.status.code(),
        Some(0),
        "pty driver failed at cols={cols}: stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let lines: Vec<String> = stdout.lines().map(str::to_owned).collect();
    assert_eq!(
        lines.len(),
        8 + 39,
        "header + row count at cols={cols}: {lines:?}"
    );
    assert_eq!(
        lines[1],
        "These shell commands are defined internally.  Type `help' to see this list."
    );
    Some(lines[8..].to_vec())
}

fn assert_rows(actual: &[String], expected: &[&str], grid: &str) {
    let expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(actual, expected, "help listing rows differ at {grid}");
}

const EXPECTED_C_100: &[&str] = &[
    " ! PIPELINE                                        help [-dms] [pattern ...]",
    " job_spec [&]                                      history [-c] [-d offset] [n] or history -anrw >",
    " (( expression ))                                  if COMMANDS; then COMMANDS; [ elif COMMANDS; t>",
    " . [-p path] filename [arguments]                  jobs [-lnprs] [jobspec ...] or jobs -x command>",
    " :                                                 kill [-s sigspec | -n signum | -sigspec] pid |>",
    " [ arg... ]                                        let arg [arg ...]",
    " [[ expression ]]                                  local [option] name[=value] ...",
    " alias [-p] [name[=value] ... ]                    logout [n]",
    " bg [job_spec ...]                                 mapfile [-d delim] [-n count] [-O origin] [-s >",
    " bind [-lpsvPSVX] [-m keymap] [-f filename] [-q >  popd [-n] [+N | -N]",
    " break [n]                                         printf [-v var] format [arguments]",
    " builtin [shell-builtin [arg ...]]                 pushd [-n] [+N | -N | dir]",
    " caller [expr]                                     pwd [-LPW]",
    " case WORD in [PATTERN [| PATTERN]...) COMMANDS >  read [-Eers] [-a array] [-d delim] [-i text] [>",
    " cd [-L|[-P [-e]]] [-@] [dir]                      readarray [-d delim] [-n count] [-O origin] [->",
    " command [-pVv] command [arg ...]                  readonly [-aAf] [name[=value] ...] or readonly>",
    " compgen [-V varname] [-abcdefgjksuv] [-o option>  return [n]",
    " complete [-abcdefgjksuv] [-pr] [-DEI] [-o optio>  select NAME [in WORDS ... ;] do COMMANDS; done>",
    " compopt [-o|+o option] [-DEI] [name ...]          set [-abefhkmnptuvxBCEHPT] [-o option-name] [->",
    " continue [n]                                      shift [n]",
    " coproc [NAME] command [redirections]              shopt [-pqsu] [-o] [optname ...]",
    " declare [-aAfFgiIlnrtux] [name[=value] ...] or >  source [-p path] filename [arguments]",
    " dirs [-clpv] [+N] [-N]                            suspend [-f]",
    " disown [-h] [-ar] [jobspec ... | pid ...]         test [expr]",
    " echo [-neE] [arg ...]                             time [-p] pipeline",
    " enable [-a] [-dnps] [-f filename] [name ...]      times",
    " env [-i] [-u name] [name=value ...] [command [a>  trap [-Plp] [[action] signal_spec ...]",
    " eval [arg ...]                                    true",
    " exec [-cl] [-a name] [command [argument ...]] [>  type [-afptP] name [name ...]",
    " exit [n]                                          typeset [-aAfFgiIlnrtux] name[=value] ... or t>",
    " export [-fn] [name[=value] ...] or export -p [->  ulimit [-SHabcdefiklmnpqrstuvxPRT] [limit]",
    " false                                             umask [-p] [-S] [mode]",
    " fc [-e ename] [-lnr] [first] [last] or fc -s [p>  unalias [-a] name [name ...]",
    " fg [job_spec]                                     unset [-f] [-v] [-n] [name ...]",
    " for NAME [in WORDS ... ] ; do COMMANDS; done      until COMMANDS; do COMMANDS-2; done",
    " for (( exp1; exp2; exp3 )); do COMMANDS; done     variables - Names and meanings of some shell v>",
    " function name { COMMANDS ; } or name () { COMMA>  wait [-fn] [-p var] [id ...]",
    " getopts optstring name [arg ...]                  while COMMANDS; do COMMANDS-2; done",
    " hash [-lr] [-p pathname] [-dt] [name ...]         { COMMANDS ; }",
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
    " caller [expr]                                               pwd [-LPW]",
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
    " caller [expr]                           pwd [-LPW]",
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
/// COLUMNS unset + 120-column window: the listing must be the 120
/// grid, not the 80 fallback (the pre-fix stub printed 80 at every
/// width).
#[test]
fn help_width_follows_winsize_120_when_columns_unset() {
    let Some(rows) = run_help_under_pty(120) else {
        return;
    };
    assert_rows(&rows, EXPECTED_C_120, "winsize 120, COLUMNS unset");
    assert_ne!(
        rows,
        EXPECTED_C_80
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        "120-wide window must not render the 80-column fallback grid"
    );
}

/// COLUMNS unset + 100-column window: a second, distinct width pins
/// that the query reads the live winsize rather than any constant.
#[test]
fn help_width_follows_winsize_100_when_columns_unset() {
    let Some(rows) = run_help_under_pty(100) else {
        return;
    };
    assert_rows(&rows, EXPECTED_C_100, "winsize 100, COLUMNS unset");
    assert_ne!(
        rows,
        EXPECTED_C_80
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        "100-wide window must not render the 80-column fallback grid"
    );
}
