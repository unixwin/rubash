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
//!   `MARKERS`, plus golden coverage: the registry-driven typed-data
//!   test in `tests/marker_registry_roundtrips.rs` iterates `MARKERS`
//!   through the core shapes (a new marker gets cells automatically), and
//!   `tests/marker_leak_golden.rs` (+ its PTY sibling) asserts one cell
//!   per output channel that
//!   sentinel never appears in stdout / `declare -p` / xtrace output on
//!   marker-free input (the #824 audit moved leak detection out of the
//!   shared decoder's output — there the codepoints are indistinguishable
//!   from restored user data).
//! - `user_reachable == true` markers can equal a byte the user can type or
//!   pipe in; the entry point producing transport text MUST re-encode that
//!   byte (raw-byte marker pair), or the marker silently corrupts data.
//!
//! ## Decode boundary entry points (governance 3.2)
//!
//! | Boundary | Entry point | Site |
//! |---|---|---|
//! | Output | `crate::locale::decode_to_visible_text` — the one sanctioned transport→visible decoder (echo paths additionally run `substitution_metadata::decode_raw_byte_markers` because echo input is already expansion-decoded text that can still carry raw-byte pairs) | `builtins/echo.rs`, `declare` printing, xtrace, `bad_substitution_display`, host `dump-strings` |
//! | Display value rendering (#824 audit) | stored values feed the Output decoder on the `declare -p` path (`builtins/declare/storage.rs` `display_text` / `quote_array_element_value`) | user-literal `__RUBASH_HD1__`/`__RUBASH_CSB1_` ASCII, `$'\uE3xx'` registry-zone chars (E400-escaped at entry, #735) and raw C0 bytes (raw-byte pairs) all decode to themselves there — whitelist: the decoder carries no output-side leak asserts; a registry codepoint in decoded output is restored user data by contract |
//! | Display xtrace rendering (#824 audit) | expanded words feed the same Output decoder (`prompt_expansion.rs` xtrace arms) | same whitelist: `set -x; v=$'\uE314Z'; echo "$v"` traces the U+E314 char, and literal `__RUBASH_*` text traces verbatim — both byte-identical to GNU bash 5.3.0 |
//! | Host AST inspection | hosts decode AST words BETWEEN parse and execution on the interactive line-editor path (`decoded_words`: easter-egg routing, this-shell dispatch, positional params); those words legitimately carry the COMPOUND_ASSIGNMENT_MARKER (U+E31A since the #1634 protocol revision) the parser pushed (parser/token_actions.rs), so the decoder whitelists it instead of asserting it away (#802) | host `decoded_words`, `decode_to_visible_text` |
//! | Storage | values reaching `env_vars`/array stores are already data bytes — the encoder side is `substitution_metadata` (`bytes_to_assignment_shell_text`, `push_escaped_text_with_carriers`) which pair-encodes every `user_reachable` byte at entry so storage never confuses a user byte with a carrier | variable/array assignment paths |
//! | Patsub replacement (rubash#843) | replacement-side literal chars are E400-escaped IN FLIGHT by the scan pass (`expand_braced_replacement.rs` `push_replacement_literal` / ANSI-C body collection), and `finish_patsub_replacement` copies E400/E000 pairs verbatim before resolving the PATSUB markers — a bare E310..=E313 in the marked text is pipeline protocol, an escaped pair is user data (`${v/b/$'\uE312x'}` emits U+E312, GNU-verified) | `${var/pat/rep}` replacement text |
//! | Reparse | marker bytes are the lexer's own protocol: text re-fed through `eval`, alias bodies and command-substitution bodies re-lex with markers intact; user bytes inside reparsed text were pair-encoded at the original entry so they decode as data, never as carriers | `eval`, alias expansion, comsub reparse |
//!
//! Golden rule (amended by the #824 audit): on marker-FREE input no
//! `MARKERS` codepoint may appear in stdout, `declare -p`, or xtrace output
//! — asserted by `tests/marker_leak_golden.rs` at the integration boundary.
//! Input that legitimately carries registry codepoints (user-literal
//! named-marker ASCII, `$'\uE3xx'` data, raw C0 bytes) decodes to those
//! codepoints verbatim: the shared decoder's output carries no "leaked
//! marker" assertions — on that boundary they could only fire on
//! well-formed user data, which is exactly the #824 panic family.
//!
//! A row whose `boundaries` is empty claims no decode boundary: the
//! carrier is internal-only (consumed inside its owning subsystem and
//! never crossing output, storage, or reparse text). Such rows must be
//! listed in the `INTERNAL_ONLY_ROWS` table in this module's tests
//! (rubash#2885 E8); an unlisted empty row fails there, and a listed
//! row that gains a claim must be removed from the table.
//!
//! The `__RUBASH_*` environment-serialization namespace — the named
//! half of this protocol — is censused in the ENV NAMESPACE CENSUS
//! block directly above the `MARKERS` table, and the
//! `env_namespace_census_is_complete` test keeps that census
//! exhaustive against the source tree.

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

