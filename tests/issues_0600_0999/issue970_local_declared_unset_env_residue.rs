//! Issue rubash#970: a `local NAME` declared-unset binding inside a
//! function still read the inherited process-env copy — `#943`'s
//! `unset_process_env` fix covered the global unbind path, but the
//! local declared-unset path (`initialize_non_inherited_locals`, the
//! funnel shared by `local`/`declare`/`typeset` at function scope) kept
//! the residue, so every table-miss read fallback resurrected it
//! (`exact_case_env_var` bare-`$NAME` reads and the `set -u` boundness
//! check alike).
//!
//! GNU references:
//! - variables.c:2755 (make_local_variable): a value-less local carries
//!   att_invisible — reads report unset while the local shadows the
//!   outer binding (`$FOO` -> `[]`, `set -u` -> `FOO: unbound variable`).
//! - variables.c:4336 export_environment_candidate + variables.c:4149
//!   vlist_add (innermost-first dedupe): the local is an export-env
//!   candidate only while bound AND exported; while invisible the
//!   shadowed outer binding still exports its value to children
//!   (`printenv FOO` inside the function -> `parent`).
//! - variables.c:3984-4005 makunbound: `unset` on a local reinserts the
//!   binding invisible (drop att_exported), the outer env entry keeps
//!   passing through.
//!
//! Baselines below were verified against WSL GNU bash 5.3.20 (script
//! files run under both shells).

#![cfg(windows)]

