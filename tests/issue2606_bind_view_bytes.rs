//! rubash#2606, GAP-3 face — the tier-A bind view's `bytes`
//! field must be the registry's byte identity, not the
//! char-domain translation of the stored transport string.
//!
//! The registry keys slots by byte identity: `keyseq_identity`
//! (src/shell/bind_registry.rs) folds the written form back to
//! the byte string the script produced — a raw byte that rides
//! the raw-byte marker pair protocol (RAW_BYTE_MARKER_*,
//! src/executor/markers.rs) is folded to the byte itself — and
//! runs rl_translate_keyseq (lib/readline/bind.c:516) on those
//! bytes. The view (niubash#863 GAP-3) is the host's mirror of
//! that registry, so its `bytes` field is that same identity:
//! when capture instead ran the char-domain `keyseq_bytes` over
//! the stored string, a binding registered under the raw-byte
//! spelling (`bind -x $'"\025": …'`) reported the marker pair's
//! UTF-8 (EE 80 80 EE 80 96) as its bytes — the carrier protocol
//! leaking through the API face, and a view that disagreed with
//! the registry's own slot for the same entry.
//!
//! These pins drive registrations through a Session's own
//! `bind` commands and read them back through
//! `Session::bind_view` only — the consumer face the GAP-3
//! contract pins use. Spellings here are produced by the shell
//! itself ($'\025'), never written as marker literals.

use rubash::api::{BindKindView, RunOptions, Session};

fn view_session() -> Session {
    // An empty INPUTRC keeps the host's own inputrc out of the
    // registry: the first `bind` runs initialize_readline.
    #[cfg(unix)]
    const EMPTY_INPUTRC: &str = "/dev/null";
    #[cfg(windows)]
    const EMPTY_INPUTRC: &str = "NUL";
    Session::builder()
        .env("INPUTRC", EMPTY_INPUTRC)
        .build()
        .expect("session builds")
}

fn view_run(session: &mut Session, source: &str) {
    let outcome = session.run(source, &RunOptions::new()).expect("run");
    assert_eq!(outcome.status, 0, "run of {source:?}");
}

#[test]
fn issue2606_bind_view_entry_bytes_fold_raw_byte_spelling() {
    let mut session = view_session();
    // Control: the plain backslash spelling. The char-domain
    // translation and the byte identity agree on any spelling
    // that carries no marker pairs, so this entry reads the
    // same before and after the fix.
    view_run(&mut session, r#"bind -x '"\C-a": echo plain-ctrl-a'"#);
    // The raw-byte spelling: $'\025' reaches the builtin as the
    // marker-pair transport form of byte 0x15, and that pair
    // form is the entry's stored keyseq.
    view_run(&mut session, r#"bind -x $'"\025": echo pair-ctrl-u'"#);
    let view = session.bind_view();
    let entry_for = |command: &str| {
        view.entries
            .iter()
            .find(
                |entry| matches!(&entry.kind, BindKindView::Execute { command: c } if c == command),
            )
            .unwrap_or_else(|| panic!("entry for {command} registered"))
    };
    assert_eq!(entry_for("echo plain-ctrl-a").bytes, vec![0x01]);
    // The stored keyseq keeps its transport spelling (the
    // GAP-3 split: keyseq is the written form, bytes the
    // translation) — it is not the "\C-u" spelling…
    let pair_entry = entry_for("echo pair-ctrl-u");
    assert_ne!(pair_entry.keyseq, "\\C-u");
    // …but its bytes are the decoded identity, the same bytes
    // the "\C-u" spelling reports: the byte 0x15 itself, never
    // the marker pair's UTF-8.
    assert_eq!(pair_entry.bytes, vec![0x15]);
}

#[test]
fn issue2606_bind_view_removal_bytes_fold_raw_byte_spelling() {
    let mut session = view_session();
    view_run(&mut session, r#"bind -x '"\C-a": echo plain-ctrl-a'"#);
    view_run(&mut session, r#"bind -x $'"\025": echo pair-ctrl-u'"#);
    // Remove both under their own spellings: the tombstone
    // recorded for the raw-byte removal keeps the pair spelling
    // as its written form, and its view bytes must fold to the
    // byte identity exactly like the entry's did.
    view_run(&mut session, r"bind -r $'\025'");
    view_run(&mut session, r"bind -r '\C-a'");
    let view = session.bind_view();
    assert_eq!(view.removals.len(), 2, "removals: {:?}", view.removals);
    let folded: Vec<Vec<u8>> = view
        .removals
        .iter()
        .map(|removal| removal.bytes.clone())
        .collect();
    assert!(
        folded.contains(&vec![0x15]),
        "pair-spelling tombstone folds to [0x15], got {folded:?}"
    );
    assert!(
        folded.contains(&vec![0x01]),
        "plain-spelling tombstone stays [0x01], got {folded:?}"
    );
}
