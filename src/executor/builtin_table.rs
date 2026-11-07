//! The builtin declaration table (Stage-2 project ④, D1).
//!
//! One `const` row per builtin name — the single source of truth the
//! design derives everything else from. The rows are the 62 names of
//! `BUILTIN_NAMES` (the 61 GNU `shell_builtins` entries plus rubash's
//! own `env` extension), each carrying:
//!
//! * `handler` — the canonical processor identity (`HandlerSlot`), one
//!   variant per distinct dispatch arm across the four existing slot
//!   enums (`PrimarySlot` / `LateSlot` / `CmdPrimarySlot` /
//!   `CmdLateSlot` / `PipelineSlot` in `command_registry.rs`). Names
//!   sharing one arm share the variant (`.`/`source` → `Source`,
//!   `declare`/`typeset` → `Declare`, `mapfile`/`readarray` →
//!   `Mapfile`), exactly as GNU's `shell_builtins` table shares one
//!   function pointer for those pairs.
//! * `flags` — the GNU `builtins.h:40-48` flag bits, read off the
//!   generated `shell_builtins` table of the GNU 5.3.20 oracle build
//!   (`build-gnu/bash-5.3/builtins/builtins.c`): `SPECIAL_BUILTIN`
//!   (16), `POSIX_BUILTIN` (19), `ASSIGNMENT_BUILTIN` (6),
//!   `LOCALVAR_BUILTIN` (3), `ARRAYREF_BUILTIN` (10). Every static
//!   builtin starts `BUILTIN_ENABLED` and `enable -n` clears it for
//!   any of them — verified live on the oracle (`enable -n set` then
//!   `set: command not found`) — so `DISABLEABLE` is set on all 62
//!   rows. `env` carries only `DISABLEABLE`: it is rubash's own
//!   extension, present in no GNU flag table.
//! * `opt_spec` — the getopt specification, read off each builtin's
//!   `.def` (`internal_getopt(list, "...")`, `no_options(list)`, or
//!   no scan at all). Dynamic specs are recorded at their constructed
//!   value: `set` is `flags.c:208/377-380`'s `optflags`
//!   (`"+abefhikmnprtuvxlBCEHPTo;"`), `ulimit` is
//!   `ulimit.def:346-356`'s built `optstring`
//!   (`"aSHP;R;T;b;c;d;e;f;i;k;l;m;n;p;q;r;s;t;u;v;w;x;"`),
//!   `declare`/`local`/`typeset` is `DECLARE_OPTS`
//!   (`"+acfgilnprtuxAFGI"`, ARRAY_VARS build),
//!   `export`/`readonly` is `ATTROPTS` (`"aAfnp"`). `cd` is
//!   `"eLP@"` (the `O_XATTR` build — the oracle's `cd --help` usage
//!   line carries `[-@]`). `fc` keeps its verbatim leading `':'`
//!   (`":e:lnrs"`; inert in `bashgetopt.c`, which rejects the option
//!   char `':'` before consulting the spec).
//! * `doc` — the `HELP_TOPICS_TABLE` key for the row's long help
//!   (`help_texts.rs`); `env` has no topic (`DocId::None`).
//! * `help_entry` — whether `--help` as an option word prints the
//!   builtin's long help (status 2). All `Getopt` rows have it
//!   (`bashgetopt.c`'s `ISHELP` branch, `common.h:27`, fires in
//!   option position). `Grammar`/`NoScan` rows have it only when
//!   their `.def` calls `CHECK_HELPOPT` (`kill`, `let`, `break`,
//!   `continue`, `exit`, `logout`, `return`, `shift`, `pushd`,
//!   `popd`, `dirs`); `:`, `true`, `false`, `echo`, `test`, `[` and
//!   `env` do NOT — probed live on the oracle (`: --help` prints
//!   nothing, rc 0; `echo --help` echoes the word; `test --help`
//!   is rc 0 silent). The design's §4.2 wording ("CHECK_HELPOPT
//!   ...统一经分发器，`--help` 不因文法形豁免") would diverge from
//!   GNU on exactly those seven rows; the dispatcher (Phase 1)
//!   must consult this bit, not force `--help` uniformly.
//!
//! Phase 0 is additive only: the table, its projections and its
//! derivatives land here, pinned equal to the existing hand-written
//! sources by the tests below. No consumer is switched except the
//! three named derivatives (`builtin_names()`, the special-builtin
//! projection, the `enable -p/-n` listing) — and each of those is
//! itself pinned equal to the source it replaces. The hand-written
//! `BUILTIN_NAMES`, `SPECIAL_BUILTINS` and the five dispatch tables
//! stay in place until Phase 6 retires them.

use super::command_registry::{
    CmdLateSlot, CmdPrimarySlot, Introspection, LateSlot, PipelineSlot, PrimarySlot,
};
use std::ops::BitOr;

/// One canonical processor identity: the distinct dispatch arm a
/// builtin name resolves to, across the four dispatch paths. The
/// five path enums keep their own variants (they are the paths'
/// execution contexts); this enum is the per-builtin identity the
/// projections map back onto them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum HandlerSlot {
    Alias,
    Bg,
    Bind,
    Break,
    Builtin,
    Caller,
    Cd,
    Colon,
    Command,
    Compgen,
    Complete,
    Compopt,
    Continue,
    Declare,
    Dirs,
    Disown,
    Echo,
    Enable,
    Env,
    Eval,
    Exec,
    Exit,
    Export,
    False,
    Fc,
    Fg,
    Getopts,
    Hash,
    Help,
    History,
    Jobs,
    Kill,
    Let,
    Local,
    Logout,
    Mapfile,
    Popd,
    Printf,
    Pushd,
    Pwd,
    Read,
    Readonly,
    Return,
    Set,
    Shift,
    Shopt,
    Source,
    Suspend,
    Test,
    TestBracket,
    Times,
    Trap,
    True,
    Type,
    Ulimit,
    Umask,
    Unalias,
    Unset,
    Wait,
}

