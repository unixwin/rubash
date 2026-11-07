//! rubash#2973: `declare`/`typeset` LISTINGS rendered the dynamic
//! variables' LIVE values, where GNU renders the variable CELL —
//! the value the last by-name fetch materialized, or a bare name
//! when the cell was never fetched.
//!
//! GNU mechanism: variables.c initialize_dynamic_variables registers
//! each dynamic as a hash-table cell whose dynamic_value getter runs
//! from find_variable_internal (variables.c:1175-1189) — by-NAME
//! lookups only. Enumeration (print_var_list / show_var_attributes
//! over all_shell_variables) never calls the getter, so a listing
//! shows the raw cell: unread BASHPID/RANDOM/SECONDS list bare
//! (SECONDS without att_integer — get_seconds earns it lazily via
//! set_int_value, variables.c:1294-1295), GROUPS lists as the empty
//! array cell until get_groupset regenerates it on a fetch, and the
//! set form omits valueless cells entirely (print_assignment) while
//! skipping att_invisible cells like BASH_ARGV0 (print_var_list,
//! variables.c:1066-1075).
//!
//! Pre-fix rubash built listing views from the live providers:
//! listings printed `declare -i BASHPID="..."`, `declare -i
//! SECONDS="0"`, `declare -a GROUPS=([0]="0")`, and the set form
//! carried an extra `BASH_ARGV0=...` line and no PIPESTATUS.
//!
//! Expectations measured against GNU bash 5.3.20 (oracle binary),
//! script probes 2026-11-07.

use std::process::{Command, Stdio};
use std::time::Duration;

mod common;
use common::wait_bounded;

fn run_script(body: &str) -> (String, String, i32, u32) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue2973-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let script = dir.join("probe.sh");
    std::fs::write(&script, body).expect("write script");
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("TERM", "xterm")
        .env("HOME", &dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rubash");
    let output = wait_bounded(&mut child, Duration::from_secs(20));
    assert!(!output.timed_out, "probe timed out");
    let result = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.code.unwrap_or(-1),
        output.pid,
    );
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn line_for<'a>(stdout: &'a str, name: &str) -> Option<&'a str> {
    // The name is the first token after the `declare` flag tokens
    // (array values may contain spaces, so it is never the last one).
    // Set-form lines (`NAME=...`) match on their leading name too.
    stdout.lines().find(|line| {
        if let Some(rest) = line.strip_prefix("declare ") {
            let Some(token) = rest
                .split_whitespace()
                .find(|token| !token.starts_with('-') && !token.starts_with('+'))
            else {
                return false;
            };
            token.split(['=', '(']).next() == Some(name)
        } else {
            line.split(['=', '(']).next() == Some(name)
                && line.contains('=')
                && !line.starts_with("declare")
        }
    })
}

fn listing_line(body: &str, name: &str) -> String {
    let (stdout, stderr, status, _) = run_script(body);
    assert_eq!(status, 0, "{body}: {stderr}");
    line_for(&stdout, name)
        .unwrap_or_else(|| panic!("no listing line for {name} in:\n{stdout}"))
        .to_string()
}

#[test]
fn unread_dynamics_list_bare() {
    let (stdout, stderr, status, _) = run_script("declare -p\n");
    assert_eq!(status, 0, "{stderr}");
    for (name, expected) in [
        ("BASHPID", "declare -i BASHPID"),
        ("RANDOM", "declare -i RANDOM"),
        ("SRANDOM", "declare -i SRANDOM"),
        ("HISTCMD", "declare -i HISTCMD"),
        ("SECONDS", "declare -- SECONDS"),
        ("EPOCHSECONDS", "declare -- EPOCHSECONDS"),
        ("EPOCHREALTIME", "declare -- EPOCHREALTIME"),
        ("BASH_MONOSECONDS", "declare -- BASH_MONOSECONDS"),
        ("BASH_SUBSHELL", "declare -- BASH_SUBSHELL"),
        ("LINENO", "declare -- LINENO"),
        ("BASH_ARGV0", "declare -- BASH_ARGV0"),
        ("BASH_COMMAND", "declare -- BASH_COMMAND"),
        ("GROUPS", "declare -a GROUPS=()"),
        // att_invisible table cell, same mechanism class as
        // BASH_ARGV0: listings render the name only.
        ("COMP_WORDBREAKS", "declare -- COMP_WORDBREAKS"),
    ] {
        assert_eq!(
            line_for(&stdout, name),
            Some(expected),
            "listing line for {name}"
        );
    }
}

