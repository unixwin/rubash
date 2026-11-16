// rubash#3533: `export NAME=v` against a readonly target must still apply
// the export attribute. GNU setattr.def binds the operand first
// (do_assignment_no_expand -> bind_variable fails with err_readonly) and
// then set_var_attribute applies att_exported regardless — the command
// fails, but `declare -p` shows `-rx`.

use std::io::Write;
use std::process::Command;

fn run_script(body: &str) -> (String, String, i32) {
    let path = std::env::temp_dir().join(format!(
        "rubash_issue3533_{}_{}.sh",
        std::process::id(),
        body.len()
    ));
    {
        let mut file = std::fs::File::create(&path).expect("create probe script");
        file.write_all(body.as_bytes()).unwrap();
    }
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.env("LC_ALL", "C.UTF-8").arg(&path);
    let out = cmd.output().expect("spawn rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn export_assignment_on_readonly_still_marks_exported() {
    let (stdout, stderr, _) = run_script("s=1; readonly s\nexport s=2\ndeclare -p s\n");
    assert!(stderr.contains("s: readonly variable"), "{stderr}");
    assert!(stdout.contains("declare -rx s=\"1\""), "{stdout}");
}

#[test]
fn export_on_unset_readonly_still_marks_exported() {
    let (stdout, stderr, _) = run_script("readonly t\nexport t=3\ndeclare -p t\n");
    assert!(stderr.contains("t: readonly variable"), "{stderr}");
    assert!(stdout.contains("declare -rx t"), "{stdout}");
}

#[test]
fn export_unset_on_readonly_still_unmarks_exported() {
    let (stdout, stderr, _) =
        run_script("x=1; export x; readonly x\nexport -n x=2\ndeclare -p x\n");
    assert!(stderr.contains("x: readonly variable"), "{stderr}");
    assert!(stdout.contains("declare -r x=\"1\""), "{stdout}");
}

#[test]
fn failed_export_assignment_keeps_prior_value() {
    let (stdout, _, _) = run_script("s=1; readonly s\nexport s=2\ndeclare -p s\n");
    // The refused bind must not overwrite the existing value.
    assert!(!stdout.contains("s=\"2\""), "{stdout}");
}
