//! The single command-name registry (rubash#2036, slice S1: data only).
//!
//! Every place the engine claims a command name — the four dispatch
//! tables, the hidden fast paths, the whole-command emulation layer,
//! the pipeline stage arms and timed models, the three command-
//! substitution evaluators, the external shortcuts and the Windows
//! display bridge — is recorded here, one row per name, with the field
//! set designed by the #2036 audit (REPORT §6.2): class, per-entry
//! implementations, admission predicate, the two-axis stand-down
//! policy, function-shadow rule, `enable -n` handling, introspection
//! reporting, cfg gating, hash recording, slash forms, and the
//! synchronization reference (the WinuxCmd OPTION table row or GNU
//! manual section the row must stay in sync with).
//!
//! **S1 was data-first and changed no behavior.** S2 lands the
//! first consumers: the introspection faces (the `type`/`command -v`
//! describe path, `compgen -b`, the `enable` lists and validation,
//! the completion builtin/keyword/helptopic candidates, and the
//! fallback describe lists in builtins/type.rs and builtins/command.rs)
//! project their name lists from this table, and the describe path
//! reads the per-row `introspection` projection — including the S2
//! disclosure ruling for claimed names (`Introspection::Emulated`).
//! Dispatch tables still do not read the registry; they project in
//! S3 (the four dispatch tables) and S4 (pipeline and comsub faces).
//! The consistency tests at the bottom of this file pin the table
//! against the live code (the name predicates, the projected lists,
//! and literal scans of the scattered claim sites), so a claim added
//! or dropped anywhere in the engine fails here until the registry
//! is updated — the registry is the ledger, the code is still the
//! truth it describes.
//!
//! Performance contract (audit §5, C1/C2): the table is a `const`
//! slice of plain data — no runtime registration, no locks, no hash
//! maps. When consumers land, a lookup stays a single static table
//! scan at most; the read-only accessor below is not on any hot path
//! in this slice.
//!
//! Stand-down two-axis policy (audit §1.9/§4): `resolvable` records
//! what the face does when the name resolves on PATH — the measured
//! headline of the audit is that almost every face *replaces* the
//! resolved body instead of yielding to it; `unresolvable` records
//! what happens when it does not resolve — the standard predicate
//! (`emulated_external_stands_down`) fails closed to a real spawn and
//! its 127. The policy value `Yield` (real body wins when resolvable)
//! exists in the design space for the S3/S4 adjudication and is used
//! by **no** row today; two cells are non-standard and flagged on
//! their rows: the pipeline sed arm's inverted recheck and the comsub
//! basename arm's unchecked cell (audit B1).

/// Which family claims the name (audit §6.2 `class`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimClass {
    /// A real shell builtin (BUILTIN_NAMES).
    Builtin,
    /// Hidden late builtin: identity/console tools claimed after the
    /// function guard, introspected as external (#154).
    HiddenFastpath,
    /// Fast path in the primary/late dispatch (sleep, dirname,
    /// basename).
    Fastpath,
    /// Whole-command emulation of an external program
    /// (external_file_builtins layer and friends).
    EmulatedExternal,
    /// Claimed only as a pipeline stage / stand-in.
    PipelineOnly,
    /// Claimed only inside command substitution.
    ComsubOnly,
    /// A grammar keyword.
    Keyword,
    /// An external-command shortcut that is not table-driven
    /// (external_inner::handle_external_shortcuts).
    Shortcut,
    /// The Windows display bridge: answers `type`/`command -v`,
    /// never executes.
    DisplayBridge,
}

/// An entry point that can claim a name (audit §6.2 `impl` keys, plus
/// the timed-model and follow-up faces the audit maps at M13/M14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// Simple-command dispatch, primary table.
    SimpPrimary,
    /// Simple-command dispatch, late table.
    SimpLate,
    /// `command`-prefix dispatch, primary table.
    CmdPrimary,
    /// `command`-prefix dispatch, late table.
    CmdLate,
    /// The whole-command emulation layer (external_file_builtins).
    ExternalFile,
    /// Pipeline stage arm (execute_pipeline_stage_inner).
    PipelineStage,
    /// Concurrent-pipeline spawn stand-in (sentinel program name).
    PipelineStandIn,
    /// Timed pipeline producer model ((sleep N; echo x) | ...).
    TimedProducer,
    /// Timed pipeline consumer model (... | read -t).
    TimedConsumer,
    /// Timed follow-up stream classification (timed_followup_keeps_stream).
    TimedFollowup,
    /// Word-level trivial command-substitution evaluator.
    ComsubWord,
    /// Timed command-substitution evaluator.
    ComsubTimed,
    /// Heredoc command-substitution producer.
    ComsubHeredoc,
    /// External command-substitution front face (values evaluator:
    /// stdio special cases, whitelist arms, decline markers).
    ComsubFront,
    /// external_inner::handle_external_shortcuts.
    ExternalShortcut,
    /// Windows display bridge in lookup_paths.
    DisplayBridge,
}

/// One implementation of a name at one entry point: the entry plus a
/// source pointer naming the owning code (file/function level), so a
/// reader can find the implementation from the registry alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImplRef {
    pub entry: Entry,
    pub impl_id: &'static str,
}

impl ImplRef {
    pub(crate) const fn new(entry: Entry, impl_id: &'static str) -> Self {
        Self { entry, impl_id }
    }
}

/// Admission predicate id (audit §6.2 `admission`): which shapes the
/// face agrees to answer before it declines to the next layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    /// No admission restriction of its own (the builtin itself, or an
    /// emulation whose per-shape declines live inside the impl).
    Full,
    /// Never admitted (display bridge rows).
    None,
    /// `simple_path_tool_operands`: any `-x` shaped operand declines.
    SimplePathOperands,
    /// The shared `parse_cat_argv` option face.
    CatArgv,
    /// `apply_simple_sed_args`-answerable shapes only.
    SedSimple,
    /// The two tty shapes (-s / bare non-tty run).
    TtyTwoShape,
    /// Only the -p family (rubash#1636 narrowing).
    MkdirPOnly,
    /// The (TERM, capability) whitelist in tput_emulation.
    TputWhitelist,
    /// A parseable total duration (builtins/sleep.rs can_execute_fast_path).
    SleepDuration,
    /// `parse_head_line_count` line-count shapes only.
    HeadLineCount,
    /// `[-cinvq] [--] PATTERN`, stdin only, no regex metacharacters.
    GrepSimpleFlags,
    /// Exactly one of -c/-l/-w, or no operands.
    WcSingleFlag,
    /// Two pure set operands (or the newline special shape).
    TrTwoSets,
}

/// What a face does when the name RESOLVES on PATH (audit §4: the
/// measured default is replacement, not yielding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolvablePolicy {
    /// The emulation answers, replacing the resolved body.
    Replace,
    /// The real body wins. **No row uses this today** — it is the
    /// policy value S3/S4 may assign per name when they adjudicate
    /// the two-axis table; the consistency tests pin its absence so
    /// the adjudication diffs against an explicit zero.
    Yield,
    /// The pipeline sed arm: answers only when the name is
    /// hash-resolved but the PATH walk misses (an inverted recheck
    /// layered on the fail-closed pre-check).
    InvertedSed,
    /// The spawn stand-in: answers only for a hash-resolved name the
    /// PATH walk cannot see, via a sentinel program name.
    HashStandin,
    /// The face never consults resolution (builtins, keywords,
    /// content shortcuts, the display bridge).
    NotApplicable,
}