impl HandlerSlot {
    /// The primary-dispatch arm for this handler, if the name is a
    /// primary claim (`PRIMARY_DISPATCH`, minus the `sleep`
    /// fast-path entry which is not a builtin row).
    pub(crate) fn primary_slot(self) -> Option<PrimarySlot> {
        Some(match self {
            HandlerSlot::Alias => PrimarySlot::Alias,
            HandlerSlot::Break => PrimarySlot::Break,
            HandlerSlot::Builtin => PrimarySlot::Builtin,
            HandlerSlot::Cd => PrimarySlot::Cd,
            HandlerSlot::Colon => PrimarySlot::Colon,
            HandlerSlot::Command => PrimarySlot::Command,
            HandlerSlot::Continue => PrimarySlot::Continue,
            HandlerSlot::Declare => PrimarySlot::Declare,
            HandlerSlot::Dirs => PrimarySlot::Dirs,
            HandlerSlot::Echo => PrimarySlot::Echo,
            HandlerSlot::Enable => PrimarySlot::Enable,
            HandlerSlot::Env => PrimarySlot::Env,
            HandlerSlot::Eval => PrimarySlot::Eval,
            HandlerSlot::Exec => PrimarySlot::Exec,
            HandlerSlot::Exit => PrimarySlot::Exit,
            HandlerSlot::Export => PrimarySlot::Export,
            HandlerSlot::False => PrimarySlot::False,
            HandlerSlot::Getopts => PrimarySlot::Getopts,
            HandlerSlot::Hash => PrimarySlot::Hash,
            HandlerSlot::Help => PrimarySlot::Help,
            HandlerSlot::Kill => PrimarySlot::Kill,
            HandlerSlot::Local => PrimarySlot::Local,
            HandlerSlot::Logout => PrimarySlot::Logout,
            HandlerSlot::Popd => PrimarySlot::Popd,
            HandlerSlot::Printf => PrimarySlot::Printf,
            HandlerSlot::Pushd => PrimarySlot::Pushd,
            HandlerSlot::Pwd => PrimarySlot::Pwd,
            HandlerSlot::Readonly => PrimarySlot::Readonly,
            HandlerSlot::Return => PrimarySlot::Return,
            HandlerSlot::Set => PrimarySlot::Set,
            HandlerSlot::Shopt => PrimarySlot::Shopt,
            HandlerSlot::Source => PrimarySlot::Source,
            HandlerSlot::True => PrimarySlot::True,
            HandlerSlot::Unalias => PrimarySlot::Unalias,
            _ => return None,
        })
    }

    /// The late-dispatch arm for this handler, if the name is a late
    /// claim (`LATE_DISPATCH`, minus the non-builtin entries `((`,
    /// `[[`, `arch`, `basename`, `dirname`, `stty`, `time`, `uname`).
    pub(crate) fn late_slot(self) -> Option<LateSlot> {
        Some(match self {
            HandlerSlot::Bg => LateSlot::Bg,
            HandlerSlot::Bind => LateSlot::Bind,
            HandlerSlot::Caller => LateSlot::Caller,
            HandlerSlot::Compgen => LateSlot::Compgen,
            HandlerSlot::Complete => LateSlot::Complete,
            HandlerSlot::Compopt => LateSlot::Compopt,
            HandlerSlot::Disown => LateSlot::Disown,
            HandlerSlot::Fc => LateSlot::Fc,
            HandlerSlot::Fg => LateSlot::Fg,
            HandlerSlot::History => LateSlot::History,
            HandlerSlot::Jobs => LateSlot::Jobs,
            HandlerSlot::Let => LateSlot::Let,
            HandlerSlot::Mapfile => LateSlot::Mapfile,
            HandlerSlot::Read => LateSlot::Read,
            HandlerSlot::Shift => LateSlot::Shift,
            HandlerSlot::Suspend => LateSlot::Suspend,
            HandlerSlot::Test => LateSlot::Test,
            HandlerSlot::TestBracket => LateSlot::TestBracket,
            HandlerSlot::Times => LateSlot::Times,
            HandlerSlot::Trap => LateSlot::Trap,
            HandlerSlot::Type => LateSlot::Type,
            HandlerSlot::Ulimit => LateSlot::Ulimit,
            HandlerSlot::Umask => LateSlot::Umask,
            HandlerSlot::Unset => LateSlot::Unset,
            HandlerSlot::Wait => LateSlot::Wait,
            _ => return None,
        })
    }

