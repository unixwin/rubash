//! Carrier/sentinel registry — governance 3.1 single declaration point.
//!
//! Every in-band marker byte/codepoint that travels inside transport text
//! (token values, AST words, expansion intermediates, stored values) is
//! declared here exactly once: meaning, producer, decode boundary, and
//! whether user-reachable input can collide with it.
//!
//! GNU analog: bash threads `CTLESC` (0x01) / `CTLNUL` (0x7f) through word
//! text (parse.y:5694-5706 `got_escaped_character`, subst.c:4692
//! `dequote_escapes`, subst.c:4807 `dequote_string`). Rubash cannot reuse
//! 0x01/0x7f — both are user-reachable bytes — so it uses a C0/PUA carrier
//! protocol instead. This file is the registry for that protocol.
//!
//! Rules (governance 3.1):
//! - Business code must `use crate::executor::markers::*` (or name the
//!   const) — never write a raw `'\x1f'` / `'\u{E302}'` literal.
//! - A new marker = a new const + encode/decode pair + a boundary entry in
//!   `MARKERS`, plus a golden assertion that the sentinel never appears in
//!   stdout / `declare -p` / xtrace output.
//! - `user_reachable == true` markers can equal a byte the user can type or
//!   pipe in; the entry point producing transport text MUST re-encode that
//!   byte (raw-byte marker pair), or the marker silently corrupts data.
//!
//! ## Decode boundary entry points (governance 3.2)
//!
//! | Boundary | Entry point | Site |
//! |---|---|---|
//! | Output | `crate::locale::decode_to_visible_text` — the one sanctioned transport→visible decoder (echo paths additionally run `substitution_metadata::decode_raw_byte_markers` because echo input is already expansion-decoded text that can still carry raw-byte pairs) | `builtins/echo.rs`, `declare` printing, xtrace, `bad_substitution_display`, host `dump-strings` |
//! | Storage | values reaching `env_vars`/array stores are already data bytes — the encoder side is `substitution_metadata` (`bytes_to_assignment_shell_text`, `push_escaped_text_with_carriers`) which pair-encodes every `user_reachable` byte at entry so storage never confuses a user byte with a carrier | variable/array assignment paths |
//! | Reparse | marker bytes are the lexer's own protocol: text re-fed through `eval`, alias bodies and command-substitution bodies re-lex with markers intact; user bytes inside reparsed text were pair-encoded at the original entry so they decode as data, never as carriers | `eval`, alias expansion, comsub reparse |
//!
//! Golden rule: no `MARKERS` codepoint may appear in stdout, `declare -p`,
//! or xtrace output — asserted by the leak tests in `locale.rs`/`markers.rs`.

/// Decode boundary at which a carrier is restored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Boundary {
    /// Restored before text reaches stdout/stderr/`declare -p`/xtrace.
    Output,
    /// Restored before a value lands in `env_vars` / array storage.
    Storage,
    /// Restored before text is re-parsed (eval/alias/comsub bodies).
    Reparse,
}

/// One registered marker.
pub(crate) struct MarkerInfo {
    /// Registry constant name.
    pub name: &'static str,
    /// First code point of the marker.
    pub code: u32,
    /// Boundaries that must decode it.
    pub boundaries: &'static [Boundary],
    /// Whether a user-supplied byte can equal the marker verbatim. If true,
    /// the transport producer must re-encode that byte on entry
    /// (substitution_metadata::push_escaped_text_with_carriers /
    /// lexer::ansi::is_assignment_carrier_byte coverage).
    pub user_reachable: bool,
}

// ==========================================================================
// C0 family — the CTLESC/CTLNUL port region. These ride inside word text
// between lexer, parser, and executor.
// ==========================================================================

/// CTLESC port: precedes a quoted glob metacharacter (`*?[@+!`) so glob and
/// quote machinery treat it as data (glob.rs `dequote_pathname`). Decode:
/// drop the marker, keep the following char.
pub(crate) const CTLESC: char = '\u{11}';

/// Marks a name boundary where quote removal deleted a quote char that
/// terminated an unbraced `$name` (lexer/quotes.rs). Decode: drop.
pub(crate) const PARAM_NAME_END_MARKER: char = '\u{13}';

/// Data backslash produced by `\\` quote removal.
pub(crate) const DATA_BACKSLASH: char = '\u{14}';

/// Protected backslash inside single-quoted/alias text (alias_helpers.rs;
/// echo's `$'...'` fast path at builtins/echo.rs:122 decodes it).
pub(crate) const PROTECTED_BACKSLASH: char = '\u{15}';

/// Protected `\'` (escaped single quote) — embedded_parameters.rs.
pub(crate) const PROTECTED_ESCAPED_SQUOTE: char = '\u{16}';

