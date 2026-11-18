//! Host services: the capability traits of API design §5.5,
//! injected at build time on [`SessionBuilder`](super::SessionBuilder).
//! Each generalizes a mechanism that exists today; capability
//! traits return data or decisions, and they are the only places
//! a host can alter execution.
//!
//! - [`HistoryProvider`](super::HistoryProvider) — the existing
//!   trait, re-exported unchanged (the design's instruction for
//!   it is "keep"); it is not redefined here.
//! - [`ExternalCommandHandler`] — formalizes the engine's
//!   host-external-command closure setter: decide/run-or-delegate
//!   for non-builtin commands, with the session's argv and
//!   environment as data.
//! - [`ElevationHandler`] (Windows) — formalizes the elevation
//!   closure setter: sudo/elevation requests as data.
//! - [`CompletionProvider`] — backing for
//!   [`Session::complete`](super::Session::complete): commands
//!   only the host knows. In this step the session consults it
//!   when completing (its candidates merge after the engine's),
//!   which keeps the engine's own programmable-completion
//!   semantics untouched.
//! - [`RcPolicy`] — the host decides profile/rc sourcing per
//!   session (§6.6): the engine stops assuming it owns startup.
//!   Without an injected policy, the session sources the
//!   engine's own invocation lists — the same lists the shipped
//!   binary's `run_startup_files` uses.
//! - [`DiagnosticSink`] — receives [`Diagnostic`] values (§6.3)
//!   as they are produced (in this step: from
//!   [`Session::parse`](super::Session::parse)).

use std::collections::HashMap;
use std::path::PathBuf;

use super::Diagnostic;
use crate::executor::HostExternalCommandOutput;
#[cfg(windows)]
use crate::executor::{ElevationOutput, ElevationRequest};
/// Decide/run-or-delegate for a non-builtin command (§5.5).
/// Returning `None` delegates to the engine's normal spawn path;
/// returning `Some` supplies the command's whole result.
pub trait ExternalCommandHandler {
    fn handle_command(
        &mut self,
        argv: &[String],
        env: &HashMap<String, String>,
    ) -> Option<HostExternalCommandOutput>;
}

/// Sudo/elevation requests as data (Windows; §5.5).
#[cfg(windows)]
pub trait ElevationHandler {
    fn elevate(&mut self, request: ElevationRequest) -> Result<ElevationOutput, String>;
}

/// One completion candidate (§6.2): the text to insert, an
/// optional description the host's renderer may show, and the
/// engine-side classification of the entry the candidate names
/// (niubash#863 GAP-11) — the kind the producing generator knew,
/// carried so the host paints and appends without re-statting
/// engine output or guessing from the text shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub description: Option<String>,
    /// The candidate's filesystem class as the producing action
    /// classified it, or `None` when the producer could not
    /// classify it from evidence already in hand (unmarked beats
    /// wrong-marked; the host's display-side fallback for
    /// unmarked entries is GNU-faithful readline stat-at-display).
    /// Dynamic candidates (`-F` COMPREPLY, `-C` output) and
    /// provider candidates the engine did not generate are
    /// `None` unless the provider set a kind itself; non-
    /// filesystem candidates (variables, functions, builtins,
    /// ...) are `None` — the taxonomy is filesystem-only.
    pub kind: Option<CandidateKind>,
}

/// The filesystem class of one completion candidate (niubash#863
/// GAP-11): the four classes the host's LS_COLORS painting and
/// append-whitespace decision consume. GNU itself carries no
/// per-candidate kind (pcomplete.c flattens every generator into
/// a STRINGLIST; lib/readline/complete.c:1087 stats at display
/// with `path_isdir`, which follows links) — this enum is a
/// contract design decision for the two-layer engine/host split,
/// with the marks' MEANING anchored to GNU.
///
/// Generator-classification principle: kind names the entry **as
/// the producing action classified it**. A directory action
/// follows links, so a symlink-to-directory produced by one is
/// `Directory`; the same entry in a plain file listing stays
/// `Symlink`. A directory entry in a file listing is `Directory`
/// (the dirent's own class); a regular file is `File`; anything
/// the generator cannot classify from in-hand evidence (a
/// fifo/socket, an entry whose class needs a fresh stat) is
/// left unmarked (`None` on the candidate), never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    /// A directory, or a symlink to one when the producing
    /// action follows links (directory actions do).
    Directory,
    /// A symlink, as the producing action left it (a plain
    /// file listing does not follow links).
    Symlink,
    /// An executable command (a PATH-walk entry that qualified
    /// as executable under the generator's own rule).
    Executable,
    /// A regular file.
    File,
}

/// One of GNU's three special compspec slots, named by a
/// [`Session::has_special_compspec`](crate::api::Session::has_special_compspec)
/// query (rubash#2738). GNU keeps the three in separate slots —
/// pcomplete.c resolves DEFAULTCMD, EMPTYCMD and INITIALWORD
/// independently — so the selective query names exactly one class;
/// the command-name form `has_compspec` ORs all three together and
/// cannot answer for a single one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialCompspecKind {
    /// The `complete -D` default compspec: governs a command word's
    /// arguments when the command has no registration of its own,
    /// and serves the empty line as `-E`'s fallback.
    Default,
    /// The `complete -E` empty-line compspec: governs completion
    /// attempted on an empty line.
    Empty,
    /// The `complete -I` initial-word compspec: governs completion
    /// of the command word itself.
    Initial,
}

