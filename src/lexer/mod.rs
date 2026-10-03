//! Lexer Module - Bash Tokenizer
//!
//! Transforms raw input strings into tokens for the parser.

mod alias_stream;
pub(crate) mod ansi;
mod brace_scan;
mod brace_scan_cache;
mod classification;
mod continuation;
pub(crate) mod dolbrace;
mod heredoc;
mod heredoc_scan;
mod number_redirect;
pub(crate) mod quotes;
mod scanner;
mod skip;
mod token;
mod word;

#[cfg(test)]
mod tests;

use brace_scan::{opens_function_body_after_previous_signature, tokens_open_unclosed_brace_group};
use continuation::{compound_residuals_advance, ends_with_unquoted_backslash, has_unclosed_quotes};
pub(crate) use continuation::{
    comsub_residuals_advance, quotes_residuals_advance, CompoundResidualState, ComsubResidualState,
    QuotesResidualState,
};

pub(crate) use alias_stream::{expand_aliases_in_source, AliasLookup};
use brace_scan_cache::BraceScanCache;
pub(crate) use continuation::has_unclosed_command_substitution;
pub(crate) use continuation::unclosed_command_substitution_depth;
pub(crate) use continuation::unclosed_input_close_char_posix;
// perf9 (#292B third wave): the parked text scanners of the group driver's
// candidate-line completeness battery (script_driver.rs GroupTextScans).
pub(crate) use continuation::balanced_residuals_advance;
pub(crate) use continuation::close_char_residuals_advance;
pub(crate) use continuation::subscript_residuals_advance;
pub(crate) use continuation::BalancedResidualState;
pub(crate) use continuation::CloseCharResidualState;
pub(crate) use continuation::SubscriptResidualState;
use heredoc::{heredoc_delimiters, HereDocDelimiter};
use scanner::{Lexer, LexerBoundaryState, LexerParseState};
pub(crate) use skip::arraysub_span_len;
pub(crate) use skip::command_substitutions_balanced;
pub(crate) use skip::extglob_pattern_group_len;
pub(crate) use skip::skip_parenthesized_unit_corrected;
pub use skip::{unclosed_comsub_eof_shape, UnclosedComsubShape};

use crate::executor::markers::DATA_DOLLAR;
pub(crate) use ansi::decode_ansi_c_quoted;
pub(crate) use quotes::remove_shell_quotes;
pub(crate) use quotes::{
    escape_decoded_ansi_c_quotes, ANSI_C_DQUOTE_MARKER, ANSI_C_DQUOTE_MARKER_STR,
    ANSI_C_QUOTE_MARKER, ANSI_C_QUOTE_MARKER_STR, PARAM_NAME_END_MARKER,
};
pub use token::{Token, TokenKind};
pub(crate) use word::{raw_has_ansi_u_escape, word_value_from_raw};

pub(crate) const QUOTED_HEREDOC_MARKER: &str = crate::executor::markers::QUOTED_HEREDOC_MARKER;

/// Set when a command carries more than `HEREDOC_MAX` (16) here-documents.
/// GNU treats that as a fatal parse error: it reports
/// `maximum here-document count exceeded` and calls `exit_shell(EX_BADUSAGE)`
/// (bash exits 2). The lexer only returns tokens, so the condition is parked
/// here for `main` to turn into the EX_BADUSAGE exit status. Holds the line
/// number to print in the diagnostic.
static HEREDOC_OVERFLOW_LINE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Record a here-document overflow on `line` (1-based). Keeps the first line
/// reported, matching GNU's already-fatal parse state.
pub(crate) fn record_heredoc_overflow(line: usize) {
    use std::sync::atomic::Ordering;
    let _ = HEREDOC_OVERFLOW_LINE.compare_exchange(0, line, Ordering::SeqCst, Ordering::SeqCst);
}

/// Returns the recorded here-document overflow line, if any, and clears it.
/// ${THIS_SH} scripts run in-process (external_finish.rs
/// execute_direct_shell_script), so a consumed flag must not leak into a
/// later tokenize in the same process.
pub fn heredoc_overflow_line() -> Option<usize> {
    use std::sync::atomic::Ordering;
    match HEREDOC_OVERFLOW_LINE.swap(0, Ordering::SeqCst) {
        0 => None,
        line => Some(line),
    }
}

