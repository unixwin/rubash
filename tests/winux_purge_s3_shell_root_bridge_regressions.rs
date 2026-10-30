//! Winux purge S3 — rubash half of the WINUXSH_ROOT bridge demolition
//! (the niubash S2 cluster is the twin half; the removal completes only
//! when both halves land in the same release window).
//!
//! `Executor::set_shell_root` used to mirror the configured shell root
//! into `WINUXSH_ROOT` and export it, so the legacy niu bridge could
//! pick the root up from a child process environment. The bridge is
//! demolished, not renamed: after S3 the root lives only under the
//! proper `__RUBASH_SHELL_ROOT` session variable, no variable named
//! `WINUXSH_ROOT` is written, and nothing under that name is exported
//! to children.

use std::path::PathBuf;

use rubash::api::Session;

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rubash-winux-purge-s3-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp root");
    dir
}

#[test]
fn winux_purge_s3_shell_root_lives_only_under_the_proper_name() {
    let root = temp_root("shell-root");
    let mut session = Session::builder()
        .shell_root(root.clone())
        .build()
        .expect("session builds");

    // The proper name carries the configured root in the session env.
    assert_eq!(
        session.variables().resolved_value("__RUBASH_SHELL_ROOT"),
        Some(root.to_string_lossy().into_owned())
    );
    // The demolished bridge: set_shell_root must not write the legacy
    // name as a session variable at all.
    assert_eq!(session.variables().resolved_value("WINUXSH_ROOT"), None);
    // And it must not reach children: the exported environment (what
    // `Session::environment` reports as child inheritance) carries no
    // entry under the legacy name.
    assert!(
        session
            .environment()
            .iter()
            .all(|(name, _)| name != "WINUXSH_ROOT"),
        "children must not inherit the demolished bridge name"
    );
    let _ = std::fs::remove_dir_all(&root);
}
