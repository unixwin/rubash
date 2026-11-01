//! `enable -f` / `enable -d` loadable builtin tests.
//!
//! GNU oracle: builtins/enable.def `dyn_load_builtin` (:318) and
//! `dyn_unload_builtin` (:536).  The fixture (tests/fixtures/dynamic_builtin/
//! dynprobe.c) is a genuine bash-ABI shared object exporting a
//! `struct builtin` plus the optional load/unload hooks.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

fn fixture() -> &'static Path {
    static FIXTURE: OnceLock<PathBuf> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/enable-dynamic-fixture");
        fs::create_dir_all(&dir).expect("create fixture dir");
        let source = dir.join("dynprobe.c");
        fs::write(&source, include_str!("fixtures/dynamic_builtin/dynprobe.c"))
            .expect("write fixture source");
        let object = dir.join("rubash_dynprobe.so");
        let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let status = Command::new(compiler)
            .args(["-shared", "-fPIC", "-O0", "-o"])
            .arg(&object)
            .arg(&source)
            .status()
            .expect("run C compiler for the loadable builtin fixture");
        assert!(status.success(), "C fixture compile failed");
        object
    })
}

fn run(script: &str, extra_env: &[(&str, &str)]) -> (String, String, Option<i32>) {
    let dir = fixture().parent().expect("fixture dir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command.args(["-c", script]).current_dir(dir).env(
        "PATH",
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
    );
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let output = command.output().expect("run rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn loads_calls_disables_reenables_and_unloads() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f ./rubash_dynprobe.so rubash_dynprobe\n",
            "echo load_rc=$?\n",
            "type -t rubash_dynprobe\n",
            "rubash_dynprobe alpha beta\n",
            "echo call_rc=$?\n",
            "rubash_dynprobe fail\n",
            "echo fail_rc=$?\n",
            "rubash_dynprobe usage\n",
            "echo usage_rc=$?\n",
            "enable -n rubash_dynprobe\n",
            "rubash_dynprobe hidden\n",
            "echo disabled_rc=$?\n",
            "enable rubash_dynprobe\n",
            "rubash_dynprobe again\n",
            "echo reenable_rc=$?\n",
            "enable -d rubash_dynprobe\n",
            "echo unload_rc=$?\n",
            "type -t rubash_dynprobe\n",
            "rubash_dynprobe gone\n",
            "echo gone_rc=$?\n",
        ),
        &[],
    );

    assert_eq!(code, Some(0), "stderr: {stderr}");
    let expected = concat!(
        "rubash_dynprobe load rubash_dynprobe\n",
        "load_rc=0\n",
        "builtin\n",
        "rubash_dynprobe: alpha beta (argc=2)\n",
        "call_rc=0\n",
        "rubash_dynprobe: fail (argc=1)\n",
        "fail_rc=7\n",
        "rubash_dynprobe: usage (argc=1)\n",
        "usage_rc=2\n",
        "disabled_rc=127\n",
        "rubash_dynprobe: again (argc=1)\n",
        "reenable_rc=0\n",
        "rubash_dynprobe unload rubash_dynprobe\n",
        "unload_rc=0\n",
        "gone_rc=127\n",
    );
    assert_eq!(stdout, expected);
    assert!(
        stderr.contains("rubash_dynprobe: command not found"),
        "stderr: {stderr}"
    );
}

#[test]
fn missing_file_reports_gnu_style_diagnostic() {
    let (stdout, stderr, code) = run("enable -f ./does-not-exist.so whatever\necho rc=$?\n", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("enable: cannot open shared object ./does-not-exist.so:"),
        "stderr: {stderr}"
    );
}

#[test]
fn missing_symbol_leaves_no_registered_entry() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f ./rubash_dynprobe.so no_such_builtin\n",
            "echo load_rc=$?\n",
            "type -t no_such_builtin\n",
            "echo type_rc=$?\n",
        ),
        &[],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "load_rc=1\ntype_rc=1\n");
    assert!(
        stderr.contains(
            "enable: cannot find no_such_builtin_struct in shared object ./rubash_dynprobe.so:"
        ),
        "stderr: {stderr}"
    );
}

