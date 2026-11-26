//! rubash#2900 — eco-corpus gate: the real bash-completion 2.14.0
//! distribution file, sourced and driven to its `_comp_expand_glob`
//! eval at runtime.
//!
//! Origin: on niubash 1.5.0 startup, loading the distro bash-completion
//! printed `eval: line 358: syntax error near unexpected token '('`
//! three times. The engine defect was rubash#2876: an eval'd compound
//! array assignment whose element carries an extglob pattern was a
//! syntax error, because eval pinned the extglob gate only for its
//! reparse while compound-assignment element splitting read the
//! restored gate at execution time. The file PARSES cleanly, so every
//! source-only gate missed the defect; it fires only when
//! `_comp_expand_glob` is actually *called*. This test is the corpus
//! gate on that fix — a chain, not a parse check:
//!
//!   step 1  source the pinned asset (its source-time caller
//!           `_comp__init_set_up_service_completions` runs against the
//!           host's real SysV init dirs, exactly as at shell startup);
//!   step 2  assert the post-source state: `_comp_expand_glob` is
//!           defined, the init caller has unset itself (proof the
//!           source-time chain ran end to end), extglob + progcomp are
//!           on (asset line 45), `_comp_backup_glob` carries the
//!           pinned literal;
//!   step 3  call the real `_comp_expand_glob` with the caller's exact
//!           argument shape against a fixture init dir — this executes
//!           the asset's `eval -- "$1=($2)"` — and assert the glob
//!           result: backup-shaped entries excluded, the rest kept;
//!   step 4  feed the step-3 array through the caller's loop
//!           (`[[ -x $svc ]] && complete -F _comp_complete_service`)
//!           and assert the completion registrations.
//!
//! Plus the eval x extglob cross-grid as corpus cells: the five
//! operators (`!`, `*`, `?`, `+`, `@`) through `eval` with the gate
//! on (all accepted, values stay literal) and off (all rejected,
//! rc=1), so the #1114-era one-direction coverage gap cannot silently
//! reopen.
//!
//! Expectations verified against GNU bash 5.3.20, run from script
//! files. Diagnostic wording is matched on its stable core because
//! the line prefix carries the script's own path.
//!
//! Fail-first: on pre-#2876 master the chain test fails at step 3
//! (the eval is a syntax error, `svcs` stays empty) and the grid's
//! gate-on cells fail with it; the asset-integrity and gate-off
//! cells are controls and stay green there.

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn asset_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ecosuite/bash-completion/bash_completion-2.14.0")
}

/// Run `script` from a real file (never `-c`) under the rubash binary,
/// with a C locale and no inherited startup hooks.
fn run_script(
    name: &str,
    script: &str,
    extra_env: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let path =
        std::env::temp_dir().join(format!("rubash-issue2900-{name}-{}.sh", std::process::id()));
    std::fs::write(&path, script).expect("write probe script");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rubash"));
    cmd.arg(&path)
        .env("LC_ALL", "C")
        .env_remove("BASH_ENV")
        .env_remove("ENV");
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("run rubash");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn pinned_asset_is_the_real_bash_completion_2_14_0_file() {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(asset_path()).expect("read pinned asset");
    // Byte-identical to `bash_completion` at scop/bash-completion tag
    // 2.14.0 (see tests/fixtures/ecosuite/MANIFEST.md). If this digest
    // ever changes, the asset was edited or swapped — it must stay the
    // distribution file itself, not a minimized fixture.
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "859ddde49ce594914f4c4ea108aed73d834c51f12839f36135e319b5f86d0abc"
    );
    let text = String::from_utf8(bytes).expect("asset is utf-8 text");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3349, "asset line count");
    assert_eq!(lines[44], "shopt -s extglob progcomp", "line 45");
    assert_eq!(lines[334], "_comp_expand_glob()", "line 335");
    assert!(
        lines[355].contains("eval -- \"$1=($2)\""),
        "line 356 is the expand_glob eval: {}",
        lines[355]
    );
    assert!(
        lines[1946].starts_with("_comp_backup_glob='@(#*#|"),
        "line 1947 defines _comp_backup_glob: {}",
        lines[1946]
    );
    assert_eq!(
        lines[2030], "_comp__init_set_up_service_completions",
        "line 2031 invokes the service-completion setup at source time"
    );
}

const CHAIN_SCRIPT: &str = "\
source \"$BC_ASSET\"
echo \"step1 source rc=$?\"
declare -F _comp_expand_glob >/dev/null && echo \"step2 expand_glob=defined\" || echo \"step2 expand_glob=MISSING\"
declare -F _comp__init_set_up_service_completions >/dev/null && echo \"step2 init_fn=present\" || echo \"step2 init_fn=self-unset\"
shopt -q extglob && echo \"step2 extglob=on\" || echo \"step2 extglob=off\"
shopt -q progcomp && echo \"step2 progcomp=on\" || echo \"step2 progcomp=off\"
echo \"step2 backup_glob=$_comp_backup_glob\"
svcdir=$BC_SVCDIR
_comp_expand_glob svcs '\"$svcdir\"/!($_comp_backup_glob)'
echo \"step3 expand rc=$?\"
echo \"step3 count=${#svcs[@]}\"
for s in \"${svcs[@]}\"; do echo \"step3 svc=$s\"; done
for svc in \"${svcs[@]}\"; do
    [[ -x $svc ]] && complete -F _comp_complete_service \"$svc\"