/// Parser reader-state sentinel for `update_compound_boundary_stack`:
/// a case pattern list is being read (mirrors GNU PST_CASEPAT,
/// parser.h:29, set at parse.y 3379/3396). NUL prefix cannot collide
/// with any token value the stack otherwise holds. NUL cannot appear in
/// transport text, so the sentinel is not user-reachable by construction.
pub(crate) const CASE_PATTERN_EXPECT: &str = "\u{0}case-pattern";
/// Entered right after `in' or a clause terminator: the NEXT token may be
/// the optional clause-opening `(' (parse.y:1231).
pub(crate) const CASE_PATTERN_MAY_OPEN: &str = "\u{0}case-pattern-may-open";
/// An extglob `(' opened inside the pattern text (parse.y:5466 absorbs
/// these into the pattern word in GNU; rubash's tokenizer emits them).
pub(crate) const CASE_PATTERN_PAREN: &str = "\u{0}case-pattern-paren";

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
//
// rubash#843: the codepoints are user-reachable — `$'\uE312'` is real
// replacement data for GNU — so the patsub scan pass E400-escapes every
// literal registry-zone char in flight (push_replacement_literal) and
// finish_patsub_replacement copies escape/raw-byte pairs verbatim before
// resolving the markers. `user_reachable` is honored in-flight rather than
// at a global entry boundary: the pipeline is the re-encoding producer.
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
/// ANSI-C `$'...'` decoded blank (space/tab/newline) guard: GNU CTLESC-
/// quotes every decoded byte (parse.y:5560-5575 ansiexpand + sh_single_
/// quote), so a decoded blank is literal word data that field splitting
/// must never treat as a separator -- only EXPANSION RESULTS split
/// (rubash#379: `A$((1+1))B$'\n'C$((2+2))D` is one argument). E401 lives
/// outside every dynamic/escape family (raw-byte pair payloads
/// E001..=E100, byte-chars E100..=E1FF, registry zone E000..=E3FF and the
/// E400 literal-char-escape introducer), so comsub readback never
/// E400-prefixes it and boundary strips always see it bare. Field-split
/// call sites convert the guard to IFS_GLUE (the splitters' existing
/// next-char-is-literal carrier); boundary strips drop it.
pub(crate) const ANSI_C_IFS_GUARD: char = '\u{E401}';

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
/// Data `\` that came out of a parameter expansion in an unquoted
/// assignment RHS (rubash#218). GNU subst.c:11862 `add_quoted_string:` runs
/// every parameter-expansion result through `quote_string` (subst.c:4773),
/// CTLESC-protecting each character so the final `dequote_list`
/// (subst.c:4388 -> dequote_string subst.c:4807) can never read expansion
/// bytes as source-word backslash syntax. This marker is the same
/// protection scoped to the assignment-RHS walker mode; restored to a real
/// `\` after the assignment path's final unescape pass.
pub(crate) const ASSIGN_EXPANSION_BACKSLASH: char = '\u{E30D}';
/// Data `"` that came out of a parameter/command expansion in a
/// preserve-quotes compound word (rubash#295): like the backslash carrier
/// above it must survive the storage tokenizer's quote removal as DATA —
/// a bare expansion `"` re-entered quote syntax and mangled the element
/// (`Y=($X'Q')` with X='a"b' stored `ab'Q'`). Decoded at the same
/// boundaries as ASSIGN_EXPANSION_BACKSLASH.
pub(crate) const ASSIGN_EXPANSION_DQUOTE: char = '\u{E30E}';
/// Data `'` from an expansion result — same contract as
/// ASSIGN_EXPANSION_DQUOTE.
pub(crate) const ASSIGN_EXPANSION_SQUOTE: char = '\u{E30F}';

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
pub(crate) const ASSIGN_EXPANSION_BACKSLASH_STR: &str = "\u{E30D}";
pub(crate) const ASSIGN_EXPANSION_DQUOTE_STR: &str = "\u{E30E}";
pub(crate) const ASSIGN_EXPANSION_SQUOTE_STR: &str = "\u{E30F}";
pub(crate) const ANSI_C_QUOTE_MARKER_STR: &str = "\u{E010}";
pub(crate) const ANSI_C_DQUOTE_MARKER_STR: &str = "\u{E011}";
pub(crate) const ANSI_C_IFS_GUARD_STR: &str = "\u{E401}";
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

/// Command-substitution payload introducer inside transport text
/// (execution_misc.rs) — `U+E319` + two hex chars + `;` encodes a byte
/// that cannot ride as a bare char (comsub output's C0 carriers).
///
/// The introducer is a registry-zone PUA char, NOT the former ASCII
/// `__RUBASH_CSB1_` prefix: the ASCII form was plain typeable text, so a
/// user-literal `__RUBASH_CSB1_41;` reaching the payload decode boundary
/// (argv materialization, field-split decisions) was consumed into its
/// payload byte where GNU prints it verbatim (rubash#840 divergence 2).
/// A bare U+E319 in transport is always protocol — user zone chars ride
/// the E400 literal-char escape at every entry (push_literal_char, the
/// same contract as ANSI_C_IFS_GUARD), so the decode is E400-aware and
/// typed `$'\uE319'` data is never claimed. Typed `__RUBASH_CSB1_<hex>;`
/// is now pure user data everywhere and passes through verbatim.
pub(crate) const COMSUB_PAYLOAD_SENTINEL: char = '\u{E319}';
/// `&str` companion of COMSUB_PAYLOAD_SENTINEL.
pub(crate) const COMSUB_PAYLOAD_SENTINEL_STR: &str = "\u{E319}";

/// `( ... )` compound-assignment operand marker (types.rs).
///
/// The introducer is a registry-zone PUA char, NOT the former ASCII
/// `__RUBASH_CA1__` prefix: the ASCII form was plain typeable text, so a
/// user value whose text merely started with it — a quoted literal, a
/// command-substitution result, a `read` line — was re-parsed as a
/// compound assignment at every storage boundary (type change, marker
/// deletion; rubash#1634). A bare U+E31A in transport is always
/// protocol — user zone chars ride the E400 literal-char escape at
/// every entry (push_literal_char, the same contract as
/// COMSUB_PAYLOAD_SENTINEL, rubash#840), so typed `$'\uE31A'` data is
/// never claimed, and typed `__RUBASH_CA1__` text is now pure user data
/// everywhere (the #802/#824 reserved-string stance for this marker is
/// retired by the protocol revision).
pub(crate) const COMPOUND_ASSIGNMENT_MARKER: &str = "\u{E31A}";
/// `char` companion of COMPOUND_ASSIGNMENT_MARKER.
pub(crate) const COMPOUND_ASSIGNMENT_MARKER_CHAR: char = '\u{E31A}';

/// Quoted-expansion-site marker: precedes a `$`/backtick expansion
/// introducer whose site lay inside a quoted segment of the raw word
/// (producer: command_prepare.rs `mark_quoted_ifs_from_raw`, the
/// pre-expansion marking pass). GNU subst.c:11229
/// expand_word_internal tracks every expansion site's quoting in the
/// walk itself and adds a quoted site's result through
/// add_quoted_string (subst.c:11862), so word_list_split never splits
/// inside it (subst.c:11552: the result "will be split, but this
/// portion should not be"). Rubash's cooked words have already lost
/// the quote characters, so the marking pass — the one place that
/// still simulates the raw word — carries the verdict in-band, and
/// the expansion walkers (embedded_parameters.rs /
/// embedded_mutations.rs) consume the marker on sight and
/// IFS-protect that one expansion's result span. A bare U+E31B in
/// transport is always protocol: a typed `$'\uE31B'` arrives
/// E400-escaped at entry (push_literal_char), hence
/// user_reachable == false. The marker is consumed by the walkers —
/// except on the diagnostic path: the bad-substitution diagnostic
/// cites the word recorded BEFORE the walk
/// (PendingDiagnosticWordGuard), so a bare marker reaches the display
/// decoder (`decode_to_visible_text`, rubash#3433) and that decoder
/// drops it, the same treatment as the sibling U+E31A marker.
pub(crate) const QUOTED_EXPANSION_SITE: char = '\u{E31B}';
/// `&str` companion of QUOTED_EXPANSION_SITE.
pub(crate) const QUOTED_EXPANSION_SITE_STR: &str = "\u{E31B}";

/// Injected tag marking a group-command redirect that the executor added
/// (support_names.rs) — `\x1e`-prefixed so it sorts into the carrier family.
pub(crate) const GROUP_REDIRECT_INJECTED_MARK: &str = "\u{1e}group-redirect";