/// Data single quote — `'`-quoted content carrier.
pub(crate) const DATA_SQUOTE: char = '\u{17}';

/// Data double quote — `\"`/quoted `"` carrier.
pub(crate) const DATA_DQUOTE: char = '\u{18}';

/// Protected literal `\` inside `${}` bodies (embedded_parameters.rs:597).
pub(crate) const PROTECTED_LITERAL_BACKSLASH: char = '\u{19}';

/// Data backtick.
pub(crate) const DATA_BACKTICK: char = '\u{1a}';

/// Word-prefix marker: the word had a quoted tilde / needs glob suppression
/// (lexer/word.rs). Decode: drop the leading marker.
pub(crate) const QUOTED_WORD_PREFIX: char = '\u{1b}';

/// IFS-protection glue: precedes a whitespace char that must survive field
/// splitting (embedded_mutations.rs, command_prepare.rs).
pub(crate) const IFS_GLUE: char = '\u{1c}';

/// Word-prefix marker: array-storage / unquoted-comsub word
/// (declare storage paths strip it before use).
pub(crate) const STORAGE_WORD_PREFIX: char = '\u{1d}';

/// Subscript carrier: hex-encoded subscript keys / hash-env field
/// separator / arithmetic subscript transport.
pub(crate) const SUBSCRIPT_CARRIER: char = '\u{1e}';

/// Pattern-domain role of \x18: inside case/parameter pattern text the
/// same codepoint is a protected literal backslash
/// (conditional/pattern.rs case_pattern_atom_matches). Word transport
/// reads it as DATA_DQUOTE — the two channels never mix, but the alias
/// keeps call sites honest about which protocol they mean.
pub(crate) const PATTERN_LITERAL_BACKSLASH: char = '\u{18}';
/// `&str` companion of PATTERN_LITERAL_BACKSLASH.
pub(crate) const PATTERN_LITERAL_BACKSLASH_STR: &str = "\u{18}";

/// Field separator inside `__RUBASH_PARSE_ERROR_*` assignment payloads
/// (parser/parse_loop.rs producer, command_execute.rs consumer). Shares
/// the SUBSCRIPT_CARRIER codepoint — disjoint channel (env-string
/// payloads, never word text).
pub(crate) const PARSE_ERROR_FIELD_SEP: char = '\u{1e}';

/// Prefix on a HereDocBody token whose delimiter was found but emitted a
/// warning (lexer/mod.rs producer; execution_misc.rs and
/// parser/parse_loop.rs consumers). Shares the SUBSCRIPT_CARRIER
/// codepoint — disjoint channel (token-body prefix position).
pub(crate) const HEREDOC_WARNED_BODY_PREFIX: char = '\u{1e}';

/// Data `$` — dollar that must not re-trigger expansion.
pub(crate) const DATA_DOLLAR: char = '\u{1f}';

/// Protected literal `$` inside `${}` bodies (embedded_parameters.rs:598).
pub(crate) const PROTECTED_LITERAL_DOLLAR: char = '\u{12}';

/// Prompt `\[`-start marker (GNU RL_PROMPT_START_IGNORE port,
/// prompt_expansion.rs).
pub(crate) const PROMPT_IGNORE_START: char = '\u{01}';

/// Prompt `\]`-end marker (GNU RL_PROMPT_END_IGNORE port).
pub(crate) const PROMPT_IGNORE_END: char = '\u{02}';

/// Lead-in inside `( ... )` compound-assignment bodies marking a
/// whole-single-quoted declare operand whose expansion GNU defers
/// (types.rs; arrayfunc.c:557 expand_compound_array_assignment).
pub(crate) const DEFERRED_COMPOUND_BODY: char = '\u{3}';

/// Marks a here-doc/stdin body that was already expanded once
/// (execution_misc.rs:286). A user 0x05 byte at body start collides —
/// mitigated by encoding user 0x05 as a raw-byte marker pair at the
/// parser boundary (parser/redirections.rs:635).
pub(crate) const PREEXPANDED_STDIN_BODY: char = '\u{5}';

/// Tags `${d[@]}` elements produced by field splitting so array storage
/// keeps them as separate elements (declare/storage/array.rs).
pub(crate) const ARRAY_FIELD_SPLIT_MARKER: char = '\u{10}';

/// Marks an `a[@]`/`a[*]` reference token for the arrayref builtin
/// (in-band W_ARRAYREF port, command_prepare.rs). Moved from \x02: that
/// byte collided with PROMPT_IGNORE_END and is user-reachable.
pub(crate) const ARRAYREF_FLAG: char = '\u{E318}';