/// Parse-time `extended_glob` (rubash#131).
///
/// GNU gates extglob pattern operators (`?(`, `*(`, `+(`, `@(`, `!(`) on the
/// runtime `extended_glob` variable inside `read_token_word`
/// (parse.y:5466 `if MBTEST(extended_glob && PATTERN_CHAR (character))`),
/// which `reset_parser` syncs from the `extglob` shopt flag (parse.y:3502
/// `extended_glob = extglob_flag`) and the shopt builtin updates live
/// (builtins/shopt.def). Bash 5.3 defaults to
/// `shell_compatibility_level` 53 (version.c DEFAULT_COMPAT_LEVEL =
/// `${dist_major}${dist_minor}`), so parse.y:4538's parse_comsub forcing
/// (`shell_compatibility_level <= 51`) does NOT apply: `$(` command
/// substitution bodies are gated by the shopt too (verified: GNU 5.3.0
/// rejects `r=$(case x in ?(a)) :;; esac)` with rc 2). The only exception
/// is the `[[ ... ]]` pattern/regexp right-hand side
/// (parse.y:5203-5210 parse_cond_command forces extended_glob under
/// PST_EXTPAT), which rubash's conditional parser already handles by
/// merging the RHS fragments (conditional_command.rs
/// merge_pattern_rhs_fragments).
///
/// Rubash tokenizes a whole script before anything executes, so this static
/// stands in for GNU's variable: the tokenizer's line loop flips it when it
/// sees a top-level `shopt -s/-u extglob` (mirroring GNU's parse-execute
/// cadence, the same granularity `line_posix_mode_change` uses for
/// `set -o posix`), and the shopt builtin keeps it current as commands
/// execute. Initialized off: `extglob` is off by default in GNU.
static PARSE_EXTENDED_GLOB: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the commands of the script being tokenized will actually
/// execute. Under `-n` (noexec) GNU still parses every command but never
/// runs the `shopt` builtin, so a top-level `shopt -s extglob` line must
/// NOT open the parse gate for later lines (verified: GNU 5.3.0 `bash -n`
/// on `shopt -s extglob` + `case x in ?(a)) ...` fails with rc 2).
static PARSE_EXECUTION_EXPECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Current parse-time extglob gate (see `PARSE_EXTENDED_GLOB`).
pub(crate) fn parse_extended_glob() -> bool {
    PARSE_EXTENDED_GLOB.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the parse-time extglob gate. Called by the shopt builtin when
/// `extglob` is turned on or off (GNU builtins/shopt.def updates
/// extglob_flag, and reset_parser parse.y:3502 propagates it to
/// extended_glob).
pub(crate) fn set_parse_extended_glob(enabled: bool) {
    PARSE_EXTENDED_GLOB.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Record whether the current shell invocation will execute commands
/// (`-n` means it will not; GNU shell.c reader_loop still parses).
pub fn set_parse_execution_expected(expected: bool) {
    PARSE_EXECUTION_EXPECTED.store(expected, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the line-loop shopt simulation may run (see
/// `PARSE_EXECUTION_EXPECTED`).
pub(crate) fn parse_execution_expected() -> bool {
    PARSE_EXECUTION_EXPECTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Identifies where lexer input came from; alias handling is reserved for later.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputOrigin {
    #[default]
    Direct,
    AliasReplacementDeferredHeredoc,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenizeOptions {
    pub initial_posix: bool,
    pub input_origin: InputOrigin,
    /// True when the input is a command-substitution body: heredoc body
    /// lines then end at delimiter-prefixed `)` lines (GNU make_cmd.c:602-611
    /// with PST_EOFTOKEN set), not only at exact delimiter matches.
    pub in_command_substitution: bool,
}

pub fn tokenize(input: &str) -> Vec<Token> {
    tokenize_with_options(input, TokenizeOptions::default())
}

pub fn tokenize_with_options(input: &str, options: TokenizeOptions) -> Vec<Token> {
    tokenize_with_initial_posix_and_origin(input, options.initial_posix, options.input_origin)
}

/// Tokenize with an initial POSIX parse mode. GNU Bash parses commands
/// lazily, so a runtime `set -o posix` changes the parse rules only for
/// commands read afterwards. Batch input is tokenized ahead of execution, so
/// the line loop below approximates that by flipping the parse mode when it
/// sees a top-level `set -o posix` / `set +o posix` command.
pub fn tokenize_with_initial_posix(input: &str, posix: bool) -> Vec<Token> {
    tokenize_with_initial_posix_and_origin(input, posix, InputOrigin::Direct)
}

/// Tokenize a substitution body whose text was extracted from a larger
/// script. `start_line` is the 1-based script line where the body begins, so
/// tokens (and the diagnostics they produce) carry original script line
/// numbers instead of restarting at 1 (GNU parse.y keeps in-place line
/// counters for command substitutions).
pub fn tokenize_with_initial_posix_and_line(
    input: &str,
    posix: bool,
    start_line: usize,
) -> Vec<Token> {
    tokenize_comsub_body(input, posix, start_line, false)
}

/// Tokenize a command-substitution body with the in-substitution heredoc
/// rules enabled (GNU parses these with PST_EOFTOKEN set).
pub fn tokenize_comsub_body(
    input: &str,
    posix: bool,
    start_line: usize,
    in_comsub: bool,
) -> Vec<Token> {
    tokenize_comsub_body_with_origin(input, posix, start_line, InputOrigin::Direct, in_comsub)
}

pub fn tokenize_comsub_body_with_origin(
    input: &str,
    posix: bool,
    start_line: usize,
    input_origin: InputOrigin,
    in_comsub: bool,
) -> Vec<Token> {
    if input.trim().is_empty() {
        return Vec::new();
    }

    let mut tokens = tokenize_with_heredocs(input, posix, input_origin, start_line, in_comsub);
    if tokens
        .last()
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        tokens.pop();
    }
    tokens
}

pub fn tokenize_with_initial_posix_and_origin(
    input: &str,
    posix: bool,
    input_origin: InputOrigin,
) -> Vec<Token> {
    tokenize_comsub_body_with_origin(input, posix, 1, input_origin, false)
}

fn tokenize_with_heredocs(
    input: &str,
    initial_posix: bool,
    input_origin: InputOrigin,
    start_line: usize,
    in_comsub: bool,
) -> Vec<Token> {
    let mut feeder =
        GroupScanFeeder::with_origin(initial_posix, input_origin, start_line, in_comsub);
    // The feeder's per-push TOKENIZE_DEPTH entry/exit reproduces the
    // original call-long bump: nothing observes the depth between pushes.
    let mut pieces = input.split('\n').peekable();
    while let Some(piece) = pieces.next() {
        // str::lines() drops the trailing empty string that split('\n')
        // produces when the input ends with '\n'; the parked feeder never
        // sees that phantom piece, exactly like the loop's peek() break.
        if piece.is_empty() && pieces.peek().is_none() {
            break;
        }
        feeder.push_line(piece, input.len());
    }
    feeder.finish()
}

/// One here-document whose body lines have not all arrived yet: the parked
/// remainder of the original pipeline's body-pull loop.
struct AwaitingHeredocBody {
    delimiter: HereDocDelimiter,
    body: String,
    continued_body_line: String,
    gather_line: usize,
}

/// rubash#281 companion for the append-only group readers: the ENTIRE
/// pipeline state of the batch tokenizer (`tokenize_with_heredocs`),
/// parked between physical lines so a reader that grows its input one line
/// at a time (the `.` source group driver, `read_next_source_group`)
/// ADVANCES the scan instead of restarting it. GNU reads its input once,
/// token by token (parse.y:3557 read_token); the original loop is this
/// tokenizer's substitute for that streaming model, and re-running it from
/// byte zero after every appended line is the O(group^2) amplifier behind
/// `. ./benchmarks/corpus/nvm.sh` taking minutes (300 lines consumed in
/// 30 s, 98.5% of it inside `tokenize`, measured 2026-09-28). Pausing the
/// SAME loop at line boundaries reproduces its decisions exactly: every
/// carried local (brace cache, boundary checkpoint, quote cache, comsub
/// heredoc headers, lexer parse state) is the state the loop itself keeps
/// across its own line iterations.
///
/// The feeder additionally maintains the `stdin_source_needs_more_posix`
/// token-summary (keyword stack + trailing connector) incrementally: the
/// fresh function re-tokenizes the WHOLE accumulated text per line and
/// folds; the feeder folds each pass's delta. Resumed passes extend the
/// previous token list (r#281 equivalence), full re-lexes restore the
/// snapshot taken at the open logical line's start, and the pass-skipping
/// paths (unclosed quotes / command substitution / compound assignment /
/// the #155 inert-line fast path) cannot change the summary's answer
/// while they are taken: each leaves the cheap text-level checks of
/// `stdin_source_needs_more_posix` TRUE (unclosed quotes / comsub / a
/// close-char construct in the accumulated text) or the keyword stack
/// non-empty (an unclosed `{` group armed the fast path), so the driver's
/// overall completeness answer matches the fresh scan's byte for byte.
/// rubash#292 plan B: the join gate's command-substitution residual
/// checkpoint — where the resumable scan (`comsub_residuals_advance`)
/// continues in `GroupScanFeeder::comsub_chars`, and the scan state at
/// that offset. `resume` only ever advances past a prefix whose forward
/// decisions (unit skips, heredoc terminators, `esac)` lookaheads) the
/// current buffer decided conclusively; an undecided position parks here
/// and is re-derived line by line, exactly reproducing what a full scan
/// of each longer buffer would decide (same soundness discipline as
/// `BraceScanCache`, rubash#241: append-only validity, hard invalidation
/// at every non-append mutation of the logical line).
#[derive(Clone)]
struct ComsubScanCheckpoint {
    resume: usize,
    snapshot: ComsubResidualState,
}

impl ComsubScanCheckpoint {
    fn initial() -> Self {
        Self {
            resume: 0,
            snapshot: ComsubResidualState::default(),
        }
    }
}

/// rubash#292 plan-B shape (perf8): the join gate's unclosed-quote scan
/// checkpoint over the same `comsub_chars` mirror the comsub residual scan
/// uses — `resume` only ever advances past a prefix whose forward decisions
/// (`${` span scans, `$(`/backtick unit skips, `esac)` lookaheads) the
/// current buffer decided conclusively; an undecided position parks here
/// and is re-derived line by line, exactly reproducing what a full scan of
/// each longer buffer would decide. Replaces the old
/// `unclosed_quotes_cache` boolean + `line_is_quote_inert` admission
/// (answer-level caching) with the exact per-prefix state carry.
#[derive(Clone)]
struct QuoteScanCheckpoint {
    resume: usize,
    snapshot: QuotesResidualState,
}

impl QuoteScanCheckpoint {
    fn initial() -> Self {
        Self {
            resume: 0,
            snapshot: QuotesResidualState::default(),
        }
    }
}

/// rubash#292 plan-B shape (perf8): the join gate's compound-assignment
/// scan checkpoint over the same mirror (parks + `forced_closed` early
/// terminals; see `CompoundResidualState`).
#[derive(Clone)]
struct CompoundScanCheckpoint {
    resume: usize,
    snapshot: CompoundResidualState,
}

impl CompoundScanCheckpoint {
    fn initial() -> Self {
        Self {
            resume: 0,
            snapshot: CompoundResidualState::default(),
        }
    }
}

/// perf15: the join gate's parameter-expansion scan checkpoint over the
/// same `comsub_chars` mirror — `resume` only ever advances past a prefix
/// whose forward decisions (quote/comment state transitions, `${` body
/// skips) the current buffer decided conclusively; the single undecided
/// position (a `${` whose body scan failed) parks and is re-derived line
/// by line, exactly reproducing what a full scan of each longer buffer
/// would decide (same soundness discipline as the quotes/compound
/// checkpoints above).
#[derive(Clone)]
struct ParamScanCheckpoint {
    resume: usize,
    snapshot: brace_scan::ParamScanState,
}

impl ParamScanCheckpoint {
    fn initial() -> Self {
        Self {
            resume: 0,
            snapshot: brace_scan::ParamScanState::default(),
        }
    }
}

pub(crate) struct GroupScanFeeder {
    initial_posix: bool,
    input_origin: InputOrigin,
    in_comsub: bool,
    output: Vec<Token>,
    position: usize,
    line_number: usize,
    logical_start_line: usize,
    logical_line: String,
    continued_line: bool,
    /// Byte offsets in `logical_line` where a `\`-newline continuation
    /// joined two physical lines without a newline separator (the backslash
    /// popped and the next line appended directly). Token positions must
    /// map back to PHYSICAL lines — GNU parse.y read_token_word elides the
    /// `\`+newline but still increments line_number (rubash#411), so a
    /// command starting after the join reports the later physical line,
    /// not the logical line's start.
    continuation_join_columns: Vec<usize>,
    parse_posix: bool,
    extglob_flips_allowed: bool,
    comsub_heredocs: Vec<ComsubHeredocHeader>,
    header_scan_from: usize,
    /// Char mirror of `logical_line` (rubash#292 plan B): the join gate's
    /// command-substitution residual scan runs over this persistent
    /// materialization instead of re-collecting `logical_line.chars()` per
    /// physical line. Extended in lockstep with the append below; rebuilt
    /// from `logical_line` at every non-append mutation (see
    /// `rebuild_comsub_mirror`).
    comsub_chars: Vec<char>,
    /// The residual scan's checkpoint over `comsub_chars` (rubash#292 plan
    /// B): `None` means the next gate call full-scans from char 0. GNU
    /// parse.y:3557 read_token streams token by token and never re-scans
    /// consumed text; this checkpoint is the residual scan's stand-in for
    /// that model — the scan advances over each newly appended line, and
    /// only the still-open construct (a `$(` whose atomic skip has not
    /// decided on the text read so far, or a pending heredoc) is re-derived
    /// per line via the park the advance returns.
    comsub_checkpoint: Option<ComsubScanCheckpoint>,
    /// The unclosed-quote scan's checkpoint over `comsub_chars` (perf8,
    /// rubash#292 plan-B shape): same streaming model for
    /// `quotes_residuals_advance` — per line the scan advances over the
    /// appended tail, and only a still-undecided `${`/`$(`/backtick skip is
    /// re-derived via its park. `None` means full-scan from char 0 (the
    /// mirror was rebuilt after a non-append mutation). This replaces the
    /// pre-perf8 `unclosed_quotes_cache: Option<bool>` +
    /// `line_is_quote_inert` answer cache.
    quotes_checkpoint: Option<QuoteScanCheckpoint>,
    /// The compound-assignment scan's checkpoint over `comsub_chars`
    /// (perf8, rubash#292 plan-B shape): same model for
    /// `compound_residuals_advance`. Before perf8 this gate had no cache at
    /// all — every physical line re-collected and re-scanned the whole
    /// accumulated logical line (nvm.sh: 5749 calls / 4.2 M chars for
    /// `-n`, 15 984 calls / 12.5 M chars for load).
    compound_checkpoint: Option<CompoundScanCheckpoint>,
    lexer_parse_state: LexerParseState,
    brace_cache: BraceScanCache,
    brace_join_active: bool,
    /// rubash#281/perf4 re-land: cached FALSE answer of
    /// `has_unclosed_parameter_expansion` over `logical_line`. The scan's
    /// only `true` exit is a `${` whose body scan failed, so a cached false
    /// plus a `$`-free appended line keeps it false (every arm that can
    /// newly open a `${` requires a literal `$`). Same invalidation set the
    /// residual checkpoints use (perf8): every non-append mutation of
    /// `logical_line` drops it.
    param_open_cache: Option<bool>,
    /// The parameter-expansion scan's checkpoint over `comsub_chars`
    /// (perf15, rubash#292 plan-B shape): `param_residuals_advance` follows
    /// the same streaming model as the quotes/compound checkpoints — per
    /// line the scan advances over the appended tail from its snapshot, and
    /// only a still-undecided `${` body (the park) is re-derived. Before
    /// perf15 the gate re-collected the WHOLE accumulated logical line into
    /// a fresh `Vec<char>` and re-scanned it on every `$`-bearing appended
    /// line (nvm.sh `-n`: the one giant logical line re-scanned ~419 MB).
    param_checkpoint: Option<ParamScanCheckpoint>,
    boundary: Option<(usize, Vec<Token>, LexerBoundaryState)>,
    awaiting_bodies: Vec<AwaitingHeredocBody>,
    /// keyword-stack summary of `stdin_source_needs_more_posix` over the
    /// tokens emitted so far (committed lines + the open line's last pass).
    keyword_stack: Vec<&'static str>,
    last_significant: Option<TokenKind>,
    /// Tokens of the current open logical line already folded, and the
    /// (stack, last) state at that line's start for full-re-lex refolds.
    open_folded: usize,
    open_snapshot: (Vec<&'static str>, Option<TokenKind>),
    /// Set when the original loop would `break` out of line processing
    /// (heredoc overflow): later pushes are inert, like the loop exit.
    overflowed: bool,
    /// Set when this feeder's per-line shopt simulation EFFECTIVELY toggled
    /// the process-global `PARSE_EXTENDED_GLOB` gate (an Enable/Disable flip
    /// that actually changed the value). Consumers that want to reuse the
    /// feeder's tokens in place of a fresh whole-text re-lex must refuse when
    /// this is set: the re-lex replays the flips starting from the group-END
    /// global, so its pre-flip lines see a different extglob gate than the
    /// streaming feed did (parse.y:5466 gates pattern chars per token at
    /// read time, and tokens carry the gate they were lexed under —
    /// `Token::extglob_gate`). A fallback to the fresh re-lex preserves
    /// today's behavior byte for byte on flipping groups.
    extglob_toggled: bool,
    /// perf21 feeder/gather battery fusion: this feeder instance is one
    /// gather group's whole life, so "the group text is append-only and
    /// identical to the caller's pending mirror" is one sticky flag. Any
    /// non-append mutation of `logical_line` (backslash-continuation pop,
    /// IFS_GLUE insert, comsub-heredoc rotation) or any heredoc-body line
    /// (consumed out of the mirror) drops it, and the gather's
    /// `GroupTextScans` battery must then keep advancing its own
    /// quotes/comsub machines over the pending mirror.
    group_clean: bool,
    /// perf21: the join gates' answers from the most recent evaluation —
    /// `advance_comsub_scan`'s open answer (including the IFS_GLUE re-scan
    /// arm) and whether that advance parked, and `advance_quotes_scan`'s
    /// open answer. The rubash#155 inert-line fast path returns before the
    /// gates run; its admission proves the line cannot flip either gate,
    /// so the stale values stay true there.
    gate_comsub_open: bool,
    gate_comsub_parked: bool,
    gate_quotes_open: bool,
    gate_quotes_parked: bool,
}

impl GroupScanFeeder {
    pub(crate) fn new(initial_posix: bool) -> Self {
        Self::with_origin(initial_posix, InputOrigin::Direct, 1, false)
    }

    fn with_origin(
        initial_posix: bool,
        input_origin: InputOrigin,
        start_line: usize,
        in_comsub: bool,
    ) -> Self {
        let open_snapshot = (Vec::new(), None);
        Self {
            initial_posix,
            input_origin,
            in_comsub,
            output: Vec::new(),
            position: 0,
            line_number: start_line,
            logical_start_line: start_line,
            logical_line: String::new(),
            continued_line: false,
            continuation_join_columns: Vec::new(),
            parse_posix: initial_posix,
            extglob_flips_allowed: true,
            comsub_heredocs: Vec::new(),
            header_scan_from: 0,
            comsub_chars: Vec::new(),
            comsub_checkpoint: Some(ComsubScanCheckpoint::initial()),
            quotes_checkpoint: Some(QuoteScanCheckpoint::initial()),
            compound_checkpoint: Some(CompoundScanCheckpoint::initial()),
            lexer_parse_state: LexerParseState::default(),
            brace_cache: BraceScanCache::default(),
            brace_join_active: false,
            param_open_cache: None,
            param_checkpoint: Some(ParamScanCheckpoint::initial()),
            boundary: None,
            awaiting_bodies: Vec::new(),
            keyword_stack: Vec::new(),
            last_significant: None,
            open_folded: 0,
            open_snapshot,
            overflowed: false,
            extglob_toggled: false,
            group_clean: true,
            gate_comsub_open: false,
            gate_comsub_parked: false,
            gate_quotes_open: false,
            gate_quotes_parked: false,
        }
    }

    /// The tokenize-derived half of `stdin_source_needs_more_posix` for the
    /// accumulated text: keyword stack non-empty, or the last significant
    /// token is a `&&`/`||`/`|`/`|&` connector. While heredoc bodies are
    /// still being awaited, a fresh scan at EOF pushes the unterminated-body
    /// marker token before its separator, so its last significant token is
    /// the HereDocBody — never a connector — exactly like this formula.
    pub(crate) fn token_level_needs_more(&self) -> bool {
        if !self.keyword_stack.is_empty() {
            return true;
        }
        self.awaiting_bodies.is_empty()
            && matches!(
                self.last_significant,
                Some(TokenKind::And)
                    | Some(TokenKind::Or)
                    | Some(TokenKind::Pipe)
                    | Some(TokenKind::PipeErr)
            )
    }

    /// Whether this feeder's line loop EFFECTIVELY toggled the process-global
    /// extglob parse gate (see the field docs). Token-reuse consumers must
    /// fall back to a fresh whole-text re-lex when this is `true`.
    pub(crate) fn extglob_toggled(&self) -> bool {
        self.extglob_toggled
    }

    /// perf21 battery-fusion gate: the feeder's join gates (quotes +
    /// command substitution, over the append-only `comsub_chars` mirror of
    /// this group's text) prove BOTH of the gather battery's duplicate
    /// quote/substitution questions answered "closed" for the line just
    /// pushed, with no undecided unit parked. When the group text is also
    /// still the pending mirror's append-only twin (`group_clean`), the
    /// battery may skip advancing its own quotes/comsub/balanced machines
    /// over the appended tail: the mirror differs from pending only by the
    /// trailing line terminator, and the '\n' arms of both machines are
    /// inert from a closed, unparked state (they only reset
    /// `comment_start`), so the machines' answers and line-boundary states
    /// over pending equal the feeder's — `QuotesResidualState::default()`
    /// at every line end. GNU anchor: parse.y:3557 read_token streams the
    /// input once; the two parallel batteries are this port's substitute,
    /// and this gate removes their duplication on the provably-identical
    /// prefix.
    pub(crate) fn gates_prove_quotes_comsub_closed(&self) -> bool {
        self.group_clean
            && !self.gate_quotes_open
            && !self.gate_quotes_parked
            && !self.gate_comsub_open
            && !self.gate_comsub_parked
    }

    /// Fold one token into the keyword-stack / last-significant summary,
    /// exactly the loop `stdin_source_needs_more_posix` runs over the fresh
    /// token list.
    fn fold_token(&mut self, token: &Token) {
        if token.kind == TokenKind::Keyword {
            match token.value.as_str() {
                "case" => self.keyword_stack.push("esac"),
                "if" => self.keyword_stack.push("fi"),
                "for" | "select" | "while" | "until" => self.keyword_stack.push("done"),
                "{" => self.keyword_stack.push("}"),
                "esac" | "fi" | "done" | "}" => {
                    if self.keyword_stack.last().copied() == Some(token.value.as_str()) {
                        self.keyword_stack.pop();
                    }
                }
                _ => {}
            }
        }
        if !(token.kind == TokenKind::Semicolon && token.line_break) {
            self.last_significant = Some(token.kind.clone());
        }
    }

    fn fold_pass_tokens(&mut self, tokens: &[Token], resumed: bool) {
        if resumed {
            for token in &tokens[self.open_folded.min(tokens.len())..] {
                self.fold_token(token);
            }
        } else {
            // Restore (a COPY of) the open line's start snapshot: the same
            // snapshot must survive every future full re-lex of this still
            // open logical line — only emit_line_separator replaces it.
            let snapshot = self.open_snapshot.clone();
            self.keyword_stack = snapshot.0;
            self.last_significant = snapshot.1;
            for token in tokens {
                self.fold_token(token);
            }
        }
        self.open_folded = tokens.len();
    }

    /// Feed one physical line (its raw text without the '\n'). The caller
    /// passes the byte length of the whole accumulated input AFTER this line
    /// was appended: the original loop derives `line_had_terminator` from
    /// its running position against the input length, and the parked feeder
    /// must keep that arithmetic (a final `...\r` line without '\n' counts
    /// as terminated there).
    pub(crate) fn push_line(&mut self, raw_line: &str, total_input_len: usize) {
        let tokenize_depth = TOKENIZE_DEPTH.with(|depth| {
            let value = depth.get() + 1;
            depth.set(value);
            value
        });
        if !self.awaiting_bodies.is_empty() {
            self.feed_awaiting_body(raw_line);
            TOKENIZE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
            return;
        }
        self.push_main_line(raw_line, total_input_len, tokenize_depth);
        TOKENIZE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }

    /// The parked body-pull loop: one pushed line serves the front awaiting
    /// heredoc (the original pulls body lines one at a time from the same
    /// input stream, in delimiter order).
    fn feed_awaiting_body(&mut self, raw_line: &str) {
        // Body lines never enter the logical-line mirror, so the gather's
        // pending text is no longer the mirror's twin (perf21 battery fusion).
        self.group_clean = false;
        let body_line = if cfg!(windows) {
            raw_line.strip_suffix('\r').unwrap_or(raw_line).to_string()
        } else {
            raw_line.to_string()
        };
        self.position += body_line.len() + 1;
        self.line_number += 1;
        let mut raw_line = body_line;
        let (found, found_with_warning) = {
            let awaiting = &mut self.awaiting_bodies[0];
            let mut comparable = if awaiting.delimiter.strip_tabs {
                raw_line.trim_start_matches('\t').to_string()
            } else {
                raw_line.clone()
            };
            if !awaiting.delimiter.quoted {
                let trailing_slashes = raw_line.chars().rev().take_while(|ch| *ch == '\\').count();
                if trailing_slashes % 2 == 1 {
                    let mut continued = raw_line;
                    continued.pop();
                    awaiting.continued_body_line.push_str(&continued);
                    return;
                }
                if !awaiting.continued_body_line.is_empty() {
                    awaiting.continued_body_line.push_str(&raw_line);
                    comparable = std::mem::take(&mut awaiting.continued_body_line);
                }
            }
            let value = awaiting.delimiter.value.clone();
            let allow_closing_paren = awaiting.delimiter.allow_closing_paren;
            let strip_suffix_match = comparable
                .strip_suffix(')')
                .is_some_and(|value| *value == awaiting.delimiter.value);
            let mut found = false;
            let mut found_with_warning = false;
            if comparable == value {
                found = true;
            } else if allow_closing_paren
                && comparable.starts_with(value.as_str())
                && comparable[value.len()..].contains(')')
            {
                found = true;
                found_with_warning = true;
            } else if allow_closing_paren && strip_suffix_match {
                found = true;
                found_with_warning = true;
            } else if self.in_comsub
                && comparable.starts_with(value.as_str())
                && comparable[value.len()..].trim().is_empty()
            {
                found = true;
            }
            if !found {
                awaiting.body.push_str(&comparable);
                awaiting.body.push('\n');
            }
            (found, found_with_warning)
        };
        if found {
            let mut awaiting = self.awaiting_bodies.remove(0);
            if found_with_warning {
                awaiting
                    .body
                    .insert(0, crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX);
            }
            if awaiting.delimiter.quoted {
                awaiting.body.insert_str(0, QUOTED_HEREDOC_MARKER);
            }
            let mut token =
                Token::new(TokenKind::HereDocBody, &awaiting.body, awaiting.gather_line);
            // rubash#305: feed_awaiting_body already advanced line_number
            // past the closing-delimiter line (one increment per physical
            // body line, mirroring GNU make_cmd.c:580). rubash's counter
            // tracks "one past the last consumed line" (same convention as
            // gather_line above), so the physical closing line — the line
            // GNU's line_number names for a syntax error on the NEWLINE
            // after gathering (parse.y:3651) — is line_number - 1.
            token.heredoc_end_line = Some(self.line_number.saturating_sub(1));
            self.output.push(token.clone());
            self.fold_token(&token);
            if self.awaiting_bodies.is_empty() {
                self.emit_line_separator();
            }
        }
    }

    fn emit_line_separator(&mut self) {
        let mut separator = Token::new(TokenKind::Semicolon, ";", self.logical_start_line);
        separator.line_break = true;
        self.output.push(separator.clone());
        self.fold_token(&separator);
        // GNU read_token reads this line break as a '\n' token before the
        // next line's first token (reserved_word_acceptable, parse.y:5902);
        // the separator above is emitted downstream of the Lexer, so the
        // carried reader state must record the break itself.
        self.lexer_parse_state.note_line_break();
        // The open logical line is fully committed: the next line starts a
        // fresh one, and a later full re-lex must refold from here.
        self.open_folded = 0;
        self.open_snapshot = (self.keyword_stack.clone(), self.last_significant.clone());
    }

    /// The parked main loop: the body of the original `while let
    /// Some(raw_line) = lines.next()` iteration.
    /// rubash#292 plan B: answer the join gate's command-substitution
    /// question from the residual checkpoint instead of full-scanning the
    /// accumulated `logical_line` (and re-collecting its chars) per
    /// physical line. Restoring the snapshot at `resume` and scanning
    /// `comsub_chars[resume..]` reproduces the full scan of the current
    /// buffer bit for bit: everything before `resume` was decided
    /// conclusively on a shorter prefix (monotone forward skips), and the
    /// advance's park covers the still-undecided open construct. The OMB
    /// load paid the full rescan ~984 times per source (2.5 MB re-read);
    /// with the checkpoint each line pays only its own bytes plus the open
    /// construct's re-derivation.
    fn advance_comsub_scan(&mut self) -> bool {
        let (resume, snapshot) = match self.comsub_checkpoint.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, ComsubResidualState::default()),
        };
        let mut state = snapshot;
        let park = comsub_residuals_advance(&self.comsub_chars, resume, &mut state);
        let open = state.is_open();
        // perf21: expose the gate answer + park for the gather's battery
        // fusion (see the field docs above).
        self.gate_comsub_open = open;
        self.gate_comsub_parked = park.is_some();
        self.comsub_checkpoint = match park {
            Some(park) => Some(ComsubScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            }),
            None => Some(ComsubScanCheckpoint {
                resume: self.comsub_chars.len(),
                snapshot: state,
            }),
        };
        open
    }

    /// rubash#292 plan-B shape (perf8): answer the join gate's
    /// unclosed-quote question from the quotes checkpoint over
    /// `comsub_chars` instead of full-scanning the accumulated
    /// `logical_line` per physical line. Restoring the snapshot at `resume`
    /// and scanning `comsub_chars[resume..]` reproduces the full scan of
    /// the current buffer bit for bit (same contract as
    /// `advance_comsub_scan` above; parks cover the undecided `${`/`$(`/
    /// backtick skips). On nvm.sh `-n` the old per-line full scan walked
    /// 3.6 M chars for a 173 KB script; with the checkpoint each line pays
    /// only its own bytes plus the open construct's re-derivation.
    fn advance_quotes_scan(&mut self) -> bool {
        let (resume, snapshot) = match self.quotes_checkpoint.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, QuotesResidualState::default()),
        };
        let mut state = snapshot;
        let park = quotes_residuals_advance(&self.comsub_chars, resume, &mut state);
        let open = state.is_open();
        self.gate_quotes_open = open;
        self.gate_quotes_parked = park.is_some();
        self.quotes_checkpoint = match park {
            Some(park) => Some(QuoteScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            }),
            None => Some(QuoteScanCheckpoint {
                resume: self.comsub_chars.len(),
                snapshot: state,
            }),
        };
        open
    }

    /// rubash#292 plan-B shape (perf8): the compound-assignment gate's
    /// checkpoint driver, mirroring `advance_quotes_scan` (parks + the
    /// `forced_closed` early terminal; see `CompoundResidualState`).
    fn advance_compound_scan(&mut self) -> bool {
        let (resume, snapshot) = match self.compound_checkpoint.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, CompoundResidualState::default()),
        };
        let mut state = snapshot;
        let park = compound_residuals_advance(&self.comsub_chars, resume, &mut state);
        let open = state.is_open();
        self.compound_checkpoint = match park {
            Some(park) => Some(CompoundScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            }),
            None => Some(CompoundScanCheckpoint {
                resume: self.comsub_chars.len(),
                snapshot: state,
            }),
        };
        open
    }

    /// perf15: the parameter-expansion gate's checkpoint driver, mirroring
    /// `advance_quotes_scan`. The park is a single still-undecided `${`
    /// body; `open` (the fresh scan's `true`) is exactly "a park exists".
    fn advance_param_scan(&mut self) -> bool {
        let (resume, snapshot) = match self.param_checkpoint.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, brace_scan::ParamScanState::default()),
        };
        let mut state = snapshot;
        match brace_scan::param_residuals_advance(&self.comsub_chars, resume, &mut state) {
            Some(park) => {
                self.param_checkpoint = Some(ParamScanCheckpoint {
                    resume: park,
                    snapshot: state,
                });
                true
            }
            None => {
                self.param_checkpoint = Some(ParamScanCheckpoint {
                    resume: self.comsub_chars.len(),
                    snapshot: state,
                });
                false
            }
        }
    }

    /// Rebuild the char mirror from `logical_line` after a non-append
    /// rewrite (IFS_GLUE insert, comsub-heredoc rotation) and drop every
    /// residual checkpoint: the scans' offsets and every decided prefix
    /// refer to byte positions that no longer exist.
    fn rebuild_comsub_mirror(&mut self) {
        self.comsub_chars = self.logical_line.chars().collect();
        self.comsub_checkpoint = None;
        self.quotes_checkpoint = None;
        self.compound_checkpoint = None;
        self.param_checkpoint = None;
    }

    fn push_main_line(&mut self, raw_line: &str, total_input_len: usize, tokenize_depth: usize) {
        if self.overflowed {
            return;
        }
        // niubash #106: a '\r' immediately before the '\n' belongs to the
        // CRLF line terminator, not to the last word — a Windows-native
        // shell must accept CRLF scripts (v1.1.1 did; the shipped
        // oh-my-niu bundle is 100% CRLF). The GNU-fidelity rule this loop
        // documents below still applies to a '\r' NOT followed by '\n':
        // `set ""<CR>` with a bare carriage return keeps $1 = "\r".
        // rubash#140: the tolerance is a Windows product decision
        // (CRT text-mode compensation, niubash#120 family). GNU on unix
        // keeps the '\r' as literal word data — CRLF scripts there fail
        // with `$'getopts\r': command not found` (parse.y read_token /
        // read_secondary_line have no CR stripping) — so gate it.
        let line = if cfg!(windows) {
            raw_line.strip_suffix('\r').unwrap_or(raw_line)
        } else {
            raw_line
        };
        if self.logical_line.is_empty() {
            self.logical_start_line = self.line_number;
            self.continuation_join_columns.clear();
        }
        if !self.logical_line.is_empty() && !self.continued_line {
            self.logical_line.push('\n');
            self.comsub_chars.push('\n');
        }
        self.continued_line = false;
        self.logical_line.push_str(line);
        self.comsub_chars.extend(line.chars());
        self.position += line.len() + 1;
        let line_had_terminator = self.position <= total_input_len;
        self.line_number += 1;

        // rubash#155 / #130 fast path. GNU reads tokens sequentially
        // (parse.y:3557 read_token): one pass over the input, with the
        // reader state carried token to token. The full-buffer
        // re-tokenization below is this tokenizer's substitute for that
        // streaming model, and re-running it per appended physical line is
        // what made a single `{` group spanning N lines O(N^2) (nvm.sh
        // shape: 4000 lines = 14.3s vs GNU 9.7ms).
        //
        // While the only reason the logical line stays open is the
        // token-level brace-group signal from the previous pass (quotes,
        // command substitutions and compound assignments were all closed —
        // reaching the join `continue` below proves that), a physical line
        // without any quote, escape, expansion, brace, paren, comment,
        // heredoc or `posix` bytes provably cannot change any of the
        // decisions this iteration would recompute (full byte-set
        // justification in the landed r#155/#130 audit). Every other
        // intermediate result is discarded by the join `continue` and
        // recomputed by the final full pass, so the accepted token stream
        // is byte-identical; only the per-line work drops from
        // O(accumulated buffer) to O(this line).
        let fast_line = self.brace_join_active && brace_join_fast_path_line(line);
        if fast_line {
            self.header_scan_from = self.logical_line.len();
            return;
        }

        let comsub_open = self.advance_comsub_scan();
        // Between this scan and the join-gate re-scan further down, the only
        // mutation `self.logical_line` can undergo on a path that reaches
        // that re-scan is the IFS_GLUE insert below (the backslash-
        // continuation pop returns early). GNU parse.y:3557 read_token is a
        // streaming reader — it never re-scans already-read input — so when
        // the buffer is byte-identical the first answer stays valid (the
        // scan is pure over its input) and the re-scan only has to run
        // after the insert actually rewrote the buffer.
        let mut comsub_state_changed = false;
        if !comsub_open {
            // GNU make_cmd.c:602-611: when the `)` closing the command
            // substitution sits on the heredoc delimiter line (e.g. `EOF)`),
            // the heredoc is "delimited by end-of-file" and a warning is
            // issued. Mark the logical line with \x1c before the `)` so
            // command_substitution_heredoc_output_mut_typed can detect this
            // case. Only mark when the current line matches a tracked heredoc
            // delimiter followed by `)` — NOT when `)` is on the header line
            // (e.g. `cat << EOF)`), which is a different case handled by
            // heredoc_header_closes_command_substitution.
            if let Some(front) = self.comsub_heredocs.first().cloned() {
                let comparable = if front.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if comparable.starts_with(front.delimiter.as_str())
                    && comparable[front.delimiter.len()..].contains(')')
                {
                    // Insert \x1c before the first `)` after the delimiter
                    // in the logical line. The search is scoped to the
                    // physical line just appended: `rfind` on the whole
                    // logical line could land on the pushed-back `)` itself
                    // (delim `)`) or on a `)` from an earlier line.
                    let line_start = self.logical_line.len() - line.len();
                    let delim_end =
                        line_start + (line.len() - comparable.len()) + front.delimiter.len();
                    if let Some(rel_pos) = self.logical_line[delim_end..].find(')') {
                        self.logical_line
                            .insert(delim_end + rel_pos, crate::executor::markers::IFS_GLUE);
                        // Mid-string rewrite: positional scan caches invalid.
                        self.brace_cache.clear();
                        self.param_open_cache = None;
                        self.boundary = None;
                        self.rebuild_comsub_mirror();
                        self.group_clean = false;
                        comsub_state_changed = true;
                    }
                }
            }
            self.comsub_heredocs.clear();
        } else if let Some(front) = self.comsub_heredocs.first().cloned() {
            // The appended line is a heredoc body line inside the open
            // substitution: close the heredoc on an exact delimiter line or
            // on a delimiter-prefixed `)` line (GNU make_cmd.c:602-611).
            let comparable = if front.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if comparable == front.delimiter
                || (comparable.starts_with(front.delimiter.as_str())
                    && comparable[front.delimiter.len()..].contains(')'))
            {
                self.comsub_heredocs.remove(0);
            }
        }
        let in_comsub_heredoc_body = comsub_open && !self.comsub_heredocs.is_empty();

        if line_had_terminator
            && ends_with_unquoted_backslash(&self.logical_line)
            && !in_comsub_heredoc_body
        {
            self.logical_line.pop();
            // GNU parse.y read_token_word on `\` + newline: the pair is
            // elided from the token text but the reader still crosses the
            // physical line (line_number++ in the loop). Record where the
            // elision happened so token positions can map back to physical
            // lines at accept time (rubash#411).
            self.continuation_join_columns.push(self.logical_line.len());
            // The popped byte changes the text every later offset depends
            // on: positional scan caches invalid. The pop also joins the
            // next line WITHOUT a '\n' separator, so a two-character
            // lookahead (`$(`, `<<`, ...) can newly straddle the join —
            // every residual checkpoint must be dropped, not just advanced
            // back (rubash#292; same rule for the quotes/compound
            // checkpoints, perf8).
            self.comsub_chars.pop();
            self.comsub_checkpoint = None;
            self.quotes_checkpoint = None;
            self.compound_checkpoint = None;
            self.param_checkpoint = None;
            self.brace_cache.clear();
            self.param_open_cache = None;
            self.boundary = None;
            self.continued_line = true;
            self.brace_join_active = false;
            self.group_clean = false;
            return;
        }
        // parse.y:5379-5384: a backslash before EOF is NOT removed — GNU's
        // read_token_word ungets EOF and keeps the `\` as a quoted literal
        // (`echo a\` prints `a\`), so no EOF-pop branch exists here. Only
        // `\`+`\n` joins lines (handled above).

        // Fresh header scan once the accumulated text is stable (after the
        // join decision): a header whose delimiter is completed by the next
        // physical line (`cat <<\EOT\` + `4` = delimiter `EOT4`) is only
        // complete after the join, so the scan resumes from the last `<<`.
        if comsub_open && self.comsub_heredocs.is_empty() {
            let slice_from = self.header_scan_from.min(self.logical_line.len());
            let slice = &self.logical_line[slice_from..];
            let (mut headers, consumed) = scan_line_for_comsub_heredoc_headers(slice);
            if consumed == slice.len() || headers.is_empty() {
                self.header_scan_from = self.logical_line.len();
            } else {
                self.header_scan_from = self.logical_line.len() - slice.len() + consumed;
            }
            self.comsub_heredocs.append(&mut headers);
        } else if !comsub_open {
            // Keep pace with consumed text: earlier substitutions' headers are
            // already gathered and must not be rediscovered on the next scan.
            self.header_scan_from = self.logical_line.len();
        }

        // rubash#292 plan-B shape (perf8): the unclosed-quote gate reads the
        // quotes checkpoint over the persistent `comsub_chars` mirror —
        // per line only the appended tail is scanned, plus the re-derivation
        // of any parked `${`/`$(`/backtick skip. This replaces the old
        // `unclosed_quotes_cache` + `line_is_quote_inert` answer cache
        // (which only covered appended lines with no quote-state bytes and
        // full-rescanned every quote-bearing line).
        let hq = self.advance_quotes_scan();
        if hq {
            self.brace_join_active = false;
            return;
        }
        // Join-gate re-scan: reuse the loop-head answer unless the IFS_GLUE
        // insert rewrote the buffer this iteration (`comsub_open || ...` —
        // an open substitution never takes the insert path, and the un-rewritten
        // buffer answer is the loop-head answer by purity). See the comment
        // at the loop-head scan. The insert invalidated the residual
        // checkpoint (rubash#292), so this advance is the rewritten buffer's
        // full scan and re-establishes the checkpoint for the next line.
        if comsub_open || (comsub_state_changed && self.advance_comsub_scan()) {
            self.brace_join_active = false;
            return;
        }
        // A `name=(` compound array assignment keeps reading physical lines
        // until its matching `)` (parse.y; ISSUE #78). rubash#292 plan-B
        // shape (perf8): answered from the compound checkpoint over the
        // mirror, replacing the unconditional whole-buffer rescan per line
        // (perf7 previously gated this on a `=(` byte admission —
        // parse.y:5785-5791 adjacency — which the checkpoint model now
        // subsumes: the opener bytes are carried in the residual state).
        let compound_open = self.advance_compound_scan();
        if compound_open {
            self.brace_join_active = false;
            return;
        }
        // GNU parse.y parse_comsub (PST_EOFTOKEN) + print_comsub
        // (parse.y:4632): a `)` on a heredoc header line inside `$(...)`
        // closes the substitution while the still-pending body was gathered
        // from the following input lines, and the substitution text is
        // reprinted with the body inside the closing `)`. Rotate
        // `$(cat <<EOF)\nfoo\nEOF` into `$(cat <<EOF\nfoo\nEOF)` so every
        // downstream consumer sees the GNU reprint order (heredoc7.sub).
        if let Some(rotated) = relocate_comsub_heredoc_paren(&self.logical_line) {
            self.logical_line = rotated;
            // Rotation rewrites the middle of the line: caches invalid.
            self.brace_cache.clear();
            self.param_open_cache = None;
            self.boundary = None;
            self.group_clean = false;
            self.rebuild_comsub_mirror();
        }
        // GNU reads tokens sequentially (parse.y read_token): the reader
        // state feeding reserved_word_acceptable (parse.y:5899) is the state
        // after the tokens BEFORE this logical line — newlines are just
        // whitespace between them. The join loop below re-tokenizes the
        // whole accumulated logical line after each joined physical line,
        // so every retry must replay from the SAME line-start state; the
        // end state of a partial tokenization describes the end of the
        // partial text, not the start of the longer one. Feeding it back
        // made `{)\t: brace ;;` fold after `esac` joined (the previous
        // partial ended with last=esac, so `{` sat in reserved-word
        // position) and swallowed the rest of the function body.
        let mut line_lex_state = self.lexer_parse_state.clone();
        let boundary_taken = self.boundary.take();
        let boundary_was_some = boundary_taken.is_some();
        // rubash#281 resume gate. The blanket rule was "a `}` byte in the
        // appended line refuses the resume" — but the ONLY thing a `}` can
        // change in the already-checkpointed prefix is completing the
        // `{`-arm fold of a still-open brace group (scanner.rs `{` arm:
        // an open group is emitted as a bare `{` Keyword plus body tokens
        // and folds into ONE token in exactly the pass where its matching
        // `}` arrives). Refine the refusal to exactly that case, decided by
        // the same scan the fold itself would run — `skip_brace` from the
        // innermost open `{`, which resumes from the shared BraceScanCache
        // continuation (O(appended tail), rubash#176/#178):
        //   - no bare `{` Keyword in the checkpoint tokens: no pre-offset
        //     opener exists, so no fold can complete — resume is safe;
        //   - the innermost open group's scan says `closed`: this pass
        //     would fold it — refuse, restoring the full re-lex exactly on
        //     fold passes (conservative for heredoc-bearing groups, whose
        //     `{` arm never folds: their scan answer only costs one extra
        //     tail scan);
        //   - not closed (the `}` bytes are quoted, word-glued or inside a
        //     nested construct): braces nest, so while the innermost group
        //     stays open no outer group's depth can return to zero either —
        //     no fold can complete anywhere — resume is safe.
        // GNU anchor: parse.y:3557 read_token reads the input once and
        // never re-lexes consumed text; the blanket `}` refusal was this
        // port's stand-in for "the fold may complete" (nvm.sh -n: 194
        // refused passes re-lexed 1.8MB — 570ms of a 2.2s run — and the
        // fold completes on only a fraction of them).
        let resume_allowed = if !line.contains('}') {
            true
        } else {
            let opener_column = boundary_taken.as_ref().and_then(|(_, tokens, _)| {
                tokens
                    .iter()
                    .rposition(|token| token.kind == TokenKind::Keyword && token.value == "{")
                    .map(|index| tokens[index].column)
            });
            match opener_column {
                None => true,
                Some(column) => {
                    let mut probe = Lexer::new_with_cache(
                        &self.logical_line,
                        self.parse_posix,
                        &mut self.brace_cache,
                    );
                    probe.position = column + 1;
                    let closed = probe.skip_brace().closed;
                    !closed
                }
            }
        };
        let (mut line_tokens, pass_boundary) = tokenize_with_boundary(
            boundary_taken,
            resume_allowed,
            &self.logical_line,
            self.parse_posix,
            &mut line_lex_state,
            &mut self.brace_cache,
        );
        // Summary fold BEFORE the join branch moves `line_tokens` into the
        // next boundary checkpoint: a resumed pass extends the previous
        // list (fold only the delta), a full re-lex restores the open
        // line's start snapshot and refolds everything.
        let fold_chars = line_tokens.len() as u64;
        self.fold_pass_tokens(&line_tokens, boundary_was_some && resume_allowed);
        if let Some(updated) = line_posix_mode_change(&line_tokens) {
            if self.parse_posix != updated {
                // The full pass would re-lex the whole line under the new
                // mode; a resumed tail must not mix modes.
                self.boundary = None;
            }
            self.parse_posix = updated;
        }
        // rubash#131: a top-level `shopt -s/-u extglob` executes before the
        // next line parses in GNU's read-execute loop; mirror that for the
        // parse-time gate (same per-line granularity as the posix flip
        // above). Disabled under -n (nothing executes) and after a top-level
        // `set -n` (GNU executes `set -n` and then only parses).
        // `tokenize_depth == 1` is the outermost script tokenization.
        if parse_execution_expected() && self.extglob_flips_allowed && tokenize_depth == 1 {
            match line_extglob_mode_change(&line_tokens) {
                ExtglobFlip::Enable => {
                    if !parse_extended_glob() {
                        set_parse_extended_glob(true);
                        self.boundary = None;
                        self.extglob_toggled = true;
                    }
                }
                ExtglobFlip::Disable => {
                    if parse_extended_glob() {
                        set_parse_extended_glob(false);
                        self.boundary = None;
                        self.extglob_toggled = true;
                    }
                }
                ExtglobFlip::ExecutionOff => self.extglob_flips_allowed = false,
                ExtglobFlip::None => {}
            }
        }
        // Record the whitespace run before each token. Token::column stays a
        // byte offset into the logical line (only the position field is
        // overwritten with the line number below), so consecutive columns
        // recover the exact inter-token spacing for raw arithmetic capture.
        let gap_tokens = line_tokens.len() as u64;
        let mut previous_end = 0usize;
        for token in line_tokens.iter_mut() {
            let start = token.column.min(self.logical_line.len());
            // Some lexer paths emit tokens whose columns do not advance
            // monotonically through the logical line; skip the gap capture
            // for those instead of slicing an inverted byte range.
            if start >= previous_end {
                let gap = &self.logical_line[previous_end..start];
                if gap.chars().all(char::is_whitespace) {
                    token.leading_ws = gap.to_string();
                }
            }
            previous_end = previous_end
                .max(start.saturating_add(token.raw.len()))
                .min(self.logical_line.len());
        }
        // perf21: compute the delimiters ONCE — the join branch below only
        // needs their emptiness, and the commit path reuses the same Vec
        // (heredoc_delimiters reads only token kinds/values/raw, none of
        // which the gap-capture loop above mutates).
        let delimiters = heredoc_delimiters(&line_tokens, &self.logical_line, self.in_comsub);
        let has_heredoc = !delimiters.is_empty();
        // Join forward only on signals the tokens themselves prove: an
        // unclosed reserved-word `{` group (see tokens_open_unclosed_brace_group)
        // or an unterminated `${...}` parameter expansion. The old text-level
        // has_unclosed_brace_group counted `case x in {)`'s pattern brace as a
        // group opener, joining the pattern line to far-away text.
        let brace_group_open = tokens_open_unclosed_brace_group(&line_tokens);
        // Admission + false-answer cache (`param_open_cache`): the scan's
        // only `true` exit is a `${` whose body scan failed, so a `$`-free
        // line admits false outright and a cached false survives a `$`-free
        // appended line (rubash#281/perf4 re-land; GNU anchor: parse.y:3557
        // read_token computes no such rescan — the expansion is consumed
        // with the word that contains it). perf15: the scan itself now runs
        // incrementally from `param_checkpoint` (only the appended tail,
        // plus re-derivation of a still-undecided `${` park), replacing the
        // whole-`logical_line` recollect-and-rescan that every `$`-bearing
        // appended line paid.
        let param_expansion_open = match self.param_open_cache {
            Some(false) if !line.contains('$') => false,
            _ => {
                let value = self.advance_param_scan();
                self.param_open_cache = Some(value);
                value
            }
        };
        let funcheck =
            opens_function_body_after_previous_signature(&self.logical_line, &self.output);
        if (brace_group_open || param_expansion_open) && !funcheck && !has_heredoc {
            // Reaching here proves quotes, command substitutions and
            // compound assignments are all closed: the join stands on the
            // token-level brace-group flag (and/or an open `${...}`, which
            // an inert line cannot close either). Arm the rubash#155 fast
            // path for the next physical line.
            //
            // rubash#281: when the join stands on the brace-group flag
            // ALONE (no open `${...}` — an open expansion means the pass
            // ended inside a word span), the pass ended between tokens and
            // `pass_boundary` is Some: checkpoint its tokens + lexer state
            // so the next pass lexes only the appended tail. The token Vec
            // moves in (the join `return` discards it anyway).
            if brace_group_open && !param_expansion_open {
                if let Some(state) = pass_boundary {
                    self.boundary = Some((self.logical_line.len(), line_tokens, state));
                }
            } else if param_expansion_open {
            }
            self.brace_join_active = true;
            return;
        }

        // GNU parse.y: a command's reported line is the reader's
        // line_number when its first token was read — the PHYSICAL line
        // the token starts on. A logical line that spans physical lines
        // (brace groups keep their newline separators; backslash-newline
        // continuations are recorded in continuation_join_columns) must
        // therefore stamp each token with its own physical line, not the
        // logical line's start (rubash#411: `readonly RO=1; \`
        // continued onto `RO=2` reports line 2 like GNU, not the
        // flattened line 1).
        if self.logical_line.contains('\n') || !self.continuation_join_columns.is_empty() {
            let joins = &self.continuation_join_columns;
            let logical_line = &self.logical_line;
            let logical_start_line = self.logical_start_line;
            for token in &mut line_tokens {
                let column = token.column.min(logical_line.len());
                token.position = logical_start_line
                    + logical_line[..column].matches('\n').count()
                    + joins.iter().filter(|&&join| join <= column).count();
                // DISCARD-family skips (GNU eval.c:111 — the rest of the
                // current command LIST is abandoned) span the whole logical
                // line; executor skip loops compare this, not `position`,
                // so continuation-joined commands stay comparable.
                token.logical_line = logical_start_line;
            }
        } else {
            for token in &mut line_tokens {
                token.position = self.logical_start_line;
                token.logical_line = self.logical_start_line;
            }
        }
        // GNU parse.y push_heredoc (shell.h HEREDOC_MAX 16): the 17th heredoc
        // on one command is a fatal parse error. GNU runs report_syntax_error
        // then exit_shell(EX_BADUSAGE), so the shell dies with status 2 and
        // nothing after the bad command runs. exportfunc1.sub line 14 (18
        // heredocs) relies on that: its golden output has the diagnostic and
        // the sub-shell exits 2, while the parent exportfunc.tests continues.
        //
        // The lexer cannot return an error, so record the fatal condition with
        // its script-relative line and stop tokenizing. `main` converts the
        // recorded flag into the EX_BADUSAGE exit status.
        if delimiters.len() > 16 {
            crate::lexer::record_heredoc_overflow(self.logical_start_line);
            self.overflowed = true;
            return;
        }
        self.output.append(&mut line_tokens);
        // Commit only when the logical line is accepted: the `return`s above
        // discard the partial state so the next, longer retry replays from
        // the same line-start state (see the comment at tokenize_plain).
        self.lexer_parse_state = line_lex_state;
        self.logical_line.clear();
        self.param_open_cache = None;
        self.boundary = None;
        // Offsets restart for the next logical line: cache invalid. Every
        // residual checkpoint resets to the fresh-scan state (the empty
        // buffer's full scan is `Default`), so the next logical line's
        // first gate call resumes instead of full-scanning (rubash#292;
        // quotes + compound, perf8).
        self.brace_cache.clear();
        self.header_scan_from = 0;
        self.brace_join_active = false;
        self.comsub_chars.clear();
        self.comsub_checkpoint = Some(ComsubScanCheckpoint::initial());
        self.quotes_checkpoint = Some(QuoteScanCheckpoint::initial());
        self.compound_checkpoint = Some(CompoundScanCheckpoint::initial());
        self.param_checkpoint = Some(ParamScanCheckpoint::initial());

        for delimiter in delimiters {
            // GNU parse.y:3120-3135 gather_here_documents passes the parser's
            // current line_number to make_here_document for each redirect: the
            // physical line on which the logical command line ended (this
            // line_number is already one past it), advanced by the body lines
            // any earlier heredoc of the same command consumed.  The
            // "here-document at line N" warning reports this gather line, not
            // the `<<` line, so it must travel with the body token.
            let gather_line = self.line_number.saturating_sub(1);
            // Alias reparsing must leave the caller's physical input available:
            // its heredoc body belongs to the outer parse, not this replacement.
            if self.input_origin == InputOrigin::AliasReplacementDeferredHeredoc {
                let body = if delimiter.quoted {
                    QUOTED_HEREDOC_MARKER.to_string()
                } else {
                    String::new()
                };
                let mut token = Token::new(TokenKind::HereDocBody, &body, gather_line);
                // rubash#305: this deferred body never gathers physical
                // lines (the outer parse owns them), so its gathering
                // "ends" on the delimiter line itself.
                token.heredoc_end_line = Some(gather_line);
                self.output.push(token.clone());
                self.fold_token(&token);
                continue;
            }
            self.awaiting_bodies.push(AwaitingHeredocBody {
                delimiter,
                body: String::new(),
                continued_body_line: String::new(),
                gather_line,
            });
        }
        if self.awaiting_bodies.is_empty() {
            self.emit_line_separator();
        }
    }

    /// End of input: the original loop's post-loop flush (leftover open
    /// logical line) plus the parked body-pull loops' EOF exits — an
    /// unterminated heredoc keeps the DATA_DOLLAR "not found" marker.
    pub(crate) fn finish(&mut self) -> Vec<Token> {
        if !self.awaiting_bodies.is_empty() {
            while let Some(mut awaiting) = self.awaiting_bodies.pop() {
                awaiting.body.insert(0, DATA_DOLLAR);
                if awaiting.delimiter.quoted {
                    awaiting.body.insert_str(0, QUOTED_HEREDOC_MARKER);
                }
                let mut token =
                    Token::new(TokenKind::HereDocBody, &awaiting.body, awaiting.gather_line);
                // rubash#305: EOF-unterminated gathering ends at the last
                // physical line read (GNU's line_number at EOF, make_cmd.c
                // 580's loop exits on read_secondary_line returning NULL).
                token.heredoc_end_line = Some(self.line_number.saturating_sub(1));
                self.output.push(token.clone());
                self.fold_token(&token);
            }
            self.emit_line_separator();
        }
        if !self.logical_line.is_empty() {
            // GNU parse.y parse_comsub (PST_EOFTOKEN) + print_comsub
            // (parse.y:4632): a `)` on a heredoc header line inside `$(...)`
            // closes the substitution while the still-pending body was gathered
            // from the following input lines, and the substitution text is
            // reprinted with the body inside the closing `)`. Rotate
            // `$(cat <<EOF)\nfoo\nEOF` into `$(cat <<EOF\nfoo\nEOF)` so every
            // downstream consumer sees the GNU reprint order (heredoc7.sub).
            if let Some(rotated) = relocate_comsub_heredoc_paren(&self.logical_line) {
                self.logical_line = rotated;
                self.brace_cache.clear();
                self.boundary = None;
                // End of input: no further gate calls read these offsets,
                // but keep the mirror honest anyway (rubash#292).
                self.rebuild_comsub_mirror();
            }
            let (mut line_tokens, _) = tokenize_with_boundary(
                self.boundary.take(),
                false,
                &self.logical_line,
                self.parse_posix,
                &mut self.lexer_parse_state,
                &mut self.brace_cache,
            );
            // The flush's full re-lex replaces the open line's pass results:
            // refold from the line-start snapshot (a fresh scan folds the
            // committed tokens plus this list exactly once).
            self.fold_pass_tokens(&line_tokens, false);
            // GNU parse.y line_number is PHYSICAL: the reader increments it per
            // physical line consumed, and the unclosed-construct EOF diagnostics
            // ("unexpected end of file from `{' command on line N") number both
            // the innermost opener and the EOF position from it. The leftover
            // logical line at end of input spans every physical line the open
            // construct accumulated, so map each token's byte offset (its
            // `column`) back to its physical line instead of stamping the whole
            // run with the first line (`{ </n>cmd1 </n>cmd2` reported the EOF at
            // line 2 where GNU reports line 4, rubash#278).
            // Same physical-line mapping as the accept path, continuation
            // joins included (rubash#411).
            let leftover_spans_lines =
                self.logical_line.contains('\n') || !self.continuation_join_columns.is_empty();
            for token in &mut line_tokens {
                token.position = if leftover_spans_lines {
                    let column = token.column.min(self.logical_line.len());
                    self.logical_start_line
                        + self.logical_line[..column].matches('\n').count()
                        + self
                            .continuation_join_columns
                            .iter()
                            .filter(|&&join| join <= column)
                            .count()
                } else {
                    self.logical_start_line
                };
                token.logical_line = self.logical_start_line;
            }
            self.output.append(&mut line_tokens);
            let mut separator = Token::new(TokenKind::Semicolon, ";", self.logical_start_line);
            separator.line_break = true;
            self.output.push(separator.clone());
            self.fold_token(&separator);
        }
        std::mem::take(&mut self.output)
    }
}