/// GNU subst.c:4807 dequote_string for word-position consumers (case word,
/// here-string word, pipeline word — rubash#153). The data carriers that
/// traveled through the expansion walkers as quote-protected bytes (the
/// CTLESC port, parse.y:5694-5706 got_escaped_character) become the literal
/// characters they denote. These consumers own the final value — stdin
/// content, strmatch subject — and have no later quote-removal pass, so
/// leaving the carriers encoded leaks \x17/\x1a into output.
pub(crate) fn decode_word_position_carriers(value: &str) -> String {
    // rubash#237 perf: the plain-word case (no marker byte present) is the
    // overwhelming majority — every `.replace` below would then be an
    // identity copy with a fresh String allocation per call. One scan
    // answers it.
    if !contains_word_marker_bytes(value) {
        return value.to_string();
    }
    // The heredoc-mode walker protects a `'`/`"` in unquoted herestring
    // text with the ANSI-C markers (the same decode
    // eval_source_for_reparse applies, execution_misc.rs) — a `\'`
    // escape pair in the raw word reaches this decoder as marker + '.
    // E400-aware restore (rubash#840 divergence 3): the PUA members of the
    // rule set can arrive as the escaped member of an E400 literal-char
    // pair — typed `$'\uE010'` is user data, not a live marker.
    restore_bare_sentinels(
        value,
        &[
            (DATA_SQUOTE, "'"),
            (DATA_DQUOTE, "\""),
            (DATA_BACKTICK, "`"),
            (DATA_DOLLAR, "$"),
            (ANSI_C_QUOTE_MARKER, "'"),
            (ANSI_C_DQUOTE_MARKER, "\""),
        ],
    )
}

/// Whether `value` contains any byte of the in-band word-text carrier
/// family: the C0 range 0x11..=0x1f hosting every CTLESC/CTLNUL-port quote
/// carrier (CTLESC, PARAM_NAME_END_MARKER, DATA_* 0x14..=0x1f) or the UTF-8
/// lead byte 0xEE of the U+E000..U+EFFF PUA plane hosting the ANSI-C
/// markers. A false positive only costs the un-guarded slow path; a false
/// negative within the predicate's SCOPE is impossible and enforced by
/// `word_marker_byte_predicate_scope_is_enforced` (rubash#728 §3): every
/// char-marker in 0x01..=0xff is either in 0x11..=0x1f or in an explicitly
/// listed out-of-scope domain.
///
/// Out of scope by domain (their owners decode them, and entry re-encoding
/// disambiguates user bytes):
/// - PROMPT_IGNORE_* (0x01/0x02): the readline prompt-ignore layer.
/// - DEFERRED_COMPOUND_BODY (0x03), PREEXPANDED_STDIN_BODY (0x05),
///   ARRAY_FIELD_SPLIT_MARKER (0x10): compound-assignment/heredoc/array
///   element transport consumed by their own boundary helpers, never by
///   the word-text decode chains this predicate gates.
/// - CASE_PATTERN_* (NUL prefix): parser reader-state sentinels; NUL
///   cannot appear in transport text.
// `pub(crate)` (not `pub(in crate::executor)`): the registry file is
// also compiled into the `tests/marker_registry_roundtrips.rs` crate via
// `#[path]`, where no `crate::executor` module exists. Still
// crate-internal — no public surface.
pub(crate) fn contains_word_marker_bytes(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| (0x11..=0x1f).contains(&byte) || byte == 0xEE)
}

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

