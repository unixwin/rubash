use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn run_script(source: &str, nested: &str) -> std::process::Output {
    let directory = std::env::temp_dir().join(format!(
        "rubash-alias-hygiene-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("nested.sh"), nested).unwrap();
    std::fs::write(directory.join("probe.sh"), source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .args(["--noprofile", "--norc", "probe.sh"])
        .current_dir(&directory)
        .env("HOME", &directory)
        .env("USERPROFILE", &directory)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("alias hygiene probe timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn unalias_clears_line_markers_across_redefine_source_and_eval() {
    let source = r#"shopt -s expand_aliases
alias hygiene='printf OLD'
hygiene
alias hygiene='printf NEW'
hygiene
printf 'before=%s\n' "${__RUBASH_ALIAS_LINE_hygiene-unset}"
unalias hygiene
printf 'after=%s\n' "${__RUBASH_ALIAS_LINE_hygiene-unset}"
alias hygiene='printf AGAIN'
eval "unalias hygiene"
printf 'eval=%s\n' "${__RUBASH_ALIAS_LINE_hygiene-unset}"
alias hygiene='printf SOURCE'
source ./nested.sh
printf 'source=%s\n' "${__RUBASH_ALIAS_LINE_hygiene-unset}"
"#;
    let nested = r#"unalias hygiene
printf 'nested=%s\n' "${__RUBASH_ALIAS_LINE_hygiene-unset}"
"#;
    let output = run_script(source, nested);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "OLDNEWbefore=4\nafter=unset\neval=unset\nnested=unset\nsource=unset\n"
    );
}

#[test]
fn unalias_a_clears_all_line_markers_without_changing_same_line_guard() {
    let source = r#"shopt -s expand_aliases
alias first='printf FIRST'
alias second='printf SECOND'
unalias -a
printf 'first=%s second=%s\n' "${__RUBASH_ALIAS_LINE_first-unset}" "${__RUBASH_ALIAS_LINE_second-unset}"
alias same='printf SAME'; same
printf 'status=%s\n' "$?"
"#;
    let output = run_script(source, "");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "first=unset second=unset\nstatus=127\n"
    );
}