/// A heredoc opened inside an unclosed command substitution, tracked while
/// the tokenizer accumulates physical lines.
#[derive(Clone)]
pub(crate) struct ComsubHeredocHeader {
    pub(crate) delimiter: String,
    pub(crate) strip_tabs: bool,
}

/// Scan accumulated text for `<<` heredoc headers (skipping quoted text and
/// `$(( ))` arithmetic regions) so the comsub heredoc state machine can keep
/// their body lines verbatim. Returns the headers found plus the consumed
/// byte offset: an incomplete delimiter (one whose raw spelling ends with an
/// unquoted backslash, completed by the next physical line) leaves the scan
/// point at its `<<` so it is re-read after the join.
pub(crate) fn scan_line_for_comsub_heredoc_headers(
    line: &str,
) -> (Vec<ComsubHeredocHeader>, usize) {
    let bytes = line.as_bytes();
    let mut headers = Vec::new();
    let mut consumed = 0usize;
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    // `index` always sits on a char boundary: ASCII tokens step one byte and
    // every other char steps by its UTF-8 length. Widening bytes with
    // `bytes[i] as char` let continuation bytes 0x85/0xa0 (inside e.g.
    // U+60A0 悠 or the U+E0A0 powerline glyph) match is_whitespace and made
    // delimiter slices land mid-char (panic, same class as niubash#92).
    while index < bytes.len() {
        let ch = line[index..].chars().next().expect("index is a boundary");
        match ch {
            '\'' if !double => {
                single = !single;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '\\' if !single => {
                index += 1 + char_len_at(line, index + 1);
            }
            '$' if !single
                && bytes.get(index + 1) == Some(&b'(')
                && bytes.get(index + 2) == Some(&b'(') =>
            {
                // Arithmetic region: `<<` there is a shift, not a heredoc.
                index += 2;
                let mut depth = 2usize;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'(' => depth += 1,
                        b')' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                }
            }
            '<' if !single
                && bytes.get(index + 1) == Some(&b'<')
                && bytes.get(index + 2) != Some(&b'<') =>
            {
                index += 2;
                let strip_tabs = bytes.get(index) == Some(&b'-');
                if strip_tabs {
                    index += 1;
                }
                while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
                    index += 1;
                }
                let start = index;
                // GNU read_token_word: quoting inside the delimiter word
                // makes metacharacters literal — `<< ')'` names `)` as the
                // delimiter, so a quoted `)` (or `;`, `|`, `&`) is delimiter
                // text, not the substitution closer (comsub-posix.tests).
                let mut delimiter_single = false;
                let mut delimiter_double = false;
                while index < bytes.len() {
                    let current = line[index..].chars().next().expect("index is a boundary");
                    match current {
                        '\'' if !delimiter_double => delimiter_single = !delimiter_single,
                        '"' if !delimiter_single => delimiter_double = !delimiter_double,
                        _ if !delimiter_single
                            && !delimiter_double
                            && (current.is_whitespace()
                                || matches!(current, ';' | '|' | '&' | ')')) =>
                        {
                            break;
                        }
                        '\\' if !delimiter_single
                            && !delimiter_double
                            && index + 1 < bytes.len() =>
                        {
                            index += 1 + char_len_at(line, index + 1);
                            continue;
                        }
                        _ => {}
                    }
                    index += current.len_utf8();
                }
                let raw = &line[start..index.min(line.len())];
                let value: String = raw
                    .chars()
                    .filter(|current| !matches!(current, '\'' | '"' | '\\'))
                    .collect();
                let value = if strip_tabs {
                    value.trim_start_matches('\t').to_string()
                } else {
                    value
                };
                if index >= bytes.len() && raw.ends_with('\\') && !raw.ends_with("\\\\") {
                    // The delimiter continues on the next physical line
                    // (`<<\EOT\` + `4`): resume this scan after the join.
                    return (headers, consumed);
                }
                if !value.is_empty() {
                    headers.push(ComsubHeredocHeader {
                        delimiter: value,
                        strip_tabs,
                    });
                }
                consumed = index;
            }
            _ => {
                index += ch.len_utf8();
                consumed = index;
            }
        }
    }
    (headers, consumed)
}