// ==========================================================================
// PATSUB family — `${var/pat/rep}` replacement quoted-region markers.
// Moved out of the C0 range (were \x0b/\x0c/\x0e/\x0f): those bytes are
// user-reachable (\v, \f, SO, SI can arrive via $'\v', printf, pipes), so
// the markers now live in the registry PUA block. Internal to
// expand_braced_replacement.rs: produced by the marking passes, consumed by
// finish_patsub_replacement before text leaves the substitution.
// ==========================================================================

/// Start of a quoted replacement region.
pub(crate) const PATSUB_QUOTED_VALUE_START: char = '\u{E310}';
/// End of a quoted replacement region.
pub(crate) const PATSUB_QUOTED_VALUE_END: char = '\u{E311}';
/// Quoted `&` inside a replacement (patsub_replacement expands it).
pub(crate) const PATSUB_QUOTED_AMP: char = '\u{E312}';
/// Quoted `\` inside a replacement.
pub(crate) const PATSUB_QUOTED_BACKSLASH: char = '\u{E313}';

// --- Function-local guards (PUA block E314-E317) ---------------------------
// Protect/restore pairs that live inside a single function. They were C0
// bytes (\x0e/\x15/\x1e) that collide with user-reachable data and with the
// cross-module carriers; moving them to the PUA block removes the collision
// for free since the producer and consumer never leave the function.

/// Guards `\` in `${var+word}`-family alternate processing
/// (parameter_words.rs) — was \x0e, which collided with PATSUB_QUOTED_AMP
/// before that family moved to PUA.
pub(crate) const PARAM_WORD_BACKSLASH_GUARD: char = '\u{E314}';

/// Guards a `\`-escaped IFS char through field splitting
/// (command_prepare.rs field_split_escaped_ifs) — was \x1e, colliding with
/// SUBSCRIPT_CARRIER keys flowing through the same text.
pub(crate) const ESCAPED_IFS_GUARD: char = '\u{E315}';

/// Guards `\!`/`\#` prompt escapes through assignment expansion
/// (assignment_expansion.rs preserve_prompt_escapes) — was \x15.
pub(crate) const PROMPT_ESCAPE_GUARD: char = '\u{E316}';

/// Guards `\` and the \x18 data-dquote carrier through case-pattern
/// expansion (compound_exec.rs expand_case_pattern) — was \x15.
pub(crate) const CASE_PATTERN_BACKSLASH_GUARD: char = '\u{E317}';

// ==========================================================================
// PUA family — multi-byte-safe sentinels for data that C0 cannot carry.
// ==========================================================================

/// Introducer of a raw-byte marker pair: E000 + payload char
/// (E001..=E100 = bytes 0x00..=0xFF). Doubled E000 = literal E000 char.
/// Owner: substitution_metadata.rs.
pub(crate) const RAW_BYTE_MARKER_ESCAPE: u32 = 0xE000;
pub(crate) const RAW_BYTE_MARKER_FIRST: u32 = 0xE001;
pub(crate) const RAW_BYTE_MARKER_LAST: u32 = 0xE100;

/// GNU CTLNUL port: an empty quoted field ('', "$e", "$(:)" with no
/// output) in an alternate expansion so the field splitter keeps an empty
/// argv entry (subst.c:11940-11944, list_string:3181-3186).
pub(crate) const QUOTED_NULL_MARKER: char = '\u{E002}';

/// ANSI-C `$'...'` decoded `'` data carrier (lexer/quotes.rs).
pub(crate) const ANSI_C_QUOTE_MARKER: char = '\u{E010}';
/// ANSI-C `$'...'` decoded `"` data carrier.
pub(crate) const ANSI_C_DQUOTE_MARKER: char = '\u{E011}';

/// Base of conditional-pattern byte-chars: U+E100+byte
/// (conditional/pattern.rs). The WHOLE E100..=E1FF range is generated
/// dynamically — no named marker may live in it (the E10A double-occupancy
//  and the E101-E109 DATA_* collision were this bug class).
pub(crate) const BYTE_CHAR_BASE: u32 = 0xE100;

/// declare-family operand marker: the subscript evaluation already failed
/// and printed its diagnostic; bind the variable with attributes without
/// re-evaluating (declare.def:988-1011). Outside the byte-char range.
pub(crate) const FAILED_SUBSCRIPT_SENTINEL: &str = "\u{E200}";

// --- Assignment DATA_* sentinels (registry block E301-E30A) -------------
// These carried data quotes/backslashes through assignment expansion.
// They previously lived at E101-E10A — inside the BYTE_CHAR_BASE range,
// colliding with pattern byte-chars 0x01-0x0A (same bug class as E10A's
// FAILED_SUBSCRIPT_SENTINEL split, commit 6e29c031). Renumbered into the
// dedicated registry block.

