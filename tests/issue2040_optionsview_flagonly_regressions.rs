//! rubash#2040: `OptionsView::set` could not arm flag-only shell
//! options. `restricted` is legal to the set machine — GNU arms it
//! through the single-letter flag table (flags.c:182
//! `{ 'r', &restricted }` in shell_flags[], the `set -r` builtin
//! branch) — but it never appears in the `set -o` enumeration
//! (set.def:194-237 o_options[]) or SHELLOPTS, so the view's
//! two-family dispatch (shell table, then shopt table) reported the
//! legal name unknown: `set("restricted", true)` returned false with
//! no state change. A legal flag-only name must arm through the same
//! `set_shell_option` machinery the `set -r` branch uses, read back
//! through the same view, and keep GNU's one-way gate (change_flag
//! refuses FLAG_OFF while the option is on, flags.c:231-233).
//!
//! Like tests/session_api.rs this file sees only the published
//! `rubash::api` surface and drives it the way an embedding host
//! would.

use std::path::PathBuf;

use rubash::api::{Flow, RunOptions, Session};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rubash-issue2040-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn opts() -> RunOptions {
    RunOptions::default()
}

// ---------------------------------------------------------------------------
// The headline: a legal flag-only name arms and reports success.
// ---------------------------------------------------------------------------

#[test]
fn flag_only_option_arms_and_returns_true() {
    let mut session = Session::builder().build().expect("session builds");

    // Pre-fix this returned false and left restriction off.
    assert!(session.options().set("restricted", true));
    assert_eq!(session.options().get("restricted"), Some(true));
}

#[test]
fn flag_only_option_reads_default_off_before_arming() {
    let mut session = Session::builder().build().expect("session builds");
    assert_eq!(session.options().get("restricted"), Some(false));
}

// ---------------------------------------------------------------------------
// Arming through the view is the real persona: the executor's
// restricted-shell guards read the same state live.
// ---------------------------------------------------------------------------

#[test]
fn armed_restriction_enforces_the_restricted_persona() {
    let dir = temp_dir("persona");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).expect("subdir");
    let mut session = Session::builder()
        .working_directory(dir.clone())
        .build()
        .expect("session builds");

    // Unrestricted: cd moves (and moves back).
    assert_eq!(session.run("cd sub", &opts()).expect("run").status, 0);
    assert_eq!(session.run("cd ..", &opts()).expect("run").status, 0);

    assert!(session.options().set("restricted", true));

    // GNU cd.def:267-274 refuses any cd in a restricted shell
    // ("cd: restricted"); the guard reads the option key live, so a
    // mid-session arm through the view is immediately enforced.
    assert_ne!(session.run("cd sub", &opts()).expect("run").status, 0);

    // maybe_make_restricted (shell.c:1278-1299) made PATH read-only on
    // arming; the assignment machinery refuses the write.
    assert_ne!(
        session
            .run("PATH=/nonexistent-rubash-2040", &opts())
            .expect("run")
            .status,
        0
    );

    assert_eq!(
        session.run("cd sub", &opts()).expect("run").flow,
        Flow::Completed
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// GNU's one-way gate: change_flag refuses FLAG_OFF while restricted
// (flags.c:231-233); the view reports the refusal as false and the
// state does not move.
// ---------------------------------------------------------------------------

#[test]
fn unarming_a_live_restriction_is_refused() {
    let mut session = Session::builder().build().expect("session builds");
    assert!(session.options().set("restricted", true));
    assert!(!session.options().set("restricted", false));
    assert_eq!(session.options().get("restricted"), Some(true));
}

#[test]
fn disarming_an_unarmed_flag_option_is_a_successful_noop() {
    // GNU: `set +r` with restriction off passes the change_flag gate
    // and writes 0 over 0 (EXECUTION_SUCCESS) — the refusal is keyed
    // on the option being ON, not on the direction.
    let mut session = Session::builder().build().expect("session builds");
    assert!(session.options().set("restricted", false));
    assert_eq!(session.options().get("restricted"), Some(false));
}

// ---------------------------------------------------------------------------
// The listing invariant is untouched: flag-only options stay out of
// `set -o`/SHELLOPTS by construction (that is why they are flag-only).
// ---------------------------------------------------------------------------

#[test]
fn restricted_stays_out_of_the_option_listing() {
    let mut session = Session::builder().build().expect("session builds");
    assert!(session.options().set("restricted", true));
    assert!(
        !session
            .options()
            .list()
            .iter()
            .any(|option| option.name == "restricted")
    );
    // SHELLOPTS is rendered from the same two tables; GNU never lists
    // restricted there either.
    let shelopts = session
        .variables()
        .resolved_value("SHELLOPTS")
        .unwrap_or_default();
    assert!(!shelopts.split(':').any(|name| name == "restricted"));
}

// ---------------------------------------------------------------------------
// The two known families keep their exact dispatch behavior.
// ---------------------------------------------------------------------------

#[test]
fn known_families_dispatch_unchanged() {
    let mut session = Session::builder().build().expect("session builds");

    // `set -o` family.
    assert!(session.options().set("errexit", true));
    assert_eq!(session.options().get("errexit"), Some(true));
    assert!(session.options().set("errexit", false));

    // shopt family.
    assert!(session.options().set("extglob", true));
    assert_eq!(session.options().get("extglob"), Some(true));

    // Unknown to every surface: refused, no state, unreadable.
    assert!(!session.options().set("not-a-real-option", true));
    assert_eq!(session.options().get("not-a-real-option"), None);
}