/// UTF-8 length of the char starting at `index`; 1 when `index` is out of
/// range (trailing escape at end of line), matching the old `+= 2` step.
fn char_len_at(line: &str, index: usize) -> usize {
    line.get(index..)
        .and_then(|rest| rest.chars().next())
        .map_or(1, char::len_utf8)
}

pub fn has_unclosed_input_syntax(input: &str) -> bool {
    has_unclosed_input_syntax_posix(input, false)
}

/// rubash#380: shared `esac` previous-token witness predicate (see
/// skip.rs) — re-exported for the executor's copies of the case word
/// machine.
pub(crate) use skip::esac_prev_token_char;

/// POSIX-aware variant: `set -o posix` changes how `'` inside `"${...}"`
/// scans (Interp 221), so the unclosed-delimiter probe must know the mode.
pub fn unclosed_array_subscript_line(input: &str) -> Option<(usize, bool)> {
    skip::unclosed_array_subscript_line(input)
}

/// rubash#390 q2: subscript-EOF diagnostic router — closer char and report
/// line follow parse_matched_pair's quote recursion (parse.y:4040-4051,
/// 3901-3912): an unterminated `'`/`"`/`$'` inside the `[` scan reports the
/// QUOTE at its open line instead of `]' at the `[` line. The third member
/// is the `[` construct's own line (the prefix cut for complete-command
/// execution stops there).
pub fn unclosed_array_subscript_eof(input: &str) -> Option<(usize, char, usize, bool)> {
    skip::unclosed_array_subscript_eof(input)
}