/// Data `'` produced inside assignment contexts.
pub(crate) const ASSIGN_DATA_SQUOTE: char = '\u{E301}';
/// Data `"` produced inside assignment contexts.
pub(crate) const ASSIGN_DATA_DQUOTE: char = '\u{E302}';
/// Data backtick inside assignment contexts.
pub(crate) const ASSIGN_DATA_BACKTICK: char = '\u{E303}';
/// Data `"` that was `\"`-escaped in source.
pub(crate) const ASSIGN_ESCAPED_DQUOTE: char = '\u{E304}';
/// Data `'` that was `\'`-escaped in source.
pub(crate) const ASSIGN_ESCAPED_SQUOTE: char = '\u{E305}';
/// Data `\` that was `\\`-escaped in source.
pub(crate) const ASSIGN_ESCAPED_BACKSLASH: char = '\u{E306}';
/// Hoisted single quote (atomic lexer path wraps `$'...'` quote content).
pub(crate) const ASSIGN_HOISTED_SQUOTE: char = '\u{E307}';
/// Hoisted backslash.
pub(crate) const ASSIGN_HOISTED_BACKSLASH: char = '\u{E308}';
/// Glue tag marking expansion-produced whitespace inside compound
/// assignments so it survives element splitting (embedded_mutations.rs).
pub(crate) const COMPOUND_EXPANSION_WS_TAG: char = '\u{E309}';
/// Data `$` hoisted out of `$'...'` content during assignment storage
/// restore (assignment_expansion.rs).
pub(crate) const ASSIGN_SQ_DOLLAR: char = '\u{E30A}';
/// Data backtick hoisted out of `$'...'` content (was SQ_BACKTICK_DATA E10B).
pub(crate) const ASSIGN_SQ_BACKTICK: char = '\u{E30B}';
/// Data `\` hoisted out of `$'...'` content (was SQ_BACKSLASH_DATA E10C).
pub(crate) const ASSIGN_SQ_BACKSLASH: char = '\u{E30C}';

// `str::replace` takes the marker as a Pattern (char works) but the
// replacement argument must be `&str`; provide &str spellings for every
// char sentinel. A registry test asserts each pair agrees.
pub(crate) const ASSIGN_DATA_SQUOTE_STR: &str = "\u{E301}";
pub(crate) const ASSIGN_DATA_DQUOTE_STR: &str = "\u{E302}";
pub(crate) const ASSIGN_DATA_BACKTICK_STR: &str = "\u{E303}";
pub(crate) const ASSIGN_ESCAPED_DQUOTE_STR: &str = "\u{E304}";
pub(crate) const ASSIGN_ESCAPED_SQUOTE_STR: &str = "\u{E305}";
pub(crate) const ASSIGN_ESCAPED_BACKSLASH_STR: &str = "\u{E306}";
pub(crate) const ASSIGN_HOISTED_SQUOTE_STR: &str = "\u{E307}";
pub(crate) const ASSIGN_HOISTED_BACKSLASH_STR: &str = "\u{E308}";
pub(crate) const COMPOUND_EXPANSION_WS_TAG_STR: &str = "\u{E309}";
pub(crate) const ASSIGN_SQ_DOLLAR_STR: &str = "\u{E30A}";
pub(crate) const ASSIGN_SQ_BACKTICK_STR: &str = "\u{E30B}";
pub(crate) const ASSIGN_SQ_BACKSLASH_STR: &str = "\u{E30C}";
pub(crate) const ANSI_C_QUOTE_MARKER_STR: &str = "\u{E010}";
pub(crate) const ANSI_C_DQUOTE_MARKER_STR: &str = "\u{E011}";
pub(crate) const QUOTED_NULL_MARKER_STR: &str = "\u{E002}";