#[test]
fn slash_names_are_rejected() {
    let (stdout, stderr, code) = run("enable -f ./rubash_dynprobe.so a/b\necho rc=$?\n", &[]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=1\n");
    assert!(
        stderr.contains("enable: a/b: builtin names may not contain slashes"),
        "stderr: {stderr}"
    );
}

#[test]
fn duplicate_load_is_diagnosed_and_keeps_first_registration() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f ./rubash_dynprobe.so rubash_dynprobe\n",
            "enable -f ./rubash_dynprobe.so rubash_dynprobe\n",
            "echo duplicate_rc=$?\n",
            "rubash_dynprobe still-here\n",
            "echo call_rc=$?\n",
            "enable -d rubash_dynprobe\n",
            "echo unload_rc=$?\n",
        ),
        &[],
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        concat!(
            "rubash_dynprobe load rubash_dynprobe\n",
            "duplicate_rc=1\n",
            "rubash_dynprobe: still-here (argc=1)\n",
            "call_rc=0\n",
            "rubash_dynprobe unload rubash_dynprobe\n",
            "unload_rc=0\n",
        )
    );
    assert!(
        stderr.contains("enable: rubash_dynprobe: dynamic builtin already loaded"),
        "stderr: {stderr}"
    );
}

#[test]
fn unload_requires_a_dynamic_builtin() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -d echo\n",
            "echo static_rc=$?\n",
            "enable -d not_a_builtin\n",
            "echo missing_rc=$?\n",
        ),
        &[],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "static_rc=1\nmissing_rc=1\n");
    assert!(
        stderr.contains("enable: echo: not dynamically loaded"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("enable: not_a_builtin: not a shell builtin"),
        "stderr: {stderr}"
    );
}

#[test]
fn bash_loadables_path_resolves_a_bare_filename_and_listing_includes_it() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f rubash_dynprobe.so rubash_dynprobe\n",
            "echo load_rc=$?\n",
            "enable -p | grep -c '^enable rubash_dynprobe$'\n",
            "enable -a | grep -c '^enable rubash_dynprobe$'\n",
            "enable -d rubash_dynprobe\n",
        ),
        &[("BASH_LOADABLES_PATH", ".")],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        concat!(
            "rubash_dynprobe load rubash_dynprobe\n",
            "load_rc=0\n",
            "1\n",
            "1\n",
            "rubash_dynprobe unload rubash_dynprobe\n",
        )
    );
}

#[test]
fn multiple_builtins_share_a_handle_until_the_last_one_unloads() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f ./rubash_dynprobe.so rubash_dynprobe rubash_dynprobe2\n",
            "echo load_rc=$?\n",
            "rubash_dynprobe2 first\n",
            "enable -d rubash_dynprobe\n",
            "echo first_unload_rc=$?\n",
            "rubash_dynprobe2 second\n",
            "echo still_loaded_rc=$?\n",
            "enable -d rubash_dynprobe2\n",
            "echo second_unload_rc=$?\n",
        ),
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        concat!(
            "rubash_dynprobe load rubash_dynprobe\n",
            "rubash_dynprobe2 load rubash_dynprobe2\n",
            "load_rc=0\n",
            "rubash_dynprobe2: first (argc=1)\n",
            "rubash_dynprobe unload rubash_dynprobe\n",
            "first_unload_rc=0\n",
            "rubash_dynprobe2: second (argc=1)\n",
            "still_loaded_rc=0\n",
            "rubash_dynprobe2 unload rubash_dynprobe2\n",
            "second_unload_rc=0\n",
        )
    );
    assert_eq!(stderr, "");
}

#[test]
fn plugin_stdio_obeys_redirects_and_command_substitution() {
    let (stdout, stderr, code) = run(
        concat!(
            "enable -f ./rubash_dynprobe.so rubash_dynprobe\n",
            "rubash_dynprobe redirected > enable-dynamic-out.txt\n",
            "read -r line < enable-dynamic-out.txt\n",
            "echo \"file=[$line]\"\n",
            "value=$(rubash_dynprobe substituted)\n",
            "echo \"comsub=[$value]\"\n",
            "enable -d rubash_dynprobe\n",
        ),
        &[],
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        concat!(
            "rubash_dynprobe load rubash_dynprobe\n",
            "file=[rubash_dynprobe: redirected (argc=1)]\n",
            "comsub=[rubash_dynprobe: substituted (argc=1)]\n",
            "rubash_dynprobe unload rubash_dynprobe\n",
        )
    );
    assert_eq!(stderr, "");
}

#[test]
fn empty_name_list_lists_like_gnu_enable() {
    // enable.def:204-216 checks `list == 0` before the FFLAG arm.
    let (stdout, stderr, code) = run(
        "enable -f ./rubash_dynprobe.so > /dev/null\necho rc=$?\n",
        &[],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "rc=0\n");
    assert_eq!(stderr, "");
}
