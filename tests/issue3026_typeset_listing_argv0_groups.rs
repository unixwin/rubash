//! rubash#3026: the no-argument `typeset` (and `set`) listing
//! showed `BASH_ARGV0=<script path>` and rendered
//! `GROUPS=([0]="0")`; GNU shows neither in a pristine listing.
//!
//! GNU ground truth (bash 5.3.20, verified by script-file probes):
//! the listing walks `print_var_list` (variables.c:1066) ->
//! `print_assignment` (variables.c:1107), which returns early for a
//! variable that is not set. `BASH_ARGV0` is a dynamic variable
//! (variables.c:1850 INIT_DYNAMIC_VAR with a NULL value cell and
//! get_bash_argv0 as its dynamic_value), so it stays unset — and
//! unlisted — until something references it BY NAME, at which point
//! the fetched value is bound and sticks. `GROUPS` is a dynamic
//! array (variables.c:1886 init_dynamic_array_var, get_groupset);
//! unmaterialized, the listing's array printer renders `GROUPS=()`;
//! a by-name reference (`declare -p GROUPS`) materializes it and
//! the materialization sticks into later listings. The defect was
//! eager materialization of both dynamic variables on the
//! startup/listing path; the regression lock below pins BOTH the
//! pristine faces and the materialization chain, so an
//! over-correction that hides the variables forever also fails.
//!
//! Environment note: the chain cell's `GROUPS=([0]="0")` is this
//! test environment's process-group shape (egid 0), the same shape
//! the captain's verification probes recorded; only the first
//! element is asserted, and the listing faces are shape-free.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_driver(tag: &str, files: &[(&str, &str)], driver: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("rubash-3026-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for (name, body) in files {
        let mut file = std::fs::File::create(dir.join(name)).expect("write fixture");
        file.write_all(body.as_bytes()).expect("write fixture");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(driver)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn rubash");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

/// Pristine `typeset` listing: no BASH_ARGV0 line at all, and
/// GROUPS renders as the unmaterialized `()`.
#[test]
fn typeset_listing_omits_bash_argv0_and_shows_unmaterialized_groups() {
    let (stdout, stderr, rc) = run_driver(
        "typeset",
        &[(
            "t1.sh",
            "typeset | grep ^BASH_ARGV0=\necho \"argv0_rc=$?\"\ntypeset | grep ^GROUPS=\n",
        )],
        "t1.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "argv0_rc=1\nGROUPS=()\n");
}

/// The `set` builtin's no-argument listing shares the print path
/// and shows the same two faces.
#[test]
fn set_listing_omits_bash_argv0_and_shows_unmaterialized_groups() {
    let (stdout, stderr, rc) = run_driver(
        "set",
        &[(
            "t2.sh",
            "set | grep ^GROUPS=\nset | grep ^BASH_ARGV0=\necho \"argv0_rc=$?\"\n",
        )],
        "t2.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(stdout, "GROUPS=()\nargv0_rc=1\n");
}

/// Chain: a by-name reference materializes the dynamic value and
/// the materialization sticks — later listings show the real
/// entries. Each step's state feeds the next.
#[test]
fn named_reference_materializes_groups_and_bash_argv0_into_later_listings() {
    let (stdout, stderr, rc) = run_driver(
        "chain",
        &[(
            "chain.sh",
            "declare -p GROUPS\ntypeset | grep ^GROUPS=\ndeclare -p BASH_ARGV0\ntypeset | grep ^BASH_ARGV0=\n",
        )],
        "chain.sh",
    );
    assert_eq!(rc, Some(0));
    assert_eq!(stderr, "");
    assert_eq!(
        stdout,
        "declare -a GROUPS=([0]=\"0\")\nGROUPS=([0]=\"0\")\ndeclare -- BASH_ARGV0=\"chain.sh\"\nBASH_ARGV0=chain.sh\n"
    );
}