/// rubash#390 q1: `((` P_ARITH-group probe for the unclosed-command-`(`
/// diagnostic router (see skip.rs).
pub fn dparen_arith_group_never_closes(input: &str, open_line: usize) -> bool {
    skip::dparen_arith_group_never_closes(input, open_line)
}

/// rubash#390 q1 class B: the `((` nested-subshell reinterpretation whose
/// reparse dies with parse_compound_assignment's clean-EOF report (see
/// skip.rs).
pub fn dparen_subshell_reparse_compound_eof(input: &str, open_line: usize) -> bool {
    skip::dparen_subshell_reparse_compound_eof(input, open_line)
}

pub fn has_unclosed_input_syntax_posix(input: &str, posix: bool) -> bool {
    has_unclosed_quotes(input)
        || (has_unclosed_command_substitution(input)
            && !skip::command_substitutions_balanced(input))
        // GNU parse.y:5635-5643: an unclosed array subscript `[` swallows
        // the rest of the input ahead of any other matched-pair construct
        // (`x=([a` reports `]`, not `)`) — check it before the generic
        // close-char scan (rubash#221).
        || skip::unclosed_array_subscript_line(input).is_some()
        // A bare `(`/`{`-class delimiter can also keep a command open:
        // `ddd=(aaa` array lists and `( cmd` subshells continue on the
        // next line (GNU parse.y reads until the matching close).
        || unclosed_input_close_char_posix(input, posix).is_some()
}

