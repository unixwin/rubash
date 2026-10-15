use std::ffi::OsStr;
use std::path::Path;

pub fn launcher_available(launcher: &Path, requirement: Option<&OsStr>) -> bool {
    if launcher
        .try_exists()
        .expect("check external bashdb launcher")
    {
        return true;
    }
    assert!(
        requirement != Some(OsStr::new("1")),
        "RUBASH_REQUIRE_BASHDB_FIXTURE=1: required external bashdb launcher missing: {}; run scripts/setup-bashdb-fixture.sh /path/to/bashdb",
        launcher.display()
    );
    eprintln!(
        "SKIP: external bashdb launcher missing; no bashdb coverage; run scripts/setup-bashdb-fixture.sh /path/to/bashdb"
    );
    false
}
