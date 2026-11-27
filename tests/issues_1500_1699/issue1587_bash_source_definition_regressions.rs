//! Issue rubash#1587: inside a function, `BASH_SOURCE[0]` must name
//! the file where the function was DEFINED (GNU execute_cmd.c
//! execute_function pushes the definition's source file for the call
//! frame). Rubash filled slot 0 from the CALLER's frame instead, so
//! slot 0 always duplicated slot 1 whenever definition and call site
//! lived in different files (and showed `bash`/$0 under -c).
//! FUNCNAME and BASH_LINENO in the same frames were already correct.

use std::path::PathBuf;
use std::process::Command;

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash1587-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn write_fixture(dir: &std::path::Path) {
    std::fs::write(
        dir.join("lib.sh"),
        "lf() {\n  echo \"src0=[${BASH_SOURCE[0]}] src1=[${BASH_SOURCE[1]}] fn=${FUNCNAME[0]}/${FUNCNAME[1]}\"\n}\n",
    )
    .expect("write lib.sh");
    std::fs::write(dir.join("lib2.sh"), "mf\n").expect("write lib2.sh");
    std::fs::write(
        dir.join("main.sh"),
        "mf() { echo \"mf src0=[${BASH_SOURCE[0]}] src1=[${BASH_SOURCE[1]}]\"; }\n. ./lib.sh\nlf\n. ./lib2.sh\n",
    )
    .expect("write main.sh");
}

/// Both directions: a function defined in a sourced file reports that
/// file even when called from the main script, and a function defined
/// in the main script reports the main script even when called from
/// inside a sourced file (GNU bash 5.3.20 measured output).
#[test]
fn bash_source_zero_is_definition_file_not_caller() {
    let dir = tempdir("files");
    write_fixture(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("main.sh")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stdout.contains("src0=[./lib.sh] src1=[main.sh] fn=lf/main"),
        "sourced definition lost: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("mf src0=[main.sh] src1=[./lib2.sh]"),
        "main definition shadowed by caller: stdout={stdout:?} stderr={stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Under -c the caller frame's label is the shell name; a function
/// defined in a sourced file must still report the sourced file.
#[test]
fn bash_source_zero_under_dash_c_reports_definition_file() {
    let dir = tempdir("dashc");
    write_fixture(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg("-c")
        .arg(". ./lib.sh; lf")
        .current_dir(&dir)
        .output()
        .expect("run rubash");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stdout.contains("src0=[./lib.sh]"),
        "-c caller label leaked into slot 0: stdout={stdout:?} stderr={stderr:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