#[test]
fn read_dynamics_list_materialized() {
    // A by-name fetch materializes the cell; the listing then shows
    // the fetched value — for RANDOM the SAME draw the script saw
    // (the getter does not run again for the listing).
    let (stdout, stderr, status, pid) =
        run_script("x=$RANDOM\necho \"DRAWN=$x\"\np=$BASHPID\ns=$SECONDS\ndeclare -p\n");
    assert_eq!(status, 0, "{stderr}");
    let drawn: String = stdout
        .lines()
        .find_map(|line| line.strip_prefix("DRAWN="))
        .expect("DRAWN marker")
        .to_string();
    assert_eq!(
        line_for(&stdout, "RANDOM"),
        Some(format!("declare -i RANDOM=\"{drawn}\"").as_str())
    );
    assert_eq!(
        line_for(&stdout, "BASHPID"),
        Some(format!("declare -i BASHPID=\"{pid}\"").as_str())
    );
    let seconds = line_for(&stdout, "SECONDS").expect("SECONDS line");
    assert!(
        seconds.starts_with("declare -i SECONDS=\""),
        "SECONDS earned its integer attribute on fetch: {seconds}"
    );
}

#[test]
fn lineno_lists_the_value_from_its_last_fetch() {
    // $LINENO fetched on line 1; the listing on line 5 still shows 1.
    let line = listing_line("y=$LINENO\n\n\n\ndeclare -p\n", "LINENO");
    assert_eq!(line, "declare -- LINENO=\"1\"");
}

#[test]
fn integer_listing_excludes_unread_seconds() {
    let (stdout, stderr, status, _) = run_script("declare -i\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(line_for(&stdout, "SECONDS").is_none(), "{stdout}");
    assert_eq!(line_for(&stdout, "BASHPID"), Some("declare -i BASHPID"));
}

#[test]
fn groups_listing_materializes_after_element_fetch() {
    let line = listing_line("g=${GROUPS[0]}\ndeclare -p\n", "GROUPS");
    assert!(
        line.starts_with("declare -a GROUPS=([0]=\""),
        "GROUPS regenerated after fetch: {line}"
    );
}

#[test]
fn set_form_uses_cell_shapes() {
    let (stdout, stderr, status, _) = run_script("true\ndeclare\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("GROUPS=()\n"), "{stdout}");
    assert!(stdout.contains("PIPESTATUS=([0]=\"0\")\n"), "{stdout}");
    assert!(
        !stdout.lines().any(|line| line.starts_with("BASH_ARGV0=")),
        "invisible BASH_ARGV0 stays out of the set form:\n{stdout}"
    );
    assert!(
        !stdout.lines().any(|line| line.starts_with("SECONDS=")),
        "valueless SECONDS is omitted from the set form:\n{stdout}"
    );
}

#[test]
fn set_form_groups_materializes_after_fetch() {
    let (stdout, stderr, status, _) = run_script("g=${GROUPS[0]}\nset\n");
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("GROUPS=([0]=\""), "{stdout}");
}

#[test]
fn named_views_still_render_live_values() {
    // Controls: by-name `declare -p NAME` fires the provider — these
    // already matched GNU before the fix and must not regress.
    let (stdout, stderr, status, pid) = run_script("declare -p BASHPID\n");
    assert_eq!(status, 0, "{stderr}");
    assert_eq!(
        line_for(&stdout, "BASHPID"),
        Some(format!("declare -i BASHPID=\"{pid}\"").as_str())
    );
    let line = listing_line("declare -p GROUPS\n", "GROUPS");
    assert!(line.starts_with("declare -a GROUPS=([0]=\""), "{line}");
    let line = listing_line("declare -p SECONDS\n", "SECONDS");
    assert!(line.starts_with("declare -i SECONDS=\""), "{line}");
}
