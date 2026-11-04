//! The completion-state view: GAP-4 of the niubash#863 API
//! queue (with GAP-4b, the `&self` `Session::has_function` —
//! that face itself lives beside `Session::functions`).
//!
//! A completer reads live session state: where the session
//! stands, the whole variable table (its content is the
//! highlighter probe's memo key), the function names, the
//! open fds behind the `/dev/fd/` spellings. Until GAP-4 the
//! Session contract carried only pieces of that — `cwd`, the
//! sorted function names, the exported subset of the
//! environment — so a host mirrored all of it through the
//! tier-C executor seam into a snapshot struct of its own and
//! re-synchronized the mirror after every line, with its own
//! fingerprint and bookkeeping beside the engine's stores
//! (the `CompletionState` machinery: niubash#594 / #627 /
//! #831 / #833 / #840 are its failure modes). This module is
//! the tier-A read face: one owned snapshot, captured through
//! `&self` from the executor's own snapshot functions at the
//! moment it is taken — the API and the shell can never
//! disagree about the completer's inputs (the module rule in
//! `super`). No shadow bookkeeping: nothing here renders,
//! sorts or counts anything the executor does not already
//! render, sort or count for its own consumers.
//!
//! Deliberately NOT in the view: the host's `/tmp` answer. A
//! host takes it from `executor().resolve_shell_path("/tmp")`
//! today; that is the path resolver's face, GAP-5's object,
//! and it does not ride this snapshot.

use crate::executor::Executor;
use std::path::PathBuf;

/// One capture of the state a completer reads: the session
/// cwd, the full variable table with its content fingerprint,
/// the function names, and the open fd numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionView {
    /// The session's current working directory, physical form
    /// — `Executor::session_cwd` verbatim (the field cd /
    /// pushd / popd move; the process cwd is never a
    /// session-semantics source).
    pub cwd: PathBuf,
    /// The whole variable table: `Executor::env_vars_snapshot`'s
    /// own rendering — every variable with its value, plus the
    /// attribute pseudo-entries (`__RUBASH_EXPORTED_VARS`,
    /// `__RUBASH_READONLY_VARS`, …) the snapshot materializes
    /// from the attribute map — presented sorted by name (the
    /// snapshot itself is a map; the sort is presentation of
    /// its items, not a second rendering). Deliberately wider
    /// than [`Session::environment`](super::Session::environment),
    /// which is the exported subset children inherit: a
    /// completer completes on unexported names too.
    pub env: Vec<(String, String)>,
    /// A content fingerprint of `env` exactly as captured
    /// above: the same state always yields the same
    /// fingerprint, and any change to a variable's name, value
    /// or attribute rendering changes it. This is the designed
    /// form of the memo key hosts used to derive from the
    /// tier-C `env_vars()` read. The algorithm is FNV-1a
    /// (64-bit) over the sorted pairs' length-prefixed names
    /// and values; it is a change detector, not a digest —
    /// equality is the only contract.
    pub env_fingerprint: u64,
    /// Function names currently defined, sorted —
    /// `Executor::functions_snapshot`, the same list
    /// [`Session::functions`](super::Session::functions)
    /// returns.
    pub functions: Vec<String>,
    /// The session's open fd numbers, ascending — the fd
    /// table's live endpoints, `Executor::open_fds` verbatim:
    /// the numbers the `/dev/fd/N` spellings resolve against.
    pub open_fds: Vec<u32>,
}

impl CompletionView {
    /// Capture the view from the executor's own snapshot
    /// functions: every field is one of those snapshots read
    /// at this moment, so the view can only ever agree with
    /// the reads hosts used to make through the tier-C seam.
    pub(crate) fn capture(executor: &Executor) -> CompletionView {
        let mut env: Vec<(String, String)> = executor.env_vars_snapshot().into_iter().collect();
        // The snapshot is a map; sorting its items by name is
        // the presentation order, and (names being unique) is
        // the same order whatever the map's iteration was.
        env.sort();
        let env_fingerprint = env_fingerprint(&env);
        CompletionView {
            cwd: executor.session_cwd().to_path_buf(),
            env,
            env_fingerprint,
            functions: executor.functions_snapshot(),
            open_fds: executor.open_fds(),
        }
    }
}

/// FNV-1a (64-bit) over the sorted pairs, each name and value
/// fed length-prefixed so pair boundaries can never blur
/// (`("ab", "c")` and `("a", "bc")` fingerprint differently).
fn env_fingerprint(env: &[(String, String)]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    fn feed(mut hash: u64, bytes: &[u8]) -> u64 {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash
    }
    let mut hash = OFFSET;
    for (name, value) in env {
        hash = feed(hash, &(name.len() as u64).to_le_bytes());
        hash = feed(hash, name.as_bytes());
        hash = feed(hash, &(value.len() as u64).to_le_bytes());
        hash = feed(hash, value.as_bytes());
    }
    hash
}
