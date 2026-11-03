//! Deep-audit lane A3 finding R58-B: `type` (and `command -v`) printed an
//! absolute path for slash-bearing names, where GNU echoes the argument
//! verbatim (builtins/type.def describe_command, disk-file branch:
//! absolute_program prints the command as given, no PATH walk).
//!
//! GNU bash 5.3.20 oracle behavior (script-file probes, twice confirmed):
//! - `type ./bin/mycmd` -> `./bin/mycmd is ./bin/mycmd`
//! - `type bin/mycmd`  -> `bin/mycmd is bin/mycmd`
//! - `type -p ./bin/mycmd` and `command -v ./bin/mycmd` print `./bin/mycmd`
//! The pre-fix build printed `<cwd>/./bin/mycmd is <cwd>/./bin/mycmd`.

use std::path::PathBuf;
use std::process::Command;

fn rubash_in(dir: &std::path::Path, script: &str) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .output()
        .expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-a3-r58b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).expect("create bin dir");
    std::fs::write(dir.join("bin").join("mycmd"), "#!/bin/sh\necho MYCMD\n")
        .expect("write mycmd");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.join("bin").join("mycmd"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("chmod mycmd");
    }
    dir
}

#[test]
fn type_slash_relative_names_stay_verbatim() {
    let dir = fixture_dir("verbatim");
    let (stdout, _, code) = rubash_in(
        &dir,
        "type ./bin/mycmd; type bin/mycmd; echo rc=$?",
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "./bin/mycmd is ./bin/mycmd\nbin/mycmd is bin/mycmd\nrc=0\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_p_and_command_v_slash_names_stay_verbatim() {
    let dir = fixture_dir("p-and-v");
    let (stdout, _, code) = rubash_in(
        &dir,
        "type -p ./bin/mycmd; command -v ./bin/mycmd; echo rc=$?",
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "./bin/mycmd\n./bin/mycmd\nrc=0\n");
    let _ = std::fs::remove_dir_all(&dir);
}