// ENV NAMESPACE CENSUS (rubash#2885 E8)
//
// The named half of the carrier protocol: every `__RUBASH_*` key the
// engine writes into a child/subshell environment (or reads back
// from one) is enumerated here. EXACT entries are complete keys;
// family entries are literal prefixes whose full keys are composed
// at runtime. The env_namespace_census_is_complete test scans
// src/ for `__RUBASH_...` tokens and fails on any token that is
// neither an EXACT entry nor FAMILY-covered, and on any EXACT
// entry that no longer occurs - a new serialization key must be
// registered here in the same change that introduces it.
// (Keys composed without a literal `__RUBASH_` fragment in source
// would evade the scan; the namespace is written with literal
// prefixes by convention - keep it that way.)
//
// FAMILY __RUBASH_SETOPT_ - shopt option state: prefix + option name, one key per option
// FAMILY __RUBASH_FD_ - per-fd state: prefix + role infix (PATH_/WPATH_/HANDLE_/WHANDLE_/TARGET_/ACCESS_/FILE_/PIPE_/CLOSED_/STDIN_/OUTPUT_/TERMINAL_/DYNAMIC_INPUT_) + fd index
// FAMILY __RUBASH_TRAP_ - per-trap state: prefix + signal/trap name (SIG* spelling, legacy bare name, or signal number)
// FAMILY __RUBASH_ULIMIT_ - ulimit state: prefix + limit letter
// FAMILY __RUBASH_PARSE_ERROR_ - parse-error payload fields: prefix + field name (fields joined by PARSE_ERROR_FIELD_SEP)
// FAMILY __RUBASH_PARSE_SOURCE__ - parse-source span table: prefix + span id
// FAMILY __RUBASH_PARSE_ - parse-state serialization keys (umbrella; PARSE_ERROR_ / PARSE_SOURCE__ subfamilies above)
// FAMILY __RUBASH_CSB1_ - comsub payload serialization v1: prefix + slot/index suffix
// FAMILY __RUBASH_COPROC_ - coproc plumbing: prefix + role/index suffix
// FAMILY __RUBASH_ALIAS_ - alias state: prefix + generation/line suffix
// FAMILY __RUBASH_FORK_ - fork/subshell state snapshot prefix literal (snapshot keys are the EXACT FORK_* entries)
//
// EXACT __RUBASH_ALIAS_STREAMED - state key (alias streamed) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARGV0_AFTER_UNSET - state key (argv0 after unset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARGV_DIALECT - argv dialect selection for respawned children (R2 frozen switch, see AGENTS.md)
// EXACT __RUBASH_ARITH_EXP_EXPANDED - state key (arith exp expanded) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARITH_NOASSIGN_ERROR - state key (arith noassign error) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARITH_READONLY_ERROR - state key (arith readonly error) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARITH_SUBSCRIPT_EXPR - state key (arith subscript expr) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ARRAY_VARS - serialized variable-name set (array) for child/subshell state transfer
// EXACT __RUBASH_ASSOC_128_VARS - serialized variable-name set (assoc 128) for child/subshell state transfer
// EXACT __RUBASH_ASSOC_VARS - serialized variable-name set (assoc) for child/subshell state transfer
// EXACT __RUBASH_ASYNC_CHILD - state key (async child) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_BACKGROUND_WRAPPER - state key (background wrapper) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_BG_BASHPID_BASE - state key (bg bashpid base) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_BG_DEPTH - state key (bg depth) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CA1__ - RETIRED string spelling of the compound-assignment marker; typeable user text since retirement - do not re-register
// EXACT __RUBASH_CAPCASE_VARS - serialized variable-name set (capcase) for child/subshell state transfer
// EXACT __RUBASH_CMD_FILE - state key (cmd file) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CMD_START_LINE - state key (cmd start line) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CNF_HANDLER_RUNNING - state key (cnf handler running) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_COMPATIBLE_SHELL_PATH - state key (compatible shell path) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_COMPOUND_SYNTAX_ERROR - state key (compound syntax error) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_COMPOUND_SYNTAX_ERROR__ - state key (compound syntax error  ) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_COMSUB_BODY - state key (comsub body) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_COPROC_CHILD - state key (coproc child) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CURRENT_COMMAND - state key (current command) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CURRENT_FILE - state key (current file) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CURRENT_FUNCTION - state key (current function) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_CURRENT_LINE - state key (current line) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_DEBUGGER_ATTEMPTED - state key (debugger attempted) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_DEBUGGER_MODE - state key (debugger mode) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_DECLARED_UNSET_VARS - serialized variable-name set (declared unset) for child/subshell state transfer
// EXACT __RUBASH_DIR_STACK - state key (dir stack) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_DISABLED_BUILTINS - state key (disabled builtins) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ERREXIT - state key (errexit) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_EVAL_CONTEXT - state key (eval context) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_EXPORTED_FUNCTIONS - state key (exported functions) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_EXPORTED_VARS - serialized variable-name set (exported) for child/subshell state transfer
// EXACT __RUBASH_EXTGLOB_ENABLED_LINE - state key (extglob enabled line) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FD0_MIRROR_FACT - fd-0 mirror fact for spawn reconstruction
// EXACT __RUBASH_FD0_STAGE_PIPE - fd-0 staging pipe marker for spawn reconstruction
// EXACT __RUBASH_FD_PROCESS_STDIN - state key (fd process stdin) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FD_STDERR - state key (fd stderr) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FD_STDOUT - state key (fd stdout) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FILE_MODES - state key (file modes) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FORK_ARGV_BOTTOM - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_BASH_ARGC - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_BASH_ARGV - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_BASH_LINENO - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_BASH_SOURCE - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_FN_SPANS - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_FUNCNAME - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_IN_FUNCTION - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_LOGIN_SHELL - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_MAIN_FRAME - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_PIPESTATUS - fork/subshell state snapshot member
// EXACT __RUBASH_FORK_ROOT_SPANS - fork/subshell state snapshot member
// EXACT __RUBASH_FUNCTION_STDIN - state key (function stdin) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FUNCTION_STDIN_OFFSET - state key (function stdin offset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_FUNC_TRACE_FUNCTIONS - state key (func trace functions) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_GAP12_JOURNAL - GAP-12 Session env-journal channel (host<->engine journal state)
// EXACT __RUBASH_GAP12_U3 - GAP-12 Session env-journal update channel U3
// EXACT __RUBASH_GETOPTS_OFFSET - state key (getopts offset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_GETOPTS_OPTIND - state key (getopts optind) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_HASH_TABLE - state key (hash table) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_HD1__ - string spelling of QUOTED_HEREDOC_MARKER (the one registered string marker)
// EXACT __RUBASH_INHERIT_PROCESS_STDIN - state key (inherit process stdin) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_INITIAL_PATH - state key (initial path) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_INTEGER_VARS - serialized variable-name set (integer) for child/subshell state transfer
// EXACT __RUBASH_INTERACTIVE - state key (interactive) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_INTERACTIVE_FLAG_OFF - state key (interactive flag off) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_INVISIBLE_VARS - serialized variable-name set (invisible) for child/subshell state transfer
// EXACT __RUBASH_IN_SOURCE - state key (in source) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ISSUE2084_HOSTVAR - host-variable surface pinned by rubash#2084 (Session API acceptance fixture name)
// EXACT __RUBASH_IS_C - child-invocation mode flag (invoked as C helper personality)
// EXACT __RUBASH_LAST_COMMAND - state key (last command) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_LINE_OFFSET - state key (line offset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_LOCAL_EXPORT_ENV - state key (local export env) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_LOGIN_SHELL - state key (login shell) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_LOWERCASE_VARS - serialized variable-name set (lowercase) for child/subshell state transfer
// EXACT __RUBASH_NAMEREF_VARS - serialized variable-name set (nameref) for child/subshell state transfer
// EXACT __RUBASH_NO_EDITING - state key (no editing) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_NO_PROFILE - state key (no profile) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_NO_RC - state key (no rc) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_PARSE - state key (parse) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_PARSE_ERROR - state key (parse error) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_PC_NOHIST - state key (pc nohist) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_PHYSICAL_PWD - state key (physical pwd) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_PIPESSTATUS_STATE - legacy spelling of the PIPESTATUS state key (kept for compat)
// EXACT __RUBASH_PIPESTATUS_STATE - PIPESTATUS array state for respawned children
// EXACT __RUBASH_POSITIONAL_PARAMS - state key (positional params) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_POSIX_MODE - state key (posix mode) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_POSIX_TOOLS_DIR - state key (posix tools dir) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_READONLY_FUNCTIONS - state key (readonly functions) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_READONLY_VARS - serialized variable-name set (readonly) for child/subshell state transfer
// EXACT __RUBASH_READ_STDIN - state key (read stdin) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_RETURN_TRAP_FUNCTION - state key (return trap function) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SCRIPT_NAME - state key (script name) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SECONDS_OFFSET - state key (seconds offset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SESSION_SCOPE - state key (session scope) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHELL_NAME - state key (shell name) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHELL_PID - state key (shell pid) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHELL_ROOT - state key (shell root) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHELL_START_EPOCH - state key (shell start epoch) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHOPT_CHECKHASH - state key (shopt checkhash) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SHOPT_STATE - state key (shopt state) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SIGNAL_TRAP_DEPTH - state key (signal trap depth) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SIGNAL_TRAP_STATUS - state key (signal trap status) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SOURCED_RC - state key (sourced rc) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SPAWNER_PID - state key (spawner pid) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_STARTUP_FILES_DONE - state key (startup files done) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_STATE_FILE - state key (state file) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_SUBSHELL_DEPTH - state key (subshell depth) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TEMP_PATH - state key (temp path) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TOP_LEVEL_NAME - state key (top level name) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TRACE_VARS - serialized variable-name set (trace) for child/subshell state transfer
// EXACT __RUBASH_TRANSPORT - transport-mode flag for respawned children (marker pair transport on/off)
// EXACT __RUBASH_TRAP - trap table serialization (whole-table key)
// EXACT __RUBASH_TRAPS - trap name set serialization
// EXACT __RUBASH_TRAP_CONTEXT - state key (trap context) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TRAP_DEBUG - state key (trap debug) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TRAP_EXIT - state key (trap exit) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TRAP_ORIG_IGN - state key (trap orig ign) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_TRAP_RESET - state key (trap reset) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_UMASK - state key (umask) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_UNSET_DYNAMIC_VARS - serialized variable-name set (unset dynamic) for child/subshell state transfer
// EXACT __RUBASH_UNSTREAMED_FUNCTION_BODIES - state key (unstreamed function bodies) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_UPPERCASE_VARS - serialized variable-name set (uppercase) for child/subshell state transfer
// EXACT __RUBASH_X - arithmetic dynamic-variable probe list member (never a real variable)
// EXACT __RUBASH_XTRACE - state key (xtrace) - serialized across the respawn/subshell boundary; see the owning subsystem
// EXACT __RUBASH_ZSH_OPTIONS - state key (zsh options) - serialized across the respawn/subshell boundary; see the owning subsystem
// END ENV NAMESPACE CENSUS

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
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_VALUE_END",
        code: PATSUB_QUOTED_VALUE_END as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_AMP",
        code: PATSUB_QUOTED_AMP as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PATSUB_QUOTED_BACKSLASH",
        code: PATSUB_QUOTED_BACKSLASH as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
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
        name: "ASSIGN_EXPANSION_BACKSLASH",
        code: ASSIGN_EXPANSION_BACKSLASH as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_EXPANSION_DQUOTE",
        code: ASSIGN_EXPANSION_DQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ASSIGN_EXPANSION_SQUOTE",
        code: ASSIGN_EXPANSION_SQUOTE as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PARAM_WORD_BACKSLASH_GUARD",
        code: PARAM_WORD_BACKSLASH_GUARD as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "ESCAPED_IFS_GUARD",
        code: ESCAPED_IFS_GUARD as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "PROMPT_ESCAPE_GUARD",
        code: PROMPT_ESCAPE_GUARD as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "CASE_PATTERN_BACKSLASH_GUARD",
        code: CASE_PATTERN_BACKSLASH_GUARD as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "COMSUB_PAYLOAD_SENTINEL",
        code: COMSUB_PAYLOAD_SENTINEL as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "DIR_STACK_FIELD_SEP",
        code: DIR_STACK_FIELD_SEP as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    // --- rubash#728: the registry must enumerate every carrier const the
    // file declares (the audited gap: 12 carrier-shaped consts lived
    // outside this table, so the self-consistency tests never saw them).
    MarkerInfo {
        name: "PATTERN_LITERAL_BACKSLASH",
        code: PATTERN_LITERAL_BACKSLASH as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "PARSE_ERROR_FIELD_SEP",
        code: PARSE_ERROR_FIELD_SEP as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "HEREDOC_WARNED_BODY_PREFIX",
        code: HEREDOC_WARNED_BODY_PREFIX as u32,
        boundaries: &[Boundary::Reparse],
        user_reachable: true,
    },
    MarkerInfo {
        name: "ANSI_C_IFS_GUARD",
        code: ANSI_C_IFS_GUARD as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "LITERAL_CHAR_ESCAPE",
        code: LITERAL_CHAR_ESCAPE as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "FAILED_SUBSCRIPT_SENTINEL",
        code: 0xE200,
        boundaries: &[Boundary::Output],
        user_reachable: false,
    },
    MarkerInfo {
        name: "QUOTED_HEREDOC_MARKER",
        code: 0x5F,
        boundaries: &[Boundary::Reparse, Boundary::Storage],
        user_reachable: false,
    },
    MarkerInfo {
        name: "COMPOUND_ASSIGNMENT_MARKER",
        code: COMPOUND_ASSIGNMENT_MARKER_CHAR as u32,
        boundaries: &[Boundary::Output, Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "QUOTED_EXPANSION_SITE",
        code: QUOTED_EXPANSION_SITE as u32,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "GROUP_REDIRECT_INJECTED_MARK",
        code: 0x1E,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "QUOTED_WORD_VALUE_PREFIX",
        code: QUOTED_WORD_VALUE_PREFIX as u32,
        boundaries: &[Boundary::Output],
        user_reachable: true,
    },
    MarkerInfo {
        name: "HASH_ENV_FIELD_SEP",
        code: HASH_ENV_FIELD_SEP as u32,
        boundaries: &[Boundary::Storage],
        user_reachable: true,
    },
    MarkerInfo {
        name: "CASE_PATTERN_EXPECT",
        code: 0x00,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "CASE_PATTERN_MAY_OPEN",
        code: 0x00,
        boundaries: &[],
        user_reachable: false,
    },
    MarkerInfo {
        name: "CASE_PATTERN_PAREN",
        code: 0x00,
        boundaries: &[],
        user_reachable: false,
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

/// IFS-protect `text`, the field-splitting half of GNU's quote_string
/// (subst.c:4773): every character that belongs to the CURRENT IFS is
/// preceded by IFS_GLUE, the splitter's protection carrier, so the
/// text can be spliced into a word that will be field-split without
/// its IFS characters acting as delimiters. A character already
/// preceded by IFS_GLUE keeps exactly one glue — a doubled glue would
/// spend the first on the second and release the character itself
/// (the rubash#868 double-mark failure). Used for text GNU adds
/// through add_quoted_string (subst.c:11862): tilde prefixes and
/// quoted expansion results.
pub(crate) fn protect_ifs_chars(text: &str, ifs: &str) -> String {
    if !text.chars().any(|ch| ifs.contains(ch)) {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len() + 4);
    let mut prev_glued = false;
    for ch in text.chars() {
        if ch == IFS_GLUE {
            prev_glued = true;
            output.push(ch);
            continue;
        }
        if ifs.contains(ch) && !prev_glued {
            output.push(IFS_GLUE);
        }
        prev_glued = false;
        output.push(ch);
    }
    output
}

/// [`protect_ifs_chars`] over the tail span of an assembled word
/// (`output[start..]`): one expansion arm's result span, protected
/// after the arm spliced it.
pub(crate) fn protect_ifs_chars_in_span(output: &mut String, start: usize, ifs: &str) {
    if start >= output.len() {
        return;
    }
    let span = output.split_off(start);
    let protected = protect_ifs_chars(&span, ifs);
    output.push_str(&protected);
}

/// Remove CTLESC (\x11) sentinels the lexer inserts before quoted glob
/// metacharacters: the \x11 is a marker, the following character is the
/// data it protects. This is the storage-side half of GNU dequote_string
/// (subst.c:4807), which strips the CTLESC pairs after pathname expansion
/// decided not to consume the word — a stored element like `"p"/"*z"`
/// keeps the quoted `*` as data, never the sentinel.
pub(crate) fn dequote_ctlesc_pairs(value: &str) -> String {
    dequote_ctlesc_pairs_cow(value).into_owned()
}

/// Borrowing form of [`dequote_ctlesc_pairs`]: a value with no CTLESC byte
/// is the identity, so the marker-restore chains borrow instead of copying.
pub(crate) fn dequote_ctlesc_pairs_cow(value: &str) -> std::borrow::Cow<'_, str> {
    if !value.contains(CTLESC) {
        return std::borrow::Cow::Borrowed(value);
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
    std::borrow::Cow::Owned(output)
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
/// Last codepoint of the registry zone. This covers ANSI_C_IFS_GUARD
/// (U+E401): a user typing `$'\uE401'` must get the E400 literal escape,
/// otherwise the char is indistinguishable from the internal guard and is
/// silently stripped (rubash#735). The internal guard producer
/// (lexer/quotes.rs) pushes ANSI_C_IFS_GUARD_STR directly, never via
/// push_literal_char, so legitimate guards stay bare.
pub(crate) const MARKER_ZONE_LAST: u32 = 0xE401;
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

/// Inverse of [`push_literal_char`] for SOURCE-DISPLAY boundaries only
/// (ast_print / `--pretty-print` / `declare -f` — rubash#856): script-entry
/// escaping (substitution_metadata::bytes_to_script_text) leaves `E400` +
/// zone-char pairs inside raw word text, and printing that spelling would
/// leak the transport escape. `E400` + marker-zone char renders as the
/// char alone; a bare `E400` or `E400` + non-zone char passes through
/// (push_literal_char only ever prefixes zone chars). This is NOT a
/// transport decoder — everything else (C0 carriers, raw-byte pairs,
/// named markers) stays encoded; only the inert literal-char escape is
/// folded so the printed text shows the user's original source chars.
pub(crate) fn fold_literal_char_escapes(text: &str) -> String {
    if !text.contains(LITERAL_CHAR_ESCAPE) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE && chars.peek().is_some_and(|next| is_marker_zone_char(*next))
        {
            out.push(chars.next().expect("peeked char exists"));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Presence gate for the decode boundaries served by
/// substitution_metadata::decode_raw_byte_markers: true when `text`
/// carries EITHER transport family that decoder folds — a raw-byte
/// marker pair (RAW_BYTE_MARKER_ESCAPE introducer) or an E400
/// literal-char pair. The check itself transforms nothing; a hit routes
/// into the pair-aware decoder. Gating a boundary on the raw-byte
/// family alone leaves E400 pairs unfolded whenever no unrelated
/// raw-byte pair is present in the same text (rubash#3038).
pub(crate) fn contains_decodable_transport(text: &str) -> bool {
    text.contains(char::from_u32(RAW_BYTE_MARKER_ESCAPE).expect("raw-byte escape is a valid char"))
        || text.contains(LITERAL_CHAR_ESCAPE)
}

/// E400-aware ANSI_C_IFS_GUARD handling (rubash#735).
///
/// The guard (U+E401) marks lexer-decoded `$'...'` blanks. User data U+E401
/// arrives E400-escaped (E400 E401) via push_literal_char. A naive
/// `.replace(E401, "")` would eat the escaped char and leave a stray E400,
/// so every guard consumer must walk the text with E400 escaping the next
/// char: bare E401 is the guard, E400+E401 is user data.
///
/// Replace every bare (unescaped) ANSI_C_IFS_GUARD with `replacement`,
/// copying E400-escaped pairs verbatim.
pub(crate) fn replace_ansi_c_ifs_guard(value: &str, replacement: &str) -> String {
    if !value.contains(ANSI_C_IFS_GUARD) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if ch == ANSI_C_IFS_GUARD {
            out.push_str(replacement);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Drop bare ANSI_C_IFS_GUARD markers, preserving E400-escaped user U+E401.
pub(crate) fn strip_ansi_c_ifs_guard(value: &str) -> String {
    replace_ansi_c_ifs_guard(value, "")
}

/// Borrowing form of [`strip_ansi_c_ifs_guard`] for hot paths: a value with
/// no guard byte is returned borrowed.
pub(crate) fn strip_ansi_c_ifs_guard_cow(value: &str) -> std::borrow::Cow<'_, str> {
    if !value.contains(ANSI_C_IFS_GUARD) {
        return std::borrow::Cow::Borrowed(value);
    }
    std::borrow::Cow::Owned(strip_ansi_c_ifs_guard(value))
}

/// Convert bare ANSI_C_IFS_GUARD to IFS_GLUE (field-splitter carrier),
/// preserving E400-escaped user U+E401.
pub(crate) fn ansi_c_ifs_guard_to_ifs_glue(value: &str) -> String {
    replace_ansi_c_ifs_guard(value, IFS_GLUE_STR)
}
/// `str::replace` that borrows when the needle is absent.
///
/// GNU's expansion passes restore markers IN PLACE over C strings — a value
/// carrying no marker bytes is never copied (subst.c dequote_string /
/// dequote_escapes walk the string once). Rust's `str::replace` always
/// allocates a fresh String, so the marker-restore chains on the expansion
/// hot paths (assignment RHS hoist/restore, the embedded walker tail, the
/// assignment quote-removal tail — five to eight chained replaces per word)
/// paid a full copy per link even when zero markers were present. This
/// helper scans first and returns `Cow::Borrowed` when the needle never
/// occurs; the output is byte-identical to `s.replace(from, to)` either way.
pub(crate) fn cow_replace<'a>(s: &'a str, from: &str, to: &str) -> std::borrow::Cow<'a, str> {
    if !s.contains(from) {
        return std::borrow::Cow::Borrowed(s);
    }
    std::borrow::Cow::Owned(s.replace(from, to))
}

/// E400-aware sentinel restore over registry-zone markers: `E400 + c`
/// copies BOTH chars verbatim (the literal-char escape — user data riding
/// transport), while a BARE `sentinel` is protocol and maps to its
/// replacement. One left-to-right scan replaces a `.replace()` chain; the
/// replacement text is appended, never re-scanned.
///
/// The blind `.replace(E30D, "\\")`-style chains this supersedes read the
/// ESCAPED member of a user pair as live protocol: typed `$'\uE30D'`
/// arrives `E400 + E30D` at entry (push_literal_char, rubash#735), so an
/// unaware restore consumed the char and left a stray E400 introducer —
/// `a=($'\uE30D')` stored `E400 + \` (rubash#840 divergence 3, the
/// "assignment-tail unescape eats typed PUA" family). Only registry-zone
/// targets need this treatment: typed C0 bytes never arrive as bare C0
/// chars (they ride raw-byte marker pairs), so `.replace()` on C0
/// carriers stays correct.
pub(crate) fn restore_bare_sentinels(value: &str, rules: &[(char, &str)]) -> String {
    if rules.iter().all(|(sentinel, _)| !value.contains(*sentinel)) {
        return value.to_string();
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            output.push(ch);
            if let Some(next) = chars.next() {
                output.push(next);
            }
            continue;
        }
        match rules.iter().find(|(sentinel, _)| *sentinel == ch) {
            Some((_, replacement)) => output.push_str(replacement),
            None => output.push(ch),
        }
    }
    output
}

/// Decode-side sibling of [`restore_bare_sentinels`] for boundaries whose
/// result is USER-VISIBLE text rather than transport (the array storage
/// mirror's element render, eval's re-parse source): `E400 + c` emits
/// `c` itself — the literal-char escape resolves to the restored user
/// char — while a BARE `sentinel` maps to its replacement. Where the
/// result stays transport text for a downstream decode, use
/// `restore_bare_sentinels` instead (its pair-preserving contract keeps
/// user chars escaped in flight).
pub(crate) fn decode_bare_sentinels(value: &str, rules: &[(char, &str)]) -> String {
    if !value.contains(LITERAL_CHAR_ESCAPE)
        && rules.iter().all(|(sentinel, _)| !value.contains(*sentinel))
    {
        return value.to_string();
    }
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            output.push(chars.next().unwrap_or(ch));
            continue;
        }
        match rules.iter().find(|(sentinel, _)| *sentinel == ch) {
            Some((_, replacement)) => output.push_str(replacement),
            None => output.push(ch),
        }
    }
    output
}

/// E400-aware `str::trim_end_matches`: strips the trailing run of BARE
/// `sentinel`s only. `trim_end_matches` (and `trim_matches`) reads the
/// escaped member of a trailing `E400 + sentinel` pair as a bare sentinel
/// and strips it, leaving a dangling E400 introducer where GNU keeps the
/// user char (rubash#840). A leading pair can never collide (the head
/// char is E400, not the sentinel), so `trim_start_matches` stays safe.
pub(crate) fn trim_bare_sentinel_suffix<'a>(value: &'a str, sentinel: char) -> &'a str {
    if !value.ends_with(sentinel) {
        return value;
    }
    // Byte offset where the trailing run of BARE sentinels begins. An
    // E400 pair is user data and breaks the run; a lone trailing E400 is
    // malformed protocol and also breaks it (nothing to trim anyway).
    let mut run_start = value.len();
    let mut chars = value.char_indices();
    while let Some((offset, ch)) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            run_start = value.len();
            chars.next();
            continue;
        }
        if ch == sentinel {
            if run_start == value.len() {
                run_start = offset;
            }
        } else {
            run_start = value.len();
        }
    }
    &value[..run_start]
}