/// parse.y:5379-5384 read_token_word: a backslash before the newline is
/// removed with it ("ignored in all cases except when quoted with single
/// quotes"), so a physical line ending in an unquoted backslash is an
/// unfinished token -- the incremental stdin driver must keep reading (PS2),
/// not submit the line with the backslash dropped. Drivers accumulate with
/// the newline attached, so probe the text before it; a retained CR (CRLF
/// line) then correctly reads as an ESCAPED carriage return, which GNU also
/// does not treat as a continuation.
pub fn stdin_line_ends_with_continuation(input: &str) -> bool {
    ends_with_unquoted_backslash(input.strip_suffix('\n').unwrap_or(input))
}

/// Rotate the `)`-that-closed-on-the-header-line segment of a command
/// substitution's heredoc past the gathered body, mirroring GNU
/// print_comsub's reprint order. Returns None when the input carries no such
/// pattern.
fn relocate_comsub_heredoc_paren(input: &str) -> Option<String> {
    if !input.contains("$(") || !input.contains("<<") {
        return None;
    }
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut depth = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        match ch {
            '\'' if !double => {
                single = true;
                index += 1;
            }
            '"' if !single => {
                double = !double;
                index += 1;
            }
            '$' if !double
                && chars.get(index + 1) == Some(&'(')
                && chars.get(index + 2) != Some(&'(') =>
            {
                depth += 1;
                index += 2;
            }
            '(' if depth > 0 && !double => {
                depth += 1;
                index += 1;
            }
            ')' if depth > 0 && !double => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            '<' if depth > 0
                && !double
                && chars.get(index + 1) == Some(&'<')
                && chars.get(index + 2) != Some(&'<') =>
            {
                let (next, closure) =
                    heredoc_scan::skip_heredoc_in_chars_with_closure(&chars, index);
                if let Some((paren, header_end)) = closure {
                    // `)` + its header-line tail move past the gathered body:
                    // `$(cat <<EOF)\nbody\nEOF` reads as GNU's reprint
                    // `$(cat <<EOF\nbody\nEOF)`.
                    let mut out: String = chars[..paren].iter().collect();
                    out.extend(chars[header_end..next].iter());
                    out.extend(chars[paren..header_end].iter());
                    out.extend(chars[next..].iter());
                    return Some(out);
                }
                index = next.max(index + 1);
            }
            _ => index += 1,
        }
    }
    None
}

