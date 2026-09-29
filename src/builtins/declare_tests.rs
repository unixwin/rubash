use super::names::valid_declare_name;

use super::execute_with_io_named;

#[test]
fn invalid_declare_names_are_rejected_before_assignment() {
    assert!(!valid_declare_name("[]=asdf"));
    assert!(!valid_declare_name("a[]=asdf"));
    assert!(!valid_declare_name("=asdf"));
    assert!(valid_declare_name("BASH_ARGV[1]=foo"));
    assert!(valid_declare_name("name=value"));
    assert!(valid_declare_name("name+=value"));
}

#[test]
fn capcase_attribute_transforms_assignments_and_prints() {
    let mut variables = crate::shell::var_table::VarTable::default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    execute_with_io_named(
        "declare",
        &["-c".into(), "name=HeLLo WoRLD".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(
        variables.get("name").map(String::as_str),
        Some("Hello world")
    );
    stdout.clear();
    execute_with_io_named(
        "declare",
        &["-p".into(), "name".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "declare -c name=\"Hello world\"\n"
    );
}

// GNU variables.c:511-526 (initialize_shell_variables) + 5064-5117
// (maybe_make_export_env): environment entries whose names are not valid
// identifiers (Windows inherits `CommonProgramFiles(x86)`-style names) are
// bound into the invisible invalid_env table instead of shell_variables, so
// `export -p` iterates shell variables and never prints them — while the
// entries still reach child processes (niubash issue #102).
#[test]
fn export_p_listing_skips_invalid_identifier_names() {
    let mut variables = crate::shell::var_table::VarTable::default();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    execute_with_io_named(
        "declare",
        &["-x".into(), "VALID=kept".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert!(variables.contains_key("VALID"));

    // GNU export.def: exporting an invalid identifier reports the standard
    // error and binds nothing.
    let status = execute_with_io_named(
        "export",
        &["CommonProgramFiles(x86)=C:/Program Files (x86)".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert!(!variables.contains_key("CommonProgramFiles(x86)"));
    assert!(String::from_utf8_lossy(&stderr).contains("not a valid identifier"));
    assert_eq!(status, super::EXECUTION_FAILURE);
    stderr.clear();

    execute_with_io_named(
        "declare",
        &["-p".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    let listing = String::from_utf8(stdout).unwrap();
    assert!(listing.contains("declare -x VALID=\"kept\"\n"), "{listing}");
    assert!(!listing.contains("CommonProgramFiles"), "{listing}");
}

// GNU declare.def (bash 5.3.0 probe): `declare -p 'X(BR)'` reports
// `X(BR): not found` (the identifier check gates attribute changes, not
// display) and prints nothing — it never falls back to the no-operand full
// listing (niubash issue #102).
#[test]
fn declare_p_rejected_operand_prints_error_not_full_listing() {
    let mut variables = crate::shell::var_table::VarTable::default();
    variables.insert("VALID".to_string(), "kept".to_string());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = execute_with_io_named(
        "declare",
        &["-p".into(), "X(BR)".into()],
        &mut variables,
        &mut stdout,
        &mut stderr,
    )
    .unwrap();
    assert!(stdout.is_empty(), "{}", String::from_utf8_lossy(&stdout));
    assert!(String::from_utf8(stderr)
        .unwrap()
        .contains("X(BR): not found"));
    assert_eq!(status, super::EXECUTION_FAILURE);
}
