//! rubash#2700 — filtered `declare` listings diverged for dynamic
//! variables: `declare -i` omitted BASHPID/HISTCMD/RANDOM/SRANDOM,
//! `declare -a` omitted GROUPS, and `declare -a` rendered a phantom
//! `BASH_LINENO=([0]="0")` from a stale cell while `declare -p
//! BASH_LINENO` showed `()`.
//!
//! GNU ground: the dynamic variables are real cells in the variable
//! table (variables.c:1844 initialize_dynamic_variables), so
//! print_var_list enumerates them like any other variable and the
//! attribute filters adjudicate the cells' own attributes — the
//! integer dynamics carry att_integer (variables.c:1858-1875), GROUPS
//! is an array cell, and the provider scalars carry att_invisible,
//! which print forms (-p and the attribute-filtered listings) include
//! while a bare `declare` omits them.
//!
//! Rubash's provider owns those names outside the table, so the
//! listing paths now consult the dynamic registry: the executor
//! supplies DynamicDeclarationView entries (rendered text + the
//! integer/array class from the same per-name table the view renderer
//! uses) for the whole registry in listing mode, and the builtin's
//! merge adjudicates them by that class instead of mark sets the
//! dynamics never carry.
//!
//! One documented exclusion from the byte-compared name sets, a
//! pre-existing face outside this finding: ours lists THIS_SH (a
//! pending product decision). (A second exclusion used to cover
//! OLDPWD, which ours did not surface in unfiltered listings; the
//! rubash#2972 candidate-chain completion lists the startup
//! exported-unset cell exactly like GNU, so the exclusion is gone.)
//!
//! Expectations byte-verified against the GNU bash 5.3.20 oracle with
//! script-file and -c probes.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const RUBASH: &str = env!("CARGO_BIN_EXE_rubash");

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2700-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_script(dir: &Path, script: &str) -> String {
    let script_path = dir.join("probe.sh");
    fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(RUBASH)
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", dir)
        .output()
        .expect("run rubash");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr for {script:?}"
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The name set of a listing: strip the `declare -flags ` prefix (or,
/// for the bare assignment form, nothing) and any `=value` tail.
fn listing_names(listing: &str) -> BTreeSet<String> {
    listing
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("declare ").unwrap_or(line);
            let rest = if line.starts_with("declare ") {
                rest.split_once(' ').map(|(_, tail)| tail).unwrap_or(rest)
            } else {
                rest
            };
            let name = rest.split([' ', '=']).next()?;
            Some(name.to_string())
        })
        .collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|name| name.to_string()).collect()
}

#[test]
fn issue2700_filtered_listings_cover_the_dynamic_registry() {
    let dir = scratch("filtered");
    // D1: declare -i — GNU lists BASHPID EUID HISTCMD OPTIND PPID
    // RANDOM SRANDOM UID, plus SECONDS only once SECONDS has been
    // read: GNU's att_integer on SECONDS is LAZY (get_seconds earns
    // it via set_int_value, variables.c:1294-1295), while
    // `declare -p SECONDS` renders -i unconditionally. This pin
    // originally modeled the class statically (SECONDS always
    // listed); rubash#2973 replaced that with the stateful cell
    // model, so the unread SECONDS this script lists is absent —
    // the GNU shape, byte-verified against the oracle.
    let out = run_script(&dir, "declare -i\n");
    assert_eq!(
        listing_names(&out),
        names(&["BASHPID", "EUID", "HISTCMD", "OPTIND", "PPID", "RANDOM", "SRANDOM", "UID"]),
        "declare -i listing: {out}"
    );
    // D2 + D3: declare -a — GROUPS present, BASH_LINENO renders its
    // live composition (script mode's main frame: ([0]="0")), never a
    // stale cell value that disagrees with the other listing forms.
    let out = run_script(&dir, "declare -a\n");
    assert_eq!(
        listing_names(&out),
        names(&[
            "BASH_ARGC",
            "BASH_ARGV",
            "BASH_LINENO",
            "BASH_SOURCE",
            "BASH_VERSINFO",
            "DIRSTACK",
            "FUNCNAME",
            "GROUPS",
        ]),
        "declare -a listing: {out}"
    );
    assert!(
        out.lines()
            .any(|line| line.starts_with("declare -a GROUPS=(")),
        "GROUPS line in declare -a: {out}"
    );
    assert!(
        out.lines()
            .any(|line| line == "declare -a BASH_LINENO=([0]=\"0\")"),
        "BASH_LINENO line in declare -a: {out}"
    );
    // An exported dynamic passes the -x filter by its mark; a
    // non-exported one stays out.
    let out = run_script(&dir, "export RANDOM\ndeclare -x\n");
    assert!(
        out.lines()
            .any(|line| line.starts_with("declare -ix RANDOM=")),
        "exported RANDOM in declare -x: {out}"
    );
    assert!(
        !listing_names(&out).contains("SECONDS"),
        "SECONDS must not appear in declare -x: {out}"
    );
}