    /// The `command`-prefix primary arm for this handler, if the
    /// name is a `CMD_PRIMARY_DISPATCH` claim (all 43 entries are
    /// builtin rows; note `env` has no entry here — the
    /// `command`-prefix path does not claim it).
    pub(crate) fn cmd_primary_slot(self) -> Option<CmdPrimarySlot> {
        Some(match self {
            HandlerSlot::Alias => CmdPrimarySlot::Alias,
            HandlerSlot::Break => CmdPrimarySlot::Break,
            HandlerSlot::Builtin => CmdPrimarySlot::Builtin,
            HandlerSlot::Cd => CmdPrimarySlot::Cd,
            HandlerSlot::Colon => CmdPrimarySlot::Colon,
            HandlerSlot::Command => CmdPrimarySlot::Command,
            HandlerSlot::Continue => CmdPrimarySlot::Continue,
            HandlerSlot::Declare => CmdPrimarySlot::Declare,
            HandlerSlot::Dirs => CmdPrimarySlot::Dirs,
            HandlerSlot::Echo => CmdPrimarySlot::Echo,
            HandlerSlot::Enable => CmdPrimarySlot::Enable,
            HandlerSlot::Eval => CmdPrimarySlot::Eval,
            HandlerSlot::Exec => CmdPrimarySlot::Exec,
            HandlerSlot::Exit => CmdPrimarySlot::Exit,
            HandlerSlot::Export => CmdPrimarySlot::Export,
            HandlerSlot::False => CmdPrimarySlot::False,
            HandlerSlot::Getopts => CmdPrimarySlot::Getopts,
            HandlerSlot::Hash => CmdPrimarySlot::Hash,
            HandlerSlot::Help => CmdPrimarySlot::Help,
            HandlerSlot::Kill => CmdPrimarySlot::Kill,
            HandlerSlot::Let => CmdPrimarySlot::Let,
            HandlerSlot::Local => CmdPrimarySlot::Local,
            HandlerSlot::Logout => CmdPrimarySlot::Logout,
            HandlerSlot::Mapfile => CmdPrimarySlot::Mapfile,
            HandlerSlot::Popd => CmdPrimarySlot::Popd,
            HandlerSlot::Printf => CmdPrimarySlot::Printf,
            HandlerSlot::Pushd => CmdPrimarySlot::Pushd,
            HandlerSlot::Pwd => CmdPrimarySlot::Pwd,
            HandlerSlot::Read => CmdPrimarySlot::Read,
            HandlerSlot::Readonly => CmdPrimarySlot::Readonly,
            HandlerSlot::Return => CmdPrimarySlot::Return,
            HandlerSlot::Set => CmdPrimarySlot::Set,
            HandlerSlot::Shift => CmdPrimarySlot::Shift,
            HandlerSlot::Shopt => CmdPrimarySlot::Shopt,
            HandlerSlot::Source => CmdPrimarySlot::Source,
            HandlerSlot::True => CmdPrimarySlot::True,
            HandlerSlot::Ulimit => CmdPrimarySlot::Ulimit,
            HandlerSlot::Umask => CmdPrimarySlot::Umask,
            HandlerSlot::Unalias => CmdPrimarySlot::Unalias,
            HandlerSlot::Unset => CmdPrimarySlot::Unset,
            _ => return None,
        })
    }

    /// The `command`-prefix late arm for this handler, if the name
    /// is a `CMD_LATE_DISPATCH` claim (minus the non-builtin
    /// entries `arch`, `basename`, `dirname`, `stty`, `uname`).
    pub(crate) fn cmd_late_slot(self) -> Option<CmdLateSlot> {
        Some(match self {
            HandlerSlot::Bg => CmdLateSlot::Bg,
            HandlerSlot::Bind => CmdLateSlot::Bind,
            HandlerSlot::Caller => CmdLateSlot::Caller,
            HandlerSlot::Compgen => CmdLateSlot::Compgen,
            HandlerSlot::Complete => CmdLateSlot::Complete,
            HandlerSlot::Compopt => CmdLateSlot::Compopt,
            HandlerSlot::Disown => CmdLateSlot::Disown,
            HandlerSlot::Fc => CmdLateSlot::Fc,
            HandlerSlot::Fg => CmdLateSlot::Fg,
            HandlerSlot::History => CmdLateSlot::History,
            HandlerSlot::Jobs => CmdLateSlot::Jobs,
            HandlerSlot::Suspend => CmdLateSlot::Suspend,
            HandlerSlot::Test => CmdLateSlot::Test,
            HandlerSlot::TestBracket => CmdLateSlot::TestBracket,
            HandlerSlot::Times => CmdLateSlot::Times,
            HandlerSlot::Trap => CmdLateSlot::Trap,
            HandlerSlot::Type => CmdLateSlot::Type,
            HandlerSlot::Wait => CmdLateSlot::Wait,
            _ => return None,
        })
    }

    /// The pipeline-stage arm for this handler, if the name is a
    /// `PIPELINE_STAGE` builtin claim. `:` and `true` share the
    /// stage's `TrueColon` arm; the seven tool names (`cat`, `grep`,
    /// `head`, `sed`, `tput`, `tr`, `wc`) are not builtin rows and
    /// stay in the stage table.
    pub(crate) fn pipeline_slot(self) -> Option<PipelineSlot> {
        Some(match self {
            HandlerSlot::Colon => PipelineSlot::TrueColon,
            HandlerSlot::Echo => PipelineSlot::Echo,
            HandlerSlot::False => PipelineSlot::False,
            HandlerSlot::Printf => PipelineSlot::Printf,
            HandlerSlot::Trap => PipelineSlot::Trap,
            HandlerSlot::True => PipelineSlot::TrueColon,
            _ => return None,
        })
    }
}

/// The GNU `builtins.h:40-48` flag bits, on their GNU values so a
/// row's bits compare directly against the oracle's generated
/// `shell_builtins` table (minus `BUILTIN_ENABLED`'s siblings
/// `BUILTIN_DELETED`/`STATIC_BUILTIN`, which are not row facts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinFlags(pub u8);