// C0 carrier &str spellings for `str::replace` targets / format strings.
pub(crate) const CTLESC_STR: &str = "\u{11}";
pub(crate) const PARAM_NAME_END_MARKER_STR: &str = "\u{13}";
pub(crate) const DATA_BACKSLASH_STR: &str = "\u{14}";
pub(crate) const PROTECTED_BACKSLASH_STR: &str = "\u{15}";
pub(crate) const PROTECTED_ESCAPED_SQUOTE_STR: &str = "\u{16}";
pub(crate) const DATA_SQUOTE_STR: &str = "\u{17}";
pub(crate) const DATA_DQUOTE_STR: &str = "\u{18}";
pub(crate) const PROTECTED_LITERAL_BACKSLASH_STR: &str = "\u{19}";
pub(crate) const DATA_BACKTICK_STR: &str = "\u{1a}";
pub(crate) const QUOTED_WORD_PREFIX_STR: &str = "\u{1b}";
pub(crate) const IFS_GLUE_STR: &str = "\u{1c}";
pub(crate) const STORAGE_WORD_PREFIX_STR: &str = "\u{1d}";
pub(crate) const SUBSCRIPT_CARRIER_STR: &str = "\u{1e}";
pub(crate) const DATA_DOLLAR_STR: &str = "\u{1f}";
pub(crate) const PROTECTED_LITERAL_DOLLAR_STR: &str = "\u{12}";
pub(crate) const PROMPT_IGNORE_START_STR: &str = "\u{01}";
pub(crate) const PROMPT_IGNORE_END_STR: &str = "\u{02}";
pub(crate) const DEFERRED_COMPOUND_BODY_STR: &str = "\u{3}";
pub(crate) const PREEXPANDED_STDIN_BODY_STR: &str = "\u{5}";
pub(crate) const ARRAY_FIELD_SPLIT_MARKER_STR: &str = "\u{10}";
pub(crate) const ARRAYREF_FLAG_STR: &str = "\u{E318}";

// ==========================================================================
// Named string markers — multi-char protocol prefixes (not byte carriers).
// ==========================================================================

/// Prefix marking a quoted here-doc body token value (lexer/mod.rs).
pub(crate) const QUOTED_HEREDOC_MARKER: &str = "__RUBASH_HD1__";

/// Command-substitution payload prefix inside transport text
/// (execution_misc.rs) — `__RUBASH_CSB1_<hex>;` encodes bytes that cannot
/// ride as chars.
pub(crate) const COMSUB_PAYLOAD_PREFIX: &str = "__RUBASH_CSB1_";

/// `( ... )` compound-assignment operand marker (types.rs).
pub(crate) const COMPOUND_ASSIGNMENT_MARKER: &str = "__RUBASH_CA1__";

/// Injected tag marking a group-command redirect that the executor added
/// (support_names.rs) — `\x1e`-prefixed so it sorts into the carrier family.
pub(crate) const GROUP_REDIRECT_INJECTED_MARK: &str = "\u{1e}group-redirect";

/// Field separator inside the `__RUBASH_DIR_STACK` env serialization
/// (builtins/pushd/stack.rs). Shares the DATA_DOLLAR byte but lives in the
/// env-serialization domain, not word text — do not decode it as a dollar.
pub(crate) const DIR_STACK_FIELD_SEP: char = '\u{1f}';

/// Prefix role of the \x1c byte: marks a word whose assignment RHS was
/// quoted so tilde/alias consumers suppress expansion roles
/// (expand/tilde/tilde.rs QUOTED_ASSIGNMENT_VALUE). Same codepoint as
/// IFS_GLUE; the prefix position disambiguates.
pub(crate) const QUOTED_WORD_VALUE_PREFIX: char = IFS_GLUE;
/// `&str` companion of QUOTED_WORD_VALUE_PREFIX.
pub(crate) const QUOTED_WORD_VALUE_PREFIX_STR: &str = "\u{1c}";

/// Field separator inside the `__RUBASH_HASH_TABLE` env serialization
/// (builtins/hash.rs). Shares the SUBSCRIPT_CARRIER byte but lives in the
/// env-serialization domain, not word text.
pub(crate) const HASH_ENV_FIELD_SEP: char = SUBSCRIPT_CARRIER;

// ==========================================================================
// Registry table — drives golden assertions and audits.
// ==========================================================================

