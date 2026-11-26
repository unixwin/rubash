//! rubash#3108: the inspection snapshot must not report the
//! startup-seeded names as exported.
//!
//! Executor construction seeds the env table with the shell's own
//! managed variables (BASH_VERSINFO, DIRSTACK, BASH_ALIASES,
//! BASH_CMDS, ...) and then builds the typed store from that whole
//! seeded table via `VariableStore::from_environment`, which binds
//! every entry as an exported Scalar. `capture_variable`'s
//! `merged_attrs` ORs the typed cell's `exported` into the snapshot
//! attrs, so every seed name snapshots as `exported: true` even
//! though the env table's own attrs say `exported: false` and GNU
//! never exports these names.
//!
//! GNU's provenance is per-entry: only the entries that arrive via
//! the process environment are bound with att_exported|att_imported
//! (variables.c:511-518, the initialize_shell_variables environment
//! loop); the shell's own cells are born without it
//! (variables.c:1010 make_vers_array builds BASH_VERSINFO as a plain
//! array variable; variables.c:1882 registers DIRSTACK via
//! init_dynamic_array_var with attrs 0). The typed binding must
//! carry the same provenance: a name the table does not mark
//! exported is not exported, whatever store it was imported into.
//!
//! These tests drive the public Session API only
//! (`Session::views().inspection()`), the same capture the debugger
//! consult consumes at a stop.

use rubash::api::{RunOptions, Session};

fn run(session: &mut Session, line: &str) {
    let outcome = session
        .run(line, &RunOptions::default())
        .expect("run succeeds");
    assert_eq!(outcome.status, 0, "run of {line:?} succeeds");
}

fn snapshot_exported(session: &mut Session, name: &str) -> bool {
    let snap = session.views().inspection();
    snap.variable(name)
        .unwrap_or_else(|| panic!("snapshot carries {name}"))
        .attrs
        .exported
}

fn snapshot_declaration(session: &mut Session, name: &str) -> String {
    let snap = session.views().inspection();
    snap.variable(name)
        .unwrap_or_else(|| panic!("snapshot carries {name}"))
        .declaration
        .clone()
        .unwrap_or_else(|| panic!("snapshot declaration for {name}"))
}

#[test]
fn seeded_names_snapshot_as_not_exported() {
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "x=1");
    // The four seeded array/assoc names GNU never exports
    // (variables.c:1010 make_vers_array; variables.c:1882
    // init_dynamic_array_var for DIRSTACK; the assoc cells for
    // BASH_ALIASES/BASH_CMDS are likewise born unexported).
    for name in ["BASH_VERSINFO", "DIRSTACK", "BASH_ALIASES", "BASH_CMDS"] {
        assert!(
            !snapshot_exported(&mut session, name),
            "seeded {name} snapshots as not exported"
        );
    }
}

#[test]
fn environment_names_snapshot_as_exported() {
    // A name that genuinely arrives through the process environment
    // keeps att_exported (variables.c:511-518). PATH is inherited by
    // every test process, so it is the hermetic witness. (The
    // Session builder's `.env()` overrides are NOT an environment
    // import — they bind like plain assignments and snapshot as
    // unexported on both the base and the fixed tree.)
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "x=1");
    assert!(
        snapshot_exported(&mut session, "PATH"),
        "inherited PATH snapshots as exported"
    );
    if session.views().inspection().variable("HOME").is_some() {
        assert!(
            snapshot_exported(&mut session, "HOME"),
            "inherited HOME snapshots as exported"
        );
    }
}

#[test]
fn declaration_text_keeps_the_export_flag_shape() {
    // Control: the snapshot's declaration text is rendered from the
    // table side and was already correct — it must not change.
    // Seed names render without -x, the environment name with it.
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "x=1");
    let seed_decl = snapshot_declaration(&mut session, "BASH_VERSINFO");
    assert!(
        !seed_decl.contains("-x"),
        "BASH_VERSINFO declaration carries no -x: {seed_decl}"
    );
    let env_decl = snapshot_declaration(&mut session, "PATH");
    assert!(
        env_decl.contains("-x"),
        "environment name declaration carries -x: {env_decl}"
    );
}

#[test]
fn write_path_export_state_is_unchanged() {
    // Control group: bindings written through the shell's own write
    // paths snapshot their export state exactly as before.
    let mut session = Session::builder().build().expect("session builds");
    run(&mut session, "plain=1; export marked=2");
    assert!(
        !snapshot_exported(&mut session, "plain"),
        "plain assignment snapshots as not exported"
    );
    assert!(
        snapshot_exported(&mut session, "marked"),
        "export builtin snapshots as exported"
    );
}
