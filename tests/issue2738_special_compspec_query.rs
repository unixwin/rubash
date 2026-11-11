//! rubash#2738: the selective special-compspec query.
//!
//! The engine honors `complete -D` / `-E` / `-I`, and the name form
//! `Session::has_compspec` ORs all three together with the command's
//! own registration, so a host gate cannot ask which special spec
//! exists — yet the `-E` empty line and the `-I` initial word are
//! exactly the positions the name form cannot name. The contract
//! added for this issue is `Session::has_special_compspec(kind)`
//! with `kind` in {Default, Empty, Initial}: a pure registration
//! query, one special class at a time. (The position-aware gate is
//! `Session::has_compspec_at`, rubash#2856.)
//!
//! GNU facts behind the classes: pcomplete.c resolves DEFAULTCMD
//! (-D), EMPTYCMD (-E) and INITIALWORD (-I) as separate compspec
//! slots; bashline.c:1682 loads the -I spec for the initial word and
//! bashline.c:1785-1792 falls through -E -> -D -> -I on the empty
//! line.

use rubash::api::{RunOptions, Session, SpecialCompspecKind};

fn opts() -> RunOptions {
    RunOptions::default()
}

fn session() -> Session {
    Session::builder().build().expect("session builds")
}

fn register(session: &mut Session, line: &str) {
    session
        .run(line, &opts())
        .unwrap_or_else(|err| panic!("registration run {line:?} failed: {err:?}"));
}

fn specials(session: &Session) -> (bool, bool, bool) {
    (
        session.has_special_compspec(SpecialCompspecKind::Default),
        session.has_special_compspec(SpecialCompspecKind::Empty),
        session.has_special_compspec(SpecialCompspecKind::Initial),
    )
}

#[test]
fn no_registration_means_no_special_compspec() {
    let session = session();
    assert_eq!(specials(&session), (false, false, false));
}

#[test]
fn empty_spec_answers_only_the_empty_class() {
    let mut session = session();
    register(&mut session, "efn() { COMPREPLY=(); }");
    register(&mut session, "complete -E -F efn");
    assert_eq!(specials(&session), (false, true, false));
}

#[test]
fn initial_spec_answers_only_the_initial_class() {
    let mut session = session();
    register(&mut session, "ifn() { COMPREPLY=(); }");
    register(&mut session, "complete -I -F ifn");
    assert_eq!(specials(&session), (false, false, true));
}

#[test]
fn default_spec_answers_only_the_default_class() {
    let mut session = session();
    register(&mut session, "dfn() { COMPREPLY=(); }");
    register(&mut session, "complete -D -F dfn");
    assert_eq!(specials(&session), (true, false, false));
}

#[test]
fn named_spec_never_leaks_into_the_special_classes() {
    let mut session = session();
    register(&mut session, "complete -W 'alpha alpine' mycmd");
    assert_eq!(specials(&session), (false, false, false));
    // The name form still sees the registration — selectivity is
    // additive, the OR gate is unchanged.
    assert!(session.has_compspec("mycmd"));
}

#[test]
fn classes_are_independent_when_several_are_registered() {
    let mut session = session();
    register(&mut session, "dfn() { COMPREPLY=(); }");
    register(&mut session, "efn() { COMPREPLY=(); }");
    register(&mut session, "complete -D -F dfn");
    register(&mut session, "complete -E -F efn");
    assert_eq!(specials(&session), (true, true, false));
}