/// rubash#155 / #130: whether a physical line appended to a logical line
/// whose only open construct is the token-level brace-group join provably
/// cannot change any lexer decision the join iteration recomputes.
///
/// The line must contain no byte that any consumer between the append and
/// the join `continue` can react to: quote characters (has_unclosed_quotes,
/// and quoting state inside every scanner), backslash (line continuations,
/// escapes), `$` (parameter/command/arithmetic substitution openers),
/// `{`/`}` (brace-group depth: a `}` could fold the group and end the join),
/// `(`/`)` (compound assignments, subshells), `` ` `` (backtick
/// substitutions), `#` (comments), `<` (heredoc operators and delimiters).
/// The `posix` substring check keeps the per-pass `set -o posix` detection
/// (`line_posix_mode_change`) exact: without quoting or escaping bytes the
/// `posix` word token implies this contiguous substring in the line, and
/// with it the slow path runs and computes the real answer.
///
/// This is an admission whitelist, not a symptom blacklist (rubash#117
/// rule): a false negative only costs the O(buffer) slow path that the
/// unchanged code below already implements; every admitted line is proven
/// inert for the decisions listed in the fast-path comment in
/// `tokenize_with_heredocs`.
fn brace_join_fast_path_line(line: &str) -> bool {
    if line.bytes().any(|b| {
        matches!(
            b,
            b'\'' | b'"' | b'`' | b'$' | b'{' | b'}' | b'(' | b')' | b'#' | b'<' | b'\\'
        )
    }) {
        return false;
    }
    !line.contains("posix")
}