impl BuiltinFlags {
    /// `BUILTIN_ENABLED` 0x01, the mutable face `enable -n`
    /// governs: every static builtin starts enabled and any of
    /// them can be disabled (verified on the oracle, incl. the
    /// special builtins).
    pub const DISABLEABLE: Self = Self(0x01);
    /// `SPECIAL_BUILTIN` 0x08: the POSIX special builtins (16).
    pub const SPECIAL: Self = Self(0x08);
    /// `POSIX_BUILTIN` 0x20: special in the POSIX command-search
    /// order (19).
    pub const POSIX_EXTENDED: Self = Self(0x20);
    /// `ASSIGNMENT_BUILTIN` 0x10: eats assignment statements (6).
    pub const ASSIGNMENT: Self = Self(0x10);
    /// `LOCALVAR_BUILTIN` 0x40: creates local variables (3).
    pub const LOCALVAR: Self = Self(0x40);
    /// `ARRAYREF_BUILTIN` 0x80: eats array-reference arguments (10).
    pub const ARRAYREF: Self = Self(0x80);
    pub const EMPTY: Self = Self(0);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for BuiltinFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BuiltinFlags {
    /// Const `|` for the table: `BitOr` cannot be `const` on stable
    /// Rust (`const_trait_impl` is unstable), so multi-flag rows
    /// spell the union with this method — `F::DISABLEABLE
    /// .or(F::SPECIAL)` — instead of the `|` operator. Runtime code
    /// keeps using `|`.
    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The option-scanning shape of a builtin, from its `.def`:
/// how `BuiltinOpt` must treat its argument words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptSpec {
    /// Scanned by `internal_getopt` with this spec string, verbatim
    /// from the `.def` (`""` = `no_options`: any option word is
    /// invalid; a leading `'+'` is plus mode; `':'` required arg,
    /// `';'` optional arg, `'#'` numeric arg).
    Getopt(&'static str),
    /// Self-parsing grammar (`test`/`[`, `let`, `kill`, and the
    /// rubash extension `env`): never passes through
    /// `internal_getopt`; the machine does not scan it.
    Grammar,
    /// Fixed positional arguments, no option scan at all (`:`,
    /// `true`, `false`, `echo`, `break`, `continue`, `exit`,
    /// `logout`, `return`, `shift`, `pushd`, `popd`, `dirs`):
    /// the machine returns the words untouched as operands.
    NoScan,
}

/// The help-table index of a builtin row (`HELP_TOPICS_TABLE` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocId {
    /// The row's topic in `HELP_TOPICS_TABLE`.
    Topic(&'static str),
    /// No help topic exists (rubash's own `env` extension).
    None,
}

impl DocId {
    /// The row's short documentation line (`short_doc`), if the
    /// row has a help topic.
    pub(crate) fn short_doc(self) -> Option<&'static str> {
        match self {
            DocId::Topic(name) => crate::builtins::help::builtin_short_doc(name),
            DocId::None => None,
        }
    }
}

/// One row of the builtin declaration table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinRow {
    /// The name as typed (`"declare"`, `"."`, `"["`).
    pub name: &'static str,
    /// The canonical processor identity.
    pub handler: HandlerSlot,
    /// The GNU `builtins.h` flag bits.
    pub flags: BuiltinFlags,
    /// The getopt specification (`§4.2`).
    pub opt_spec: OptSpec,
    /// The help-table index (`§4.4`; `DocId::None` for `env`).
    pub doc: DocId,
    /// The `type` / `command -v` answer category.
    pub introspection: Introspection,
    /// Whether `--help` in option position prints the long help
    /// (status 2). True for every `Getopt` row and for the
    /// `Grammar`/`NoScan` rows whose `.def` calls `CHECK_HELPOPT`;
    /// false for `:`, `true`, `false`, `echo`, `test`, `[`, `env`
    /// (probed on the oracle — see the module doc).
    pub help_entry: bool,
}

use BuiltinFlags as F;
use DocId as D;
use HandlerSlot as H;
use Introspection as I;
use OptSpec as O;

