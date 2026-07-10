use super::*;
use std::collections::HashMap;

fn env_map() -> HashMap<String, String> {
    HashMap::new()
}

#[test]
fn exports_assignment() {
    let mut vars = env_map();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let status = export_with_io(["NAME=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

    assert_eq!(status, EXECUTION_SUCCESS);
    assert_eq!(vars.get("NAME"), Some(&"value".to_string()));
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn rejects_invalid_identifier() {
    let mut vars = env_map();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let status = export_with_io(["1BAD=value"], &mut vars, &mut stdout, &mut stderr).unwrap();

    assert_eq!(status, EXECUTION_FAILURE);
    assert!(String::from_utf8(stderr)
        .unwrap()
        .contains("not a valid identifier"));
}

#[test]
fn prints_exported_variables() {
    let mut vars = env_map();
    vars.insert("NAME".to_string(), "value".to_string());
    vars.insert(EXPORTED_VARS.to_string(), "NAME".to_string());
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let status = export_with_io(["-p"], &mut vars, &mut stdout, &mut stderr).unwrap();

    assert_eq!(status, EXECUTION_SUCCESS);
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "declare -x NAME=\"value\"\n"
    );
    assert!(stderr.is_empty());
}