thread_local! {
    static TOKENIZE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// What a top-level line's tokens say about the parse-time extglob gate.
enum ExtglobFlip {
    None,
    /// `shopt -s extglob` seen: later lines parse with the gate open.
    Enable,
    /// `shopt -u extglob` seen: later lines parse with the gate closed.
    Disable,
    /// A top-level `set -n` executed: from here on GNU parses without
    /// executing, so shopt lines must stop flipping the gate.
    ExecutionOff,
}

/// Detect a top-level `shopt -s/-u extglob` (or `set -n`) command in a
/// tokenized logical line. `shopt` commands nested inside a compound
/// (function body, brace group, subshell, if/while/for/case) do not count:
/// GNU parses the whole enclosing definition before any of its body
/// commands could execute (verified: GNU 5.3.0 rejects
/// `g() { shopt -s extglob; case x in ?(a)) :;; esac; }` with rc 2).
fn line_extglob_mode_change(tokens: &[Token]) -> ExtglobFlip {
    let mut result = ExtglobFlip::None;
    let mut command_start = true;
    // Compound opener stack (innermost last). A closer pops only when it
    // matches the top, so a case-clause `)` inside `case ... esac` never
    // pops the case frame, and a folded `{ ... }' keyword token (a complete
    // compound in one token) neither opens nor closes a frame.
    let mut compound: Vec<&'static str> = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        let is_separator = token.line_break
            || matches!(
                token.kind,
                TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Background
                    | TokenKind::Pipe
                    | TokenKind::PipeErr
            );
        if is_separator {
            command_start = true;
            index += 1;
            continue;
        }
        if token.kind == TokenKind::Keyword {
            let closes = match token.value.as_str() {
                "}" => Some("}"),
                ")" => Some(")"),
                "fi" => Some("fi"),
                "done" => Some("done"),
                "esac" => Some("esac"),
                "]]" => Some("]]"),
                _ => None,
            };
            let opens = match token.value.as_str() {
                "{" => Some("}"),
                "(" => Some(")"),
                "if" => Some("fi"),
                "while" | "until" | "for" | "select" => Some("done"),
                "case" => Some("esac"),
                "[[" => Some("]]"),
                _ => None,
            };
            if let Some(closer) = closes {
                if compound.last() == Some(&closer) {
                    compound.pop();
                }
            } else if let Some(closer) = opens {
                if !token.value.starts_with('{') || token.value.trim() == "{" {
                    compound.push(closer);
                }
            }
        }
        if command_start && compound.is_empty() {
            if token.kind == TokenKind::Word && token.value == "shopt" {
                if let Some(enabled) = shopt_extglob_change(&tokens[index + 1..]) {
                    result = if enabled {
                        ExtglobFlip::Enable
                    } else {
                        ExtglobFlip::Disable
                    };
                }
            } else if token.kind == TokenKind::Word && token.value == "set" {
                if tokens[index + 1..]
                    .iter()
                    .take_while(|next| next.kind == TokenKind::Word)
                    .any(|next| next.value == "-n")
                {
                    result = ExtglobFlip::ExecutionOff;
                }
            }
        }
        command_start = false;
        index += 1;
    }
    result
}

/// `shopt` argument scan: does this command turn `extglob` on or off?
/// Mirrors the argument walk of GNU `shopt_builtin`
/// (builtins/shopt.def:292-340) and `toggle_shopts` (shopt.def:469-485):
/// internal_getopt consumes the `-p/-s/-u/-o/-q` flags, then EVERY
/// remaining word is an option NAME — each recognized name is set or unset,
/// and an unrecognized name only fails that one entry (shopt_error,
/// shopt.def:472-476) while the walk continues. So `shopt -s nullglob
/// extglob` DOES turn extglob on for later parses (shopt.def:640 propagates
/// extglob_flag to the parser's extended_glob; read_token_word then accepts
/// `@(` at parse.y:5464-5466) — the printf.tests line-358 form. Two GNU
/// error paths set nothing and must report no flip: `-s` and `-u` together
/// ("cannot set and unset shell options simultaneously", shopt.def:327-330)
/// and an unknown flag letter (builtin_usage, shopt.def:317-318). A `-o`
/// invocation routes extglob through set_shopt_o_options, where it is not
/// a -o option name and is never set (shopt.def:337-341) — also no flip.
fn shopt_extglob_change(args: &[Token]) -> Option<bool> {
    // The single mode GNU collects from the flags (-s/-u; both = error).
    let mut mode = None;
    // `shopt -o(-s|-u) ...` sets only -o option names, never extglob.
    let mut o_names_only = false;
    let mut flags_ended = false;
    for token in args {
        if token.kind != TokenKind::Word {
            // A redirection (`shopt -s extglob 2>/dev/null`) is not part of
            // the builtin's WORD_LIST (GNU loptend skips it), so it only
            // ENDS the scan here — an `extglob' name already seen still
            // counts.
            break;
        }
        if !flags_ended && token.value.starts_with('-') && token.value != "-" {
            if token.value == "--" {
                flags_ended = true;
                continue;
            }
            for flag in token.value[1..].chars() {
                match flag {
                    's' => {
                        if mode == Some(false) {
                            // -s and -u together: GNU fails the whole builtin.
                            return None;
                        }
                        mode = Some(true);
                    }
                    'u' => {
                        if mode == Some(true) {
                            return None;
                        }
                        mode = Some(false);
                    }
                    'o' => o_names_only = true,
                    'p' | 'q' => {}
                    // Unknown flag letter: GNU prints usage and sets nothing.
                    _ => return None,
                }
            }
            continue;
        }
        match token.value.as_str() {
            "extglob" => return if o_names_only { None } else { mode },
            // Any other word is one more option NAME for the same mode;
            // GNU continues past names it does not recognize.
            _ => continue,
        }
    }
    None
}

fn tokenize_plain(
    input: &str,
    posix: bool,
    parse_state: &mut LexerParseState,
    brace_cache: &mut BraceScanCache,
) -> (Vec<Token>, Option<LexerBoundaryState>) {
    let mut lexer = Lexer::new_with_cache(input, posix, brace_cache);
    lexer.extended_glob = parse_extended_glob();
    // parse.y keeps a single parser_state for the whole input — resume the
    // PST_CASEPAT / last_read_token state left by the previous logical line.
    lexer.restore_parse_state(parse_state.clone());
    let mut tokens = Vec::new();
    for token in &mut lexer {
        if token.kind == TokenKind::Eof {
            break;
        }
        tokens.push(token);
    }
    *parse_state = lexer.take_parse_state();
    let boundary = lexer.boundary_state();
    (tokens, boundary)
}

/// rubash#281 complete-command-boundary resume: when a checkpoint from the
/// previous pass over this logical line is valid (append-only growth, see
/// the `boundary` declaration), continue lexing from the checkpoint offset
/// with the captured between-token state instead of re-lexing the whole
/// accumulated buffer. The produced token list and the final
/// `LexerParseState` are identical to a full `tokenize_plain` pass by
/// construction: the checkpointed prefix tokens ARE the full pass's prefix
/// results, and the lexer is a deterministic scanner over
/// (position, state, remaining text) — GNU's own read_token model
/// (parse.y:3557).
///
/// `resume_allowed` carries the one whole-class safety gate the caller
/// owns: the appended physical line must contain no `}` byte. A `{` group
/// still open in the prefix is emitted as a bare `{` Keyword plus
/// individually-lexed body tokens (scanner.rs scan_token `{` arm), and it
/// FOLDS into one token in the exact pass where its `}` arrives — a fold
/// the resumed lexer cannot perform for a `{` that sits before the
/// checkpoint offset (it never re-visits the opener). Every fold
/// completion needs a `}` byte in the appended text, so refusing to
/// resume when the line carries one restores full re-lexing exactly on
/// the fold passes (once per group close); false positives (`}` inside a
/// comment or string) only cost the full pass.
fn tokenize_with_boundary(
    checkpoint: Option<(usize, Vec<Token>, LexerBoundaryState)>,
    resume_allowed: bool,
    input: &str,
    posix: bool,
    parse_state: &mut LexerParseState,
    brace_cache: &mut BraceScanCache,
) -> (Vec<Token>, Option<LexerBoundaryState>) {
    if resume_allowed {
        if let Some((offset, mut tokens, state)) = checkpoint {
            if offset <= input.len() {
                let mut lexer = Lexer::new_resumed_at(input, posix, brace_cache, offset, &state);
                lexer.extended_glob = parse_extended_glob();
                for token in &mut lexer {
                    if token.kind == TokenKind::Eof {
                        break;
                    }
                    tokens.push(token);
                }
                *parse_state = lexer.take_parse_state();
                return (tokens, lexer.boundary_state());
            }
        }
        // resume was allowed but no checkpoint existed: full re-lex under
        // the ALLOWED stat (tail = whole input).
        let started = std::time::Instant::now();
        let result = tokenize_plain(input, posix, parse_state, brace_cache);
        return result;
    }
    let result = tokenize_plain(input, posix, parse_state, brace_cache);
    result
}

/// Detect top-level `set -o posix` / `set +o posix` commands in a tokenized
/// logical line, returning the POSIX mode that should apply to later lines.
fn line_posix_mode_change(tokens: &[Token]) -> Option<bool> {
    let mut result = None;
    let mut command_start = true;
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        let is_separator = token.line_break
            || matches!(
                token.kind,
                TokenKind::Semicolon
                    | TokenKind::And
                    | TokenKind::Or
                    | TokenKind::Background
                    | TokenKind::Pipe
                    | TokenKind::PipeErr
            );
        if is_separator {
            command_start = true;
            index += 1;
            continue;
        }
        if command_start && token.kind == TokenKind::Word && token.value == "set" {
            if let Some(enabled) = set_command_posix_change(&tokens[index + 1..]) {
                result = Some(enabled);
            }
        }
        command_start = false;
        index += 1;
    }
    result
}

fn set_command_posix_change(tokens: &[Token]) -> Option<bool> {
    let mut index = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.kind != TokenKind::Word {
            return None;
        }
        let value = token.value.as_str();
        if value == "--" {
            return None;
        }
        if value == "-o" || value == "+o" {
            let next = tokens.get(index + 1)?;
            if next.kind == TokenKind::Word && next.value == "posix" {
                return Some(value == "-o");
            }
            return None;
        }
        if value.starts_with('-') || value.starts_with('+') {
            index += 1;
            continue;
        }
        return None;
    }
    None
}