/// The declaration table: one row per builtin name, sorted by
/// name (byte order, matching `BUILTIN_NAMES`). A `const` (not a
/// `static`) so the member set can be derived from it in a `const fn`.
#[rustfmt::skip]
pub(crate) const BUILTIN_TABLE: &[BuiltinRow] = &[
    BuiltinRow { name: ".", handler: H::Source, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt("p:"), doc: D::Topic("."), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: ":", handler: H::Colon, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic(":"), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "[", handler: H::TestBracket, flags: F::DISABLEABLE.or(F::ARRAYREF), opt_spec: O::Grammar, doc: D::Topic("["), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "alias", handler: H::Alias, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED).or(F::ASSIGNMENT), opt_spec: O::Getopt("p"), doc: D::Topic("alias"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "bg", handler: H::Bg, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt(""), doc: D::Topic("bg"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "bind", handler: H::Bind, flags: F::DISABLEABLE, opt_spec: O::Getopt("lvpVPsSXf:q:u:m:r:x:"), doc: D::Topic("bind"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "break", handler: H::Break, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic("break"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "builtin", handler: H::Builtin, flags: F::DISABLEABLE, opt_spec: O::Getopt(""), doc: D::Topic("builtin"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "caller", handler: H::Caller, flags: F::DISABLEABLE, opt_spec: O::Getopt(""), doc: D::Topic("caller"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "cd", handler: H::Cd, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("eLP@"), doc: D::Topic("cd"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "command", handler: H::Command, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("pvV"), doc: D::Topic("command"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "compgen", handler: H::Compgen, flags: F::DISABLEABLE, opt_spec: O::Getopt("abcdefgjko:prsuvA:G:W:P:S:X:F:C:V:DEI"), doc: D::Topic("compgen"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "complete", handler: H::Complete, flags: F::DISABLEABLE, opt_spec: O::Getopt("abcdefgjko:prsuvA:G:W:P:S:X:F:C:V:DEI"), doc: D::Topic("complete"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "compopt", handler: H::Compopt, flags: F::DISABLEABLE, opt_spec: O::Getopt("+o:DEI"), doc: D::Topic("compopt"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "continue", handler: H::Continue, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic("continue"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "declare", handler: H::Declare, flags: F::DISABLEABLE.or(F::ASSIGNMENT).or(F::LOCALVAR).or(F::ARRAYREF), opt_spec: O::Getopt("+acfgilnprtuxAFGI"), doc: D::Topic("declare"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "dirs", handler: H::Dirs, flags: F::DISABLEABLE, opt_spec: O::NoScan, doc: D::Topic("dirs"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "disown", handler: H::Disown, flags: F::DISABLEABLE, opt_spec: O::Getopt("ahr"), doc: D::Topic("disown"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "echo", handler: H::Echo, flags: F::DISABLEABLE, opt_spec: O::NoScan, doc: D::Topic("echo"), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "enable", handler: H::Enable, flags: F::DISABLEABLE, opt_spec: O::Getopt("adnpsf:"), doc: D::Topic("enable"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "env", handler: H::Env, flags: F::DISABLEABLE, opt_spec: O::Grammar, doc: D::None, introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "eval", handler: H::Eval, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt(""), doc: D::Topic("eval"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "exec", handler: H::Exec, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt("cla:"), doc: D::Topic("exec"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "exit", handler: H::Exit, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic("exit"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "export", handler: H::Export, flags: F::DISABLEABLE.or(F::SPECIAL).or(F::ASSIGNMENT), opt_spec: O::Getopt("aAfnp"), doc: D::Topic("export"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "false", handler: H::False, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::NoScan, doc: D::Topic("false"), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "fc", handler: H::Fc, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt(":e:lnrs"), doc: D::Topic("fc"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "fg", handler: H::Fg, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt(""), doc: D::Topic("fg"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "getopts", handler: H::Getopts, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt(""), doc: D::Topic("getopts"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "hash", handler: H::Hash, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("dlp:rt"), doc: D::Topic("hash"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "help", handler: H::Help, flags: F::DISABLEABLE, opt_spec: O::Getopt("dms"), doc: D::Topic("help"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "history", handler: H::History, flags: F::DISABLEABLE, opt_spec: O::Getopt("acd:npsrw"), doc: D::Topic("history"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "jobs", handler: H::Jobs, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("lpnxrs"), doc: D::Topic("jobs"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "kill", handler: H::Kill, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Grammar, doc: D::Topic("kill"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "let", handler: H::Let, flags: F::DISABLEABLE.or(F::ARRAYREF), opt_spec: O::Grammar, doc: D::Topic("let"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "local", handler: H::Local, flags: F::DISABLEABLE.or(F::ASSIGNMENT).or(F::LOCALVAR).or(F::ARRAYREF), opt_spec: O::Getopt("+acfgilnprtuxAFGI"), doc: D::Topic("local"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "logout", handler: H::Logout, flags: F::DISABLEABLE, opt_spec: O::NoScan, doc: D::Topic("logout"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "mapfile", handler: H::Mapfile, flags: F::DISABLEABLE, opt_spec: O::Getopt("d:u:n:O:tC:c:s:"), doc: D::Topic("mapfile"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "popd", handler: H::Popd, flags: F::DISABLEABLE, opt_spec: O::NoScan, doc: D::Topic("popd"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "printf", handler: H::Printf, flags: F::DISABLEABLE.or(F::ARRAYREF), opt_spec: O::Getopt("v:"), doc: D::Topic("printf"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "pushd", handler: H::Pushd, flags: F::DISABLEABLE, opt_spec: O::NoScan, doc: D::Topic("pushd"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "pwd", handler: H::Pwd, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("LP"), doc: D::Topic("pwd"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "read", handler: H::Read, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED).or(F::ARRAYREF), opt_spec: O::Getopt("Eersa:d:i:n:p:t:u:N:"), doc: D::Topic("read"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "readarray", handler: H::Mapfile, flags: F::DISABLEABLE, opt_spec: O::Getopt("d:u:n:O:tC:c:s:"), doc: D::Topic("readarray"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "readonly", handler: H::Readonly, flags: F::DISABLEABLE.or(F::SPECIAL).or(F::ASSIGNMENT), opt_spec: O::Getopt("aAfnp"), doc: D::Topic("readonly"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "return", handler: H::Return, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic("return"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "set", handler: H::Set, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt("+abefhikmnprtuvxlBCEHPTo;"), doc: D::Topic("set"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "shift", handler: H::Shift, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::NoScan, doc: D::Topic("shift"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "shopt", handler: H::Shopt, flags: F::DISABLEABLE, opt_spec: O::Getopt("psuoq"), doc: D::Topic("shopt"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "source", handler: H::Source, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt("p:"), doc: D::Topic("source"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "suspend", handler: H::Suspend, flags: F::DISABLEABLE, opt_spec: O::Getopt("f"), doc: D::Topic("suspend"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "test", handler: H::Test, flags: F::DISABLEABLE.or(F::ARRAYREF), opt_spec: O::Grammar, doc: D::Topic("test"), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "times", handler: H::Times, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt(""), doc: D::Topic("times"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "trap", handler: H::Trap, flags: F::DISABLEABLE.or(F::SPECIAL), opt_spec: O::Getopt("lpP"), doc: D::Topic("trap"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "true", handler: H::True, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::NoScan, doc: D::Topic("true"), introspection: I::Builtin, help_entry: false },
    BuiltinRow { name: "type", handler: H::Type, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("afptP"), doc: D::Topic("type"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "typeset", handler: H::Declare, flags: F::DISABLEABLE.or(F::ASSIGNMENT).or(F::LOCALVAR).or(F::ARRAYREF), opt_spec: O::Getopt("+acfgilnprtuxAFGI"), doc: D::Topic("typeset"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "ulimit", handler: H::Ulimit, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("aSHP;R;T;b;c;d;e;f;i;k;l;m;n;p;q;r;s;t;u;v;w;x;"), doc: D::Topic("ulimit"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "umask", handler: H::Umask, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("Sp"), doc: D::Topic("umask"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "unalias", handler: H::Unalias, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED), opt_spec: O::Getopt("a"), doc: D::Topic("unalias"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "unset", handler: H::Unset, flags: F::DISABLEABLE.or(F::SPECIAL).or(F::ARRAYREF), opt_spec: O::Getopt("fnv"), doc: D::Topic("unset"), introspection: I::Builtin, help_entry: true },
    BuiltinRow { name: "wait", handler: H::Wait, flags: F::DISABLEABLE.or(F::POSIX_EXTENDED).or(F::ARRAYREF), opt_spec: O::Getopt("fnp:"), doc: D::Topic("wait"), introspection: I::Builtin, help_entry: true },
];

/// The table row for `name`, if it is a builtin (binary search —
/// the table is sorted by name).
pub(crate) fn builtin_table_row(name: &str) -> Option<&'static BuiltinRow> {
    BUILTIN_TABLE
        .binary_search_by(|row| row.name.cmp(name))
        .ok()
        .map(|i| &BUILTIN_TABLE[i])
}

/// The member set, derived from the table (const-built so the
/// `&'static [&'static str]` shape of `builtin_names()` is kept).
const fn table_names_array() -> [&'static str; 62] {
    let mut names = [""; 62];
    let mut i = 0;
    while i < BUILTIN_TABLE.len() {
        names[i] = BUILTIN_TABLE[i].name;
        i += 1;
    }
    names
}

/// The 62 builtin names, in table order.
pub(crate) static TABLE_BUILTIN_NAMES: [&str; 62] = table_names_array();

/// The member set, derived from the table.
pub(crate) fn table_builtin_names() -> &'static [&'static str] {
    &TABLE_BUILTIN_NAMES
}

/// Whether `name` is a static builtin — the table half of
/// `command_registry::is_static_introspection_builtin` (the two sets
/// are pinned equal by `table_covers_registry_builtins` below).
pub(crate) fn is_table_builtin_name(name: &str) -> bool {
    builtin_table_row(name).is_some()
}

/// Whether `name` is a POSIX special builtin — the table half of
/// `command_registry::SPECIAL_BUILTINS` membership (pinned equal by
/// `table_special_names_match_special_builtins`).
pub(crate) fn is_table_special_builtin_name(name: &str) -> bool {
    builtin_table_row(name)
        .map(|row| row.flags.contains(BuiltinFlags::SPECIAL))
        .unwrap_or(false)
}

/// The POSIX special builtins, projected from the table
/// (`flags & SPECIAL`), in table order.
pub(crate) fn table_special_builtin_names() -> Vec<&'static str> {
    BUILTIN_TABLE
        .iter()
        .filter(|row| row.flags.contains(BuiltinFlags::SPECIAL))
        .map(|row| row.name)
        .collect()
}

/// The primary-dispatch projection: `(name, slot)` pairs for the
/// builtin rows the primary path claims, sorted by name — the
/// table-derived form of `PRIMARY_DISPATCH` minus its `sleep`
/// fast-path entry.
pub(crate) fn primary_projection() -> Vec<(&'static str, PrimarySlot)> {
    let mut out: Vec<(&'static str, PrimarySlot)> = BUILTIN_TABLE
        .iter()
        .filter_map(|row| row.handler.primary_slot().map(|slot| (row.name, slot)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// The late-dispatch projection: the table-derived form of
/// `LATE_DISPATCH` minus its eight non-builtin entries (`((`,
/// `[[`, `arch`, `basename`, `dirname`, `stty`, `time`, `uname`).
pub(crate) fn late_projection() -> Vec<(&'static str, LateSlot)> {
    let mut out: Vec<(&'static str, LateSlot)> = BUILTIN_TABLE
        .iter()
        .filter_map(|row| row.handler.late_slot().map(|slot| (row.name, slot)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// The `command`-prefix primary projection: the table-derived form
/// of `CMD_PRIMARY_DISPATCH` (all 43 entries are builtin rows).
pub(crate) fn cmd_primary_projection() -> Vec<(&'static str, CmdPrimarySlot)> {
    let mut out: Vec<(&'static str, CmdPrimarySlot)> = BUILTIN_TABLE
        .iter()
        .filter_map(|row| row.handler.cmd_primary_slot().map(|slot| (row.name, slot)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// The `command`-prefix late projection: the table-derived form of
/// `CMD_LATE_DISPATCH` minus its five non-builtin entries (`arch`,
/// `basename`, `dirname`, `stty`, `uname`).
pub(crate) fn cmd_late_projection() -> Vec<(&'static str, CmdLateSlot)> {
    let mut out: Vec<(&'static str, CmdLateSlot)> = BUILTIN_TABLE
        .iter()
        .filter_map(|row| row.handler.cmd_late_slot().map(|slot| (row.name, slot)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

/// The pipeline-stage projection: the table-derived form of
/// `PIPELINE_STAGE` minus its seven tool entries (`cat`, `grep`,
/// `head`, `sed`, `tput`, `tr`, `wc`).
pub(crate) fn pipeline_projection() -> Vec<(&'static str, PipelineSlot)> {
    let mut out: Vec<(&'static str, PipelineSlot)> = BUILTIN_TABLE
        .iter()
        .filter_map(|row| row.handler.pipeline_slot().map(|slot| (row.name, slot)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::super::command_registry;
    use super::*;
    use crate::executor::builtin_names::BUILTIN_NAMES_FOR_TEST;

    #[test]
    fn table_has_62_sorted_unique_rows() {
        assert_eq!(BUILTIN_TABLE.len(), 62);
        let mut names: Vec<&str> = BUILTIN_TABLE.iter().map(|row| row.name).collect();
        let sorted = {
            let mut s = names.clone();
            s.sort();
            s
        };
        assert_eq!(names, sorted, "table rows must be sorted by name");
        names.dedup();
        assert_eq!(names.len(), 62, "table rows must be unique");
    }

    #[test]
    fn table_names_match_builtin_names() {
        assert_eq!(table_builtin_names(), BUILTIN_NAMES_FOR_TEST);
    }

    #[test]
    fn table_special_names_match_special_builtins() {
        let mut expected: Vec<&str> = command_registry::SPECIAL_BUILTINS.to_vec();
        expected.sort();
        let mut got = table_special_builtin_names();
        got.sort();
        assert_eq!(got, expected);
        assert_eq!(got.len(), 16);
    }

    #[test]
    fn table_flags_match_gnu_shell_builtins() {
        // The oracle's generated shell_builtins table
        // (build-gnu/bash-5.3/builtins/builtins.c), masked to the six
        // flag bits this table records. `env` is rubash's own
        // extension and carries only DISABLEABLE.
        let expected: &[(&str, u8)] = &[
            (".", 0x01 | 0x08),
            (":", 0x01 | 0x08),
            ("[", 0x01 | 0x80),
            ("alias", 0x01 | 0x20 | 0x10),
            ("bg", 0x01 | 0x20),
            ("bind", 0x01),
            ("break", 0x01 | 0x08),
            ("builtin", 0x01),
            ("caller", 0x01),
            ("cd", 0x01 | 0x20),
            ("command", 0x01 | 0x20),
            ("compgen", 0x01),
            ("complete", 0x01),
            ("compopt", 0x01),
            ("continue", 0x01 | 0x08),
            ("declare", 0x01 | 0x10 | 0x40 | 0x80),
            ("dirs", 0x01),
            ("disown", 0x01),
            ("echo", 0x01),
            ("enable", 0x01),
            ("env", 0x01),
            ("eval", 0x01 | 0x08),
            ("exec", 0x01 | 0x08),
            ("exit", 0x01 | 0x08),
            ("export", 0x01 | 0x08 | 0x10),
            ("false", 0x01 | 0x20),
            ("fc", 0x01 | 0x20),
            ("fg", 0x01 | 0x20),
            ("getopts", 0x01 | 0x20),
            ("hash", 0x01 | 0x20),
            ("help", 0x01),
            ("history", 0x01),
            ("jobs", 0x01 | 0x20),
            ("kill", 0x01 | 0x20),
            ("let", 0x01 | 0x80),
            ("local", 0x01 | 0x10 | 0x40 | 0x80),
            ("logout", 0x01),
            ("mapfile", 0x01),
            ("popd", 0x01),
            ("printf", 0x01 | 0x80),
            ("pushd", 0x01),
            ("pwd", 0x01 | 0x20),
            ("read", 0x01 | 0x20 | 0x80),
            ("readarray", 0x01),
            ("readonly", 0x01 | 0x08 | 0x10),
            ("return", 0x01 | 0x08),
            ("set", 0x01 | 0x08),
            ("shift", 0x01 | 0x08),
            ("shopt", 0x01),
            ("source", 0x01 | 0x08),
            ("suspend", 0x01),
            ("test", 0x01 | 0x80),
            ("times", 0x01 | 0x08),
            ("trap", 0x01 | 0x08),
            ("true", 0x01 | 0x20),
            ("type", 0x01 | 0x20),
            ("typeset", 0x01 | 0x10 | 0x40 | 0x80),
            ("ulimit", 0x01 | 0x20),
            ("umask", 0x01 | 0x20),
            ("unalias", 0x01 | 0x20),
            ("unset", 0x01 | 0x08 | 0x80),
            ("wait", 0x01 | 0x20 | 0x80),
        ];
        assert_eq!(expected.len(), 62);
        for (name, bits) in expected {
            let row = builtin_table_row(name).unwrap_or_else(|| panic!("no row for {name}"));
            assert_eq!(row.flags.0, *bits, "flags mismatch for {name}");
        }
    }

    #[test]
    fn table_docs_resolve_except_env() {
        for row in BUILTIN_TABLE {
            match row.doc {
                DocId::Topic(name) => {
                    assert_eq!(name, row.name, "doc key must be the row name");
                    assert!(
                        row.doc.short_doc().is_some(),
                        "no help topic for {}",
                        row.name
                    );
                }
                DocId::None => assert_eq!(row.name, "env", "only env has no topic"),
            }
        }
    }

    #[test]
    fn table_introspection_matches_registry_rows() {
        for row in BUILTIN_TABLE {
            let reg = command_registry::registry_row(row.name)
                .unwrap_or_else(|| panic!("no registry row for {}", row.name));
            assert_eq!(
                row.introspection, reg.introspection,
                "introspection mismatch for {}",
                row.name
            );
        }
    }

    #[test]
    fn table_covers_registry_builtins() {
        // The reverse direction of `table_introspection_matches_registry_rows`:
        // every registry row with `Introspection::Builtin` is a table row,
        // so `is_table_builtin_name` is exactly
        // `is_static_introspection_builtin`.
        for row in command_registry::COMMAND_REGISTRY {
            if row.introspection == command_registry::Introspection::Builtin {
                assert!(
                    builtin_table_row(row.name).is_some(),
                    "registry builtin {} has no table row",
                    row.name
                );
            }
        }
    }

    #[test]
    fn table_help_entry_set_is_the_def_truth() {
        // False exactly for the seven rows with no --help entry:
        // the three NoScan builtins whose .def never checks
        // (--help is an operand or ignored), the two Grammar
        // builtins whose own parser takes --help as an operand,
        // and env (coreutils-style parser, --help is a command).
        let false_rows: Vec<&str> = BUILTIN_TABLE
            .iter()
            .filter(|row| !row.help_entry)
            .map(|row| row.name)
            .collect();
        assert_eq!(
            false_rows,
            [":", "[", "echo", "env", "false", "test", "true"]
        );
        // Every Getopt row has the entry (ISHELP in option position).
        for row in BUILTIN_TABLE {
            if matches!(row.opt_spec, OptSpec::Getopt(_)) {
                assert!(
                    row.help_entry,
                    "{} is Getopt but has no help entry",
                    row.name
                );
            }
        }
    }

    #[test]
    fn primary_projection_matches_dispatch_table() {
        let expected: Vec<(&str, PrimarySlot)> = command_registry::PRIMARY_DISPATCH
            .iter()
            .filter(|(name, _)| *name != "sleep")
            .copied()
            .collect();
        assert_eq!(primary_projection(), expected);
    }

    #[test]
    fn late_projection_matches_dispatch_table() {
        let non_builtin = [
            "((", "[[", "arch", "basename", "dirname", "stty", "time", "uname",
        ];
        let expected: Vec<(&str, LateSlot)> = command_registry::LATE_DISPATCH
            .iter()
            .filter(|(name, _)| !non_builtin.contains(name))
            .copied()
            .collect();
        assert_eq!(late_projection(), expected);
    }

    #[test]
    fn cmd_primary_projection_matches_dispatch_table() {
        let expected: Vec<(&str, CmdPrimarySlot)> = command_registry::CMD_PRIMARY_DISPATCH.to_vec();
        assert_eq!(cmd_primary_projection(), expected);
    }

    #[test]
    fn cmd_late_projection_matches_dispatch_table() {
        let non_builtin = ["arch", "basename", "dirname", "stty", "uname"];
        let expected: Vec<(&str, CmdLateSlot)> = command_registry::CMD_LATE_DISPATCH
            .iter()
            .filter(|(name, _)| !non_builtin.contains(name))
            .copied()
            .collect();
        assert_eq!(cmd_late_projection(), expected);
    }

    #[test]
    fn pipeline_projection_matches_dispatch_table() {
        let non_builtin = ["cat", "grep", "head", "sed", "tput", "tr", "wc"];
        let expected: Vec<(&str, PipelineSlot)> = command_registry::PIPELINE_STAGE
            .iter()
            .filter(|(name, _)| !non_builtin.contains(name))
            .copied()
            .collect();
        assert_eq!(pipeline_projection(), expected);
    }

    #[test]
    fn every_handler_is_claimed_on_some_path() {
        for row in BUILTIN_TABLE {
            let claimed = row.handler.primary_slot().is_some()
                || row.handler.late_slot().is_some()
                || row.handler.cmd_primary_slot().is_some()
                || row.handler.cmd_late_slot().is_some()
                || row.handler.pipeline_slot().is_some();
            assert!(claimed, "{} is claimed on no path", row.name);
        }
    }

    #[test]
    fn opt_specs_are_verbatim_def_strings() {
        // Spot pins for the specs that are easy to get wrong: the
        // dynamic ones, the plus-mode ones, and the leading-colon one.
        let spec = |name: &str| builtin_table_row(name).unwrap().opt_spec;
        assert_eq!(spec("set"), OptSpec::Getopt("+abefhikmnprtuvxlBCEHPTo;"));
        assert_eq!(
            spec("ulimit"),
            OptSpec::Getopt("aSHP;R;T;b;c;d;e;f;i;k;l;m;n;p;q;r;s;t;u;v;w;x;")
        );
        assert_eq!(spec("declare"), OptSpec::Getopt("+acfgilnprtuxAFGI"));
        assert_eq!(spec("local"), OptSpec::Getopt("+acfgilnprtuxAFGI"));
        assert_eq!(spec("typeset"), OptSpec::Getopt("+acfgilnprtuxAFGI"));
        assert_eq!(spec("compopt"), OptSpec::Getopt("+o:DEI"));
        assert_eq!(spec("fc"), OptSpec::Getopt(":e:lnrs"));
        assert_eq!(spec("cd"), OptSpec::Getopt("eLP@"));
        assert_eq!(spec("source"), OptSpec::Getopt("p:"));
        assert_eq!(spec("."), OptSpec::Getopt("p:"));
        assert_eq!(spec("export"), OptSpec::Getopt("aAfnp"));
        assert_eq!(spec("kill"), OptSpec::Grammar);
        assert_eq!(spec("test"), OptSpec::Grammar);
        assert_eq!(spec("env"), OptSpec::Grammar);
        assert_eq!(spec(":"), OptSpec::NoScan);
        assert_eq!(spec("echo"), OptSpec::NoScan);
    }
}
