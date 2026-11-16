//! rubash#2983: a background-job child must not inherit the parent's
//! non-exported readonly ARRAY variable.
//!
//! Root cause (fix #1): the `__RUBASH_*_VARS` variable-attribute
//! transport keys — internal cross-process state for fork-substitute
//! respawns (the `rubash -c` wrapper, command substitution) — were
//! projected into EVERY external child's environment by
//! `compose_child_environment` (src/executor/readonly_functions.rs),
//! because the process-env mirror projection kept every name where
//! `shell_assignment_needs_process_env` is true (any `__RUBASH_*`
//! name). A descendant rubash re-imported the keys via
//! `VarTable::from_values` (init.rs:342) and recreated the parent's
//! readonly/array attributes in a process that is not a fork
//! substitute. GNU passes only exported variables to a child
//! (variables.c:4850 `make_env_array_from_var_list`; arrays/assocs
//! `continue` — "XXX array vars cannot yet be exported", ARRAY_EXPORT
//! off). The fix excludes the `__RUBASH_*_VARS` keys from the mirror
//! projection; the fork substitutes build env_map directly from
//! `env_vars_snapshot()` and are unaffected.
//!
//! Reproduction (minimal, no bats needed): a parent that opens fd 3
//! (`exec 3<&1`, as bats-exec-test does — the fd>=3 gate forces the
//! `rubash -c` wrapper respawn for the background job), binds a
//! non-exported readonly array, and runs a script child in the
//! background. On the unfixed tip the child sees
//! `declare -ar MYARR` (readonly, no value) and its assignment fails
//! (`MYARR: readonly variable`, rc=1); on the fixed tip (and GNU) the
//! child sees no such variable and the assignment succeeds (rc=0).
//! Without fd 3 open the direct spawn is used and no leak occurs —
//! that is why isolated probes without `exec 3<&1` never reproduced
//! it, and why the leak needs the bats context.
//!
//! Fail-first was banked on the unfixed tip 61712935 (debug):
//! `background_script_child_does_not_inherit_readonly_array` RED
//! (child: `declare -ar MYARR`, assign rc=1); the same cell is GREEN
//! on the fixed tip and on GNU bash 5.3.20. The child script must be
//! shebangless (see the comment in `write_scripts`): a shebang'd
//! child is exec'd via its shebang under the system bash and never
//! shows the leak, which is why the first pin draft (absolute-path,
//! shebang'd child) passed on the unfixed tip.
//!
//! NOTE: this file pins fix #1 (the attribute-transport leak) only.
//! The #2983 bats #8 face also needs fix #2 (the background-wrapper
//! SIGINT relay: a group-delivered SIGINT must not kill the wrapper
//! before its foreground script child runs its INT trap — the wrapper
//! dies, PDEATHSIG SIGKILLs the script, teardown never runs). Fix #2
//! publishes a `WrapperRelayTargetGuard` for the simple-command
//! foreground wait (wait_external_child); it is designed but
//! guard-blocked (open PRs touch src/executor/external_inner.rs —
//! see REPORT.md §10) and its pin cell (a `kill -SIGINT -- -$PID`
//! pgrp probe) lands with it. Until fix #2 merges, bats #8 still
//! fails on the fixed tip even though the leak itself is gone.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

fn probe_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2983-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create probe directory");
    dir
}