done
for name in alpha beta plain; do
    if complete -p \"$svcdir/$name\" >/dev/null 2>&1; then
        echo \"step4 $name=registered\"
    else
        echo \"step4 $name=absent\"
    fi
done
echo \"chain done\"
";

#[test]
fn bash_completion_chain_reaches_expand_glob_eval() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("rubash-issue2900-svcdir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture init dir");
    // alpha/beta are executable services; plain is a non-executable
    // file (kept by the glob, dropped by the caller's -x filter);
    // alpha.bak, beta~ and #alpha# match _comp_backup_glob and must be
    // excluded by the negated extglob group.
    for (name, mode) in [
        ("alpha", 0o755),
        ("beta", 0o755),
        ("plain", 0o644),
        ("alpha.bak", 0o755),
        ("beta~", 0o755),
        ("#alpha#", 0o755),
    ] {
        let file = dir.join(name);
        std::fs::write(&file, b"#!/bin/sh\n").expect("write fixture service");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode))
            .expect("chmod fixture service");
    }
    let dir = dir.to_string_lossy().into_owned();
    let asset = asset_path().to_string_lossy().into_owned();
    let (stdout, stderr, code) = run_script(
        "chain",
        CHAIN_SCRIPT,
        &[("BC_ASSET", &asset), ("BC_SVCDIR", &dir)],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        stdout,
        format!(
            "step1 source rc=0\n\
             step2 expand_glob=defined\n\
             step2 init_fn=self-unset\n\
             step2 extglob=on\n\
             step2 progcomp=on\n\
             step2 backup_glob=@(#*#|*@(~|.@(bak|orig|rej|swp|@(dpkg|ucf)-*|rpm@(orig|new|save))))\n\
             step3 expand rc=0\n\
             step3 count=3\n\
             step3 svc={dir}/alpha\n\
             step3 svc={dir}/beta\n\
             step3 svc={dir}/plain\n\
             step4 alpha=registered\n\
             step4 beta=registered\n\
             step4 plain=absent\n\
             chain done\n"
        ),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("syntax error"),
        "the #2876 symptom must not reappear; stderr: {stderr}"
    );
    assert_eq!(code, Some(0));
}

#[test]
fn eval_extglob_cross_grid() {
    let (stdout, stderr, code) = run_script(
        "grid-on",
        "shopt -s extglob\n\
         eval 'arr=(x/!(a))'; echo \"on ! rc=$? arr=${arr[*]}\"\n\
         eval 'arr=(x/*(a))'; echo \"on * rc=$? arr=${arr[*]}\"\n\
         eval 'arr=(x/?(a))'; echo \"on ? rc=$? arr=${arr[*]}\"\n\
         eval 'arr=(x/+(a))'; echo \"on + rc=$? arr=${arr[*]}\"\n\
         eval 'arr=(x/@(a|b))'; echo \"on @ rc=$? arr=${arr[*]}\"\n",
        &[],
    );
    assert_eq!(
        stdout,
        "on ! rc=0 arr=x/!(a)\n\
         on * rc=0 arr=x/*(a)\n\
         on ? rc=0 arr=x/?(a)\n\
         on + rc=0 arr=x/+(a)\n\
         on @ rc=0 arr=x/@(a|b)\n",
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("syntax error"), "stderr: {stderr}");
    assert_eq!(code, Some(0));

    let (stdout, _, code) = run_script(
        "grid-off",
        "eval 'arr=(x/!(a))' 2>&1; echo \"off ! rc=$?\"\n\
         eval 'arr=(x/*(a))' 2>&1; echo \"off * rc=$?\"\n\
         eval 'arr=(x/?(a))' 2>&1; echo \"off ? rc=$?\"\n\
         eval 'arr=(x/+(a))' 2>&1; echo \"off + rc=$?\"\n\
         eval 'arr=(x/@(a|b))' 2>&1; echo \"off @ rc=$?\"\n",
        &[],
    );
    for op in ["!", "*", "?", "+", "@"] {
        assert!(
            stdout.contains(&format!("off {op} rc=1\n")),
            "gate-off cell {op} must be rejected with rc=1; stdout: {stdout}"
        );
    }
    assert_eq!(
        stdout
            .matches("syntax error near unexpected token `('")
            .count(),
        5,
        "each gate-off cell reports the token error; stdout: {stdout}"
    );
    assert_eq!(code, Some(0));
}