#[test]
fn issue2700_full_listings_match_gnus_registry_visibility() {
    let dir = scratch("full");
    // declare -p: the invisible provider scalars join the listing.
    let out = run_script(&dir, "declare -p\n");
    let mut got = listing_names(&out);
    got.remove("THIS_SH"); // documented exclusion (decision item)
    assert_eq!(
        got,
        names(&[
            "BASH",
            "BASHOPTS",
            "BASHPID",
            "BASH_ALIASES",
            "BASH_ARGC",
            "BASH_ARGV",
            "BASH_ARGV0",
            "BASH_CMDS",
            "BASH_COMMAND",
            "BASH_LINENO",
            "BASH_LOADABLES_PATH",
            "BASH_MONOSECONDS",
            "BASH_SOURCE",
            "BASH_SUBSHELL",
            "BASH_VERSINFO",
            "BASH_VERSION",
            "COMP_WORDBREAKS",
            "DIRSTACK",
            "EPOCHREALTIME",
            "EPOCHSECONDS",
            "EUID",
            "FUNCNAME",
            "GROUPS",
            "HISTCMD",
            "HOME",
            "HOSTNAME",
            "HOSTTYPE",
            "IFS",
            "LINENO",
            "MACHTYPE",
            "OLDPWD",
            "OPTERR",
            "OPTIND",
            "OSTYPE",
            "PATH",
            "PPID",
            "PS4",
            "PWD",
            "RANDOM",
            "SECONDS",
            "SHELL",
            "SHELLOPTS",
            "SHLVL",
            "SRANDOM",
            "TERM",
            "UID",
            "_",
        ]),
        "declare -p listing: {out}"
    );
    // Bare declare: the visible registry members (GROUPS, the stack
    // arrays) are listed; the invisible provider scalars are not.
    let out = run_script(&dir, "declare\n");
    let got = listing_names(&out);
    for visible in [
        "GROUPS",
        "BASH_ARGC",
        "BASH_ARGV",
        "BASH_LINENO",
        "BASH_SOURCE",
    ] {
        assert!(got.contains(visible), "bare declare lists {visible}: {out}");
    }
    for invisible in [
        "BASHPID",
        "HISTCMD",
        "RANDOM",
        "SRANDOM",
        "SECONDS",
        "EPOCHSECONDS",
        "EPOCHREALTIME",
        "BASH_MONOSECONDS",
        "LINENO",
        "BASH_SUBSHELL",
        "BASH_COMMAND",
    ] {
        assert!(
            !got.contains(invisible),
            "bare declare must not list {invisible}: {out}"
        );
    }
    // Bare declare renders the assignment (set) form; the stack array
    // renders its live composition there too, never the stale cell.
    assert!(
        out.lines().any(|line| line == "BASH_LINENO=([0]=\"0\")"),
        "BASH_LINENO line in bare declare: {out}"
    );
    assert!(
        out.lines().any(|line| line.starts_with("GROUPS=(")),
        "GROUPS line in bare declare: {out}"
    );
}