impl Candidate {
    /// A candidate without a description (and unmarked).
    pub fn new(text: impl Into<String>) -> Candidate {
        Candidate {
            text: text.into(),
            description: None,
            kind: None,
        }
    }

    /// A candidate with a description (and unmarked).
    pub fn described(text: impl Into<String>, description: impl Into<String>) -> Candidate {
        Candidate {
            text: text.into(),
            description: Some(description.into()),
            kind: None,
        }
    }

    /// A candidate classified by its producer (niubash#863
    /// GAP-11): the `kind` the generator assigned from
    /// evidence already in hand.
    pub fn kinded(text: impl Into<String>, kind: CandidateKind) -> Candidate {
        Candidate {
            text: text.into(),
            description: None,
            kind: Some(kind),
        }
    }
}

/// Host commands the engine cannot know (§5.5): consulted by
/// [`Session::complete`](super::Session::complete) for the same
/// line and cursor; the returned candidates merge after the
/// engine's own (a text the engine already produced is not
/// duplicated).
pub trait CompletionProvider {
    fn candidates(&mut self, line: &str, cursor: usize) -> Vec<Candidate>;
}

/// How the host editor should treat the engine's candidates for one
/// completion attempt — the port of pcomplete.c:1458-1477
/// `pcomp_set_readline_variables`, which bashline.c calls with the
/// USED compspec's options (compopt mutations included) to set the
/// `rl_*` variables the host's display/insertion code consumes.
/// Before rubash#2573 the `-o nospace` / `-o noquote` /
/// `-o filenames` / `-o fullquote` / `-o nosort` options were parsed
/// and printed but never crossed this channel, so no host could
/// honor them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompletionPolicy {
    /// COPT_FILENAMES → `rl_filename_completion_desired`: the
    /// candidates are filenames; the editor applies filename
    /// quoting/display treatment.
    pub filename_completion_desired: bool,
    /// COPT_FULLQUOTE → `rl_full_quoting_desired`: quote the inserted
    /// match fully, even when it contains no special characters.
    pub full_quoting_desired: bool,
    /// COPT_NOSPACE → `rl_completion_suppress_append`: insert the
    /// match WITHOUT appending a space (the input to the host's
    /// append-whitespace decision).
    pub suppress_append: bool,
    /// NOT COPT_NOQUOTE → `rl_filename_quoting_desired`: filenames
    /// among the candidates may be quoted on insertion. True unless
    /// the governing compspec carries `-o noquote`.
    pub filename_quoting_desired: bool,
    /// NOT COPT_NOSORT → `rl_sort_completion_matches`: the host may
    /// sort the matches for display; with `-o nosort` the engine
    /// preserves generation order and the host must not re-sort.
    /// True unless the governing compspec carries `-o nosort`.
    pub sort_matches: bool,
    /// Filenames-intent (niubash#863 GAP-11, the Q-D rider): true
    /// when the attempt's generation was filename-flavored — the
    /// governing spec's generation included file/directory
    /// actions, a filename fallback sourced candidates
    /// (`-o dirnames` / `-o plusdirs` / `-o default`, or the
    /// no-spec filename fallback), or `-o filenames` is in the
    /// effective options. A truthful spec-intent fact about the
    /// ATTEMPT (GNU analogue: `rl_filename_completion_desired`),
    /// never a per-candidate claim: dynamic candidates (`-F`
    /// COMPREPLY, `-C` output) stay individually unmarked
    /// (`Candidate.kind` is `None`) even when this bit is set,
    /// because the engine did not classify user text. It is the
    /// wider sibling of `filename_completion_desired`, which
    /// records only the `-o filenames` option bit.
    pub filenames_intent: bool,
}

impl Default for CompletionPolicy {
    fn default() -> Self {
        CompletionPolicy {
            filename_completion_desired: false,
            full_quoting_desired: false,
            suppress_append: false,
            filename_quoting_desired: true,
            sort_matches: true,
            filenames_intent: false,
        }
    }
}

/// The result of one engine completion attempt: the candidates plus
/// the insertion/display policy of the compspec that governed the
/// attempt (see [`super::Session::complete_detailed`]).
#[derive(Debug, Clone, Default)]
pub struct CompletionOutcome {
    pub candidates: Vec<Candidate>,
    pub policy: CompletionPolicy,
}

/// Receives [`Diagnostic`] values as they are produced (§6.3).
pub trait DiagnosticSink {
    fn diagnostic(&mut self, diagnostic: &Diagnostic);
}

/// Which startup-file class a sourcing decision is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StartupClass {
    /// The login profile chain (`/etc/profile`, then the first
    /// existing personal profile).
    LoginProfile,
    /// The non-login interactive rc chain.
    InteractiveRc,
    /// The `$BASH_ENV` file of a non-interactive shell.
    EnvFile,
}

/// What an [`RcPolicy`] decides from: the session facts the
/// engine's own list functions consult, precomputed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StartupContext {
    /// The session's `$HOME`, when set.
    pub home: Option<String>,
    /// The `--rcfile` replacement for the personal-rc slot, when
    /// the host supplied one.
    pub rc_override: Option<String>,
    /// The expanded `$BASH_ENV` file, when it resolves to an
    /// existing regular file.
    pub bash_env: Option<String>,
}

/// The host decides startup sourcing per session (§5.5, §6.6).
/// The returned files are sourced in order with the engine's
/// startup-file semantics (a parse error aborts only that file;
/// an `exit` in one ends the chain and is reported through the
/// run outcome's flow).
pub trait RcPolicy {
    fn startup_files(&mut self, class: StartupClass, context: &StartupContext) -> Vec<PathBuf>;
}