/// What a face does when the name does NOT resolve on PATH.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnresolvablePolicy {
    /// Consults `emulated_external_stands_down` and falls through to
    /// a real spawn (which reports 127) — the standard predicate.
    FailClosed,
    /// Never consults the predicate: the face answers even when the
    /// name is unresolvable. Exactly one cell today (comsub basename,
    /// audit B1), flagged on its row.
    Unchecked,
    /// The face never consults resolution.
    NotApplicable,
}

/// The two-axis stand-down policy of a row (audit §6.2 `stand_down`).
/// Per-face divergences from the row value are flagged on the
/// diverging face's ImplRef text until S3/S4 give the field per-entry
/// structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StandDown {
    pub resolvable: ResolvablePolicy,
    pub unresolvable: UnresolvablePolicy,
}

impl StandDown {
    pub(crate) const fn new(
        resolvable: ResolvablePolicy,
        unresolvable: UnresolvablePolicy,
    ) -> Self {
        Self {
            resolvable,
            unresolvable,
        }
    }
}

/// Whether a shell function may shadow the name (audit §6.2
/// `function_shadow`, §6.3 R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FunctionShadow {
    /// Functions win over this face (the GNU shape for non-special
    /// builtins and for every external face).
    Allow,
    /// POSIX special builtins block function shadowing in POSIX mode
    /// (is_posix_special_builtin, M7).
    PosixSpecialBlock,
    /// Keywords cannot be function names at all.
    NotApplicable,
}

/// How `enable -n` interacts with the name (audit §6.2 `enable_n`,
/// §6.3 R4: the checks are hand-written per dispatch arm today; the
/// per-arm equivalence pins are S3 work).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnableN {
    /// A true builtin: `enable -n` disables it and dispatch honors
    /// the disabled set.
    Respected,
    /// Not a builtin: `enable` refuses the name outright.
    NotApplicable,
}

/// What introspection reports for the name (audit §6.2
/// `introspection`, §1.8): `type`, `command -v`/`-V`, `compgen -b`,
/// `enable -a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Introspection {
    /// Reported as a shell builtin.
    Builtin,
    /// Disclosed as an in-process emulation of the resolved PATH body
    /// (#2036 S2 ruling, reversing the S1-era zero-disclosure
    /// contract): while the row's simple-command claim is live — the
    /// name resolves and the face's gates are on — `type` prints
    /// "X is a shell emulation of <path>", `type -t` prints
    /// `emulation`, `command -V` prints the same sentence and
    /// `command -v` the bare name. When the claim stands down, the
    /// plain file answer (or not-found) returns, exactly as dispatch
    /// behaves. Whether an emulated execution also writes a hash-table
    /// resolution record is the separate `hash_record` field.
    Emulated,
    /// Reported as the PATH file (or not found): the row's claims are
    /// shape-scoped (pipeline stage / command substitution only) or
    /// display-only, so the simple-command question honestly gets the
    /// file answer.
    External,
    /// Reported as a shell keyword.
    Keyword,
}

/// Platform / host gating of the row's claims (audit §6.2 `cfg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CfgGate {
    /// Claimed on every platform.
    AllPlatforms,
    /// `#[cfg(windows)]` faces only.
    WindowsOnly,
    /// The simple-command face is gated by the host flag
    /// `external_file_builtins_enabled` (Executor::set_external_file_builtins_enabled).
    HostFlagExternalFileBuiltins,
}

/// Whether an emulated execution records a hash-table resolution
/// (audit §6.2 `hash_record`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HashRecord {
    /// Recorded via the layer's hash set (external_file_builtins).
    Yes,
    /// Not recorded.
    No,
    /// Recorded through a name-specific path outside the layer's set
    /// (tput's record_tput_resolution — the two lists in the layer
    /// have already diverged on exactly this, audit §1.4).
    Separate,
}

/// What a slash-prefixed form of the name does (audit §6.3 R5: the
/// slash dimension must be registered per form, never assumed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashKind {
    /// The slash form is emulated like the bare name.
    Emulated,
    /// The slash form is renamed and redirected to the true builtin
    /// (GNU would execve the real body — an escape-contract wart the
    /// audit flags at §2.1, adjudicated in S3/S4).
    RedirectToBuiltin,
}

/// One slash-prefixed form of a name and its semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlashForm {
    pub form: &'static str,
    pub kind: SlashKind,
}

impl SlashForm {
    pub(crate) const fn new(form: &'static str, kind: SlashKind) -> Self {
        Self { form, kind }
    }
}

/// One registry row: everything the engine claims about one name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandRow {
    pub name: &'static str,
    pub class: ClaimClass,
    pub impls: &'static [ImplRef],
    pub admission: Admission,
    pub stand_down: StandDown,
    pub function_shadow: FunctionShadow,
    pub enable_n: EnableN,
    pub introspection: Introspection,
    pub cfg: CfgGate,
    pub hash_record: HashRecord,
    pub slash_forms: &'static [SlashForm],
    /// The reference surface this row must stay in sync with: the
    /// WinuxCmd OPTION table row or the GNU manual/coreutils section
    /// (audit §6.2 `sync_ref`; sync itself is S5's corpus).
    pub sync_ref: &'static str,
    /// The docs/builtins.md machine block or section that documents
    /// this row, or the gap where it should (audit §6.2 `doc_id`).
    pub doc_id: &'static str,
}

