use std::ffi::OsStr;
use std::path::PathBuf;

#[path = "support/bashdb_fixture.rs"]
mod bashdb_fixture;

fn missing_launcher() -> PathBuf {
    let path = std::env::temp_dir()
        .join(format!(
            "rubash-615-missing-launcher-{}",
            std::process::id()
        ))
        .join("bashdb-generated");
    assert!(!path.try_exists().expect("check missing test launcher"));
    path
}

#[test]
fn optional_missing_launcher_is_not_coverage() {
    assert!(!bashdb_fixture::launcher_available(
        &missing_launcher(),
        None
    ));
}

#[test]
#[should_panic(
    expected = "RUBASH_REQUIRE_BASHDB_FIXTURE=1: required external bashdb launcher missing"
)]
fn strict_missing_launcher_fails() {
    bashdb_fixture::launcher_available(&missing_launcher(), Some(OsStr::new("1")));
}

#[test]
fn strict_present_launcher_reaches_caller() {
    assert!(bashdb_fixture::launcher_available(
        std::path::Path::new("tests/fixtures/bashdb/probe-target.sh"),
        Some(OsStr::new("1")),
    ));
}

#[test]
fn only_one_enables_strict_requirement() {
    assert!(!bashdb_fixture::launcher_available(
        &missing_launcher(),
        Some(OsStr::new("0")),
    ));
}