use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn run_script(script: &str, env: &[(&str, &str)]) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!(
        "rubash-issue970-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create issue970 scratch dir");
    let script_path = dir.join("probe.sh");
    std::fs::write(&script_path, script).expect("write probe script");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rubash"));
    command
        .arg(&script_path)
        .stdin(Stdio::null())
        .env_remove("BASH_ENV")
        .env_remove("WINUXSH_ROOT");
    for (name, value) in env {
        command.env(name, value);
    }
    let output = command.output().expect("run rubash script");
    let _ = std::fs::remove_dir_all(&dir);
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// The issue's P22 probe: `local FOO` on an inherited name expands empty,
/// and the frame pop restores the outer binding intact.
#[test]
fn local_declared_unset_drops_inherited_process_env_residue() {
    // WSL GNU 5.3.20: declared=[], after-unset=[], global=[parent], bar=[].
    let (code, stdout, _) = run_script(
        "foo() { local FOO; echo \"declared=[$FOO]\"; unset FOO; echo \"after-unset=[$FOO]\"; }\nfoo\necho \"global=[$FOO]\"\nbar() { local FOO; unset FOO; echo \"bar=[$FOO]\"; }\nbar\n",
        &[("FOO", "parent")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        stdout,
        "declared=[]\nafter-unset=[]\nglobal=[parent]\nbar=[]\n"
    );
}

/// Dynamic scoping: an inner function reading the name sees the invisible
/// local, not the residue.
#[test]
fn local_declared_unset_shadows_dynamically() {
    // WSL GNU 5.3.20: g=[].
    let (_, stdout, _) = run_script(
        "g() { echo \"g=[$FOO]\"; }\nf() { local FOO; g; }\nf\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "g=[]\n");
}

/// Every declared-unset local shape — `local`, `local -x`, `local -a`,
/// `typeset`, `declare` — drops the residue the same way (one funnel:
/// initialize_non_inherited_locals).
#[test]
fn local_declared_unset_variants_drop_residue() {
    // WSL GNU 5.3.20: fx=[], fa=[], ft=[], fd=[].
    let (_, stdout, _) = run_script(
        "fx() { local -x FOO; echo \"fx=[$FOO]\"; }\nfx\nfa() { local -a FOO; echo \"fa=[${FOO[@]}]\"; }\nfa\nft() { typeset FOO; echo \"ft=[$FOO]\"; }\nft\nfd() { declare FOO; echo \"fd=[$FOO]\"; }\nfd\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "fx=[]\nfa=[]\nft=[]\nfd=[]\n");
}

/// Under `set -u` the residue used to count as bound; GNU aborts on the
/// invisible local's expansion.
#[test]
fn local_declared_unset_reports_unbound_under_nounset() {
    // WSL GNU 5.3.20: `FOO: unbound variable`, nothing on stdout.
    let (code, stdout, stderr) = run_script(
        "w() { local FOO; set -u; echo \"w=[$FOO]\"; }\nw\necho unreachable\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("FOO: unbound variable"), "stderr: {stderr}");
    assert_ne!(code, Some(0));
}

/// The shadowed outer binding still exports to children while the local
/// is invisible (export_environment_candidate drops the invisible local,
/// not the outer) — the residue must be gone for READS but preserved for
/// the child env through the LOCAL_EXPORT_ENV stash.
#[test]
fn shadowed_export_passes_through_to_children() {
    // WSL GNU 5.3.20: printenv FOO -> parent (rc 0) inside the function,
    // still parent after `unset FOO` (makunbound leaves the local
    // invisible+unexported, so the outer export keeps shining through),
    // and comsub/background children see the same pass-through.
    let (code, stdout, _) = run_script(
        "f() {\n  local FOO\n  printenv FOO; echo \"pv=$?\"\n  unset FOO\n  printenv FOO; echo \"pv2=$?\"\n  X=$(printenv FOO); echo \"comsub=[$X]\"\n}\nf\n",
        &[("FOO", "parent")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "parent\npv=0\nparent\npv2=0\ncomsub=[parent]\n");
}

/// The LOCAL_EXPORT_ENV stash yields when the local itself is a live
/// export candidate (bound + exported): the child must see the local's
/// value, not the shadowed outer's.
#[test]
fn bound_local_export_wins_over_shadowed_inherited() {
    // WSL GNU 5.3.20: `local FOO=lv` exports lv, `local FOO; FOO=lv2`
    // exports lv2, `local FOO=` exports the empty cell.
    let (_, stdout, _) = run_script(
        "bv() { local FOO=lv; printenv FOO; echo \"bv=$?\"; }\nbv\nba() { local FOO; FOO=lv2; printenv FOO; echo \"ba=$?\"; }\nba\nbe() { local FOO; FOO=; printenv FOO; echo \"be=$?\"; }\nbe\n",
        &[("FOO", "parent")],
    );
    // printenv on a bound-but-empty cell prints a bare newline.
    assert_eq!(stdout, "lv\nbv=0\nlv2\nba=0\n\nbe=0\n");
}

/// `export -n` on a bound local drops the export attribute, so the outer
/// binding's env entry passes through again.
#[test]
fn export_n_on_local_restores_shadowed_passthrough() {
    // WSL GNU 5.3.20: `local FOO=val; export -n FOO; printenv FOO`
    // prints parent — the local is bound but no longer an export
    // candidate, so the outer (exported, imported) binding wins.
    let (_, stdout, _) = run_script(
        "en() { local FOO=val; export -n FOO; printenv FOO; echo \"en=$?\"; }\nen\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "parent\nen=0\n");
}

/// The frame pop restores the outer binding — value for reads AND the
/// exported child-env entry — even though the residue was dropped while
/// the local covered it.
#[test]
fn inherited_binding_restores_after_frame_pop() {
    // WSL GNU 5.3.20: out=[parent], printenv -> parent.
    let (code, stdout, _) = run_script(
        "f() { local FOO; echo \"in=[$FOO]\"; }\nf\necho \"out=[$FOO]\"\nprintenv FOO; echo \"pv=$?\"\n",
        &[("FOO", "parent")],
    );
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "in=[]\nout=[parent]\nparent\npv=0\n");
}

/// Nested frames: an inner `local` shadows the caller's local the same
/// way, and its env pass-through is the CALLER's binding.
#[test]
fn nested_local_frames_shadow_and_restore() {
    // WSL GNU 5.3.20: inner sees [] (caller local a shadowed), children
    // see `a` while covered; back in the caller the local a is live
    // again, then the inherited parent restores at the outer pop.
    let (_, stdout, _) = run_script(
        "i() { local FOO; echo \"i=[$FOO]\"; printenv FOO; echo \"ipv=$?\"; }\nh() { local FOO=a; i; echo \"h=[$FOO]\"; }\nh\necho \"tail=[$FOO]\"\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "i=[]\na\nipv=0\nh=[a]\ntail=[parent]\n");
}

/// `local -I` / localvar_inherit keep the shadowed binding's value — the
/// cell is bound, never invisible.
#[test]
fn local_inherit_keeps_the_outer_value() {
    // WSL GNU 5.3.20: ii=[parent], and a tempenv prefix wins over the
    // inherited copy.
    let (_, stdout, _) = run_script(
        "ii() { local -I FOO; echo \"ii=[$FOO]\"; }\nii\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "ii=[parent]\n");

    let (_, stdout, _) = run_script(
        "te() { local FOO; echo \"te=[$FOO]\"; }\nFOO=tempval te\necho \"tail=[$FOO]\"\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "te=[tempval]\ntail=[parent]\n");
}

/// A `local` blocked by a readonly global never touches the binding — or
/// the residue.
#[test]
fn readonly_global_still_blocks_local() {
    // WSL GNU 5.3.20: `local: FOO: readonly variable` on stderr, value
    // and child-env entry untouched; the function still exits 0 in GNU
    // (the blocked operand is dropped, local succeeds).
    let (_, stdout, stderr) = run_script(
        "readonly FOO\nf() { local FOO; echo \"in=[$FOO]\"; }\nf\necho \"rc=$? out=[$FOO]\"\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "in=[parent]\nrc=0 out=[parent]\n");
    assert!(
        stderr.contains("local: FOO: readonly variable"),
        "stderr: {stderr}"
    );
}

/// `unset FOO` on the local keeps GNU's makunbound shape — the invisible
/// local stays `declare -- FOO` and reports unbound under `set -u` even
/// after `declare -p` looked at it (a print must not resurrect the cell).
#[test]
fn unset_local_reports_unbound_even_after_declare_p() {
    // WSL GNU 5.3.20: `declare -- FOO` on stdout, `FOO: unbound variable`
    // on stderr, script aborts.
    let (code, stdout, stderr) = run_script(
        "a() { local FOO; unset FOO; declare -p FOO; set -u; echo \"a=[$FOO]\"; }\na\necho unreachable\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "declare -- FOO\n");
    assert!(stderr.contains("FOO: unbound variable"), "stderr: {stderr}");
    assert_ne!(code, Some(0));
}

/// The same declared-unset invariant holds at global scope: a bare
/// `declare FOO` creates an invisible binding — unbound under `set -u`.
#[test]
fn global_declared_unset_reports_unbound_under_nounset() {
    // WSL GNU 5.3.20: `FOO: unbound variable` (declare -p shows
    // `declare -- FOO` first).
    let (code, stdout, stderr) = run_script(
        "unset FOO\ndeclare FOO\nset -u\necho \"g=[$FOO]\"\n",
        &[("FOO", "parent")],
    );
    assert_eq!(stdout, "");
    assert!(stderr.contains("FOO: unbound variable"), "stderr: {stderr}");
    assert_ne!(code, Some(0));
}

/// `local NAME` on the mirrored special names drops the residue too —
/// the special-name write gate never governed inherited copies.
#[test]
fn local_special_name_drops_residue() {
    // WSL GNU 5.3.20: in=[unset], out=[set] — PATH's inherited copy is
    // unreadable inside the frame and restored at pop.
    let (_, stdout, _) = run_script(
        "p() { local PATH; echo \"in=[${PATH-unset}]\"; }\np\necho \"out=[${PATH+set}]\"\n",
        &[],
    );
    assert_eq!(stdout, "in=[unset]\nout=[set]\n");
}