pub(crate) const MARKERS: &[MarkerInfo] = &[
    MarkerInfo {
        name: "CTLESC",
        code: CTLESC as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PARAM_NAME_END_MARKER",
        code: PARAM_NAME_END_MARKER as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DATA_BACKSLASH",
        code: DATA_BACKSLASH as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROTECTED_BACKSLASH",
        code: PROTECTED_BACKSLASH as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROTECTED_ESCAPED_SQUOTE",
        code: PROTECTED_ESCAPED_SQUOTE as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DATA_SQUOTE",
        code: DATA_SQUOTE as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DATA_DQUOTE",
        code: DATA_DQUOTE as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROTECTED_LITERAL_BACKSLASH",
        code: PROTECTED_LITERAL_BACKSLASH as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DATA_BACKTICK",
        code: DATA_BACKTICK as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "QUOTED_WORD_PREFIX",
        code: QUOTED_WORD_PREFIX as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "IFS_GLUE",
        code: IFS_GLUE as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "STORAGE_WORD_PREFIX",
        code: STORAGE_WORD_PREFIX as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "SUBSCRIPT_CARRIER",
        code: SUBSCRIPT_CARRIER as u32,
        boundaries: &[Boundary::Storage, Boundary::Reparse],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DATA_DOLLAR",
        code: DATA_DOLLAR as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROTECTED_LITERAL_DOLLAR",
        code: PROTECTED_LITERAL_DOLLAR as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROMPT_IGNORE_START",
        code: PROMPT_IGNORE_START as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PROMPT_IGNORE_END",
        code: PROMPT_IGNORE_END as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DEFERRED_COMPOUND_BODY",
        code: DEFERRED_COMPOUND_BODY as u32,
        boundaries: &[Boundary::Reparse],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PREEXPANDED_STDIN_BODY",
        code: PREEXPANDED_STDIN_BODY as u32,
        boundaries: &[Boundary::Reparse],
        user_reachable: true,
    },
    MarkerInfo {
        name: "ARRAY_FIELD_SPLIT_MARKER",
        code: ARRAY_FIELD_SPLIT_MARKER as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "ARRAYREF_FLAG",
        code: ARRAYREF_FLAG as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_VALUE_START",
        code: PATSUB_QUOTED_VALUE_START as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_VALUE_END",
        code: PATSUB_QUOTED_VALUE_END as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_AMP",
        code: PATSUB_QUOTED_AMP as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_BACKSLASH",
        code: PATSUB_QUOTED_BACKSLASH as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "RAW_BYTE_MARKER_ESCAPE",
        code: RAW_BYTE_MARKER_ESCAPE as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "QUOTED_NULL_MARKER",
        code: QUOTED_NULL_MARKER as u32,
        boundaries: &[Boundary::Output],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ANSI_C_QUOTE_MARKER",
        code: ANSI_C_QUOTE_MARKER as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ANSI_C_DQUOTE_MARKER",
        code: ANSI_C_DQUOTE_MARKER as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_DATA_SQUOTE",
        code: ASSIGN_DATA_SQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_DATA_DQUOTE",
        code: ASSIGN_DATA_DQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_DATA_BACKTICK",
        code: ASSIGN_DATA_BACKTICK as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_ESCAPED_DQUOTE",
        code: ASSIGN_ESCAPED_DQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_ESCAPED_SQUOTE",
        code: ASSIGN_ESCAPED_SQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_ESCAPED_BACKSLASH",
        code: ASSIGN_ESCAPED_BACKSLASH as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_HOISTED_SQUOTE",
        code: ASSIGN_HOISTED_SQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_HOISTED_BACKSLASH",
        code: ASSIGN_HOISTED_BACKSLASH as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "COMPOUND_EXPANSION_WS_TAG",
        code: COMPOUND_EXPANSION_WS_TAG as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_SQ_DOLLAR",
        code: ASSIGN_SQ_DOLLAR as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_SQ_BACKTICK",
        code: ASSIGN_SQ_BACKTICK as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_SQ_BACKSLASH",
        code: ASSIGN_SQ_BACKSLASH as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PARAM_WORD_BACKSLASH_GUARD",
        code: PARAM_WORD_BACKSLASH_GUARD as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ESCAPED_IFS_GUARD",
        code: ESCAPED_IFS_GUARD as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PROMPT_ESCAPE_GUARD",
        code: PROMPT_ESCAPE_GUARD as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "CASE_PATTERN_BACKSLASH_GUARD",
        code: CASE_PATTERN_BACKSLASH_GUARD as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "DIR_STACK_FIELD_SEP",
        code: DIR_STACK_FIELD_SEP as u32,
        boundaries: &[],
        user_reachable: true,
    },
];

// ==========================================================================
// Paired encode/decode helpers.
// ==========================================================================

/// Encode one raw byte as a marker pair (E000 + E001+byte).
pub(crate) fn push_raw_byte_marker(output: &mut String, byte: u8) {
    output.push(char::from_u32(RAW_BYTE_MARKER_ESCAPE).expect("sentinel is valid"));
    output.push(char::from_u32(RAW_BYTE_MARKER_FIRST + byte as u32).expect("marker char is valid"));
}

pub(crate) fn encode_raw_byte_marker(byte: u8) -> String {
    let mut output = String::new();
    push_raw_byte_marker(&mut output, byte);
    output
}

/// CTLESC pair: mark `data` as a protected literal char.
pub(crate) fn push_ctlesc_escaped(output: &mut String, data: char) {
    output.push(CTLESC);
    output.push(data);
}

/// Remove CTLESC (\x11) sentinels the lexer inserts before quoted glob
/// metacharacters: the \x11 is a marker, the following character is the
/// data it protects. This is the storage-side half of GNU dequote_string
/// (subst.c:4807), which strips the CTLESC pairs after pathname expansion
/// decided not to consume the word — a stored element like `"p"/"*z"`
/// keeps the quoted `*` as data, never the sentinel.
pub(crate) fn dequote_ctlesc_pairs(value: &str) -> String {
    if !value.contains(CTLESC) {
        return value.to_string();
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == CTLESC {
            if let Some(next) = chars.next() {
                output.push(next);
            }
        } else {
            output.push(ch);
        }
    }
    output
}