fn write_scripts(dir: &PathBuf, with_fd3: bool) -> PathBuf {
    let child_path = dir.join("child.sh");
    let result_path = dir.join("result.txt");
    let parent_path = dir.join("parent.sh");
    // The result path is baked into the child script so no environment
    // passing through the wrapper is needed to observe the outcome.
    // The child script has NO shebang on purpose: a `#!/usr/bin/env
    // bash` child is exec'd via its shebang (the kernel hands it to
    // the system bash) and the attribute transport is invisible — the
    // leak only shows when the child runs as a rubash script, which
    // the shebangless form forces via the ENOEXEC fallback (the same
    // reason the bats scripts, run through the wrapper's `-c` source,
    // are interpreted by rubash rather than exec'd).
    std::fs::write(
        &child_path,
        format!(
            "declare -p MYARR > \"{result}\" 2>&1 || echo 'MYARR not found' > \"{result}\"\n\
             MYARR=(x y)\n\
             echo \"assign_rc=$?\" >> \"{result}\"\n\
             echo \"MYARR=${{MYARR[*]}}\" >> \"{result}\"\n",
            result = result_path.display(),
        ),
    )
    .expect("write child script");
    std::fs::set_permissions(&child_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod child script");
    // The background job MUST use the bare name (found via PATH) plus
    // a redirect: that is the shape that takes the `rubash -c` wrapper
    // respawn and has the child run as a rubash script (an absolute
    // path, with or without a redirect, is exec'd via its shebang
    // instead — the child then runs under the system bash and the
    // attribute transport is invisible). The `exec 3<&1` opens fd 3,
    // as bats-exec-test does, so the fd>=3 gate keeps the wrapper.
    let fd3 = if with_fd3 { "exec 3<&1\n" } else { "" };
    std::fs::write(
        &parent_path,
        format!(
            "#!/bin/bash\n\
             {fd3}MYARR=(a b)\n\
             readonly MYARR\n\
             PATH=\"{dir}:$PATH\"\n\
             child.sh > \"{dir}/child-out.log\" 2>&1 &\n\
             wait\n",
            fd3 = fd3,
            dir = dir.display(),
        ),
    )
    .expect("write parent script");
    parent_path
}

fn run_parent(parent_path: &PathBuf, dir: &PathBuf) -> (String, String, Option<i32>) {
    let output = Command::new(env!("CARGO_BIN_EXE_rubash"))
        .arg(parent_path)
        .current_dir(dir)
        .env_remove("BASH_ENV")
        .output()
        .expect("spawn rubash");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn background_script_child_does_not_inherit_readonly_array() {
    let dir = probe_dir("bg-readonly-array");
    let parent_path = write_scripts(&dir, true);
    let (stdout, stderr, code) = run_parent(&parent_path, &dir);
    assert_eq!(code, Some(0), "parent stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "parent stdout={stdout:?}");
    let result = std::fs::read_to_string(dir.join("result.txt"))
        .expect("child result file exists (the background child ran)");
    // The child must NOT see the parent's non-exported readonly array:
    // GNU leaves it unbound in the child, so the assignment succeeds.
    assert!(
        result.contains("MYARR not found"),
        "child must not see MYARR at all; result={result:?}"
    );
    assert!(
        result.contains("assign_rc=0"),
        "child assignment to MYARR must succeed; result={result:?}"
    );
    assert!(
        result.contains("MYARR=x y"),
        "child MYARR must hold the assigned value; result={result:?}"
    );
}

#[test]
fn background_script_child_without_fd3_also_leaks_without_fix() {
    // Second leak cell, no fd 3 open: a shebangless child forces the
    // wrapper through the script-fallback/ENOEXEC path even without
    // an open fd>=3 (the direct gate declines it via
    // is_enoexec_candidate), so the attribute transport reaches it
    // and it fails on the unfixed tip exactly like the fd3 cell.
    // Both cells are green on the fixed tip.
    let dir = probe_dir("bg-readonly-array-nofd3");
    let parent_path = write_scripts(&dir, false);
    let (stdout, stderr, code) = run_parent(&parent_path, &dir);
    assert_eq!(code, Some(0), "parent stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(stderr, "", "parent stdout={stdout:?}");
    let result = std::fs::read_to_string(dir.join("result.txt")).expect("child result file");
    assert!(result.contains("MYARR not found"), "result={result:?}");
    assert!(result.contains("assign_rc=0"), "result={result:?}");
}