/// Whether `value` ends with a BARE `sentinel` (live protocol at the
/// edge). `str::ends_with` reads the escaped member of a trailing
/// `E400 + sentinel` pair as a hit (rubash#840).
pub(crate) fn ends_with_bare_sentinel(value: &str, sentinel: char) -> bool {
    value.ends_with(sentinel) && trim_bare_sentinel_suffix(value, sentinel).len() < value.len()
}

/// E400-aware sentinel presence test: true only for a BARE `sentinel`
/// (live protocol). The escaped member of an `E400 + sentinel` pair is
/// user data riding transport and does not count — the check answers "is
/// the protocol already present", the question admission gates such as
/// `command_substitution_value_needs_payload_protection` actually ask.
/// `value.contains(sentinel)` reads the pair's second char as a hit.
pub(crate) fn contains_bare_sentinel(value: &str, sentinel: char) -> bool {
    if !value.contains(sentinel) {
        return false;
    }
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == LITERAL_CHAR_ESCAPE {
            chars.next();
            continue;
        }
        if ch == sentinel {
            return true;
        }
    }
    false
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
        let pairs: [(char, &str); 37] = [
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
            (ANSI_C_IFS_GUARD, ANSI_C_IFS_GUARD_STR),
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

    // =====================================================================
    // rubash#728 — the registry is the single declaration point, and the
    // table is machine-read rather than prose (746 case 728.1).
    // =====================================================================

    /// Carrier-shaped const literals declared in this file. Every one must
    /// be either a MARKERS row, a `_STR` companion of a row, or an explicit
    /// range bound (not itself a marker). A new carrier const that misses
    /// the table fails here — the audited #728 gap (12 consts outside the
    /// table) cannot regrow silently.
    #[test]
    fn registry_covers_every_carrier_const() {
        let source = include_str!("markers.rs");
        let row_names: Vec<&str> = MARKERS.iter().map(|m| m.name).collect();
        // Range bounds are codepoint arithmetic, not markers.
        const NON_MARKER_BOUNDS: [&str; 5] = [
            "RAW_BYTE_MARKER_FIRST",
            "RAW_BYTE_MARKER_LAST",
            "BYTE_CHAR_BASE",
            "MARKER_ZONE_FIRST",
            "MARKER_ZONE_LAST",
        ];
        let mut declared: Vec<(String, String)> = Vec::new();
        for line in source.lines() {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix("pub(crate) const ") else {
                continue;
            };
            let Some((name, tail)) = rest.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let is_char = tail.trim_start().starts_with("char");
            let Some(eq) = tail.find('=') else { continue };
            let literal = tail[eq + 1..].trim().trim_end_matches(';').trim();
            let carrier = if is_char {
                decode_rust_char_literal(literal)
                    .is_some_and(|c| (c as u32) <= 0x1f || (0xE000..=0xEFFF).contains(&(c as u32)))
            } else if tail.trim_start().starts_with("&str") {
                literal.starts_with('"')
                    && (literal.contains("__RUBASH")
                        || decode_rust_str_first_codepoint(literal).is_some_and(|c| {
                            (c as u32) <= 0x1f || (0xE000..=0xEFFF).contains(&(c as u32))
                        }))
            } else {
                false
            };
            if carrier {
                declared.push((name.to_string(), literal.to_string()));
            }
        }
        assert!(
            declared.len() >= 60,
            "scanner regressed: expected the full carrier const set, saw {}",
            declared.len()
        );
        let mut failures = Vec::new();
        for (name, literal) in &declared {
            // A companion const (the `&str` twin of a `char` marker or,
            // since #1634, the `char` twin of an `&str` marker) is
            // covered by the canonical const's row.
            let companion_base = name
                .strip_suffix("_STR")
                .or_else(|| name.strip_suffix("_CHAR"));
            let registered = row_names.contains(&name.as_str())
                || companion_base.is_some_and(|base| row_names.contains(&base));
            let bound = NON_MARKER_BOUNDS.contains(&name.as_str());
            if !registered && !bound {
                failures.push(format!("{name} = {literal} is not a MARKERS row"));
            }
        }
        assert!(
            failures.is_empty(),
            "carrier consts outside the registry (rubash#728):\n{}",
            failures.join("\n")
        );
    }

    /// rubash#728 §3 — `contains_word_marker_bytes` documents a scope, so
    /// the "false negative is impossible" claim is now falsifiable: every
    /// char-marker below U+0100 must either live in the predicate's C0
    /// range or belong to the explicitly documented out-of-scope domains.
    /// A new marker below U+0100 that fits neither fails here.
    #[test]
    fn word_marker_byte_predicate_scope_is_enforced() {
        const OUT_OF_SCOPE_DOMAINS: [&str; 8] = [
            // readline prompt-ignore layer
            "PROMPT_IGNORE_START",
            "PROMPT_IGNORE_END",
            // compound-assignment / heredoc / array-element transport
            "DEFERRED_COMPOUND_BODY",
            "PREEXPANDED_STDIN_BODY",
            "ARRAY_FIELD_SPLIT_MARKER",
            // parser reader-state sentinels (NUL prefix)
            "CASE_PATTERN_EXPECT",
            "CASE_PATTERN_MAY_OPEN",
            "CASE_PATTERN_PAREN",
        ];
        for m in MARKERS {
            // Only C0 codepoints are in the predicate's byte domain; a
            // named-string row's first char ('_' etc.) is ordinary ASCII.
            if m.code >= 0x20 {
                continue;
            }
            let in_range = (0x11..=0x1f).contains(&m.code);
            let out_of_scope = OUT_OF_SCOPE_DOMAINS.contains(&m.name);
            assert!(
                in_range || out_of_scope,
                "{} at U+{:04X} is a word-text byte outside the \
                 contains_word_marker_bytes scope and not a documented domain exception",
                m.name,
                m.code
            );
        }
    }

    /// Decode a Rust char literal like `'\u{E401}'` / `'\x15'` / `'a'`.
    fn decode_rust_char_literal(literal: &str) -> Option<char> {
        let inner = literal.strip_prefix('\'')?.strip_suffix('\'')?;
        if let Some(hex) = inner.strip_prefix("\\u{") {
            let hex = hex.strip_suffix('}')?;
            return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
        }
        if let Some(hex) = inner.strip_prefix("\\x") {
            return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
        }
        inner.chars().next()
    }

    /// First codepoint of a Rust string literal (escapes understood).
    fn decode_rust_str_first_codepoint(literal: &str) -> Option<char> {
        let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
        if let Some(hex) = inner.strip_prefix("\\u{") {
            let hex = hex.split('}').next()?;
            return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
        }
        if let Some(hex) = inner.strip_prefix("\\x") {
            let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            return u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32);
        }
        inner.chars().next()
    }

    /// rubash#2885 E8 — every MARKERS row either claims at least one
    /// decode boundary or is explicitly listed here as internal-only,
    /// with the reason. An unlisted empty row means a carrier exists
    /// whose boundary story nobody has written down — the #2879
    /// sweep's unclaimed-row gap. A listed row that gains a claim
    /// must be removed (the staleness assert below enforces it).
    #[test]
    fn every_row_claims_boundaries_or_is_listed_internal() {
        const INTERNAL_ONLY_ROWS: [&str; 5] = [
            // Parse-time site marker for ${...} quoting decisions;
            // consumed during parse, never enters stored/output text.
            "QUOTED_EXPANSION_SITE",
            // Prefix of the executor's group-redirect state string;
            // an internal state encoding read back by the same
            // subsystem, not a value/output carrier.
            "GROUP_REDIRECT_INJECTED_MARK",
            // parser/support.rs boundary-stack sentinels (NUL-prefixed
            // strings); they live on the parser's own stack and never
            // enter stored or output text.
            "CASE_PATTERN_EXPECT",
            "CASE_PATTERN_MAY_OPEN",
            "CASE_PATTERN_PAREN",
        ];
        for m in MARKERS {
            if m.boundaries.is_empty() {
                assert!(
                    INTERNAL_ONLY_ROWS.contains(&m.name),
                    "{} claims no boundary and is not listed internal-only (rubash#2885 E8)",
                    m.name
                );
            }
        }
        for name in INTERNAL_ONLY_ROWS {
            let row = MARKERS
                .iter()
                .find(|m| m.name == name)
                .unwrap_or_else(|| panic!("{name} listed internal-only but not in MARKERS"));
            assert!(
                row.boundaries.is_empty(),
                "{name} now claims {:?} — remove it from INTERNAL_ONLY_ROWS",
                row.boundaries
            );
        }
    }

    /// rubash#2885 E8 — the ENV NAMESPACE CENSUS block above the
    /// MARKERS table enumerates every `__RUBASH_*` env-serialization
    /// key in the tree. This test scans src/ for such tokens: each
    /// must be an EXACT census entry or covered by a FAMILY prefix,
    /// each EXACT entry must still occur, and each FAMILY prefix must
    /// cover at least one occurring token.
    #[test]
    fn env_namespace_census_is_complete() {
        use std::collections::BTreeSet;
        let own = include_str!("markers.rs");
        let block_start = own
            .find("// ENV NAMESPACE CENSUS (rubash#2885 E8)")
            .expect("census block present");
        let block_end = own[block_start..]
            .find("// END ENV NAMESPACE CENSUS")
            .map(|i| block_start + i)
            .expect("census block terminated");
        let block = &own[block_start..block_end];
        let mut exact: BTreeSet<String> = BTreeSet::new();
        let mut families: Vec<String> = Vec::new();
        for line in block.lines() {
            let line = line.trim_start_matches('/').trim();
            if let Some(rest) = line.strip_prefix("EXACT ") {
                let token = rest.split_whitespace().next().unwrap();
                assert!(
                    token.starts_with("__RUBASH_"),
                    "census line is not an entry: {line}"
                );
                exact.insert(token.to_string());
            } else if let Some(rest) = line.strip_prefix("FAMILY ") {
                let token = rest.split_whitespace().next().unwrap();
                assert!(
                    token.starts_with("__RUBASH_"),
                    "census line is not an entry: {line}"
                );
                families.push(token.to_string());
            }
        }
        assert!(
            exact.len() >= 100 && families.len() >= 8,
            "census parse looks empty ({} exact, {} family)",
            exact.len(),
            families.len()
        );

        fn scan_tokens(text: &str, found: &mut BTreeSet<String>) {
            let bytes = text.as_bytes();
            let mut i = 0;
            while i + 9 < bytes.len() {
                if &bytes[i..i + 9] == b"__RUBASH_" {
                    let mut j = i + 9;
                    let mut has_alnum = false;
                    while j < bytes.len()
                        && (bytes[j].is_ascii_uppercase()
                            || bytes[j].is_ascii_digit()
                            || bytes[j] == b'_')
                    {
                        has_alnum |= bytes[j] != b'_';
                        j += 1;
                    }
                    if has_alnum {
                        found.insert(text[i..j].to_string());
                    }
                    i = j.max(i + 1);
                } else {
                    i += 1;
                }
            }
        }

        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut found: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![manifest.join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("read src tree") {
                let path = entry.expect("dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    if path.ends_with("src/executor/markers.rs") {
                        // The census block itself spells every entry;
                        // blank it so only real uses count.
                        let mut blanked = own.to_string();
                        blanked.replace_range(block_start..block_end, " ");
                        scan_tokens(&blanked, &mut found);
                    } else {
                        let text = std::fs::read_to_string(&path).expect("read source");
                        scan_tokens(&text, &mut found);
                    }
                }
            }
        }
        assert!(
            found.len() >= 100,
            "namespace scan looks empty ({} tokens)",
            found.len()
        );
        for token in &found {
            assert!(
                exact.contains(token)
                    || families.iter().any(|p| token.starts_with(p.as_str())),
                "{token} occurs in src/ but is neither an EXACT census entry nor FAMILY-covered (rubash#2885 E8)"
            );
        }
        for name in &exact {
            assert!(
                found.contains(name),
                "census EXACT entry {name} no longer occurs in src/ — remove it or restore the use"
            );
        }
        for prefix in &families {
            assert!(
                found.iter().any(|t| t.starts_with(prefix.as_str())),
                "census FAMILY prefix {prefix} covers no occurring token — remove it"
            );
        }
    }
}