// ==========================================================================
// Literal-char escape — user-supplied chars that collide with the registry
// zone are E400-prefixed at entry (the same rule as user bytes vs C0
// carriers, B1 \x05 lesson). A dedicated introducer is required because the
// raw-byte payload range E000-E0FF overlaps the zone itself: `E000+E0A0`
// must stay byte 0xA0, so it cannot also mean literal U+E0A0. Decode:
// `E400 + c` restores `c` verbatim; `E400` alone is emitted literally.
// ==========================================================================

/// First codepoint of the registry zone a user char must never carry raw.
pub(crate) const MARKER_ZONE_FIRST: u32 = 0xE000;
/// Last codepoint of the registry zone (reserves headroom past E318).
pub(crate) const MARKER_ZONE_LAST: u32 = 0xE3FF;
/// Introducer for the literal-char escape; sits just above the zone.
pub(crate) const LITERAL_CHAR_ESCAPE: char = '\u{E400}';
pub(crate) const LITERAL_CHAR_ESCAPE_STR: &str = "\u{E400}";

/// True when `c` collides with a transport codepoint and therefore must
/// be escaped when it arrives as user data: the whole registry zone plus
/// the escape introducer itself.
pub(crate) fn is_marker_zone_char(c: char) -> bool {
    (MARKER_ZONE_FIRST..=MARKER_ZONE_LAST).contains(&(c as u32)) || c == LITERAL_CHAR_ESCAPE
}