/// Constructor for the uniform true-builtin row: only the name, the
/// dispatch/comsub impl claims and the function-shadow rule vary.
const fn builtin(
    name: &'static str,
    impls: &'static [ImplRef],
    function_shadow: FunctionShadow,
) -> CommandRow {
    CommandRow {
        name,
        class: ClaimClass::Builtin,
        impls,
        admission: Admission::Full,
        stand_down: StandDown::new(
            ResolvablePolicy::NotApplicable,
            UnresolvablePolicy::NotApplicable,
        ),
        function_shadow,
        enable_n: EnableN::Respected,
        introspection: Introspection::Builtin,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU bash 5.3 builtins (builtins/*.def)",
        doc_id: "builtins.md#builtins-list",
    }
}

/// A true builtin whose slash forms redirect back to the builtin
/// (echo/printf/pwd, audit §2.1): same row as `builtin`, plus the
/// registered slash forms.
const fn builtin_redirect(
    name: &'static str,
    impls: &'static [ImplRef],
    function_shadow: FunctionShadow,
    slash_forms: &'static [SlashForm],
) -> CommandRow {
    CommandRow {
        slash_forms,
        ..builtin(name, impls, function_shadow)
    }
}

/// Constructor for the uniform keyword row.
const fn keyword(name: &'static str, impls: &'static [ImplRef]) -> CommandRow {
    CommandRow {
        name,
        class: ClaimClass::Keyword,
        impls,
        admission: Admission::None,
        stand_down: StandDown::new(
            ResolvablePolicy::NotApplicable,
            UnresolvablePolicy::NotApplicable,
        ),
        function_shadow: FunctionShadow::NotApplicable,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Keyword,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU bash 5.3 grammar reserved words (parse.y)",
        doc_id: "n/a",
    }
}

/// The registry. Sorted within each class group; the class groups
/// follow the audit's §1/§2 presentation order. Data provenance: the
/// #2036 audit ledger (measured on master 5ec322cb) re-extracted and
/// adjudicated against master 1a6f9c9a sources by the consistency
/// tests below — where the audit's counts and the code disagreed
/// (keyword count, the second reserved-word table), the code won and
/// the difference is recorded in the S1 report.
pub(crate) const COMMAND_REGISTRY: &[CommandRow] = &[
    // -- true builtins (BUILTIN_NAMES, 62) --
    builtin(".", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage restricted-shell refusal guard")], FunctionShadow::PosixSpecialBlock),
    builtin(":", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm"), ImplRef::new(Entry::TimedProducer, "pipeline_exec::timed_pipeline_step"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm")], FunctionShadow::PosixSpecialBlock),
    builtin("[", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("alias", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("bg", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("bind", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("break", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("builtin", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("caller", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("cd", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::ComsubWord, "command_substitution cd ...; pwd idiom arm (pair claim with pwd)"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage restricted-shell refusal guard")], FunctionShadow::Allow),
    builtin("command", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage restricted-shell refusal guard (operand slash scan, #804)")], FunctionShadow::Allow),
    builtin("compgen", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("complete", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("compopt", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("continue", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("declare", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("dirs", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("disown", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin_redirect("echo", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm"), ImplRef::new(Entry::TimedProducer, "pipeline_exec::timed_pipeline_step"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm")], FunctionShadow::Allow, &[SlashForm::new("/bin/echo", SlashKind::RedirectToBuiltin), SlashForm::new("/usr/bin/echo", SlashKind::RedirectToBuiltin)]),
    builtin("enable", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("env", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary -> external_inner::execute_env_command")], FunctionShadow::Allow),
    builtin("eval", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("exec", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage restricted-shell refusal guard")], FunctionShadow::PosixSpecialBlock),
    builtin("exit", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("export", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("false", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm")], FunctionShadow::Allow),
    builtin("fc", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("fg", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("getopts", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("hash", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("help", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("history", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("jobs", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("kill", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::ComsubWord, "command_substitution kill -l arm")], FunctionShadow::Allow),
    builtin("let", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("local", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("logout", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("mapfile", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("popd", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin_redirect("printf", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm")], FunctionShadow::Allow, &[SlashForm::new("/bin/printf", SlashKind::RedirectToBuiltin), SlashForm::new("/usr/bin/printf", SlashKind::RedirectToBuiltin)]),
    builtin("pushd", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin_redirect("pwd", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm (+ cd ...; pwd idiom)")], FunctionShadow::Allow, &[SlashForm::new("/bin/pwd", SlashKind::RedirectToBuiltin), SlashForm::new("/usr/bin/pwd", SlashKind::RedirectToBuiltin)]),
    builtin("read", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::TimedConsumer, "pipeline_exec::timed_read_consumer_output (the modeled command)")], FunctionShadow::Allow),
    builtin("readarray", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("readonly", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("return", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("set", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("shift", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("shopt", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("source", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage restricted-shell refusal guard")], FunctionShadow::PosixSpecialBlock),
    builtin("suspend", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("test", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    builtin("times", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::PosixSpecialBlock),
    builtin("trap", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (EXIT-install shapes decline to a subshell stage)")], FunctionShadow::PosixSpecialBlock),
    builtin("true", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm"), ImplRef::new(Entry::TimedProducer, "pipeline_exec::timed_pipeline_step"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm")], FunctionShadow::Allow),
    builtin("type", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late"), ImplRef::new(Entry::ComsubWord, "command_substitution type -t arm")], FunctionShadow::Allow),
    builtin("typeset", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("ulimit", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm (real builtin impl)")], FunctionShadow::Allow),
    builtin("umask", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm (real builtin impl)")], FunctionShadow::Allow),
    builtin("unalias", &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::Allow),
    builtin("unset", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdPrimary, "command_no_alias")], FunctionShadow::PosixSpecialBlock),
    builtin("wait", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late"), ImplRef::new(Entry::CmdLate, "command_no_alias_late")], FunctionShadow::Allow),
    // -- grammar keywords (is_shell_keyword, 23) --
    keyword("if", &[]),
    keyword("then", &[]),
    keyword("else", &[]),
    keyword("elif", &[]),
    keyword("fi", &[]),
    keyword("case", &[]),
    keyword("esac", &[]),
    keyword("for", &[]),
    keyword("select", &[]),
    keyword("while", &[]),
    keyword("until", &[]),
    keyword("do", &[]),
    keyword("done", &[]),
    keyword("in", &[]),
    keyword("function", &[]),
    keyword("time", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late (time-prefix pipeline arm)")]),
    keyword("{", &[]),
    keyword("}", &[]),
    keyword("!", &[]),
    keyword("[[", &[ImplRef::new(Entry::SimpLate, "command_dispatch_late (conditional command)")]),
    keyword("]]", &[]),
    keyword("coproc", &[]),
    // -- hidden fast-path: identity / console tools (#154) --
    CommandRow {
        name: "uname",
        class: ClaimClass::HiddenFastpath,
        impls: &[ImplRef::new(Entry::SimpLate, "command_dispatch_late -> printf_path_builtins::execute_identity_tool (builtins/uname.rs)"), ImplRef::new(Entry::CmdLate, "command_no_alias_late -> printf_path_builtins::execute_identity_tool")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd uname OPTION table; GNU coreutils 9.4 uname",
        doc_id: "builtins.md#fastpath-list",
    },
    CommandRow {
        name: "arch",
        class: ClaimClass::HiddenFastpath,
        impls: &[ImplRef::new(Entry::SimpLate, "command_dispatch_late -> printf_path_builtins::execute_identity_tool (builtins/uname.rs)"), ImplRef::new(Entry::CmdLate, "command_no_alias_late -> printf_path_builtins::execute_identity_tool")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd arch OPTION table; GNU coreutils 9.4 arch",
        doc_id: "builtins.md#fastpath-list",
    },
    CommandRow {
        name: "stty",
        class: ClaimClass::HiddenFastpath,
        impls: &[ImplRef::new(Entry::SimpLate, "command_dispatch_late -> printf_path_builtins::execute_console_mode_tool (builtins/stty.rs)"), ImplRef::new(Entry::CmdLate, "command_no_alias_late -> printf_path_builtins::execute_console_mode_tool")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd stty OPTION table; GNU coreutils 9.4 stty",
        doc_id: "builtins.md#fastpath-list",
    },
    // -- fast paths: sleep / dirname / basename --
    CommandRow {
        name: "sleep",
        class: ClaimClass::Fastpath,
        impls: &[ImplRef::new(Entry::SimpPrimary, "command_dispatch_primary -> builtins/sleep.rs (interruptible virtual-child wait)"), ImplRef::new(Entry::TimedProducer, "pipeline_exec::timed_pipeline_step (stand-down checked)"), ImplRef::new(Entry::TimedConsumer, "pipeline_exec::timed_read_consumer_output (stand-down NOT checked)"), ImplRef::new(Entry::TimedFollowup, "pipeline_exec::timed_followup_keeps_stream")],
        admission: Admission::SleepDuration,
        // Row policy = the simple-command face; the timed-consumer model
        // diverges (it never consults the predicate, audit §1.5) and is
        // flagged on its ImplRef above until S4 adjudicates it.
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd sleep NUMBER[SUFFIX]...; GNU coreutils 9.4 sleep",
        doc_id: "builtins.md#fastpath-list",
    },
    CommandRow {
        name: "dirname",
        class: ClaimClass::Fastpath,
        impls: &[ImplRef::new(Entry::SimpLate, "command_dispatch_late -> printf_path_builtins (simple_path_tool_operands)"), ImplRef::new(Entry::CmdLate, "command_no_alias_late -> printf_path_builtins (simple_path_tool_operands)")],
        admission: Admission::SimplePathOperands,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd dirname OPTION table; GNU coreutils 9.4 dirname",
        doc_id: "builtins.md#fastpath-list",
    },
    CommandRow {
        name: "basename",
        class: ClaimClass::Fastpath,
        impls: &[ImplRef::new(Entry::SimpLate, "command_dispatch_late -> printf_path_builtins (simple_path_tool_operands, <=2 operands)"), ImplRef::new(Entry::CmdLate, "command_no_alias_late -> printf_path_builtins (simple_path_tool_operands)"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm (independent impl; no operand cap; stand-down NOT checked)")],
        admission: Admission::SimplePathOperands,
        // Row policy = the simple/command faces. The comsub word arm is
        // Replace/Unchecked (audit B1) — the one unchecked cell in the
        // table; S4 must adjudicate it explicitly, not normalize it away.
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd basename OPTION table; GNU coreutils 9.4 basename",
        doc_id: "builtins.md#fastpath-list",
    },
    // -- whole-command emulation: external_file_builtins layer --
    CommandRow {
        name: "cat",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins (parse_cat_argv, shared with the pipeline arm)"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (shared parse_cat_argv)"), ImplRef::new(Entry::ComsubWord, "command_substitution word arm (file operands only, stand-down checked)"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm (stand-down checked)"), ImplRef::new(Entry::ComsubHeredoc, "command_substitution_pipelines heredoc producer (stand-down checked)"), ImplRef::new(Entry::ExternalShortcut, "external_inner::handle_external_shortcuts (hash/checkhash special case)"), ImplRef::new(Entry::DisplayBridge, "lookup_paths display bridge (windows; type/command -v only, never executes)")],
        admission: Admission::CatArgv,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        // The host flag gates the simple-command face; the pipeline and
        // comsub faces are not flag-gated (audit §1.4/§1.5/§1.6).
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[SlashForm::new("/bin/cat", SlashKind::Emulated), SlashForm::new("/usr/bin/cat", SlashKind::Emulated)],
        sync_ref: "WinuxCmd cat getopt table; GNU coreutils 9.4 cat",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "chmod",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_chmod")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd chmod OPTION table; GNU coreutils 9.4 chmod",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "cp",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_cp")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd cp OPTION table; GNU coreutils 9.4 cp",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "mkdir",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_mkdir")],
        admission: Admission::MkdirPOnly,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd mkdir OPTION table; GNU coreutils 9.4 mkdir",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "mkfifo",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_mkfifo")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd mkfifo OPTION table; GNU coreutils 9.4 mkfifo",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "rm",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_rm")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd rm OPTION table; GNU coreutils 9.4 rm",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "rmdir",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_rmdir")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd rmdir OPTION table; GNU coreutils 9.4 rmdir",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "sed",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_sed (stdin shapes only)"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (inverted recheck: yields when find_user_command hits)")],
        admission: Admission::SedSimple,
        // Two-layer policy (audit §1.5): the stage pre-check is the
        // ordinary fail-closed gate; the arm then INVERTS — it answers
        // only when the name is hash-resolved but the PATH walk misses.
        // S4 must adjudicate this cell explicitly.
        stand_down: StandDown::new(ResolvablePolicy::InvertedSed, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd sed [OPTION]...; GNU sed (coreutils 9.4 differential set)",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "touch",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_touch (gnulib getopt emulation)")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[],
        sync_ref: "WinuxCmd touch OPTION table; GNU coreutils 9.4 touch",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "tty",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_tty"), ImplRef::new(Entry::ComsubTimed, "command_substitution_pipelines timed arm (stand-down checked)"), ImplRef::new(Entry::ComsubFront, "command_substitution_values stdio special case (records no hash entry)")],
        admission: Admission::TtyTwoShape,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::HostFlagExternalFileBuiltins,
        hash_record: HashRecord::Yes,
        slash_forms: &[SlashForm::new("/bin/tty", SlashKind::Emulated), SlashForm::new("/usr/bin/tty", SlashKind::Emulated)],
        sync_ref: "WinuxCmd tty [OPTION]...; GNU coreutils 9.4 tty",
        doc_id: "builtins.md §4.1",
    },
    CommandRow {
        name: "tput",
        class: ClaimClass::EmulatedExternal,
        impls: &[ImplRef::new(Entry::ExternalFile, "external_file_builtins::external_tput (windows)"), ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm + tput_inline_stage routing (windows)"), ImplRef::new(Entry::ComsubFront, "command_substitution_values whitelist arm (windows; stand-down NOT checked, admission is the whitelist)")],
        admission: Admission::TputWhitelist,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::Emulated,
        cfg: CfgGate::WindowsOnly,
        // tput records through its own record_tput_resolution, not the
        // layer's hash set (audit §1.4 — the two lists in the layer have
        // already diverged on exactly this name).
        hash_record: HashRecord::Separate,
        slash_forms: &[SlashForm::new("/bin/tput", SlashKind::Emulated), SlashForm::new("/usr/bin/tput", SlashKind::Emulated)],
        sync_ref: "WinuxCmd tput OPTION table (registration synopsis underreports: -T exists); ncurses tput",
        doc_id: "builtins.md §4.1",
    },
    // -- pipeline-only arms --
    CommandRow {
        name: "grep",
        class: ClaimClass::PipelineOnly,
        impls: &[ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm ([-cinvq] [--] PATTERN, stdin only, no regex metachars)")],
        admission: Admission::GrepSimpleFlags,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 grep (pipeline arm; the WinuxCmd applet is escape-face only)",
        doc_id: "builtins.md#pipeline-fastpath-list",
    },
    CommandRow {
        name: "head",
        class: ClaimClass::PipelineOnly,
        impls: &[ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (parse_head_line_count shapes)"), ImplRef::new(Entry::PipelineStandIn, "pipeline_exec spawn stand-in (sentinel program name)")],
        admission: Admission::HeadLineCount,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 head (pipeline faces; the WinuxCmd applet is escape-face only)",
        doc_id: "builtins.md#pipeline-fastpath-list",
    },
    CommandRow {
        name: "tr",
        class: ClaimClass::PipelineOnly,
        impls: &[ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (two pure set operands; carrier-byte face — registration only, impl stays put per audit R3)")],
        admission: Admission::TrTwoSets,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 tr (pipeline arm; the WinuxCmd applet is escape-face only)",
        doc_id: "builtins.md#pipeline-fastpath-list",
    },
    CommandRow {
        name: "wc",
        class: ClaimClass::PipelineOnly,
        impls: &[ImplRef::new(Entry::PipelineStage, "pipeline_exec stage arm (single -c/-l/-w flag or bare; carrier-byte face — registration only per audit R3)"), ImplRef::new(Entry::PipelineStandIn, "pipeline_exec spawn stand-in (sentinel program name)")],
        admission: Admission::WcSingleFlag,
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 wc (pipeline faces; the WinuxCmd applet is escape-face only)",
        doc_id: "builtins.md#pipeline-fastpath-list",
    },
    CommandRow {
        name: "yes",
        class: ClaimClass::PipelineOnly,
        impls: &[ImplRef::new(Entry::PipelineStandIn, "pipeline_exec spawn stand-in (sentinel program name; the only face that claims yes)")],
        admission: Admission::Full,
        // Hash-standin policy (audit §1.5): the stand-in answers only
        // for a name find_user_command's PATH walk cannot see but the
        // hash table resolves — stand_down must be false via the hash
        // leg, never via the walk.
        stand_down: StandDown::new(ResolvablePolicy::HashStandin, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 yes (stand-in face only)",
        doc_id: "builtins.md#pipeline-fastpath-list (stand-in set undocumented — audit gap)",
    },
    // -- comsub-only claims --
    CommandRow {
        name: "mktemp",
        class: ClaimClass::ComsubOnly,
        impls: &[ImplRef::new(Entry::ComsubWord, "command_substitution word arm (in-process file creation)")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::NotApplicable, UnresolvablePolicy::NotApplicable),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 mktemp (comsub face only; simple commands spawn the real binary)",
        doc_id: "n/a",
    },
    CommandRow {
        name: "sort",
        class: ClaimClass::ComsubOnly,
        impls: &[ImplRef::new(Entry::ComsubHeredoc, "command_substitution_pipelines heredoc producer [sort, -u] arm (rides the cat gate)")],
        admission: Admission::Full,
        // The arm sits behind the heredoc producer's cat stand-down
        // check, so its effective policy is the cat row's.
        stand_down: StandDown::new(ResolvablePolicy::Replace, UnresolvablePolicy::FailClosed),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "GNU coreutils 9.4 sort (-u heredoc face only)",
        doc_id: "n/a",
    },
    // -- external shortcut (not table-driven; registered so the set is enumerable) --
    CommandRow {
        name: "diff",
        class: ClaimClass::Shortcut,
        impls: &[ImplRef::new(Entry::ExternalShortcut, "external_inner::handle_external_shortcuts (two-file, shebang-only difference shortcut)")],
        admission: Admission::Full,
        stand_down: StandDown::new(ResolvablePolicy::NotApplicable, UnresolvablePolicy::NotApplicable),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::AllPlatforms,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "WinuxCmd diff; GNU coreutils 9.4 diff (shortcut face; audit A4)",
        doc_id: "n/a (test-suite compat shortcut)",
    },
    // -- Windows display bridge (never executes; registered for enumerability) --
    CommandRow {
        name: "ls",
        class: ClaimClass::DisplayBridge,
        impls: &[ImplRef::new(Entry::DisplayBridge, "lookup_paths display bridge (windows; type/command -v answer /bin/ls, never executes)")],
        admission: Admission::None,
        stand_down: StandDown::new(ResolvablePolicy::NotApplicable, UnresolvablePolicy::NotApplicable),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::WindowsOnly,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "display bridge only — no execution claim",
        doc_id: "n/a (source-only, lookup_paths.rs)",
    },
    CommandRow {
        name: "mv",
        class: ClaimClass::DisplayBridge,
        impls: &[ImplRef::new(Entry::DisplayBridge, "lookup_paths display bridge (windows; type/command -v answer /usr/bin/mv, never executes)")],
        admission: Admission::None,
        stand_down: StandDown::new(ResolvablePolicy::NotApplicable, UnresolvablePolicy::NotApplicable),
        function_shadow: FunctionShadow::Allow,
        enable_n: EnableN::NotApplicable,
        introspection: Introspection::External,
        cfg: CfgGate::WindowsOnly,
        hash_record: HashRecord::No,
        slash_forms: &[],
        sync_ref: "display bridge only — no execution claim",
        doc_id: "n/a (source-only, lookup_paths.rs)",
    },
];

/// Read-only lookup by name (linear scan over const data; not used
/// on any dispatch path in this slice — S2+ consumers call it from
/// introspection-speed contexts only, per audit §5 C1/C2).
pub(crate) fn registry_row(name: &str) -> Option<&'static CommandRow> {
    COMMAND_REGISTRY.iter().find(|row| row.name == name)
}

/// The POSIX special-builtin set, owned by the registry since S2.
///
/// GNU's table is name-based, not a flag on the `.` row: the names in
/// `special_builtins[]` (bash 5.3 `builtins/mkbuiltins.c:148-154`)
/// get the SPECIAL flag wherever they appear (`mkbuiltins.c:819-820`),
/// so `source` — the synonym spelling — is special exactly like `.`.
/// Measured on GNU bash 5.3.20: `enable -s` lists these 16 names, and
/// in POSIX mode `type` reports each as "a special shell builtin"
/// (`builtins/type.def:262-270` looks the class up in the builtin
/// table before functions).
///
/// The executor's execution-side predicate
/// (`support_names::is_posix_special_builtin`) names the same 16 —
/// KD-2 closed on the execution side in S3 (the predicate gained
/// `source`; the registry row's function-shadow rule and the
/// consistency pins below moved with it).
pub(crate) const SPECIAL_BUILTINS: &[&str] = &[
    ".", ":", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return", "set",
    "shift", "source", "times", "trap", "unset",
];

/// Whether `name` is a POSIX special builtin (registry projection;
/// the introspection faces read this set, the execution side reads
/// `support_names::is_posix_special_builtin` — the same 16 names
/// since S3, pinned equal by the consistency tests).
pub(crate) fn is_special_builtin(name: &str) -> bool {
    SPECIAL_BUILTINS.contains(&name)
}

/// The builtin name list every introspection face projects (registry
/// rows whose introspection answer is Builtin), in registry order —
/// the order BUILTIN_NAMES, the old complete.rs SHELL_BUILTINS and
/// the old enable.rs ALL_BUILTINS all agreed on (KD-1: the copies
/// lacked `env`; the projection includes it).
pub(crate) fn introspection_builtin_names() -> Vec<&'static str> {
    COMMAND_REGISTRY
        .iter()
        .filter(|row| row.introspection == Introspection::Builtin)
        .map(|row| row.name)
        .collect()
}

/// Whether `name` belongs to the introspection builtin set.
pub(crate) fn is_introspection_builtin(name: &str) -> bool {
    registry_row(name).is_some_and(|row| row.introspection == Introspection::Builtin)
}

/// The shell keyword list the completion faces project (registry
/// rows of class Keyword), in registry order.
pub(crate) fn introspection_keyword_names() -> Vec<&'static str> {
    COMMAND_REGISTRY
        .iter()
        .filter(|row| row.class == ClaimClass::Keyword)
        .map(|row| row.name)
        .collect()
}

#[cfg(test)]
mod registry_consistency {
    //! The registry is a ledger of claims the code makes; these tests
    //! re-derive the claim sets from the code itself (callable
    //! predicates, data tables, and literal scans of the claim sites)
    //! and diff them against the registry. The only tolerated
    //! differences are the explicitly listed known drifts, each with
    //! its adjudication slice. Anything else failing here means a
    //! claim was added, dropped, or moved without updating the
    //! registry — update the registry in the same change.

    use super::*;
    use std::collections::BTreeSet;

    const SRC_COMPLETE: &str = include_str!("../builtins/complete.rs");
    const SRC_ENABLE: &str = include_str!("../builtins/enable.rs");
    const SRC_DISPATCH_PRIMARY: &str = include_str!("command_dispatch_primary.rs");
    const SRC_DISPATCH_LATE: &str = include_str!("command_dispatch_late.rs");
    const SRC_NO_ALIAS: &str = include_str!("command_no_alias.rs");
    const SRC_NO_ALIAS_LATE: &str = include_str!("command_no_alias_late.rs");
    const SRC_EXTERNAL_FILE: &str = include_str!("external_file_builtins.rs");
    const SRC_EXTERNAL_INNER: &str = include_str!("external_inner.rs");
    const SRC_PIPELINE: &str = include_str!("pipeline_exec.rs");
    const SRC_COMSUB: &str = include_str!("command_substitution.rs");
    const SRC_COMSUB_PIPELINES: &str = include_str!("command_substitution_pipelines.rs");
    const SRC_COMSUB_VALUES: &str = include_str!("command_substitution_values.rs");
    const SRC_LOOKUP_PATHS: &str = include_str!("lookup_paths.rs");

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn registry_names() -> BTreeSet<String> {
        COMMAND_REGISTRY
            .iter()
            .map(|r| r.name.to_string())
            .collect()
    }

    fn class_rows(class: ClaimClass) -> BTreeSet<String> {
        COMMAND_REGISTRY
            .iter()
            .filter(|r| r.class == class)
            .map(|r| r.name.to_string())
            .collect()
    }

    fn claims(entry: Entry) -> BTreeSet<String> {
        COMMAND_REGISTRY
            .iter()
            .filter(|r| r.impls.iter().any(|i| i.entry == entry))
            .map(|r| r.name.to_string())
            .collect()
    }

    /// Name-shaped string literals in a text fragment: lowercase
    /// command-name characters only (this filters out diagnostics,
    /// format strings with spaces, and option flags).
    fn literals(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'"' {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] != b'"' {
                    if bytes[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let lit = &text[start..j.min(bytes.len())];
                if !lit.is_empty()
                    && lit.len() <= 24
                    && lit.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || "./:_![]{}-".contains(c)
                    })
                    && !lit.contains("..")
                    && !lit.starts_with('-')
                    && (!lit.contains('{') || lit == "{")
                    && (!lit.contains('}') || lit == "}")
                {
                    out.insert(lit.to_string());
                }
                i = j;
            }
            i += 1;
        }
        out
    }

    /// The parenthesized block of the `matches!(...)` that contains
    /// `needle`, as literal set.
    fn matches_block_containing(text: &str, needle: &str) -> BTreeSet<String> {
        let at = text.find(needle).expect("needle present");
        let start = text[..at]
            .rfind("matches!(")
            .expect("matches! before needle");
        let open = start + "matches!".len();
        let bytes = text.as_bytes();
        let mut depth = 0i32;
        let mut i = open;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return literals(&text[open..=i]);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        panic!("unbalanced matches! block");
    }

    /// Claim literals of a whole file: match-arm pattern segments
    /// (text before the first `=>` on a line), `matches!(...)` blocks,
    /// `Some("...")` comparisons, and `==`/`!=` string comparisons —
    /// the four shapes the scattered claim sites use.
    fn claim_literals(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in text.lines() {
            if let Some(arrow) = line.find("=>") {
                out.extend(literals(&line[..arrow]));
            }
        }
        let mut rest = text;
        while let Some(at) = rest.find("matches!(") {
            let open = at + "matches!".len();
            let bytes = rest.as_bytes();
            let mut depth = 0i32;
            let mut i = open;
            let mut closed = false;
            while i < bytes.len() {
                match bytes[i] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            out.extend(literals(&rest[open..=i]));
                            rest = &rest[i + 1..];
                            closed = true;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if !closed {
                break;
            }
        }
        for (pat, _) in text.match_indices("Some(\"") {
            let start = pat + "Some(\"".len();
            if let Some(end) = text[start..].find('"') {
                let lit = &text[start..start + end];
                out.extend(literals(&format!("\"{lit}\"")));
            }
        }
        for op in ["== \"", "!= \""] {
            for (pat, _) in text.match_indices(op) {
                let start = pat + op.len();
                if let Some(end) = text[start..].find('"') {
                    let lit = &text[start..start + end];
                    out.extend(literals(&format!("\"{lit}\"")));
                }
            }
        }
        out
    }

    /// Match-arm pattern literals only: the quoted segments before
    /// the first `=>` on each line. Used where a window also contains
    /// arm bodies with their own string literals (renames, messages).
    fn arm_literals(text: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for line in text.lines() {
            if let Some(arrow) = line.find("=>") {
                out.extend(literals(&line[..arrow]));
            }
        }
        out
    }

    /// The source window from `sig` up to (not including) the next
    /// item boundary `next`.
    fn window<'a>(text: &'a str, sig: &str, next: &str) -> &'a str {
        let start = text.find(sig).unwrap_or_else(|| panic!("sig {sig}"));
        let rest = &text[start + sig.len()..];
        let end = rest.find(next).unwrap_or(rest.len());
        &rest[..end]
    }

    /// Literals of a `const NAME: &[&str] = &[...];` block.
    fn const_block(text: &str, marker: &str) -> BTreeSet<String> {
        let start = text.find(marker).expect("const marker");
        let rest = &text[start..];
        let end = rest.find("];").expect("const terminator");
        literals(&rest[..end])
    }

    /// Map slash forms (`/bin/cat`) to their row name (`cat`) when the
    /// base is a registered name.
    fn normalized(claims_in: BTreeSet<String>) -> BTreeSet<String> {
        let names = registry_names();
        claims_in
            .into_iter()
            .map(|lit| {
                for prefix in ["/bin/", "/usr/bin/"] {
                    if let Some(base) = lit.strip_prefix(prefix) {
                        if names.contains(base) {
                            return base.to_string();
                        }
                    }
                }
                lit
            })
            .collect()
    }

    fn only_registered(claims_in: BTreeSet<String>) -> BTreeSet<String> {
        let names = registry_names();
        claims_in
            .into_iter()
            .filter(|c| names.contains(c))
            .collect()
    }

    // ------------------------------------------------------------ T0

    #[test]
    fn registry_well_formed() {
        let names = registry_names();
        assert_eq!(names.len(), COMMAND_REGISTRY.len(), "duplicate rows");
        for row in COMMAND_REGISTRY {
            assert!(
                !row.impls.is_empty() || row.class == ClaimClass::Keyword,
                "{}: non-keyword row without impls",
                row.name
            );
        }
        // The audit §2.2 conflict universe (34 names claimed by some
        // face AND shipped by WinuxCmd) must be fully registered.
        let conflict34 = set(&[
            "[", "echo", "env", "false", "kill", "printf", "pwd", "test", "true", "uname", "arch",
            "stty", "sleep", "dirname", "basename", "cat", "chmod", "cp", "mkdir", "mkfifo", "rm",
            "rmdir", "sed", "touch", "tty", "tput", "grep", "head", "tr", "wc", "yes", "sort",
            "diff", "time",
        ]);
        assert_eq!(conflict34.len(), 34);
        for name in &conflict34 {
            assert!(names.contains(name), "{name} missing from registry");
        }
        // The audit's headline in executable form: no face yields to a
        // resolved real body today. S3/S4 adjudication changes rows
        // one by one and updates this pin deliberately.
        assert!(
            COMMAND_REGISTRY
                .iter()
                .all(|r| r.stand_down.resolvable != ResolvablePolicy::Yield),
            "a row started yielding — update this pin with the adjudication reference"
        );
        // The one unchecked cell: comsub basename (audit B1) is a
        // per-face divergence recorded on the face, not the row; the
        // row-level unchecked set must stay empty until S4 rules.
        assert!(
            COMMAND_REGISTRY
                .iter()
                .all(|r| r.stand_down.unresolvable != UnresolvablePolicy::Unchecked),
            "row-level Unchecked is reserved; per-face cells ride their ImplRef text in S1"
        );
    }

    // ------------------------------------------------- T1/T2: builtins

    #[test]
    fn builtin_rows_match_builtin_names() {
        let table: BTreeSet<String> = super::super::builtin_names::builtin_names()
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(table.len(), 62);
        assert_eq!(class_rows(ClaimClass::Builtin), table);
    }

    #[test]
    fn shell_builtins_drift_is_exactly_env() {
        // KD-1 / KD-2, closed on the report side by S2 (this test was
        // the drift pin in S1; it now pins the unified projection).
        //
        // KD-1: complete.rs SHELL_BUILTINS (compgen -b) and enable.rs
        // ALL_BUILTINS (enable -a + validation) were hand copies of
        // the GNU-61 list lacking `env`. Both consts are gone; the
        // faces project from the registry. The source scans below keep
        // a hand copy from silently returning, and the runtime checks
        // pin the completion face's accessors to the registry order.
        for (src, marker) in [
            (SRC_COMPLETE, "const SHELL_BUILTINS"),
            (SRC_COMPLETE, "const SHELL_KEYWORDS"),
            (SRC_COMPLETE, "const HELP_TOPIC_COMPLETIONS"),
            (SRC_ENABLE, "const ALL_BUILTINS"),
            (SRC_ENABLE, "const SPECIAL_BUILTINS"),
        ] {
            assert!(!src.contains(marker), "hand list returned: {marker}");
        }
        assert_eq!(
            crate::builtins::complete::shell_builtin_names(),
            introspection_builtin_names()
        );
        assert_eq!(
            crate::builtins::complete::shell_keyword_names(),
            introspection_keyword_names()
        );
        // `env` is in the projection (the KD-1 fix): 62 names.
        let projection = introspection_builtin_names();
        assert_eq!(projection.len(), 62);
        assert!(projection.contains(&"env"));

        // KD-2: the registry's special-builtin set is GNU's 16-name
        // table (mkbuiltins.c:148-154; `enable -s` measurement on GNU
        // bash 5.3.20), including `source`. S1–S2 pinned the
        // execution-side difference at exactly {"source"}; S3 closed
        // it (the predicate gained `source` and this row set moved
        // with it), so the symmetric difference is now empty and the
        // two sets must stay equal.
        assert_eq!(SPECIAL_BUILTINS.len(), 16);
        let special = set(SPECIAL_BUILTINS);
        let builtins = class_rows(ClaimClass::Builtin);
        assert!(special.is_subset(&builtins), "special set must be builtins");
        let predicate_rows: BTreeSet<String> = COMMAND_REGISTRY
            .iter()
            .filter(|r| r.function_shadow == FunctionShadow::PosixSpecialBlock)
            .map(|r| r.name.to_string())
            .collect();
        let diff: BTreeSet<String> = special
            .symmetric_difference(&predicate_rows)
            .cloned()
            .collect();
        assert_eq!(diff, BTreeSet::new());
        assert_eq!(predicate_rows, special);
    }

    #[test]
    fn claim_classes_project_introspection_disclosure() {
        // S2 disclosure ruling: the simple-command claim classes
        // (HiddenFastpath / Fastpath / EmulatedExternal) project
        // Introspection::Emulated — the describe faces disclose the
        // in-process claim while it is live. The shape-scoped classes
        // (pipeline-only, comsub-only, the diff shortcut) and the
        // display bridge keep Introspection::External: `type` answers
        // the simple-command question, and those rows make no
        // simple-command claim to disclose.
        let emulated: BTreeSet<String> = COMMAND_REGISTRY
            .iter()
            .filter(|r| r.introspection == Introspection::Emulated)
            .map(|r| r.name.to_string())
            .collect();
        assert_eq!(
            emulated,
            set(&[
                "uname", "arch", "stty", "sleep", "dirname", "basename", "cat", "chmod", "cp",
                "mkdir", "mkfifo", "rm", "rmdir", "sed", "touch", "tty", "tput",
            ])
        );
        for row in COMMAND_REGISTRY {
            match row.class {
                ClaimClass::HiddenFastpath
                | ClaimClass::Fastpath
                | ClaimClass::EmulatedExternal => {
                    assert_eq!(row.introspection, Introspection::Emulated, "{}", row.name)
                }
                ClaimClass::PipelineOnly
                | ClaimClass::ComsubOnly
                | ClaimClass::Shortcut
                | ClaimClass::DisplayBridge => {
                    assert_eq!(row.introspection, Introspection::External, "{}", row.name)
                }
                ClaimClass::Builtin => {
                    assert_eq!(row.introspection, Introspection::Builtin, "{}", row.name)
                }
                ClaimClass::Keyword => {
                    assert_eq!(row.introspection, Introspection::Keyword, "{}", row.name)
                }
            }
        }
    }

    #[test]
    fn help_topics_match_registry_projection() {
        // The help face's topic set is prose data (help_texts.rs) and
        // cannot be generated from the registry, so it is pinned
        // against the projection instead: every builtin row is a
        // topic (S2 added the missing `env` topic — env was a builtin
        // with no help face), the keyword topics are the head words
        // GNU has topics for, and the only non-registry topics are
        // the four measured GNU extras. GNU bash 5.3.20 measurements:
        // `compgen -A helptopic` lists exactly this equation's
        // GNU part (77 names), and `help then|fi|esac|do|in|]]|}`
        // each fail with "no help topics match" — the closing words
        // of compound commands are not topics in GNU either.
        let topics = crate::builtins::help::help_topic_names();
        let topic_set: BTreeSet<String> = topics.iter().map(|s| s.to_string()).collect();
        assert_eq!(topics.len(), topic_set.len(), "duplicate help topics");
        let mut expected = class_rows(ClaimClass::Builtin);
        for head in [
            "!",
            "[[ ... ]]",
            "case",
            "coproc",
            "for",
            "function",
            "if",
            "select",
            "time",
            "until",
            "while",
            "{ ... }",
        ] {
            expected.insert(head.to_string());
        }
        for extra in ["%", "(( ... ))", "for ((", "variables"] {
            expected.insert(extra.to_string());
        }
        assert_eq!(topic_set, expected);
        // Every keyword row is either a topic (literally or in its
        // bracket display form) or a measured GNU non-topic.
        let non_topics = set(&[
            "then", "else", "elif", "fi", "esac", "do", "done", "in", "]]", "}",
        ]);
        for name in class_rows(ClaimClass::Keyword) {
            let display = match name.as_str() {
                "[[" => "[[ ... ]]".to_string(),
                "{" => "{ ... }".to_string(),
                other => other.to_string(),
            };
            assert!(
                topic_set.contains(&display) || non_topics.contains(&name),
                "keyword {name} has no help-topic accounting"
            );
        }
    }

    // ------------------------------------------- T3/T4/T5: predicates

    #[test]
    fn hidden_late_rows_match_predicate() {
        let names = registry_names();
        let positives: BTreeSet<String> = names
            .iter()
            .filter(|n| super::super::command_dispatch_late::is_hidden_late_builtin(n))
            .cloned()
            .collect();
        assert_eq!(positives, set(&["uname", "arch", "stty"]));
        assert_eq!(positives, class_rows(ClaimClass::HiddenFastpath));
        assert_eq!(
            class_rows(ClaimClass::Fastpath),
            set(&["sleep", "dirname", "basename"])
        );
    }

    #[test]
    fn posix_special_rows_match_predicate() {
        let names = registry_names();
        let positives: BTreeSet<String> = names
            .iter()
            .filter(|n| super::super::support_names::is_posix_special_builtin(n))
            .cloned()
            .collect();
        let rows: BTreeSet<String> = COMMAND_REGISTRY
            .iter()
            .filter(|r| r.function_shadow == FunctionShadow::PosixSpecialBlock)
            .map(|r| r.name.to_string())
            .collect();
        assert_eq!(positives, rows);
        // GNU's 16-name table (mkbuiltins.c:148-154); `source` joined
        // in S3 (KD-2) — before that this pin read 15 with the
        // difference carried explicitly.
        assert_eq!(positives.len(), 16);
        assert_eq!(positives, set(SPECIAL_BUILTINS));
    }

    #[test]
    fn keyword_rows_match_predicates() {
        let names = registry_names();
        let positives: BTreeSet<String> = names
            .iter()
            .filter(|n| super::super::builtin_names::is_shell_keyword(n))
            .cloned()
            .collect();
        // The audit's M8 count said 23; the predicate names 22. The
        // predicate is ground truth (recorded in the S1 report).
        assert_eq!(positives.len(), 22);
        assert_eq!(positives, class_rows(ClaimClass::Keyword));
        // Second reserved-word table (support_names::is_reserved_word)
        // — a scatter point the audit map did not itemize (M24
        // candidate): a 17-word parser-side subset of the keywords.
        let reserved: BTreeSet<String> = names
            .iter()
            .filter(|n| super::super::support_names::is_reserved_word(n))
            .cloned()
            .collect();
        assert_eq!(
            reserved,
            set(&[
                "if", "then", "else", "elif", "fi", "while", "do", "done", "until", "for", "case",
                "esac", "in", "function", "select", "time", "coproc",
            ])
        );
        assert!(reserved.is_subset(&positives));
    }

    // ------------------------------------- T6: the emulation layer

    #[test]
    fn external_layer_hash_set_matches_registry() {
        let hash_block = matches_block_containing(SRC_EXTERNAL_FILE, "\"/bin/pwd\"");
        let mut expected = BTreeSet::new();
        for row in COMMAND_REGISTRY
            .iter()
            .filter(|r| r.hash_record == HashRecord::Yes)
        {
            expected.insert(row.name.to_string());
            for slash in row.slash_forms {
                expected.insert(slash.form.to_string());
            }
        }
        // The layer's hash set also records the slash-redirect pair
        // (pwd/printf forms pass through this function on their way
        // to the builtins, and the resolution is recorded the same
        // way — the set in the code is the authority here).
        for form in ["/bin/pwd", "/usr/bin/pwd", "/bin/printf", "/usr/bin/printf"] {
            expected.insert(form.to_string());
        }
        assert_eq!(hash_block, expected);
    }

    #[test]
    fn external_layer_dispatch_matches_registry() {
        let start = SRC_EXTERNAL_FILE
            .find("match cmd.words[0].as_str()")
            .expect("dispatch match");
        let rest = &SRC_EXTERNAL_FILE[start..];
        let end = rest.find("_ => Ok(false)").expect("dispatch tail");
        let dispatch = arm_literals(&rest[..end]);
        let mut expected: BTreeSet<String> = claims(Entry::ExternalFile);
        for row in COMMAND_REGISTRY
            .iter()
            .filter(|r| r.impls.iter().any(|i| i.entry == Entry::ExternalFile))
        {
            for slash in row.slash_forms {
                expected.insert(slash.form.to_string());
            }
        }
        // The layer's slash-redirect pair (pwd/printf are builtins,
        // not ExternalFile rows, but their forms dispatch here).
        for form in ["/bin/pwd", "/usr/bin/pwd", "/bin/printf", "/usr/bin/printf"] {
            expected.insert(form.to_string());
        }
        assert_eq!(dispatch, expected);
    }

    // ------------------------------------- T7: the dispatch tables

    #[test]
    fn dispatch_tables_match_registry() {
        for (src, entry) in [
            (SRC_DISPATCH_PRIMARY, Entry::SimpPrimary),
            (SRC_DISPATCH_LATE, Entry::SimpLate),
            (SRC_NO_ALIAS, Entry::CmdPrimary),
            (SRC_NO_ALIAS_LATE, Entry::CmdLate),
        ] {
            let scanned = only_registered(claim_literals(src));
            assert_eq!(scanned, claims(entry), "dispatch drift at {entry:?}");
        }
    }

    // ------------------------------------- T8: pipeline faces

    #[test]
    fn pipeline_stage_arms_match_registry() {
        let body = window(SRC_PIPELINE, "fn execute_pipeline_stage_inner", "\n    fn ");
        let scanned = only_registered(claim_literals(body));
        assert_eq!(scanned, claims(Entry::PipelineStage));
        // The stage pre-check stand-down list (the seven emulated
        // names) equals the non-builtin PipelineStage rows.
        let precheck = matches_block_containing(SRC_PIPELINE, "\"head\" | \"cat\" | \"sed\"");
        let expected: BTreeSet<String> = COMMAND_REGISTRY
            .iter()
            .filter(|r| {
                r.class != ClaimClass::Builtin
                    && r.impls.iter().any(|i| i.entry == Entry::PipelineStage)
            })
            .map(|r| r.name.to_string())
            .collect();
        assert_eq!(precheck, expected);
        // The spawn stand-in set.
        let standin = matches_block_containing(SRC_PIPELINE, "\"yes\" | \"head\" | \"wc\"");
        assert_eq!(standin, claims(Entry::PipelineStandIn));
    }

    #[test]
    fn timed_models_match_registry() {
        let producer = window(
            SRC_PIPELINE,
            "fn timed_pipeline_step",
            "\n    fn timed_read_consumer_output",
        );
        assert_eq!(
            only_registered(claim_literals(producer)),
            claims(Entry::TimedProducer)
        );
        let consumer = window(
            SRC_PIPELINE,
            "fn timed_read_consumer_output",
            "\n    fn timed_read_options",
        );
        assert_eq!(
            only_registered(claim_literals(consumer)),
            claims(Entry::TimedConsumer)
        );
        let followup = window(SRC_PIPELINE, "fn timed_followup_keeps_stream", "\nfn ");
        assert_eq!(
            matches_block_containing(followup, "\"echo\" | \"printf\""),
            claims(Entry::TimedFollowup)
        );
    }

    // ------------------------------------- T9–T11: comsub faces

    #[test]
    fn comsub_word_arms_match_registry() {
        let scanned = only_registered(claim_literals(SRC_COMSUB));
        assert_eq!(scanned, claims(Entry::ComsubWord));
    }

    #[test]
    fn comsub_timed_and_heredoc_match_registry() {
        let scanned = only_registered(normalized(claim_literals(SRC_COMSUB_PIPELINES)));
        let mut expected = claims(Entry::ComsubTimed);
        expected.extend(claims(Entry::ComsubHeredoc));
        // Known non-claims in this file, pinned both directions so a
        // real claim cannot hide behind them: "mktemp" is a negative
        // guard in the heredoc producer; "time" is the leading
        // time-prefix skip in the timed word scan.
        let diff: BTreeSet<String> = scanned.difference(&expected).cloned().collect();
        assert_eq!(diff, set(&["mktemp", "time"]));
        assert!(
            expected.is_subset(&scanned),
            "missing: {:?}",
            expected.difference(&scanned).collect::<Vec<_>>()
        );
    }

    #[test]
    fn comsub_front_face_matches_registry() {
        let scanned = only_registered(normalized(claim_literals(SRC_COMSUB_VALUES)));
        let expected = claims(Entry::ComsubFront);
        // Known non-claims, pinned both directions: "command" is a
        // negative guard on the front face's first word; "mktemp"
        // appears only as decline markers (the front face refuses it
        // so the word-level arm can claim it).
        let diff: BTreeSet<String> = scanned.difference(&expected).cloned().collect();
        assert_eq!(diff, set(&["command", "mktemp"]));
        assert!(expected.is_subset(&scanned));
    }

    // ------------------------------------- T12/T13: bridge/shortcuts

    #[test]
    fn display_bridge_matches_registry() {
        let scanned = only_registered(claim_literals(SRC_LOOKUP_PATHS));
        assert_eq!(scanned, claims(Entry::DisplayBridge));
    }

    #[test]
    fn external_shortcuts_face_is_registered() {
        // handle_external_shortcuts + execute_env_command's home: the
        // file's registered-name literals are exactly these four —
        // echo (via its RedirectToBuiltin slash forms), cat (the
        // hash/checkhash special case, a registered ExternalShortcut
        // face), diff (the shebang shortcut row), env (the impl home
        // of the env builtin, registered on env's row).
        let scanned = only_registered(normalized(claim_literals(SRC_EXTERNAL_INNER)));
        assert_eq!(scanned, set(&["cat", "diff", "echo", "env"]));
        assert_eq!(claims(Entry::ExternalShortcut), set(&["cat", "diff"]));
    }
}
