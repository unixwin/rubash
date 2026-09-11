use super::names::valid_declare_name;
use std::collections::HashMap;

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
    let mut variables = HashMap::new();
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