/// Emit `c` as transport text: colliding chars get the `E400` literal
/// prefix so a later decode boundary restores them verbatim instead of
/// reading them as markers (E1xx byte-pair / UTF-8 mixed ambiguity fix).
pub(crate) fn push_literal_char(output: &mut String, c: char) {
    if is_marker_zone_char(c) {
        output.push(LITERAL_CHAR_ESCAPE);
    }
    output.push(c);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No named marker may sit inside the byte-char range E100..=E1FF —
    /// that range is generated dynamically from arbitrary user bytes, so a
    /// fixed sentinel there collides (the E10A FAILED_SUBSCRIPT_SENTINEL /
    /// SQ_DOLLAR_DATA double-occupancy and the E101-E10C DATA_* overlap).
    #[test]
    fn no_marker_inside_byte_char_range() {
        for m in MARKERS {
            assert!(
                !(BYTE_CHAR_BASE..BYTE_CHAR_BASE + 0x100).contains(&m.code),
                "{} at U+{:04X} collides with BYTE_CHAR_BASE byte-chars",
                m.name,
                m.code
            );
        }
        // Regression pins for the historical collisions.
        assert_eq!(FAILED_SUBSCRIPT_SENTINEL, "\u{E200}");
        assert_eq!(ASSIGN_SQ_DOLLAR, '\u{E30A}');
        assert!('\u{E10A}' as u32 != ASSIGN_SQ_DOLLAR as u32);
    }

    /// PUA-block markers must be unique — they share one transport domain.
    /// (C0 bytes may legitimately repeat across domains, e.g. \x1f is both
    /// DATA_DOLLAR in word text and DIR_STACK_FIELD_SEP in env serialization.)
    #[test]
    fn pua_markers_are_unique() {
        let mut pua: Vec<u32> = MARKERS
            .iter()
            .map(|m| m.code)
            .filter(|&c| c >= 0xE000)
            .collect();
        pua.sort_unstable();
        let before = pua.len();
        pua.dedup();
        assert_eq!(pua.len(), before, "duplicate PUA marker codepoint");
    }

    /// `E400` literal-char escape round-trips marker-zone chars so a user
    /// `$'\uE314'` can never be read back as a guard.
    #[test]
    fn literal_char_escape_roundtrips_marker_zone() {
        let mut out = String::new();
        push_literal_char(&mut out, '\u{E314}');
        assert_eq!(out, "\u{E400}\u{E314}");
        let mut plain = String::new();
        push_literal_char(&mut plain, 'q');
        assert_eq!(plain, "q");
        // Payload-range literal char: E0A0 must NOT use the E000 prefix
        // (that would decode as raw byte 0xA0, not the PUA glyph).
        let mut payload = String::new();
        push_literal_char(&mut payload, '\u{E0A0}');
        assert_eq!(payload, "\u{E400}\u{E0A0}");
        // The escape introducer escapes itself.
        let mut esc = String::new();
        push_literal_char(&mut esc, '\u{E400}');
        assert_eq!(esc, "\u{E400}\u{E400}");
        // Every registered PUA codepoint is inside the zone.
        for m in MARKERS {
            assert!(
                m.code < 0xE000 || is_marker_zone_char(char::from_u32(m.code).unwrap()),
                "{} outside marker zone",
                m.name
            );
        }
    }

    /// Every `_STR` companion must spell the same codepoint as its `char`.
    #[test]
    fn str_companions_match_char_consts() {
        let pairs: [(char, &str); 36] = [
            (CTLESC, CTLESC_STR),
            (PARAM_NAME_END_MARKER, PARAM_NAME_END_MARKER_STR),
            (DATA_BACKSLASH, DATA_BACKSLASH_STR),
            (PROTECTED_BACKSLASH, PROTECTED_BACKSLASH_STR),
            (PROTECTED_ESCAPED_SQUOTE, PROTECTED_ESCAPED_SQUOTE_STR),
            (DATA_SQUOTE, DATA_SQUOTE_STR),
            (DATA_DQUOTE, DATA_DQUOTE_STR),
            (PROTECTED_LITERAL_BACKSLASH, PROTECTED_LITERAL_BACKSLASH_STR),
            (DATA_BACKTICK, DATA_BACKTICK_STR),
            (QUOTED_WORD_PREFIX, QUOTED_WORD_PREFIX_STR),
            (IFS_GLUE, IFS_GLUE_STR),
            (STORAGE_WORD_PREFIX, STORAGE_WORD_PREFIX_STR),
            (SUBSCRIPT_CARRIER, SUBSCRIPT_CARRIER_STR),
            (DATA_DOLLAR, DATA_DOLLAR_STR),
            (PROTECTED_LITERAL_DOLLAR, PROTECTED_LITERAL_DOLLAR_STR),
            (PROMPT_IGNORE_START, PROMPT_IGNORE_START_STR),
            (PROMPT_IGNORE_END, PROMPT_IGNORE_END_STR),
            (DEFERRED_COMPOUND_BODY, DEFERRED_COMPOUND_BODY_STR),
            (PREEXPANDED_STDIN_BODY, PREEXPANDED_STDIN_BODY_STR),
            (ARRAY_FIELD_SPLIT_MARKER, ARRAY_FIELD_SPLIT_MARKER_STR),
            (ARRAYREF_FLAG, ARRAYREF_FLAG_STR),
            (ANSI_C_QUOTE_MARKER, ANSI_C_QUOTE_MARKER_STR),
            (ANSI_C_DQUOTE_MARKER, ANSI_C_DQUOTE_MARKER_STR),
            (QUOTED_NULL_MARKER, QUOTED_NULL_MARKER_STR),
            (ASSIGN_DATA_SQUOTE, ASSIGN_DATA_SQUOTE_STR),
            (ASSIGN_DATA_DQUOTE, ASSIGN_DATA_DQUOTE_STR),
            (ASSIGN_DATA_BACKTICK, ASSIGN_DATA_BACKTICK_STR),
            (ASSIGN_ESCAPED_DQUOTE, ASSIGN_ESCAPED_DQUOTE_STR),
            (ASSIGN_ESCAPED_SQUOTE, ASSIGN_ESCAPED_SQUOTE_STR),
            (ASSIGN_ESCAPED_BACKSLASH, ASSIGN_ESCAPED_BACKSLASH_STR),
            (ASSIGN_HOISTED_SQUOTE, ASSIGN_HOISTED_SQUOTE_STR),
            (ASSIGN_HOISTED_BACKSLASH, ASSIGN_HOISTED_BACKSLASH_STR),
            (COMPOUND_EXPANSION_WS_TAG, COMPOUND_EXPANSION_WS_TAG_STR),
            (ASSIGN_SQ_DOLLAR, ASSIGN_SQ_DOLLAR_STR),
            (ASSIGN_SQ_BACKTICK, ASSIGN_SQ_BACKTICK_STR),
            (ASSIGN_SQ_BACKSLASH, ASSIGN_SQ_BACKSLASH_STR),
        ];
        for (ch, s) in pairs {
            assert_eq!(s, ch.to_string());
        }
    }

    #[test]
    fn raw_byte_marker_pair_roundtrips() {
        for byte in [0x00u8, 0x05, 0x11, 0x1f, 0x80, 0xff] {
            let encoded = encode_raw_byte_marker(byte);
            let mut chars = encoded.chars();
            assert_eq!(chars.next().map(|c| c as u32), Some(RAW_BYTE_MARKER_ESCAPE));
            assert_eq!(
                chars.next().map(|c| c as u32),
                Some(RAW_BYTE_MARKER_FIRST + byte as u32)
            );
            assert!(chars.next().is_none());
        }
    }
}
