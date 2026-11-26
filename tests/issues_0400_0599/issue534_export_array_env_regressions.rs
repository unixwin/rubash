//! Issue rubash#534: GNU variables.c make_env_array_from_var_list skips
//! array and associative variables when the child environment list is
//! assembled (ARRAY_EXPORT off) — `declare -a A=(x y); export A` keeps the
//! exported attribute (declare -p shows -a -x) while the name never crosses
//! the child env boundary. Rubash leaked the storage-encoded value (leading
//! 0x1d marker) into the child env.
//!
//! Rust semantic owners: builtins/setattr/apply.rs (no process-env write for
//! array-typed export/readonly operands), executor/env_helpers.rs
//! is_exportable_env_name + executor/readonly_functions.rs
//! apply_child_environment, builtins/exec.rs (exec + internal printenv
//! fallback), executor/external_inner.rs (env emulation),
//! builtins/declare/attrs.rs (declare -x).

#![cfg(windows)]

use std::process::{Command, Stdio};

fn run_script(script: &str) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("rubash-issue534-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue534 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT")
        .output()
        .expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn exported_array_never_reaches_child_env() {
    // WSL GNU bash 5.3.0: printenv A prints nothing, rc 1.
    let (code, stdout, _stderr) =
        run_script("declare -a A=(x y)\nexport A\nprintenv A\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n", "printenv A must print nothing");
    assert_eq!(code, Some(0));

    // The assoc form is equally opaque to children.
    let (_, stdout, _) = run_script("declare -A M=([k]=v)\nexport M\nprintenv M\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");

    // Pipeline stages and exec go through the same boundary.
    let (_, stdout, _) =
        run_script("declare -a A=(x y)\nexport A\nprintenv A | od -An -tx1 | head -1\n");
    assert_eq!(stdout, "", "no bytes may leak through a pipeline");
    let (_, stdout, _) =
        run_script("declare -a A=(x y)\nexport A\nexec printenv A\necho unreachable\n");
    assert_eq!(stdout, "", "exec must not publish the array either");

    // `env`'s emulation mirrors the spawn path.
    let (_, stdout, _) = run_script("declare -a A=(x y)\nexport A\nenv | grep -c '^A='\n");
    assert_eq!(stdout, "0\n");
}

#[test]
fn array_export_attribute_view_is_preserved() {
    // WSL GNU bash 5.3.0: `declare -ax A=([0]="x" [1]="y")`.
    let (_, stdout, _) = run_script("declare -a A=(x y)\nexport A\ndeclare -p A\n");
    assert_eq!(stdout, "declare -ax A=([0]=\"x\" [1]=\"y\")\n");

    // Scalars are unaffected.
    let (_, stdout, _) = run_script("S=hello\nexport S\nprintenv S\n");
    assert_eq!(stdout, "hello\n");

    // `export name=value` on an existing array assigns the element and
    // still exports nothing to children.
    let (_, stdout, _) =
        run_script("declare -a B=(1 2)\nexport B=zz\nprintenv B\necho \"rc=$?\"\ndeclare -p B\n");
    assert!(stdout.starts_with("rc=1\n"));
    assert!(stdout.contains("declare -ax B=([0]=\"zz\" [1]=\"2\")\n"));

    // readonly arrays do not leak either.
    let (_, stdout, _) = run_script("declare -a R=(p q)\nreadonly R\nprintenv R\necho \"rc=$?\"\n");
    assert_eq!(stdout, "rc=1\n");
}
