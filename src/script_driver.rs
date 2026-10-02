//! Script drivers shared by the binary entry points and the in-process
//! `${THIS_SH} ./x.sub` child path (executor/external_finish.rs).
//!
//! GNU Bash runs a script through a line-oriented reader that calls
//! bashhist.c pre_process_line on each syntactically complete group:
//! history expansion first, then history recording, then parse+execute.
//! The in-process THIS_SH child must take the same path, or `set -H` /
//! `set -o histexpand` scripts lose `!!`/`!str`/word-designator expansion
//! exactly like a spawned rubash.exe would not.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::rc::Rc;

use crate::executor::{ExecuteError, Executor};
use crate::history::SessionHistory;
use crate::history_expand::{HistChars, HistCtx};
use crate::lexer::{
    expand_aliases_in_source, tokenize, tokenize_with_initial_posix, AliasLookup, Token, TokenKind,
};
use crate::parser::CommandNode;

/// bashhist.c: does this script turn history on? Detects the long-form
/// option (set -o history / -o histexpand) and the short flag cluster
/// containing -H, which is how the histexp tests enable expansion.
pub fn script_uses_history(contents: &str) -> bool {
    for line in contents.lines() {
        let Some(rest) = line.trim_start().strip_prefix("set ") else {
            continue;
        };
        let mut expecting_option = false;
        for token in rest.split_whitespace() {
            if expecting_option {
                if token == "history" || token == "histexpand" {
                    return true;
                }
                expecting_option = false;
                continue;
            }
            if token == "-o" {
                expecting_option = true;
            } else if token.len() >= 2
                && token.starts_with('-')
                && !token.starts_with("--")
                && token[1..].contains('H')
            {
                return true;
            }
        }
    }
    false
}

/// Does this script turn alias expansion on? `shopt -s expand_aliases` is
/// a runtime command, but its result must be visible to the READER of the
/// following lines (GNU reads and executes one complete command at a
/// time), so scripts mentioning the option take the grouped driver where
/// each group is lexed against the alias table live at that point.
/// `set -o posix` likewise flips expand_aliases at runtime (general.c
/// posix_initialize), so it must take the same driver — otherwise
/// `$(...)` bodies are extracted before the alias table applies
/// (comsub5.sub).
pub fn script_uses_aliases(contents: &str) -> bool {
    contents.contains("expand_aliases") || contents.contains("set -o posix")
}

/// GNU parse.y alias_expand_token + push_string, run over the text of one
/// command group. The lookup yields (value, AL_EXPANDNEXT); alias values
/// store '$' as DATA_DOLLAR when the word carried it in data position.
pub(crate) fn expand_group_aliases(executor: &Executor, source: &str) -> String {
    if !executor.alias_expansion_enabled() || executor.shell_state.aliases.is_empty() {
        return source.to_string();
    }
    let lookup = move |word: &str| {
        executor.shell_state.aliases.get(word).map(|alias| {
            (
                alias
                    .value
                    .replace(crate::executor::markers::DATA_DOLLAR, "$"),
                alias.expand_next,
            )
        })
    };
    expand_aliases_in_source(source, &lookup as &AliasLookup<'_>)
}

/// shell.c run_pending_command style driver for scripts with history on:
/// gather each syntactically complete group (like the stdin driver), run
/// history expansion over its lines, record the joined entry, then execute.
///
/// `redirect_cmd`, when present, is the invocation command whose output
/// redirections apply to every executed group (`${THIS_SH} f >out` keeps
/// the redirect for the child's lifetime, GNU execute_cmd.c:6139-6233);
/// target files are prepared once by the caller, per-group injection uses
/// the append form like trap_exec's compound handling.
pub fn run_script_with_history(
    executor: &mut Executor,
    contents: &str,
    redirect_cmd: Option<&CommandNode>,
) -> i32 {
    let session = Rc::new(RefCell::new(SessionHistory::new()));
    run_script_with_history_in(executor, contents, session, redirect_cmd)
}

/// Same grouped driver with a host-owned session history (data plane).
/// Hosts embedding rubash (niubash) preload the session from their own
/// history file so `!!`/`!str` inside scripts expands against — and records
/// back into — the live session list. Spawned and in-process THIS_SH
/// children keep run_script_with_history's fresh list: GNU gives each new
/// shell process an empty history list (`set -o history` in a script does
/// not read $HISTFILE), so only the embedding host may share its session.
pub fn run_script_with_history_in(
    executor: &mut Executor,
    contents: &str,
    session: Rc<RefCell<SessionHistory>>,
    redirect_cmd: Option<&CommandNode>,
) -> i32 {
    executor.set_session_history(Some(session.clone()));
    let raw_lines: Vec<&str> = contents.split_inclusive('\n').collect();
    let mut index = 0usize;
    while index < raw_lines.len() {
        // One gather implementation for every grouped reader (the `source`
        // builtin shares it via read_next_source_group). The gather loop
        // must not re-scan the accumulated pending per physical line: GNU
        // reads its input token by token (parse.y:3557 read_token) and
        // never re-tokenizes text it has already consumed; the parked
        // GroupScanFeeder advances the token-level completeness state per
        // appended line instead. The alias-live arm keeps the exact fresh
        // whole-pending scan (expansion may rewrite any part of the text).
        // The returned feeder tokens are unused here: this driver's exec
        // text re-joins the group's line texts (and history expansion may
        // rewrite them), so run_history_group's parse keeps its own scan.
        let Some((pending, start_line, group, mut feeder_tokens)) =
            read_next_source_group(executor, &raw_lines, &mut index)
        else {
            break;
        };
        let status = run_history_group(
            executor,
            &session,
            &group,
            start_line,
            redirect_cmd,
            false,
            feeder_tokens.take(),
        );
        let parse_error = executor.take_parse_error();
        // A group that ended by unwinding (exit builtin, errexit, POSIX
        // special-builtin failure) stops the reader unconditionally — GNU's
        // jump_to_top_level cannot be resumed at the next command.
        if parse_error
            || executor.take_exit_jump_pending()
            || (status != 0
                && stdin_script_errexit_enabled(executor)
                // GNU execute_cmd.c:652-656: `! CMD` gains CMD_IGNORE_RETURN
                // under errexit — the inverted command's status is exempt.
                && !executor.last_command_inverted())
        {
            break;
        }
    }
    executor.last_exit_code()
}

/// builtins/evalfile.c source_file -> evalstring.c parse_and_execute: a
/// sourced file is read and executed INCREMENTALLY, so an `alias` command
/// takes effect for every group read after it. This pulls the next
/// syntactically complete command group out of `raw_lines` using the same
/// completeness test as run_script_with_history_in — heredoc bodies,
/// procsub paren depth, and the alias-expanded pending text all decide the
/// boundary. Returns the group's raw text, its 1-based starting line, the
/// per-physical-line `(text, is_heredoc_body)` pairs the history driver
/// records per group, and — in the non-alias arm — the feeder's committed
/// tokens for the group text (see the perf10 note below).
///
/// rubash#281 companion: while the alias table cannot influence the text
/// (expansion disabled or table empty — the expanded pending IS the raw
/// pending), the token-level half of the completeness test runs on a
/// parked `GroupScanFeeder` that advances per appended line instead of
/// re-tokenizing the whole accumulated group (the O(group^2) amplifier
/// behind `. ./benchmarks/corpus/nvm.sh --no-use` at 200 s+). With a live
/// alias table the exact fresh scan is kept: expansion may rewrite any
/// part of the pending text, so the feeder's append-only checkpoint has no
/// valid prefix to resume from.
///
/// perf10 token reuse: `tokenize_with_heredocs` IS a fresh
/// `GroupScanFeeder` plus one `push_line` per physical line plus
/// `finish()` — the gather's feeder runs the identical code on the
/// identical line pieces (the group breaks only at a committed state, so
/// every batch piece within the group's bytes is one of this feeder's
/// pushes), with the identical initial posix mode (nothing executes
/// between the gather's mode capture and the caller's parse, so mid-group
/// `set -o posix` flips replay at the same logical lines in both). GNU
/// anchor: parse.y:3557 read_token streams the input once and never
/// re-tokenizes consumed text — the caller's fresh whole-group re-lex was
/// this port's substitute for that model. The committed token stream is
/// therefore returned for the caller to reuse instead of re-lexing, under
/// exactly the conditions that make it byte-identical to the fresh
/// re-lex:
///   - the non-alias arm only (`expand_group_aliases` is the identity
///     there, so the caller's exec text == pending; with live aliases the
///     caller keeps its fresh re-lex of the possibly rewritten text —
///     parse.y:3249 alias_expand_token fires per token while READING);
///   - the loop broke at a COMPLETE group (an EOF-exhausted incomplete
///     group takes run_source_with_line_offset's unclosed-diagnostics
///     re-lex);
///   - the feeder performed no EFFECTIVE extglob toggle
///     (`extglob_toggled`): the fresh re-lex replays shopt flips starting
///     from the group-END global gate, so on a flipping group its
///     pre-flip lines see a different gate than the streaming feed did —
///     refusing reuse preserves today's re-lex behavior byte for byte
///     (parse.y:5466 gates pattern chars per token; tokens snapshot the
///     gate they were lexed under, `Token::extglob_gate`).
/// The trailing line-break separator the feeder emits per committed
/// logical line is left in place; the caller pops it with the same rule
/// `tokenize_comsub_body_with_origin` applies to the fresh stream.
pub(crate) fn read_next_source_group(
    executor: &Executor,
    raw_lines: &[&str],
    index: &mut usize,
) -> Option<(String, usize, Vec<(String, bool)>, Option<Vec<Token>>)> {
    if *index >= raw_lines.len() {
        return None;
    }
    let mut pending = String::new();
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    let mut paren_depth: i64 = 0;
    let mut saw_heredoc = false;
    let mut heredoc_arith_depth: i64 = 0;
    let mut group: Vec<(String, bool)> = Vec::new();
    let start_line = *index + 1;
    let posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let aliases_live =
        executor.alias_expansion_enabled() && !executor.shell_state.aliases.is_empty();
    let mut scan = if aliases_live {
        None
    } else {
        Some(crate::lexer::GroupScanFeeder::new(posix))
    };
    let mut broke_complete = false;
    // perf9 (#292B third wave): the parked text-scan battery over the exact
    // `pending` mirror. `pending` is append-only inside this loop (only
    // push_str(raw) ever touches it), so the checkpoints never invalidate:
    // each candidate line advances every still-undecided machine over its
    // own tail instead of re-scanning the whole group (GNU parse.y:3557
    // read_token streams token by token and never re-reads consumed text).
    // The alias-live arm below keeps the exact fresh whole-pending scan:
    // alias expansion may rewrite any part of the text.
    let mut text_scans = GroupTextScans::new(posix);
    // perf21 battery fusion: heredoc-body lines live in the pending mirror
    // but not in the feeder's logical-line mirror, so a group that saw any
    // must keep advancing the battery's quotes/comsub machines itself.
    let mut group_has_body = false;
    while *index < raw_lines.len() {
        let raw = raw_lines[*index];
        let text = raw.trim_end_matches('\n');
        *index += 1;
        let mut is_body = false;
        // perf21: with the alias table inert, expand_group_aliases is the
        // identity — skip its per-line String copy and scan the raw text
        // directly (aliases_live above computed the same predicate).
        let expanded_owned;
        let expanded_line: &str = if aliases_live {
            expanded_owned = expand_group_aliases(executor, text);
            &expanded_owned
        } else {
            text
        };
        // perf21: borrow the front delimiter instead of cloning the
        // (String, bool) per line; the remove happens after the borrow ends.
        enum Front {
            None,
            Matched,
            Body,
        }
        let front = match pending_heredocs.first() {
            Some((delimiter, strip_tabs)) => {
                let candidate = if *strip_tabs {
                    text.trim_start_matches('\t')
                } else {
                    text
                };
                if candidate == delimiter.as_str() {
                    Front::Matched
                } else {
                    Front::Body
                }
            }
            None => Front::None,
        };
        match front {
            Front::Matched => {
                pending_heredocs.remove(0);
            }
            Front::Body => {
                is_body = true;
                group_has_body = true;
            }
            Front::None => {
                let declared =
                    stdin_heredoc_line_declarations(&expanded_line, &mut heredoc_arith_depth);
                saw_heredoc = saw_heredoc
                    || (!declared.is_empty()
                        && (expanded_line.contains("$(")
                            || expanded_line.contains("<(")
                            || expanded_line.contains(">(")));
                pending_heredocs.extend(declared);
            }
        }
        if !is_body {
            paren_depth += line_paren_delta(&expanded_line);
        }
        group.push((text.to_string(), is_body));
        pending.push_str(raw);
        text_scans.push_raw_line(raw);
        if let Some(scan) = scan.as_mut() {
            scan.push_line(text, pending.len());
        }
        if pending_heredocs.is_empty() && (!saw_heredoc || paren_depth <= 0) {
            if let Some(scan) = scan.as_ref() {
                // Token-level answer FIRST: it is O(1) here (maintained
                // incrementally by the feeder), while the text-level scans
                // walk the whole accumulated group. While any `{`/`if`/
                // `case`... keeps the group open (nvm.sh wraps its entire
                // body in one brace group), this short-circuits the O(group)
                // rescans entirely; the text scans then run only at
                // candidate-complete lines. `||` is commutative, so the
                // answer is byte-identical to the fresh scan's.
                //
                // perf9 (#292B third wave): the text-level half now advances
                // the parked GroupTextScans machines instead of full-scanning
                // `pending` through every scanner per candidate line — arm
                // for arm the same predicates stdin_source_text_needs_more
                // evaluates, each answered from its checkpoint over the
                // append-only pending mirror (quotes / comsub residual +
                // corrected balance / array subscript / matched-pair close
                // char / function-body delimiters). Per-prefix equivalence
                // is enforced by the continuation.rs incremental tests.
                // perf21 battery fusion: when the feeder's own join gates
                // prove the just-pushed line closed every quote and
                // substitution over the append-only twin of this pending
                // text, the battery's duplicate quotes/comsub machines can
                // take the answer instead of re-advancing (see
                // GroupScanFeeder::gates_prove_quotes_comsub_closed).
                let skip_quote_comsub = !group_has_body && scan.gates_prove_quotes_comsub_closed();
                let needs_more = scan.token_level_needs_more()
                    || text_scans.needs_more(&pending, skip_quote_comsub);
                if !needs_more {
                    broke_complete = true;
                    break;
                }
                continue;
            }
            let expanded_pending = expand_group_aliases(executor, &pending);
            if !stdin_source_needs_more_posix(&expanded_pending, posix) {
                broke_complete = true;
                break;
            }
        }
    }
    let feeder_tokens = match scan {
        Some(mut scan) if broke_complete && !scan.extglob_toggled() => Some(scan.finish()),
        _ => None,
    };
    Some((pending, start_line, group, feeder_tokens))
}

/// One parked text-scan checkpoint over the group's `pending` mirror:
/// the char offset the scan resumes from, and the machine state there.
/// `None` (in the owning struct) means the next call full-scans from 0.
struct ScanCheckpoint<S> {
    resume: usize,
    snapshot: S,
}

/// Phase of a function-body delimiter scan
/// (incremental mirror of stdin_source_has_unclosed_function_delimited_body).
#[derive(Clone, Debug, PartialEq, Eq)]
enum FnBodyPhase {
    /// Still scanning for the first unquoted delimiter.
    Search,
    /// Delimiter found: counting its unquoted depth from that fixed point.
    /// `signature` is the once-computed function-signature answer for the
    /// text BEFORE the delimiter — that prefix never changes again (the
    /// group only appends), so the oracle's per-call signature re-check
    /// collapses to this stored bit.
    Depth { signature: bool },
}

/// Incremental state of one function-body delimiter scan
/// (`stdin_source_has_unclosed_function_delimited_body(source, delim)`).
///
/// The oracle, per candidate line: (1) `first_unquoted_function_body_delimiter`
/// scans for the first unquoted `{` (or `(`), where a `(` followed by
/// optional whitespace and `)` is a `name ()` signature, not a body opener —
/// and after such a rejection the search RESTARTS with a fresh
/// `CommentAwareScan` from just past the rejected `(`; (2) if found at `d`
/// and `unquoted_delimiter_depth(source[d..]) != 0`, the text before `d`
/// must look like a function signature. Append-only prefix model: the
/// search advances monotonically (a found delimiter is fixed forever), the
/// depth scan is a fresh fold from `d`, and the only forward decision that
/// can flip on a longer buffer is the `(`-rejection test when only
/// whitespace remains to the buffer end — that one position parks (the
/// committed trajectory still accepts the `(` for THIS prefix, exactly like
/// the oracle's trim_start-at-EOF; the park re-derives from `search_from`
/// with a fresh scan, exactly like the oracle's next loop iteration).
#[derive(Clone, Debug, PartialEq, Eq)]
struct FnBodyResidualState {
    phase: FnBodyPhase,
    scan: CommentAwareScan,
    depth: usize,
    search_from: usize,
}

impl Default for FnBodyResidualState {
    fn default() -> Self {
        Self {
            phase: FnBodyPhase::Search,
            scan: CommentAwareScan::new(),
            depth: 0,
            search_from: 0,
        }
    }
}

impl FnBodyResidualState {
    /// The oracle's answer for the current prefix:
    /// `stdin_source_has_unclosed_function_delimited_body`.
    fn is_open(&self) -> bool {
        matches!(self.phase, FnBodyPhase::Depth { signature: true }) && self.depth != 0
    }
}

/// A re-derivation point returned by [`fnbody_residuals_advance`].
struct FnBodyResidualPark {
    pos: usize,
    snapshot: FnBodyResidualState,
}

/// Advance one function-body delimiter scan over `chars[from..]`. `pending`
/// is the whole accumulated group text (needed once, at the delimiter-found
/// transition, for the signature check over the text before the delimiter).
fn fnbody_residuals_advance(
    chars: &[char],
    from: usize,
    state: &mut FnBodyResidualState,
    delim: char,
    pending: &str,
) -> Option<FnBodyResidualPark> {
    let close = match delim {
        '{' => '}',
        '(' => ')',
        _ => return None,
    };
    let mut index = from.min(chars.len());
    let mut park: Option<FnBodyResidualPark> = None;

    while index < chars.len() {
        let ch = chars[index];
        if let FnBodyPhase::Search = state.phase {
            let is_active = state.scan.active(ch);
            if is_active && ch == delim {
                if delim == '(' {
                    // The `name ()` rejection test consults unbounded
                    // forward text (whitespace run, then `)` or not).
                    let mut look = index + 1;
                    while look < chars.len() && chars[look].is_whitespace() {
                        look += 1;
                    }
                    if look >= chars.len() {
                        // Undecided: only whitespace to the buffer end. The
                        // oracle accepts this `(` for THIS prefix
                        // (trim_start at EOF leaves no `)`); commit that,
                        // and park a fresh search iteration at search_from
                        // so the next line re-derives the rejection.
                        if park.is_none() {
                            park = Some(FnBodyResidualPark {
                                pos: state.search_from,
                                snapshot: FnBodyResidualState {
                                    phase: FnBodyPhase::Search,
                                    scan: CommentAwareScan::new(),
                                    depth: 0,
                                    search_from: state.search_from,
                                },
                            });
                        }
                    } else if chars[look] == ')' {
                        // Decided rejection (`name ()` signature): restart
                        // the search FRESH from just past the `(`, exactly
                        // the oracle's next first_unquoted_char iteration.
                        state.search_from = index + 1;
                        state.scan = CommentAwareScan::new();
                        index += 1;
                        continue;
                    }
                }
                // Accept: the delimiter is fixed forever; compute the
                // signature ONCE and start the depth fold fresh AT the
                // delimiter (the oracle's unquoted_delimiter_depth(source[d..]).
                let signature = fnbody_signature_before(chars, index, pending);
                state.phase = FnBodyPhase::Depth { signature };
                state.scan = CommentAwareScan::new();
                state.depth = 0;
            }
        }
        if let FnBodyPhase::Depth { .. } = state.phase {
            if state.scan.active(ch) {
                if ch == delim {
                    state.depth += 1;
                } else if ch == close {
                    state.depth = state.depth.saturating_sub(1);
                }
            }
            index += 1;
            continue;
        }
        index += 1;
    }
    park
}

/// The signature check the oracle runs over `source[..delimiter].trim_end()`
/// (stdin_source_has_unclosed_function_delimited_body's tail): the peeled
/// `name ()` form or the `function NAME` keyword form. `delimiter` is a
/// CHAR index into the pending mirror; `pending` carries the same text.
fn fnbody_signature_before(chars: &[char], delimiter: usize, pending: &str) -> bool {
    let byte = pending
        .char_indices()
        .nth(delimiter)
        .map(|(byte, _)| byte)
        .unwrap_or(pending.len());
    let signature = pending[..byte].trim_end();
    function_body_opener_signature(signature)
}

/// Signature forms shared by stdin_source_has_unclosed_function_delimited_body
/// (the oracle) and the incremental FnBody scanner (perf9).
fn function_body_opener_signature(signature: &str) -> bool {
    // Word-initial unquoted `#` comment spans never become tokens (the
    // strip helper below), so a head like `function d # note` still reads
    // as a signature awaiting its body delimiter (rubash#388).
    let signature = strip_word_initial_comment_spans(signature);
    // Same fall-through as stdin_source_is_function_signature: a
    // `function f()` header peels to `function f`, which is not a single
    // WORD, so the keyword-form check below must still run
    // (parse.y:1056 FUNCTION WORD '(' ')' newline_list function_body).
    if let Some(before_close) = signature.strip_suffix(')') {
        if let Some(name) = before_close.trim_end().strip_suffix('(') {
            if is_stdin_function_name(name.trim_end()) {
                return true;
            }
        }
    }

    signature
        .strip_prefix("function ")
        .map(function_keyword_operand_name)
        .is_some_and(is_stdin_function_keyword_name)
}

/// The parked text scanners of the group reader's candidate-line
/// completeness battery (perf9, #292B third wave).
///
/// `stdin_source_text_needs_more(pending, posix)` re-ran every text
/// predicate over the WHOLE accumulated group per candidate line —
/// measured at 3.56 G chars per arm for GNU bash's configure (`-n` 22.7 s
/// vs GNU 5.3.0's 37 ms). This struct mirrors `pending` char-for-char
/// (append-only: only push_raw_line ever extends it) and answers each
/// predicate from a checkpointed scan machine that advances over the
/// appended tail, parking wherever a forward decision
/// (`$(`/`${`/backtick/`[`/`esac)` lookaheads, buffer-tail two-char
/// lookaheads) is not yet decided by the text read so far. The machines
/// live next to the #292 family in lexer/continuation.rs; the oracles
/// (has_unclosed_quotes, has_unclosed_command_substitution +
/// command_substitutions_balanced, unclosed_array_subscript_line,
/// unclosed_input_close_char_posix, the function-body checks) stay
/// authoritative for every other call site, and the per-prefix equivalence
/// is enforced by the incremental tests there.
struct GroupTextScans {
    /// Exact char mirror of the accumulated `pending` group text.
    chars: Vec<char>,
    posix: bool,
    quotes: Option<ScanCheckpoint<crate::lexer::QuotesResidualState>>,
    comsub: Option<ScanCheckpoint<crate::lexer::ComsubResidualState>>,
    balanced: Option<ScanCheckpoint<crate::lexer::BalancedResidualState>>,
    subscript: Option<ScanCheckpoint<crate::lexer::SubscriptResidualState>>,
    close_char: Option<ScanCheckpoint<crate::lexer::CloseCharResidualState>>,
    fnbody_brace: Option<ScanCheckpoint<FnBodyResidualState>>,
    fnbody_paren: Option<ScanCheckpoint<FnBodyResidualState>>,
}

impl GroupTextScans {
    fn new(posix: bool) -> Self {
        Self {
            chars: Vec::new(),
            posix,
            quotes: Some(ScanCheckpoint {
                resume: 0,
                snapshot: crate::lexer::QuotesResidualState::default(),
            }),
            comsub: Some(ScanCheckpoint {
                resume: 0,
                snapshot: crate::lexer::ComsubResidualState::default(),
            }),
            balanced: Some(ScanCheckpoint {
                resume: 0,
                snapshot: crate::lexer::BalancedResidualState::default(),
            }),
            subscript: Some(ScanCheckpoint {
                resume: 0,
                snapshot: crate::lexer::SubscriptResidualState::default(),
            }),
            close_char: Some(ScanCheckpoint {
                resume: 0,
                snapshot: crate::lexer::CloseCharResidualState::default(),
            }),
            fnbody_brace: Some(ScanCheckpoint {
                resume: 0,
                snapshot: FnBodyResidualState::default(),
            }),
            fnbody_paren: Some(ScanCheckpoint {
                resume: 0,
                snapshot: FnBodyResidualState::default(),
            }),
        }
    }

    /// Extend the mirror with one raw group line (the same `raw` the caller
    /// pushed into `pending`, newline included).
    fn push_raw_line(&mut self, raw: &str) {
        self.chars.extend(raw.chars());
    }

    /// The parked equivalent of `stdin_source_text_needs_more(pending,
    /// posix)`: arm for arm the same predicates, each answered from its
    /// checkpoint. `pending` must be exactly the text whose chars were
    /// pushed so far (the signature-once check inside the function-body
    /// scanners reads it).
    ///
    /// `skip_quote_comsub` (perf21 battery fusion): the caller's feeder
    /// proved the just-appended line closed every quote and command
    /// substitution over the append-only twin of this pending text, with no
    /// undecided unit parked. The battery then takes that answer for its
    /// quotes and comsub/balanced arms instead of re-advancing the duplicate
    /// machines: at a line terminator both machines' states from a closed,
    /// unparked position are exactly `QuotesResidualState::default()` /
    /// closed (the '\n' arms only reset `comment_start`, and a closed
    /// quotes gate means the substitution machine saw no open `$(`/backtick
    /// unit), so recording that state at the new mirror end reproduces the
    /// machines' own advance bit for bit. The comsub/balanced checkpoints
    /// are simply left where they were — a later dirty line re-advances
    /// them from there, exactly what the full scan would re-derive.
    fn needs_more(&mut self, pending: &str, skip_quote_comsub: bool) -> bool {
        // has_unclosed_input_syntax_posix:
        if skip_quote_comsub {
            // perf21 battery fusion: record the proven line-boundary state
            // (see the method doc) instead of advancing the duplicate
            // quotes machine.
            self.quotes = Some(ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: crate::lexer::QuotesResidualState::default(),
            });
        } else if self.advance_quotes() {
            return true;
        }
        if !skip_quote_comsub && self.advance_comsub() && !self.advance_balanced() {
            return true;
        }
        if self.advance_subscript() {
            return true;
        }
        if self.advance_close_char() {
            return true;
        }
        // parse.y:5379-5384: trailing unquoted backslash keeps the line
        // open — an O(tail) fresh probe over the group text's end.
        if crate::lexer::stdin_line_ends_with_continuation(pending) {
            return true;
        }
        // O(tail) fresh probe (fast-reject on the last char / `function `
        // prefix; the peel only runs on signature-shaped tails).
        if stdin_source_is_function_signature(pending) {
            return true;
        }
        if self.advance_fnbody('{', pending) {
            return true;
        }
        if self.advance_fnbody('(', pending) {
            return true;
        }
        false
    }

    fn advance_quotes(&mut self) -> bool {
        let (resume, snapshot) = match self.quotes.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, crate::lexer::QuotesResidualState::default()),
        };
        let mut state = snapshot;
        let park = crate::lexer::quotes_residuals_advance(&self.chars, resume, &mut state);
        let open = state.is_open();
        self.quotes = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        open
    }

    fn advance_comsub(&mut self) -> bool {
        let (resume, snapshot) = match self.comsub.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, crate::lexer::ComsubResidualState::default()),
        };
        let mut state = snapshot;
        let park = crate::lexer::comsub_residuals_advance(&self.chars, resume, &mut state);
        let open = state.is_open();
        self.comsub = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        open
    }

    fn advance_balanced(&mut self) -> bool {
        let (resume, snapshot) = match self.balanced.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, crate::lexer::BalancedResidualState::default()),
        };
        let mut state = snapshot;
        let park = crate::lexer::balanced_residuals_advance(&self.chars, resume, &mut state);
        let balanced = !state.is_unbalanced();
        self.balanced = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        balanced
    }

    fn advance_subscript(&mut self) -> bool {
        let (resume, snapshot) = match self.subscript.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, crate::lexer::SubscriptResidualState::default()),
        };
        let mut state = snapshot;
        let park = crate::lexer::subscript_residuals_advance(&self.chars, resume, &mut state);
        let open = state.is_open();
        self.subscript = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        open
    }

    fn advance_close_char(&mut self) -> bool {
        let (resume, snapshot) = match self.close_char.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, crate::lexer::CloseCharResidualState::default()),
        };
        let mut state = snapshot;
        let park =
            crate::lexer::close_char_residuals_advance(&self.chars, resume, &mut state, self.posix);
        let open = state.is_open();
        self.close_char = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        open
    }

    fn advance_fnbody(&mut self, delim: char, pending: &str) -> bool {
        let slot = match delim {
            '{' => &mut self.fnbody_brace,
            _ => &mut self.fnbody_paren,
        };
        let (resume, snapshot) = match slot.take() {
            Some(checkpoint) => (checkpoint.resume, checkpoint.snapshot),
            None => (0, FnBodyResidualState::default()),
        };
        let mut state = snapshot;
        let park = fnbody_residuals_advance(&self.chars, resume, &mut state, delim, pending);
        let open = state.is_open();
        *slot = Some(match park {
            Some(park) => ScanCheckpoint {
                resume: park.pos,
                snapshot: park.snapshot,
            },
            None => ScanCheckpoint {
                resume: self.chars.len(),
                snapshot: state,
            },
        });
        open
    }
}

/// Process one syntactically complete group: expand (when history expansion
/// is on), record the delimited entry, then execute. Lines whose expansion
/// fails or is print-only do not execute; print-only lines still record.
fn run_history_group(
    executor: &mut Executor,
    session: &Rc<RefCell<SessionHistory>>,
    group: &[(String, bool)],
    start_line: usize,
    redirect_cmd: Option<&CommandNode>,
    interactive: bool,
    feeder_tokens: Option<Vec<Token>>,
) -> i32 {
    let history_on = executor.get_env("__RUBASH_SETOPT_history").as_deref() == Some("1");
    let histexpand_on = executor.get_env("__RUBASH_SETOPT_histexpand").as_deref() == Some("1");
    let posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let cmdhist = shopt_state_enabled(executor, "cmdhist", true);
    let lithist = shopt_state_enabled(executor, "lithist", false);
    let control = executor
        .get_env("HISTCONTROL")
        .unwrap_or_default()
        .to_string();
    let ignore = executor
        .get_env("HISTIGNORE")
        .unwrap_or_default()
        .to_string();
    let histsize = crate::history::SessionHistory::size_limit(executor.get_env("HISTSIZE"));
    let chars = executor.get_env("histchars").unwrap_or("!^#");
    let mut chars = chars.chars();
    let ctx = HistCtx {
        chars: HistChars {
            expand: chars.next().unwrap_or('!'),
            subst: chars.next().unwrap_or('^'),
            comment: chars.next().unwrap_or('#'),
        },
        posix,
    };

    let mut exec_parts: Vec<String> = Vec::new();
    let mut record_texts: Vec<Option<String>> = Vec::new();
    let mut modified_any = false;
    // Physical line of each group entry: every entry consumes at least one
    // input line; embedded newlines (quoted strings, heredoc bodies read as
    // one text) consume more.
    let mut physical_offset = 0usize;
    for (text, is_body) in group {
        let line_no = start_line + physical_offset;
        physical_offset += text.lines().count().max(1);
        if *is_body || !history_on || !histexpand_on {
            exec_parts.push(text.clone());
            // The recorded entry is only consumed when history is on;
            // skipping the clone halves this loop's per-line allocations
            // for history-off scripts (GNU bashhist.c pre_process_line is
            // itself gated on history being enabled).
            if history_on {
                record_texts.push(Some(text.clone()));
            }
            continue;
        }
        let result = session.borrow_mut().expand(text, ctx);
        match result.status {
            -1 => {
                // bashhist.c pre_process_line: failed history expansion is
                // an internal_error-class diagnostic reported at the line
                // being READ (current_command_line_count), so pin the
                // location to this entry's physical line, not the last
                // executed command's line.
                executor.set_current_line_value(line_no);
                let prefix = match (
                    executor.get_env("__RUBASH_SCRIPT_NAME"),
                    executor.get_env("__RUBASH_CURRENT_LINE"),
                ) {
                    (Some(script), Some(line)) => {
                        if executor.get_env("__RUBASH_EVAL_CONTEXT").is_some() {
                            format!("{script}: eval: line {line}: ")
                        } else {
                            format!("{script}: line {line}: ")
                        }
                    }
                    _ => "bash: ".to_string(),
                };
                eprintln!("{}{}", prefix, result.text);
                exec_parts.push(String::new());
                record_texts.push(None);
                // A dropped line changes the executed text even when no other
                // line expanded: the group must run from exec_parts, not the
                // verbatim text (bash does not execute the failed line).
                modified_any = true;
            }
            2 => {
                eprintln!("{}", result.text);
                exec_parts.push(String::new());
                record_texts.push(Some(result.text));
                // Print-only (:p) lines are never executed; use exec_parts so
                // the group text omits them entirely.
                modified_any = true;
            }
            status => {
                if status == 1 {
                    eprintln!("{}", result.text);
                    modified_any = true;
                }
                exec_parts.push(result.text.clone());
                record_texts.push(Some(result.text));
            }
        }
    }

    // Record the entry (bashhist.c history_delimiting_chars join).
    if history_on {
        if cmdhist {
            let mut record = build_recorded_entry(&record_texts, group, lithist);
            // bashhist.c:892-893: heredoc body lines (including the
            // delimiter) preserve their trailing newlines. The entry
            // ends with \n after the delimiter, producing a blank line
            // in the history listing (%5d%c %s\n adds another \n).
            if group.iter().any(|(_, is_body)| *is_body) && !record.ends_with('\n') {
                record.push('\n');
            }
            let was_recorded = if record.trim().is_empty() {
                false
            } else {
                session
                    .borrow_mut()
                    .record(&record, &control, &ignore, histsize)
            };

            // bashhist.c:961 really_add_history: recording the line resets
            // hist_last_line_pushed so `history -s` may pop it.
            {
                let mut shell = session.borrow_mut();
                shell.last_line_added = was_recorded;
                if was_recorded {
                    shell.last_line_pushed = false;
                }
            }
        } else {
            for text in record_texts.iter().flatten() {
                let was_recorded = session
                    .borrow_mut()
                    .record(text, &control, &ignore, histsize);
                let mut shell = session.borrow_mut();
                shell.last_line_added = was_recorded;
                if was_recorded {
                    shell.last_line_pushed = false;
                }
            }
        }
    } else {
        session.borrow_mut().last_line_added = false;
    }

    let exec_text = if modified_any {
        exec_parts.join("\n")
    } else {
        group
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let pre_alias_text = exec_text.clone();
    let exec_text = expand_group_aliases(executor, &exec_text);
    if exec_text.trim().is_empty() {
        return executor.last_exit_code();
    }
    // The group's words are final: executor-level alias expansion would
    // expand them a second time. Inner re-parses (eval, command
    // substitution, source, traps) suspend the marker and expand their own
    // input, matching GNU's per-stream alias handling.
    executor
        .shell_state
        .env_vars
        .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string());
    // perf19 token reuse: when nothing rewrote the group's text (history
    // expansion off or inert AND alias expansion an identity — the
    // exec_text == pre_alias_text comparison below already answers the
    // alias half) the gather feeder's committed tokens are byte-identical
    // to a fresh re-lex of exec_text (read_next_source_group's perf10
    // certification: complete break, no extglob toggle, non-alias arm).
    // GNU parse.y:3557 read_token streams once and never re-tokenizes
    // consumed text — the same reuse the `.` driver (perf10) applies. The
    // trailing per-logical-line separator is popped with the exact rule
    // tokenize_comsub_body_with_origin applies to the fresh stream.
    let reuse_tokens = if exec_text == pre_alias_text && !modified_any {
        feeder_tokens.map(|mut tokens| {
            if tokens
                .last()
                .is_some_and(|token| token.kind == TokenKind::Semicolon)
            {
                tokens.pop();
            }
            tokens
        })
    } else {
        None
    };
    let status = match reuse_tokens {
        Some(tokens) => run_source_pre_lexed_with_line_offset(
            executor,
            &exec_text,
            interactive,
            start_line.saturating_sub(1),
            redirect_cmd,
            None,
            tokens,
        ),
        None => run_source_with_line_offset(
            executor,
            &exec_text,
            interactive,
            start_line.saturating_sub(1),
            redirect_cmd,
            if exec_text == pre_alias_text {
                None
            } else {
                Some(pre_alias_text.as_str())
            },
        ),
    };
    executor
        .shell_state
        .env_vars
        .remove("__RUBASH_ALIAS_STREAMED");
    status
}

/// Join the recorded line texts with GNU history_delimiting_chars rules:
/// backslash continuation removes the backslash, heredoc bodies keep real
/// newlines, reserved words and operators join with a space, everything else
/// with semicolon-space. lithist saves newlines instead of semicolons.
fn build_recorded_entry(
    texts: &[Option<String>],
    group: &[(String, bool)],
    lithist: bool,
) -> String {
    const NO_SEMI: &[&str] = &[
        "{", "(", ")", "[", ";", "&", "|", "case", "do", "else", "if", "in", "then", "until",
        "while", "time",
    ];
    let mut out = String::new();
    let mut prev_kept: Option<usize> = None;
    // parse.y history_delimiting_chars: while a quoted construct opened on
    // an earlier line is still open (dstack delimiter is ' " or `), lines
    // join with a real newline, not "; ". Heredoc bodies never feed the
    // quote scanner (GNU reads them raw, PST_HEREDOC path).
    let mut quote_state: Option<char> = None;
    for (index, text) in texts.iter().enumerate() {
        let Some(text) = text else { continue };
        if out.is_empty() {
            out.push_str(text);
            prev_kept = Some(index);
            continue;
        }
        let prev_text = texts[prev_kept.unwrap_or(index)]
            .as_deref()
            .unwrap_or_default();
        let prev_was_body = index > 0 && group[index - 1].1;
        if !prev_was_body && index > 0 {
            quote_state = advance_quote_state(quote_state, &group[index - 1].0);
        }
        let cur_is_body = group[index].1;
        let delim = if quote_state.is_some() {
            "\n"
        } else if prev_text.ends_with('\\') {
            if out.ends_with('\\') {
                out.pop();
            }
            ""
        } else if prev_was_body || cur_is_body {
            "\n"
        } else if prev_text
            .split_whitespace()
            .next_back()
            .map(|word| NO_SEMI.contains(&word))
            .unwrap_or(false)
        {
            " "
        } else if lithist {
            "\n"
        } else {
            "; "
        };
        out.push_str(delim);
        out.push_str(text);
        prev_kept = Some(index);
    }
    out
}

/// Track the open quote delimiter across lines the way parse.y's dstack
/// does for history delimiting: returns the still-open ' " or ` delimiter,
/// or None when the text ends outside quotes. Backslash escapes work
/// outside single quotes; inside double quotes only the shell-escaped set.
fn advance_quote_state(state: Option<char>, text: &str) -> Option<char> {
    let mut state = state;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        match state {
            Some('\'') => {
                if c == '\'' {
                    state = None;
                }
            }
            Some('`') => {
                if c == '\\' && i + 1 < chars.len() {
                    i += 1;
                } else if c == '`' {
                    state = None;
                }
            }
            Some('"') => {
                if c == '\\'
                    && i + 1 < chars.len()
                    && matches!(chars[i + 1], '"' | '\\' | '$' | '`' | '\n')
                {
                    i += 1;
                } else if c == '"' {
                    state = None;
                }
            }
            _ => match c {
                '\\' => i += 1,
                '\'' | '"' | '`' => state = Some(c),
                '#' if i == 0 || matches!(chars[i - 1], ' ' | '\t' | ';' | '\n') => break,
                _ => {}
            },
        }
        i += 1;
    }
    state
}

/// builtins/shopt.rs SHOPT_STATE membership with the built-in default.
fn shopt_state_enabled(executor: &Executor, name: &str, default: bool) -> bool {
    match executor.get_env("__RUBASH_SHOPT_STATE") {
        None => default,
        Some(state) => state.split('\u{1f}').any(|entry| entry == name),
    }
}

pub fn stdin_script_errexit_enabled(executor: &Executor) -> bool {
    executor
        .get_env("SHELLOPTS")
        .is_some_and(|options| options.split(':').any(|option| option == "errexit"))
}

pub fn stdin_source_needs_more(source: &str) -> bool {
    stdin_source_needs_more_posix(source, false)
}

/// POSIX-aware variant: `set -o posix` changes `'` scanning inside
/// `"${...}"` (Interp 221), which decides whether the input is complete.
pub fn stdin_source_needs_more_posix(source: &str, posix: bool) -> bool {
    if stdin_source_text_needs_more(source, posix) {
        return true;
    }

    let tokens = tokenize(source);
    let mut stack = Vec::new();
    for token in &tokens {
        if token.kind != TokenKind::Keyword {
            continue;
        }
        match token.value.as_str() {
            "case" => stack.push("esac"),
            "if" => stack.push("fi"),
            "for" | "select" | "while" | "until" => stack.push("done"),
            // `{` at command position opens a brace group that must see
            // its `}` — GNU reads until the closing brace (parse.y
            // brace_group), so a multi-line `{ ... }` keeps the group open.
            "{" => stack.push("}"),
            "esac" | "fi" | "done" | "}" if stack.last() == Some(&token.value.as_str()) => {
                stack.pop();
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return true;
    }
    // GNU parse.y grammar consumes the newline AFTER a binary connector as
    // part of the same production — `list1: list1 AND_AND newline_list
    // list1` (parse.y:1286), `... OR_OR newline_list ...` (parse.y:1288),
    // `pipeline: pipeline '|' newline_list pipeline` (parse.y:1471) and
    // BAR_AND (parse.y:1473-1474) — so input whose last significant token
    // is `&&`, `||`, `|`, or `|&` is an INCOMPLETE command: the reader
    // keeps reading (PS2) instead of submitting the truncated text, whose
    // parse would die with `syntax error: unexpected end of file`
    // (rubash#255: ble.sh line 28570, the file's first line-spanning `&&`).
    // `&` and `;` do NOT continue — they terminate the command
    // (parse.y:1290 `list1 '&' newline_list list1` completes $1 first).
    // tokenize emits one Semicolon with line_break=true per physical line,
    // so skip those trailing separators before inspecting the last token.
    let last_significant = tokens
        .iter()
        .rev()
        .find(|token| !(token.kind == TokenKind::Semicolon && token.line_break))
        .map(|token| token.kind.clone());
    matches!(
        last_significant,
        Some(TokenKind::And)
            | Some(TokenKind::Or)
            | Some(TokenKind::Pipe)
            | Some(TokenKind::PipeErr)
    )
}

/// The text-level (non-tokenizing) half of `stdin_source_needs_more_posix`:
/// the completeness signals computable without the token stream. The
/// incremental group reader pairs this with a parked `GroupScanFeeder`
/// (src/lexer/mod.rs), which maintains the token-level half's keyword
/// stack and trailing-connector answer across appended lines.
pub fn stdin_source_text_needs_more(source: &str, posix: bool) -> bool {
    if crate::lexer::has_unclosed_input_syntax_posix(source, posix) {
        return true;
    }
    // parse.y:5379-5384: trailing unquoted backslash keeps the physical line
    // open (PS2 continuation) -- the joined line is assembled later by the
    // lexer logical-line loop, which removes the backslash-newline pair.
    if crate::lexer::stdin_line_ends_with_continuation(source) {
        return true;
    }
    if stdin_source_is_function_signature(source) {
        return true;
    }
    if stdin_source_has_unclosed_function_body(source) {
        return true;
    }
    false
}

/// Net open-paren count for one command line, ignoring quoted spans and
/// comments (parse.y:3922: while LEX_INCOMMENT, "don't bother counting
/// parens" — a `#` at word start runs to EOL). Used by the history driver
/// to keep a group open across a heredoc declared inside a process
/// substitution.
pub fn line_paren_delta(line: &str) -> i64 {
    // perf21: both counted bytes must be present for a non-zero delta; a
    // byte-gate admission turns the quote-aware walk into a memcmp-speed
    // scan for the majority of lines (GNU parse.y:3557 read_token streams
    // the input once; the walk below is this port's substitute).
    let bytes = line.as_bytes();
    if !bytes.contains(&b'(') && !bytes.contains(&b')') {
        return 0;
    }
    let mut scan = CommentAwareScan::new();
    let mut depth = 0i64;
    for c in line.chars() {
        if scan.active(c) {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            }
        }
    }
    depth
}

/// Net heredoc bodies still unread after processing `text`, which may span
/// multiple physical lines (an alias value can carry an entire heredoc:
/// `alias 'heredoc=cat <<EOF\nhello\nworld\nEOF'` — GNU reads the pushed
/// text on the same input stream, so the body lines inside it satisfy the
/// declaration and the next SOURCE line is normal input, parse.y:2055
/// push_string + here_document_to_fd). Lines consumed as body text are
/// never scanned for `<<` operators.
pub fn stdin_heredoc_declarations(text: &str) -> Vec<(String, bool)> {
    let mut pending: Vec<(String, bool)> = Vec::new();
    let mut arith_depth = 0i64;
    for line in text.split('\n') {
        if let Some((delimiter, strip_tabs)) = pending.first().cloned() {
            let candidate = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if candidate == delimiter {
                pending.remove(0);
            }
            continue;
        }
        pending.extend(scan_heredoc_operators_with_state(line, &mut arith_depth));
    }
    pending
}

/// rubash#351 oracle: the first line of `input` whose heredoc delimiter word
/// leaves a quote open, with that quote character and the 1-based line.
pub fn heredoc_delimiter_unclosed_quote(input: &str) -> Option<(char, usize)> {
    let mut arith_depth = 0i64;
    for (line_index, line) in input.split('\n').enumerate() {
        let (_, unclosed) = scan_heredoc_operators_full(line, &mut arith_depth);
        if let Some(quote) = unclosed {
            return Some((quote, line_index + 1));
        }
    }
    None
}

/// One physical line of the driver's heredoc-declaration scan, carrying the
/// open arithmetic-command paren depth across lines so a multi-line `((`
/// never leaks its shift operators as heredoc declarations.
pub fn stdin_heredoc_line_declarations(line: &str, arith_depth: &mut i64) -> Vec<(String, bool)> {
    scan_heredoc_operators_with_state(line, arith_depth)
}

/// `<<`/`<<-` operator declarations on one physical line. GNU parse.y
/// read_token: `<<` inside quotes is word text, not a redirection —
/// `alias lb='cat <<E2'` declares no heredoc. Unquoted < > | & ; ( ) are
/// token breaks (`x<<EOF` attaches EOF to x), `#` starts a comment only at
/// a word boundary, `$(...)`/`<(...)`/backquote bodies are reparsed so a
/// `<<` inside them is live syntax, and the delimiter is remembered
/// dequoted (quoting only suppresses body expansion, never the match).
///
/// GNU parse.y:3727-3728 read_token hands a `((' to parse_dparen
/// (parse.y:4895) BEFORE the redirection branch (parse.y:3690-3706) can lex
/// `<<`: the whole `((...))` body is arithmetic text consumed by
/// parse_arith_cmd/parse_matched_pair, so `<<`/`<<=` there are the shift
/// operators, never REDIR_LESSLESS (same class as rubash#181's
/// inside_arithmetic_command guard in the tokenizer). A `$((...))`
/// arithmetic expansion in a word gets the same treatment. Without this,
/// `((__ble_weight<<=1,...))` (ble.sh:6733) declared a phantom heredoc with
/// delimiter `=1,` that never terminates, so the group driver accumulated
/// the rest of the file into one pending group — O(n^2) re-scans per line
/// and a mis-parse of the merged text (rubash#244/#245: ble.sh -n died at
/// line 28355 after 117 s; GNU parses instantly, rc=0).
fn scan_heredoc_operators(line: &str) -> Vec<(String, bool)> {
    let mut arith_depth = 0i64;
    scan_heredoc_operators_with_state(line, &mut arith_depth)
}

fn scan_heredoc_operators_with_state(line: &str, arith_depth: &mut i64) -> Vec<(String, bool)> {
    scan_heredoc_operators_full(line, arith_depth).0
}

/// scan_heredoc_operators_with_state plus the unterminated-quote signal of
/// the delimiter word (rubash#351): GNU read_token_word (parse.y:5419-5437)
/// reads the here-document delimiter as an ordinary WORD, so a quote that
/// never closes keeps the read open to EOF — no heredoc is declared and the
/// parser dies with `unexpected EOF while looking for matching `q''.
fn scan_heredoc_operators_full(
    line: &str,
    arith_depth: &mut i64,
) -> (Vec<(String, bool)>, Option<char>) {
    // perf21 byte admission: every declaration arm needs a `<` byte, and the
    // only cross-line state (`arith_depth`) can be opened solely by a `(`
    // byte (`((` command position or `$((` word expansion — both arms below
    // read a `(` from the text). With neither byte present and depth already
    // 0, the walk below cannot declare anything and cannot move the state,
    // so skip the char materialization entirely (GNU read_token streams the
    // input once; this walk is the gather's per-line substitute).
    if *arith_depth == 0 {
        let bytes = line.as_bytes();
        if !bytes.contains(&b'<') && !bytes.contains(&b'(') {
            return (Vec::new(), None);
        }
    }
    // GNU read_token_word (parse.y:5419-5437) reads the heredoc delimiter
    // as an ordinary WORD: a quote that never closes keeps reading lines
    // until EOF, where the word read FAILS with `unexpected EOF while
    // looking for matching `q'' and NO heredoc is ever declared — the
    // redirection never gathers a body (rubash#351: `cat <<'E` + line `E`
    // was silently accepted as delimiter E with an EOF warning). An
    // unterminated quote in the delimiter therefore suppresses the
    // declaration and is reported for the EOF diagnostic.
    let mut unclosed_delimiter_quote: Option<char> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut declarations = Vec::new();
    let mut expect_delim: Option<bool> = None;
    let mut i = 0usize;
    // Command-position tracking for the `((` dispatch, mirroring GNU
    // parse_dparen's reserved_word_acceptable gate: a line start, an
    // operator boundary, or a command keyword puts a following `((` in
    // command position.
    let mut at_command = true;
    while i < chars.len() {
        if *arith_depth > 0 {
            // Arithmetic body: parens nest the span; quoted spans are
            // opaque; `<<` is the shift operator, never a heredoc.
            match chars[i] {
                '\\' => i += 2,
                '\'' | '"' => {
                    let q = chars[i];
                    i += 1;
                    while i < chars.len() && chars[i] != q {
                        if q == '"' && chars[i] == '\\' && i + 1 < chars.len() {
                            i += 1;
                        }
                        i += 1;
                    }
                    i += 1;
                }
                '(' => {
                    *arith_depth += 1;
                    i += 1;
                }
                ')' => {
                    *arith_depth -= 1;
                    i += 1;
                    if *arith_depth == 0 {
                        at_command = true;
                    }
                }
                _ => i += 1,
            }
            continue;
        }
        match chars[i] {
            ' ' | '\t' => i += 1,
            '#' => break,
            '$' if chars.get(i + 1) == Some(&'(') && chars.get(i + 2) == Some(&'(') => {
                // `$((` arithmetic expansion inside a word: the body is
                // arithmetic text (parse.y read_token_word ->
                // parse_matched_pair), so `<<` there never declares.
                *arith_depth = 2;
                i += 3;
            }
            '(' if chars.get(i + 1) == Some(&'(') && at_command => {
                // Command-position `((` arithmetic command: parse_dparen
                // consumes the whole body before read_token's redirection
                // branch ever runs (parse.y:3727-3728).
                *arith_depth = 2;
                i += 2;
            }
            '<' => {
                if chars.get(i + 1) == Some(&'<') {
                    match chars.get(i + 2) {
                        Some(&'<') => i += 3, // <<< herestring: no body
                        Some(&'-') => {
                            expect_delim = Some(true);
                            i += 3;
                        }
                        _ => {
                            expect_delim = Some(false);
                            i += 2;
                        }
                    }
                } else {
                    i += 1;
                }
                at_command = false;
            }
            '>' | '|' | '&' | ';' | '(' | ')' => {
                i += 1;
                at_command = true;
            }
            _ => {
                // One word, honoring quoting; comparison text is dequoted.
                let mut word = String::new();
                while i < chars.len() {
                    match chars[i] {
                        ' ' | '\t' | '<' | '>' | '|' | '&' | ';' | '(' | ')' => break,
                        '$' if chars.get(i + 1) == Some(&'(') && chars.get(i + 2) == Some(&'(') => {
                            // `$((` arithmetic expansion is one atomic word
                            // piece (read_token_word -> parse_matched_pair):
                            // consume the whole span into the word so its
                            // shift operators never lex as `<<` redirections
                            // (parse.y:3727 keeps the body away from
                            // read_token's redirection branch). A span still
                            // open at end of line propagates its depth into
                            // the carried state so continuation lines keep
                            // the suppression.
                            *arith_depth = 2;
                            word.push('$');
                            i += 1;
                            while i < chars.len() && *arith_depth > 0 {
                                match chars[i] {
                                    '\\' => {
                                        word.push(chars[i]);
                                        i += 1;
                                        if i < chars.len() {
                                            word.push(chars[i]);
                                            i += 1;
                                        }
                                    }
                                    '\'' | '"' => {
                                        let q = chars[i];
                                        word.push(chars[i]);
                                        i += 1;
                                        while i < chars.len() && chars[i] != q {
                                            if q == '"' && chars[i] == '\\' && i + 1 < chars.len() {
                                                word.push(chars[i]);
                                                i += 1;
                                            }
                                            word.push(chars[i]);
                                            i += 1;
                                        }
                                        if i < chars.len() {
                                            word.push(chars[i]);
                                            i += 1;
                                        }
                                    }
                                    '(' => {
                                        *arith_depth += 1;
                                        word.push(chars[i]);
                                        i += 1;
                                    }
                                    ')' => {
                                        *arith_depth -= 1;
                                        word.push(chars[i]);
                                        i += 1;
                                    }
                                    _ => {
                                        word.push(chars[i]);
                                        i += 1;
                                    }
                                }
                            }
                        }
                        '\\' => {
                            i += 1;
                            if i < chars.len() {
                                word.push(chars[i]);
                                i += 1;
                            }
                        }
                        '\'' | '"' => {
                            let q = chars[i];
                            i += 1;
                            while i < chars.len() && chars[i] != q {
                                if q == '"' && chars[i] == '\\' && i + 1 < chars.len() {
                                    i += 1;
                                }
                                word.push(chars[i]);
                                i += 1;
                            }
                            // Only the DELIMITER word (expect_delim still
                            // pending) carries the heredoc contract; an
                            // unclosed quote in any other word is the
                            // generic quote oracle's business (heredoc
                            // bodies are literal data).
                            if i >= chars.len() && expect_delim.is_some() {
                                unclosed_delimiter_quote = Some(q);
                            }
                            i += 1;
                        }
                        _ => {
                            word.push(chars[i]);
                            i += 1;
                        }
                    }
                }
                // A following `((` is in command position only after these
                // keywords (GNU reserved_word_acceptable word set).
                at_command = i >= chars.len()
                    || matches!(
                        word.as_str(),
                        "if" | "then"
                            | "else"
                            | "elif"
                            | "while"
                            | "until"
                            | "do"
                            | "in"
                            | "case"
                            | "!"
                            | "time"
                            | "coproc"
                            | "function"
                    );
                if let Some(strip_tabs) = expect_delim.take() {
                    if !word.is_empty() && unclosed_delimiter_quote.is_none() {
                        declarations.push((word, strip_tabs));
                    }
                }
            }
        }
    }
    (declarations, unclosed_delimiter_quote)
}

/// Extract the NAME from the operand of a `function` keyword header.
/// parse.y:1056-1061 `function_def`: `FUNCTION WORD`, `FUNCTION WORD '(' ')'`
/// — the operand is the WORD plus at most the optional whitespace-separated
/// parens pair, so `function f()` / `function f ()` are headers awaiting a
/// body exactly like `function f` (parse.y:1058). Any trailing content
/// after the name or parens is not part of a bare header.
fn function_keyword_operand_name(operand: &str) -> &str {
    let operand = operand.trim();
    match operand.strip_suffix(')') {
        Some(before) => before
            .trim_end()
            .strip_suffix('(')
            .map(str::trim_end)
            .unwrap_or(operand),
        None => operand,
    }
}

/// Strip every word-initial unquoted `#` comment SPAN — from the `#`
/// through end of line, the newline kept as the line separator — from a
/// function-head candidate text. GNU read_token's comment rule (the `#`
/// branch of read_token's fetch loop, parse.y:3631-3644: a `#` read at
/// token start, `!interactive || interactive_comments`, discards until
/// EOL) and read_token_word's word-start test (parse.y:3937-3940:
/// `retind == 0` or the previous word char is a newline or shellblank)
/// keep `function NAME<TAB># note` a function head awaiting its body —
/// the comment never becomes a token, so the group-completeness scanners
/// must not let it defeat the signature predicate (rubash#388).
///
/// Word-initial means token-initial: after IFS blanks AND after each
/// shell metacharacter `; & | ( ) < >` (every one of those is its own
/// single-character token in read_token's dispatch, so the next `#` is
/// fetched at token start — verified `f()# c` defines f, and
/// `echo a;#b` comments, on GNU 5.3.0 script files). A `#` glued into a
/// word (`d#x`) is name text; quote interiors are inert.
///
/// Spans are cut PER LINE, never to end of text: a head line's comment
/// hides only its own line, so a complete definition whose head carries
/// a comment still fails the head-only shape once its body lines
/// follow (cutting to end of text would glue the whole script into one
/// group — `function d # c\n{ :; }\nread x` must close at `}` so the
/// trailing `read` consumes the next script line like GNU).
fn strip_word_initial_comment_spans(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut word_start = true;
    let mut stripped: Option<String> = None;
    let mut copy_from = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            word_start = false;
            index += 1;
            continue;
        }
        if single {
            if byte == b'\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            match byte {
                b'"' => double = false,
                b'\\' => escaped = true,
                _ => {}
            }
            index += 1;
            continue;
        }
        match byte {
            b'\\' => {
                escaped = true;
                word_start = false;
            }
            b'\'' => {
                single = true;
                word_start = false;
            }
            b'"' => {
                double = true;
                word_start = false;
            }
            b' ' | b'\t' | b'\r' | b'\n' | b';' | b'&' | b'|' | b'(' | b')' | b'<' | b'>' => {
                word_start = true
            }
            b'#' if word_start => {
                let out = stripped.get_or_insert_with(|| String::with_capacity(text.len()));
                out.push_str(&text[copy_from..index]);
                // The comment runs through EOL (parse.y:3634
                // discard_until('\n')); the newline itself stays as the
                // line separator.
                let mut end = index + 1;
                while end < bytes.len() && bytes[end] != b'\n' {
                    end += 1;
                }
                copy_from = end;
                index = end;
                continue;
            }
            _ => word_start = false,
        }
        index += 1;
    }
    match stripped {
        Some(mut out) => {
            out.push_str(&text[copy_from..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(text),
    }
}

fn stdin_source_is_function_signature(source: &str) -> bool {
    let stripped = strip_word_initial_comment_spans(source);
    let trimmed = stripped.trim();
    // `name ()` / `name()` signature: peel the trailing parens with
    // optional whitespace (`'a b c' ( )' is still a signature — GNU's
    // grammar accepts any WORD; validity is judged at exec time). A
    // `function f()` header also peels here, but `function f` is not a
    // single WORD — fall through to the keyword-form check below instead
    // of returning (parse.y:1056 FUNCTION WORD '(' ')' newline_list
    // function_body).
    if let Some(before_close) = trimmed.strip_suffix(')') {
        if let Some(name) = before_close.trim_end().strip_suffix('(') {
            if is_stdin_function_name(name.trim_end()) {
                return true;
            }
        }
    }

    // Keyword form: `function NAME`, `function NAME()`, `function NAME ()`.
    trimmed
        .strip_prefix("function ")
        .map(function_keyword_operand_name)
        .is_some_and(is_stdin_function_keyword_name)
}

fn stdin_source_has_unclosed_function_body(source: &str) -> bool {
    stdin_source_has_unclosed_function_delimited_body(source, '{')
        || stdin_source_has_unclosed_function_delimited_body(source, '(')
}

fn stdin_source_has_unclosed_function_delimited_body(source: &str, delimiter: char) -> bool {
    let Some(open_delimiter) = first_unquoted_function_body_delimiter(source, delimiter) else {
        return false;
    };
    if unquoted_delimiter_depth(&source[open_delimiter..], delimiter) == 0 {
        return false;
    }

    let signature = source[..open_delimiter].trim_end();
    // Shared with the incremental FnBody scanner (perf9): the peeled
    // `name ()` form or the `function NAME` keyword form.
    function_body_opener_signature(signature)
}

/// `name ()` signature form: GNU accepts non-identifier words
/// (`11111 () { ...; }' is legal non-posix), but an `=`-bearing word is an
/// assignment-shaped token, not a name (`a=2 ()` is a syntax error).
/// `<( ... )` is lexed as one WORD in GNU (parse.y scans the
/// process-substitution shape inside a word), so `<( : ) () { }' is a
/// function named `<(:)' — allow the spaced form as a signature too;
/// validity is judged later by the executor.
fn is_stdin_function_name(name: &str) -> bool {
    if name == "!!" || (name.starts_with("<(") && name.ends_with(')')) {
        return true;
    }
    // A fully quoted name is one WORD in GNU's grammar (`'a b c' () { }'
    // parses as a definition and is rejected by the executor).
    if name.len() > 1
        && (name.starts_with('\'') && name.ends_with('\'')
            || name.starts_with('"') && name.ends_with('"'))
    {
        return true;
    }
    !name.is_empty()
        && !name.chars().any(|ch| {
            ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|' | '=')
        })
}

/// `function WORD` signature form: GNU function_def takes a single WORD
/// verbatim, so `=` and digit-leading names are legal (`function a=2`,
/// `function 11111`); only shell metacharacters end the word.
fn is_stdin_function_keyword_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

/// Quote/comment-aware character classification for the text-layer
/// completeness scanners (`first_unquoted_char`,
/// `unquoted_delimiter_depth`, `line_paren_delta`).
///
/// GNU never lets comment text enter quote state: an unquoted `#` at the
/// start of a word discards the rest of the line (parse.y:3630 shell_getc
/// comment branch — `#` when `!interactive || interactive_comments` sets
/// PST_COMMENT and discards until EOL), and read_token_word only enters
/// LEX_INCOMMENT when the `#` begins a word (`retind == 0` or the previous
/// word char is a newline or shellblank, parse.y:3937-3940) — while
/// LEX_INCOMMENT is set, "don't bother counting parens or doing anything
/// else" (parse.y:3922-3931), so quote characters inside comments are
/// inert. Scanning raw text without that rule lets an apostrophe inside a
/// comment (`# DragonflyBSD's ...`, `# it's known`) toggle single-quote
/// state and corrupt the brace/paren depth — a complete function body is
/// then misjudged as an unclosed group (rubash#282: modernish _IN/sig).
#[derive(Clone, Debug, PartialEq, Eq)]
struct CommentAwareScan {
    single: bool,
    double: bool,
    escaped: bool,
    in_comment: bool,
    prev: Option<char>,
}

impl CommentAwareScan {
    fn new() -> Self {
        CommentAwareScan {
            single: false,
            double: false,
            escaped: false,
            in_comment: false,
            prev: None,
        }
    }

    /// Feed the next character; `true` when it is ACTIVE — unquoted,
    /// unescaped, and outside comment text — so the scanner may count it
    /// as a delimiter. Comment state ends at the newline itself, which is
    /// fed through normally on the next call.
    fn active(&mut self, ch: char) -> bool {
        if self.in_comment {
            if ch == '\n' {
                self.in_comment = false;
                self.prev = Some('\n');
            }
            return false;
        }
        if self.escaped {
            self.escaped = false;
            self.prev = Some(ch);
            return false;
        }
        if self.single {
            if ch == '\'' {
                self.single = false;
            }
            self.prev = Some(ch);
            return false;
        }
        if ch == '\\' {
            self.escaped = true;
            self.prev = Some(ch);
            return false;
        }
        // parse.y:3937: `#` opens a comment only at word start.
        if ch == '#'
            && !self.double
            && self
                .prev
                .map_or(true, |p| p == '\n' || p == ' ' || p == '\t')
        {
            self.in_comment = true;
            self.prev = Some(ch);
            return false;
        }
        match ch {
            '\'' if !self.double => self.single = true,
            '"' if !self.double => self.double = true,
            '"' => self.double = false,
            _ => {}
        }
        let active = !self.single && !self.double && ch != '\'' && ch != '"';
        self.prev = Some(ch);
        active
    }
}

fn first_unquoted_char(source: &str, target: char) -> Option<usize> {
    let mut scan = CommentAwareScan::new();
    for (index, ch) in source.char_indices() {
        if scan.active(ch) && ch == target {
            return Some(index);
        }
    }
    None
}

fn first_unquoted_function_body_delimiter(source: &str, target: char) -> Option<usize> {
    let mut search_from = 0usize;
    while let Some(relative_index) = first_unquoted_char(&source[search_from..], target) {
        let index = search_from + relative_index;
        if target == '('
            && source[index + target.len_utf8()..]
                .trim_start()
                .starts_with(')')
        {
            search_from = index + target.len_utf8();
            continue;
        }
        return Some(index);
    }
    None
}

fn unquoted_delimiter_depth(source: &str, open: char) -> usize {
    let close = match open {
        '{' => '}',
        '(' => ')',
        _ => return 0,
    };
    let mut scan = CommentAwareScan::new();
    let mut depth = 0usize;
    for ch in source.chars() {
        if scan.active(ch) {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth = depth.saturating_sub(1);
            }
        }
    }
    depth
}

pub fn run_source(executor: &mut Executor, input: &str, interactive: bool) -> i32 {
    run_source_with_line_offset(executor, input, interactive, 0, None, None)
}

/// perf19 token-reuse entry: identical to [`run_source_with_line_offset`]
/// except the caller supplies the group's ALREADY-COMMITTED token stream
/// (the gather feeder's output, perf10 model) instead of re-lexing
/// `input`. Eligibility is the caller's contract — the same conditions
/// read_next_source_group documents for its returned feeder tokens
/// (complete-group break, no effective extglob toggle, non-alias arm so
/// `input` is byte-identical to the text the feeder scanned, and nothing
/// rewrote the text in between): GNU parse.y:3557 read_token streams the
/// input ONCE and never re-tokenizes consumed text; a certified token
/// stream is that model's stand-in, exactly as the `.` source driver
/// (builtins/source/execution.rs, perf10) already consumes it.
pub(crate) fn run_source_pre_lexed_with_line_offset(
    executor: &mut Executor,
    input: &str,
    interactive: bool,
    line_offset: usize,
    redirect_cmd: Option<&CommandNode>,
    diagnostic_text: Option<&str>,
    pre_lexed: Vec<Token>,
) -> i32 {
    run_source_impl(
        executor,
        input,
        interactive,
        line_offset,
        redirect_cmd,
        diagnostic_text,
        Some(pre_lexed),
    )
}

/// rubash#372: GNU decides command-position `(' completeness by parsing
/// (parse.y read_token drives the subshell grammar), so a command-`('
/// unclosed verdict from the text-level close-char scanner may be
/// disproved by the real parser: tokenize and parse the whole input
/// exactly as the normal execution path would and report whether the
/// resulting AST is free of parse-error nodes. Other verdict shapes
/// (quotes, `${`, `$(`, array lists, `[` subscripts) are read-time
/// matched-pair verdicts the tokenizer does not model and are never
/// overridden here.
fn command_paren_verdict_disproved_by_parser(input: &str, parse_posix: bool) -> bool {
    let Some((close, _, _, _, command, _)) =
        crate::lexer::unclosed_input_close_char_posix(input, parse_posix)
    else {
        return false;
    };
    if close != ')' || !command {
        return false;
    }
    let tokens = tokenize_with_initial_posix(input, parse_posix);
    if crate::lexer::heredoc_overflow_line().is_some() {
        return false;
    }
    let ast = crate::parser::parse_with_options(
        &tokens,
        crate::parser::ParseLoopOptions {
            stray_close_is_error: true,
            source_text: Some(input.into()),
            diagnostic_text: None,
            source_line_offset: 0,
        },
    );
    !ast.commands.iter().any(command_tree_has_parse_error)
}

/// True when this command node, or any command nested inside it (pipeline
/// stages, and-or lists, compound bodies, clause bodies), carries one of
/// the parser's error markers. Mirrors the marker set the executor's
/// diagnostics read (command_execute.rs `__RUBASH_PARSE_ERROR*` arms).
fn command_tree_has_parse_error(command: &CommandNode) -> bool {
    if command
        .assignments
        .iter()
        .any(|(name, _)| name.starts_with("__RUBASH_PARSE_ERROR"))
    {
        return true;
    }
    let mut nested = |body: &[CommandNode]| body.iter().any(command_tree_has_parse_error);
    command
        .pipeline_command
        .as_ref()
        .is_some_and(|pipeline| nested(&pipeline.stages))
        || command
            .and_or_list
            .as_ref()
            .is_some_and(|list| nested(&list.commands))
        || command
            .time_command
            .as_ref()
            .is_some_and(|time| command_tree_has_parse_error(&time.command))
        || command
            .background_command
            .as_ref()
            .is_some_and(|background| command_tree_has_parse_error(&background.command))
        || command
            .inverted_command
            .as_ref()
            .is_some_and(|inverted| command_tree_has_parse_error(&inverted.command))
        || command
            .for_command
            .as_ref()
            .is_some_and(|for_command| nested(&for_command.body))
        || command.if_command.as_ref().is_some_and(|if_command| {
            nested(&if_command.condition)
                || nested(&if_command.then_body)
                || if_command
                    .elif_branches
                    .iter()
                    .any(|branch| nested(&branch.condition) || nested(&branch.body))
                || if_command
                    .else_body
                    .as_ref()
                    .is_some_and(|else_body| nested(else_body))
        })
        || command.loop_command.as_ref().is_some_and(|loop_command| {
            nested(&loop_command.condition) || nested(&loop_command.body)
        })
        || command
            .subshell_command
            .as_ref()
            .is_some_and(|subshell| nested(&subshell.body))
        || command
            .case_command
            .as_ref()
            .is_some_and(|case| case.clauses.iter().any(|clause| nested(&clause.body)))
        || command
            .select_command
            .as_ref()
            .is_some_and(|select| nested(&select.body))
        || command
            .function_command
            .as_ref()
            .is_some_and(|function| nested(&function.body))
        || command
            .brace_group
            .as_ref()
            .is_some_and(|brace_group| nested(&brace_group.body))
        || command
            .coproc_command
            .as_ref()
            .is_some_and(|coproc| coproc.body.as_ref().is_some_and(|body| nested(body)))
}

pub fn run_source_with_line_offset(
    executor: &mut Executor,
    input: &str,
    interactive: bool,
    line_offset: usize,
    redirect_cmd: Option<&CommandNode>,
    diagnostic_text: Option<&str>,
) -> i32 {
    run_source_impl(
        executor,
        input,
        interactive,
        line_offset,
        redirect_cmd,
        diagnostic_text,
        None,
    )
}

fn run_source_impl(
    executor: &mut Executor,
    input: &str,
    interactive: bool,
    line_offset: usize,
    redirect_cmd: Option<&CommandNode>,
    diagnostic_text: Option<&str>,
    pre_lexed: Option<Vec<Token>>,
) -> i32 {
    // TODO(shell.c/eval.c/parse.y): GNU Bash parses complete command streams,
    // including pending here-documents, rather than executing script files one
    // physical line at a time. This keeps batch input whole; interactive mode
    // still feeds one line at a time from the REPL.
    // The heredoc collector must see the complete script before command
    // substitution balance is checked: parentheses in a heredoc body are
    // literal data, not shell syntax.
    let parse_posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    // rubash#351: a `<<` whose delimiter word carries an unterminated quote
    // fails the WORD read at EOF — GNU read_token_word (parse.y:5419-5437)
    // reports `unexpected EOF while looking for matching `q'' and gathers
    // no heredoc. The blanket `!input.contains("<<")` guard on the generic
    // unclosed route below excludes every heredoc-bearing input, so the
    // delimiter shape is detected and reported here first (GNU runs the
    // complete lines before the failing one, like the other unclosed arms).
    if pre_lexed.is_none() && !interactive {
        if let Some((quote, open_line)) = heredoc_delimiter_unclosed_quote(input) {
            let source = input.trim_end_matches('\n');
            let prefix = if open_line > 1 {
                source
                    .lines()
                    .take(open_line - 1)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            };
            if !prefix.trim().is_empty() {
                let _ = run_source_with_line_offset(
                    executor,
                    &prefix,
                    interactive,
                    line_offset,
                    redirect_cmd,
                    diagnostic_text,
                );
            }
            executor.mark_parse_error();
            eprintln!(
                "{}unexpected EOF while looking for matching `{quote}'",
                executor.parser_diagnostic_prefix_for_line(open_line)
            );
            return 2;
        }
    }
    // perf19: a pre-lexed group broke COMPLETE at the gather (the
    // feeder-token certification) — the same predicate family
    // (has_unclosed_quotes / comsub / subscript / close-char) already
    // answered closed over this exact text at the break line, and the
    // final-newline difference between the pending mirror and `input`
    // cannot open any construct. GNU never re-reads consumed text
    // (parse.y:3557 read_token); the fresh whole-group rescan is only the
    // unclosed-diagnostics route's gate, which a complete group skips.
    // rubash#372 (rubash#117 move 2 — converge to the real parser): the
    // unclosed scanners are text-level fast paths for GNU's read-time EOF
    // diagnostics; their case-word machine has no pattern-region state
    // (GNU parse.y:3177-3186 CHECK_FOR_RESERVED_WORD is suppressed inside
    // PST_CASEPAT, parser.h:29), so a compound keyword in PATTERN position
    // (`case x in (a|case) ...`) counts as a real `case' and the scanner
    // reports a phantom unclosed command `('. GNU decides `(' completeness
    // by PARSING (parse.y read_token drives the subshell grammar); when
    // the real parser produces a clean AST over the whole input, the
    // command-`(' verdict is disproved and the normal execution path must
    // run. Only this verdict shape is overridable: quotes, `${`, `$(`,
    // array lists and `[' subscripts are read_token_word/parse_matched_
    // pair verdicts (parse.y:5419-5437), where rubash's tokenizer folds
    // instead of failing and the parser is NOT the authority. The
    // verification only pays on inputs the scanner flagged (and never on
    // heredoc inputs, which the gate already excludes).
    if pre_lexed.is_none()
        && !interactive
        && crate::lexer::has_unclosed_input_syntax_posix(input, parse_posix)
        && !input.contains("<<")
        && !command_paren_verdict_disproved_by_parser(input, parse_posix)
    {
        // GNU parse.y:5635-5643 read_token_word + parse_matched_pair
        // (parse.y:3906-3912): an unclosed array subscript `[` reports
        // `unexpected EOF while looking for matching `]'' at the line the
        // `[` opened and exits 1 — and the scan swallows any later `)`,
        // so it must be reported ahead of the generic close-char path
        // (rubash#221: `foo=([)` was silently accepted).
        if let Some((open_line, inside_compassign)) =
            crate::lexer::unclosed_array_subscript_line(input)
        {
            let source = input.trim_end_matches('\n');
            let prefix = if open_line > 1 {
                source
                    .lines()
                    .take(open_line - 1)
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            };
            if !prefix.trim().is_empty() {
                let _ = run_source_with_line_offset(
                    executor,
                    &prefix,
                    interactive,
                    line_offset,
                    redirect_cmd,
                    diagnostic_text,
                );
            }
            executor.mark_parse_error();
            eprintln!(
                "{}unexpected EOF while looking for matching `]'",
                executor.parser_diagnostic_prefix_for_line(open_line)
            );
            // GNU exits 1 when the subscript opened inside a compound array
            // assignment (parse_compound_assignment EOF family) and 2 for a
            // command word (parse.y:3567 read_token error path) — but
            // error.c:324-327 forces exit status 2 whenever errexit is live
            // at the parser_error (rubash#306).
            if inside_compassign && executor.errexit_live_at_diagnostic() {
                return 2;
            }
            return if inside_compassign { 1 } else { 2 };
        }
        // GNU's incremental reader executes complete input lines before the
        // line where the unclosed construct opened; that line itself is part
        // of the failed parse and runs nothing.
        let unclosed = crate::lexer::unclosed_input_close_char_posix(input, parse_posix);
        let cut_line = unclosed.map(|(_, open, _, _, _, _)| open);
        let source = input.trim_end_matches('\n');
        let prefix = match cut_line {
            Some(open) if open > 1 => source.lines().take(open - 1).collect::<Vec<_>>().join("\n"),
            Some(_) => String::new(),
            None => source
                .rsplit_once('\n')
                .map(|(prefix, _)| prefix.to_string())
                .unwrap_or_default(),
        };
        if !prefix.trim().is_empty() {
            let _ = run_source_with_line_offset(
                executor,
                &prefix,
                interactive,
                line_offset,
                redirect_cmd,
                diagnostic_text,
            );
        }
        executor.mark_parse_error();
        // GNU parse.y names the close delimiter of the innermost unclosed
        // matched-pair construct ("unexpected EOF while looking for
        // matching `}'"); only an unrecognized residue falls back to the
        // generic end-of-file diagnostic.
        match unclosed {
            Some((close, open_line, eof_line, report_open, command, array_list)) => {
                // GNU parse.y yyerror (parse.y:6890-6901): an unterminated
                // command-position `(` names the construct — "unexpected
                // end of file from `(' command on line N" — while
                // `$(`/`$((`/array/quote matched pairs keep the "matching
                // `X'" wording.
                if close == ')' && command {
                    eprintln!(
                        "{}syntax error: unexpected end of file from `(' command on line {open_line}",
                        executor.parser_diagnostic_prefix_for_line(eof_line)
                    );
                } else {
                    let reported = if report_open { open_line } else { eof_line };
                    eprintln!(
                        "{}unexpected EOF while looking for matching `{close}'",
                        executor.parser_diagnostic_prefix_for_line(reported)
                    );
                }
                // GNU exits 1 for an unterminated `name=(` array list —
                // but error.c:324-327 forces exit status 2 whenever
                // errexit is live at the parser_error (rubash#306).
                if array_list && executor.errexit_live_at_diagnostic() {
                    return 2;
                }
                if array_list {
                    return 1;
                }
            }
            None => eprintln!("rubash: syntax error: unexpected end of file"),
        }
        return 2;
    }

    // perf19: pre_lexed is the gather feeder's committed stream for THIS
    // group (see run_source_pre_lexed_with_line_offset's doc) — tokenizing
    // `input` again would re-read every byte the feeder already scanned.
    let mut tokens = match pre_lexed {
        Some(tokens) => tokens,
        None => tokenize_with_initial_posix(input, parse_posix),
    };
    // A command with more than HEREDOC_MAX (16) here-documents is fatal in
    // GNU (parse.y push_heredoc -> report_syntax_error + exit_shell with
    // EX_BADUSAGE). The lexer only returns tokens, so it parks the condition
    // for us: report it with the script-relative line and exit 2 without
    // running anything.
    if let Some(line) = crate::lexer::heredoc_overflow_line() {
        executor.mark_parse_error();
        eprintln!(
            "{}maximum here-document count exceeded",
            executor.parser_diagnostic_prefix_for_line(line)
        );
        return 2;
    }
    if line_offset != 0 {
        for token in &mut tokens {
            token.position += line_offset;
            token.column += line_offset;
        }
    }
    // GNU parse.y: a `)` or case-clause terminator at command position is
    // `syntax error near unexpected token` (yyerror aborts the input). The
    // lexer folds multi-line `$(...)` bodies, so a top-level `)` token is
    // genuinely stray (comsub6.sub `math1)` after alias expansion).
    let mut ast = crate::parser::parse_with_options(
        &tokens,
        crate::parser::ParseLoopOptions {
            stray_close_is_error: true,
            source_text: Some(input.into()),
            diagnostic_text: diagnostic_text.map(std::rc::Rc::from),
            source_line_offset: line_offset,
        },
    );
    if let Some(cmd) = redirect_cmd {
        if let Err(error) = executor.apply_inherited_command_output_redirects(cmd, &mut ast) {
            eprintln!("{error}");
            return 1;
        }
    }

    match executor.execute_ast(&ast) {
        Ok(()) => executor.last_exit_code(),
        // ExitCode/FatalFunctionError reached the list top: GNU unwound via
        // jump_to_top_level (exit, errexit, POSIX special-builtin failure).
        // The grouped drivers must stop reading — record it before the
        // status conversion loses the distinction.
        Err(ExecuteError::ExitCode(code)) => {
            executor.exit_jump_pending.set(true);
            code
        }
        Err(ExecuteError::FatalFunctionError(code)) => {
            executor.exit_jump_pending.set(true);
            code
        }
        // ExpansionFailure is GNU's DISCARD: the command list aborted but
        // the reader continues with the next complete command.
        Err(ExecuteError::ExpansionFailure(code)) => code,
        Err(e) => {
            if interactive {
                eprintln!("Error: {}", e);
            } else {
                eprintln!("{}", e);
            }
            1
        }
    }
}

// ===========================================================================
// Interactive stdin driver (non-tty `bash -i`) — moved from main.rs so
// product shells (niu) can delegate forced-interactive piped input here
// instead of driving a terminal REPL that cannot run without a tty.
// ===========================================================================

/// (rows, cols) of the console attached to `handle`, or None when the
/// handle is not a console. Only the visible window rectangle is used —
/// TIOCGWINSZ semantics, not the (usually wider) buffer dwSize.
#[cfg(windows)]
fn windows_console_size_of(handle: *mut std::ffi::c_void) -> Option<(i32, i32)> {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Coord {
        x: i16,
        y: i16,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ConsoleScreenBufferInfo {
        dw_size: Coord,
        dw_cursor_position: Coord,
        w_attributes: u16,
        sr_window: SmallRect,
        dw_maximum_window_size: Coord,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleScreenBufferInfo(
            console_output: *mut std::ffi::c_void,
            console_screen_buffer_info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
    }
    let mut info = ConsoleScreenBufferInfo::default();
    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return None;
    }
    let SmallRect {
        left,
        top,
        right,
        bottom,
    } = info.sr_window;
    let rows = i32::from(bottom) - i32::from(top) + 1;
    let cols = i32::from(right) - i32::from(left) + 1;
    Some((rows, cols))
}

#[cfg(not(windows))]
fn windows_console_size_of(_handle: *mut std::ffi::c_void) -> Option<(i32, i32)> {
    None
}

/// Windows analog of the controlling-terminal winsize query
/// (winsize.c:97 input_tty -> tcgetwinsize): the process console, found via
/// CONOUT$ so it works even when stdin/stdout/stderr are all redirected,
/// with the std handles as fallbacks for consoles without a CONOUT$ open.
#[cfg(windows)]
fn process_console_size() -> Option<(i32, i32)> {
    use std::os::windows::io::AsRawHandle;
    if let Ok(conout) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
    {
        if let Some(size) = windows_console_size_of(conout.as_raw_handle() as *mut _) {
            return Some(size);
        }
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> *mut std::ffi::c_void;
    }
    // STD_INPUT_HANDLE / STD_OUTPUT_HANDLE / STD_ERROR_HANDLE
    for std_handle in [-10i32 as u32, -11i32 as u32, -12i32 as u32] {
        let handle = unsafe { GetStdHandle(std_handle) };
        if !handle.is_null() {
            if let Some(size) = windows_console_size_of(handle) {
                return Some(size);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn process_console_size() -> Option<(i32, i32)> {
    None
}

/// Size of the console on STDIN itself (readline's rl_instream winsize).
#[cfg(windows)]
fn stdin_console_size() -> Option<(i32, i32)> {
    use std::os::windows::io::AsRawHandle;
    windows_console_size_of(std::io::stdin().as_raw_handle() as *mut _)
}

#[cfg(not(windows))]
fn stdin_console_size() -> Option<(i32, i32)> {
    None
}

/// variables.c:1040 sh_set_lines_and_columns: bind LINES then COLUMNS as
/// (non-exported) shell variables. bind_variable does not export, so the
/// process environment is only updated for names the parent had already
/// exported (an exported var stays exported across rebinds).
fn bind_lines_columns(executor: &mut Executor, lines: i32, columns: i32) {
    executor
        .shell_state
        .env_vars
        .insert("LINES".to_string(), lines.to_string());
    executor
        .shell_state
        .env_vars
        .insert("COLUMNS".to_string(), columns.to_string());
    if std::env::var_os("LINES").is_some() {
        std::env::set_var("LINES", lines.to_string());
    }
    if std::env::var_os("COLUMNS").is_some() {
        std::env::set_var("COLUMNS", columns.to_string());
    }
}

/// jobs.c:2618 get_tty_state -> jobs.c:2647-2648 `if (check_window_size)
/// get_new_window_size(0, NULL, NULL)` -> winsize.c:98-100: with checkwinsize
/// on (default, config-top.h:148 CHECKWINSIZE_DEFAULT 1), an interactive
/// shell binds LINES/COLUMNS from the controlling terminal's winsize
/// (ws_row > 0 && ws_col > 0). Called from initialize_job_control's mirror
/// (pre-startup-files, shell.c:1969 -> jobs.c:4871 `if (interactive)
/// get_tty_state ()`) and again after the startup files (shell.c:816) —
/// rubash's post-file site is prepare_interactive_history, the port of the
/// shell.c:799-811 block that precedes it. Verified interactive-only: GNU
/// never binds COLUMNS/LINES for non-interactive shells
/// (jobs.c:4871 guard).
pub fn bind_interactive_console_winsize(executor: &mut Executor) {
    if !crate::builtins::shopt::option_enabled(&executor.shell_state.env_vars, "checkwinsize") {
        return;
    }
    let Some((rows, cols)) = process_console_size() else {
        return;
    };
    // winsize.c:98: both dimensions must be positive for the bind to fire.
    if rows > 0 && cols > 0 {
        bind_lines_columns(executor, rows, cols);
    }
}

/// lib/readline/terminal.c:293-374 _rl_get_screen_size — the bind readline
/// performs when it initializes at the first prompt (bashline.c:524
/// initialize_readline -> rl_initialize readline.c:1313 ->
/// _rl_init_terminal_io terminal.c:646 -> _rl_get_screen_size ->
/// sh_set_lines_and_columns terminal.c:374). Precedence, per the C:
///   1. winsize of rl_instream (stdin): width=ws_col, height=ws_row —
///      recomputed unconditionally when stdin itself is a console;
///   2. otherwise the query runs only when no screen size is set yet
///      (terminal.c:640-643 application-supplied size wins): positive
///      LINES/COLUMNS shell values bound by the console path survive;
///   3. env COLUMNS / env LINES (getenv, not shell vars) when unset;
///   4. the 80x24 defaults (terminal.c:366-371), with `width <= 1 -> 80`
///      applied last so a degenerate 1-column console still binds 80.
/// Windows has no termcap/tgetnum("co"/"li") step; the defaults 80/24
/// replace it exactly (GNU with tgetent but no winsize also lands on
/// 80/24, verified: no-ctty WSL GNU binds COLUMNS=80 LINES=24 at the
/// first prompt while the rcfile still sees them unset).
pub fn bind_interactive_screen_size(executor: &mut Executor) {
    // terminal.c:318-319 with a working stdin ioctl: wc/ws_col, wr/ws_row.
    if let Some((rows, cols)) = stdin_console_size() {
        let mut width = cols;
        let mut height = rows;
        // terminal.c:366-371.
        if width <= 1 {
            width = 80;
        }
        if height <= 0 {
            height = 24;
        }
        bind_lines_columns(executor, height, width);
        return;
    }
    // terminal.c:640-643: an application-provided size (winsize.c:103
    // rl_set_screen_size from get_new_window_size, or sv_winsize from a
    // COLUMNS/LINES assignment) suppresses the query when both sides are
    // already positive.
    let columns_bound = executor
        .get_env("COLUMNS")
        .and_then(|v| v.parse::<i32>().ok())
        .is_some_and(|v| v > 0);
    let lines_bound = executor
        .get_env("LINES")
        .and_then(|v| v.parse::<i32>().ok())
        .is_some_and(|v| v > 0);
    if columns_bound && lines_bound {
        return;
    }
    let env_positive = |name: &str| {
        std::env::var_os(name)
            .and_then(|v| v.to_str()?.parse::<i32>().ok())
            .filter(|v| *v > 0)
    };
    let mut width = env_positive("COLUMNS").unwrap_or(-1);
    let mut height = env_positive("LINES").unwrap_or(-1);
    if width <= 1 {
        width = 80;
    }
    if height <= 0 {
        height = 24;
    }
    bind_lines_columns(executor, height, width);
}

/// GNU shell.c:806-811: interactive shells run bash_initialize_history and
/// load_history at startup. bashhist.c:320-345 load_history: default
/// HISTSIZE (500) and HISTFILESIZE (=HISTSIZE), apply sv_histsize to the
/// file (truncating it to HISTFILESIZE lines), then read HISTFILE into the
/// list when it exists.
pub fn initialize_interactive_history(executor: &mut Executor) {
    executor.set_shell_option("history", true);
    // shell.c init_interactive: interactive shells also default histexpand
    // (set -H) on, so !!/!str expand on each accepted line.
    executor.set_shell_option("histexpand", true);
    if executor.get_env("HISTSIZE").is_none() {
        executor.set_env("HISTSIZE", "500");
    }
    if executor.get_env("HISTFILESIZE").is_none() {
        if let Some(histsize) = executor.get_env("HISTSIZE").map(String::from) {
            executor.set_env("HISTFILESIZE", &histsize);
        }
    }
    let Some(session) = executor.get_session_history() else {
        return;
    };
    let Some(histfile) = executor.get_env("HISTFILE").map(String::from) else {
        return;
    };
    if histfile.is_empty() {
        return;
    }
    let path = executor.resolve_shell_path(&histfile);
    let write_ts = executor.get_env("HISTTIMEFORMAT").is_some();
    // bashhist.c:331-332: sv_histsize("HISTFILESIZE") truncates the file
    // before it is read.
    if let Some(max) = SessionHistory::size_limit(executor.get_env("HISTFILESIZE")) {
        let _ = session
            .borrow_mut()
            .truncate_file(&path.to_string_lossy(), max, write_ts);
    }
    if path.exists() {
        let histsize = SessionHistory::size_limit(executor.get_env("HISTSIZE"));
        let mut shell = session.borrow_mut();
        shell.histfile_loaded = true;
        let _ = shell.load_file(&path.to_string_lossy(), histsize);
    }
}

fn session_entries_len(executor: &Executor) -> usize {
    executor
        .get_session_history()
        .map(|s| s.borrow().entries.len())
        .unwrap_or(0)
}

/// history_base: the absolute 1-based history number of entries[0]
/// (history.c stifle_history advances it as entries drop off the front).
fn session_history_base(executor: &Executor) -> usize {
    executor
        .get_session_history()
        .map(|s| s.borrow().base)
        .unwrap_or(1)
}

/// Incremental reverse-i-search state (readline/isearch.c). `search` is the
/// accumulated search string; `match_index` is the history entry currently
/// displayed.
struct ISearchState {
    search: String,
    orig_buffer: String,
    orig_index: Option<usize>,
    match_index: Option<usize>,
}

/// flags.c:294 which_set_flags appends `s` to `$-` while GNU's
/// `read_from_stdin` (shell.c:301) is set: explicit `-s` at startup
/// (shell.c:928), a non-interactive shell reading commands from a
/// piped/redirected stdin (shell.c:780-786), or an interactive shell with
/// no script operand (shell.c:787-790). This marker is rubash's
/// process-local equivalent, set by the stdin command drivers; like the C
/// global it never reaches child processes (the `__RUBASH_` env filter,
/// compound_exec.rs rubash_spawn_inherited_state, keeps it internal).
pub const READ_STDIN_MARKER: &str = "__RUBASH_READ_STDIN";

/// bash -i reading commands from a non-tty stdin. GNU still drives readline
/// here (parse.y yy_readline_get -> bashline.c bash_readline): the prompt is
/// written to stderr, the input line is echoed, and editing keystrokes in
/// the stream are honored — C-p/C-n walk the history list, C-r starts
/// reverse-i-search, and C-o (operate-and-get-next, readline/misc.c
/// rl_operate_and_get_next) executes the current line and replaces the
/// buffer with the next history entry.
pub fn run_interactive_stdin(executor: &mut Executor) -> i32 {
    executor.set_env(READ_STDIN_MARKER, "1");
    executor.inherit_process_stdin();
    // The first primary-prompt read initializes readline
    // (parse.y:1665 yy_readline_get -> bashline.c:524), which is where GNU
    // binds LINES/COLUMNS for an interactive shell without a controlling
    // terminal (terminal.c:374). rubash#300: this driver IS the non-tty
    // readline port, so the bind belongs at its entry.
    bind_interactive_screen_size(executor);
    // variables.c:568-579 (set_if_not PS1/PS2 for interactive shells): GNU
    // renders the default `\s-\v\$ ` (config-top.h:82 PPROMPT) when no
    // startup file set PS1 — the clean-env stderr of `bash --rcfile
    // /dev/null -i </dev/null` shows `bash-5.3# exit`. Bound unexported,
    // like bind_variable.
    if executor.get_env("PS1").is_none() {
        executor
            .shell_state
            .env_vars
            .insert("PS1".to_string(), "\\s-\\v\\$ ".to_string());
    }
    if executor.get_env("PS2").is_none() {
        executor
            .shell_state
            .env_vars
            .insert("PS2".to_string(), "> ".to_string());
    }
    let mut pending = String::new();
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    let mut group: Vec<(String, bool)> = Vec::new();
    let mut next_line = 1usize;
    let mut pending_start_line = 1usize;

    // readline edit state: the line being edited, the cursor (byte offset),
    // and where the buffer came from in the history list.
    let mut buffer = String::new();
    let mut cursor = 0usize;
    let mut history_index: Option<usize> = None;
    let mut saved_line = String::new();
    let mut isearch: Option<ISearchState> = None;
    let mut quoted_insert = false;
    let mut eof = false;

    // One accepted readline line -> accumulate into `pending` and run it
    // once the command is complete (same grouping as run_stdin_script).
    macro_rules! accept_line {
        ($line:expr) => {{
            let line: &str = $line;
            if pending.is_empty() {
                pending_start_line = next_line;
            }
            next_line += line.matches('\n').count().max(1);
            pending.push_str(line);
            if !pending.ends_with('\n') {
                pending.push('\n');
            }
            let phys: Vec<&str> = line.split_inclusive('\n').collect();
            for phys_line in phys.iter().map(|l| l.trim_end_matches('\n')) {
                let mut is_body = false;
                if let Some((delimiter, strip_tabs)) = pending_heredocs.first().cloned() {
                    let candidate = phys_line.trim_end_matches('\r');
                    let candidate = if strip_tabs {
                        candidate.trim_start_matches('\t')
                    } else {
                        candidate
                    };
                    if candidate == delimiter {
                        pending_heredocs.remove(0);
                    } else {
                        is_body = true;
                    }
                } else {
                    pending_heredocs.extend(stdin_heredoc_declarations(phys_line));
                }
                group.push((phys_line.to_string(), is_body));
            }

            let stdin_posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
            if pending_heredocs.is_empty() && !stdin_source_needs_more_posix(&pending, stdin_posix)
            {
                // bashhist.c pre_process_line + bash_add_history: each
                // complete command runs history expansion first (set -H
                // defaults on for interactive shells), then records, then
                // executes — the same pipeline as run_script_with_history.
                let status = if let Some(session) = executor.get_session_history() {
                    run_history_group(
                        executor,
                        &session,
                        &group,
                        pending_start_line,
                        None,
                        true,
                        None,
                    )
                } else {
                    run_source_with_line_offset(
                        executor,
                        &pending,
                        true,
                        pending_start_line.saturating_sub(1),
                        None,
                        None,
                    )
                };
                let parse_error = executor.take_parse_error();
                pending.clear();
                group.clear();
                if parse_error
                    || executor.take_exit_jump_pending()
                    || (status != 0
                        && stdin_script_errexit_enabled(executor)
                        && !executor.last_command_inverted())
                {
                    eof = true;
                }
            }
        }};
    }

    while !eof {
        // eval.c:336 parse_command runs PROMPT_COMMAND before each primary
        // prompt read; continuation (PS2) reads do not. `pending` empty
        // marks the start of a new command, i.e. a primary prompt.
        if pending.is_empty() {
            executor.execute_prompt_command();
        }
        // readline.c readline(): print the EXPANDED PS1 on stderr, then echo
        // the input line (non-tty input is echoed by readline's
        // dumb-terminal path). bashline.c:461-462 rl_outstream = stderr;
        // parse.y:6158-6159 prompt_again passes PS1 through
        // decode_prompt_string, so raw-byte markers (ESC carriers) must be
        // rendered as their real bytes — visible ESC noise in the echo was
        // rubash#297's family symptom.
        let ps1 = executor.get_env("PS1").unwrap_or_default().to_string();
        let rendered =
            crate::executor::substitution_metadata::decode_raw_byte_markers_to_byte_chars(
                &executor.expand_prompt_string(&ps1),
            );
        eprint!("{rendered}");

        let mut raw = String::new();
        let line_result = match executor.script_fd0_line(&mut raw) {
            Some(count) => Ok(count),
            None => read_unbuffered_line(&mut raw),
        };
        match line_result {
            // bashline.c bash_readline: interactive EOF synthesizes the
            // `exit` command, whose builtin echoes "exit" (or "logout")
            // to stderr before exiting (builtins/exit.def:59-62). The
            // prompt is already on stderr, so the line reads `$ exit`.
            Ok(0) => {
                if executor.get_env("__RUBASH_INTERACTIVE").as_deref() == Some("1") {
                    let login = executor.get_env("__RUBASH_LOGIN_SHELL").as_deref() == Some("1");
                    eprintln!("{}", if login { "logout" } else { "exit" });
                }
                eof = true;
            }
            Ok(_) => {}
            Err(_) => eof = true,
        }
        eprint!("{raw}");

        let chars: Vec<char> = raw.chars().collect();
        let mut i = 0usize;
        while i < chars.len() && !eof {
            let c = chars[i];
            i += 1;

            if quoted_insert {
                buffer.insert(cursor, c);
                cursor += c.len_utf8();
                quoted_insert = false;
                continue;
            }

            if let Some(search) = isearch.as_mut() {
                // readline/isearch.c rl_isearch_dispatch: printable bytes
                // extend the search string; editing keys accept the current
                // match and are then re-processed in normal mode; C-g aborts.
                let entries_len = session_entries_len(executor);
                match c {
                    '\x07' | '\x1b' => {
                        // abort: restore the pre-search line
                        buffer = search.orig_buffer.clone();
                        cursor = buffer.len();
                        history_index = search.orig_index;
                        isearch = None;
                        continue;
                    }
                    '\x12' => {
                        // repeat search further back
                        let search_str = search.search.clone();
                        if !search_str.is_empty() {
                            if let Some(session) = executor.get_session_history() {
                                let shell = session.borrow();
                                let upper = search.match_index.unwrap_or(entries_len);
                                if let Some(found) = shell.entries[..upper]
                                    .iter()
                                    .rposition(|e| e.contains(&search_str))
                                {
                                    search.match_index = Some(found);
                                    buffer = shell.entries[found].clone();
                                    cursor = buffer.len();
                                    history_index = Some(found);
                                }
                            }
                        }
                        continue;
                    }
                    '\x7f' | '\x08' => {
                        search.search.pop();
                        let search_str = search.search.clone();
                        if let Some(session) = executor.get_session_history() {
                            let shell = session.borrow();
                            let upper = search.match_index.map(|m| m + 1).unwrap_or(entries_len);
                            if let Some(found) = shell.entries[..upper]
                                .iter()
                                .rposition(|e| e.contains(&search_str))
                            {
                                search.match_index = Some(found);
                                buffer = shell.entries[found].clone();
                                cursor = buffer.len();
                                history_index = Some(found);
                            }
                        }
                        continue;
                    }
                    c if c >= ' '
                        || c == '\n'
                        || c == '\r'
                        || c == '\x0f'
                        || c == '\x10'
                        || c == '\x0e' =>
                    {
                        if c == '\n' || c == '\r' || c == '\x0f' || c == '\x10' || c == '\x0e' {
                            // Accept the match (if any) and reprocess the
                            // terminating key in normal mode.
                            if let Some(m) = search.match_index {
                                if let Some(session) = executor.get_session_history() {
                                    if let Some(entry) = session.borrow().entries.get(m).cloned() {
                                        buffer = entry;
                                        cursor = buffer.len();
                                        history_index = Some(m);
                                    }
                                }
                            }
                            isearch = None;
                            // fall through to normal dispatch below
                        } else {
                            // extend the search string and re-search
                            search.search.push(c);
                            let search_str = search.search.clone();
                            if let Some(session) = executor.get_session_history() {
                                let shell = session.borrow();
                                let upper =
                                    search.match_index.map(|m| m + 1).unwrap_or(entries_len);
                                if let Some(found) = shell.entries[..upper]
                                    .iter()
                                    .rposition(|e| e.contains(&search_str))
                                {
                                    search.match_index = Some(found);
                                    buffer = shell.entries[found].clone();
                                    cursor = buffer.len();
                                    history_index = Some(found);
                                }
                            }
                            continue;
                        }
                    }
                    _ => {
                        // other control chars end the search and reprocess
                        if let Some(m) = search.match_index {
                            if let Some(session) = executor.get_session_history() {
                                if let Some(entry) = session.borrow().entries.get(m).cloned() {
                                    buffer = entry;
                                    cursor = buffer.len();
                                    history_index = Some(m);
                                }
                            }
                        }
                        isearch = None;
                    }
                }
            }

            match c {
                '\n' | '\r' => {
                    let line = std::mem::take(&mut buffer);
                    cursor = 0;
                    history_index = None;
                    saved_line.clear();
                    accept_line!(&line);
                }
                '\x12' => {
                    isearch = Some(ISearchState {
                        search: String::new(),
                        orig_buffer: buffer.clone(),
                        orig_index: history_index,
                        match_index: None,
                    });
                }
                '\x0f' => {
                    // rl_operate_and_get_next (readline/misc.c): accept the
                    // line for execution, then preload the next readline with
                    // the entry AFTER the accepted line — identified by its
                    // absolute history number (where_history()+history_base+1),
                    // so a HISTSIZE stifle dropping entries during the accepted
                    // command's own add_history cannot shift the target.
                    let src_abs = history_index.map(|p| session_history_base(executor) + p + 1);
                    let line = std::mem::take(&mut buffer);
                    cursor = 0;
                    accept_line!(&line);
                    match src_abs {
                        Some(abs) => {
                            let base = session_history_base(executor);
                            let idx = abs.saturating_sub(base);
                            let len = session_entries_len(executor);
                            if idx < len {
                                history_index = Some(idx);
                                if let Some(session) = executor.get_session_history() {
                                    buffer = session.borrow().entries[idx].clone();
                                }
                                cursor = buffer.len();
                            } else {
                                history_index = None;
                                buffer.clear();
                            }
                        }
                        None => {
                            history_index = None;
                            buffer.clear();
                        }
                    }
                }
                '\x10' => {
                    // previous-history-line
                    let len = session_entries_len(executor);
                    if len > 0 {
                        let idx = match history_index {
                            None => {
                                saved_line = buffer.clone();
                                len - 1
                            }
                            Some(i0) => i0.saturating_sub(1),
                        };
                        history_index = Some(idx);
                        if let Some(session) = executor.get_session_history() {
                            buffer = session.borrow().entries[idx].clone();
                        }
                        cursor = buffer.len();
                    }
                }
                '\x0e' => {
                    // next-history-line
                    let len = session_entries_len(executor);
                    if let Some(i0) = history_index {
                        if i0 + 1 < len {
                            history_index = Some(i0 + 1);
                            if let Some(session) = executor.get_session_history() {
                                buffer = session.borrow().entries[i0 + 1].clone();
                            }
                        } else {
                            history_index = None;
                            buffer = saved_line.clone();
                        }
                        cursor = buffer.len();
                    }
                }
                '\x01' => cursor = 0,
                '\x05' => cursor = buffer.len(),
                '\x02' => cursor = cursor.saturating_sub(1),
                '\x06' => cursor = (cursor + 1).min(buffer.len()),
                '\x0b' => buffer.truncate(cursor),
                '\x15' => {
                    buffer.drain(..cursor);
                    cursor = 0;
                }
                '\x7f' | '\x08' => {
                    if cursor > 0 {
                        cursor -= 1;
                        while !buffer.is_char_boundary(cursor) {
                            cursor -= 1;
                        }
                        let mut end = cursor + 1;
                        while !buffer.is_char_boundary(end) && end <= buffer.len() {
                            end += 1;
                        }
                        buffer.drain(cursor..end.min(buffer.len()));
                    }
                }
                '\x04' => {
                    // C-d: EOF on an empty buffer, delete-char otherwise.
                    if buffer.is_empty() && history_index.is_none() {
                        eof = true;
                    } else if cursor < buffer.len() {
                        let mut end = cursor + 1;
                        while !buffer.is_char_boundary(end) && end <= buffer.len() {
                            end += 1;
                        }
                        buffer.drain(cursor..end.min(buffer.len()));
                    }
                }
                '\x03' => {
                    // SIGINT-style abort: discard the edit line.
                    buffer.clear();
                    cursor = 0;
                    history_index = None;
                    saved_line.clear();
                }
                '\x16' => quoted_insert = true,
                '\x1b' => {
                    // Escape sequences: consume the introducer plus the
                    // sequence; only arrow keys get bindings here.
                    if i < chars.len() {
                        let introducer = chars[i];
                        i += 1;
                        if introducer == '[' || introducer == 'O' {
                            if i < chars.len() {
                                let final_byte = chars[i];
                                i += 1;
                                match final_byte {
                                    'A' => {
                                        let len = session_entries_len(executor);
                                        if len > 0 {
                                            let idx = match history_index {
                                                None => {
                                                    saved_line = buffer.clone();
                                                    len - 1
                                                }
                                                Some(i0) => i0.saturating_sub(1),
                                            };
                                            history_index = Some(idx);
                                            if let Some(session) = executor.get_session_history() {
                                                buffer = session.borrow().entries[idx].clone();
                                            }
                                            cursor = buffer.len();
                                        }
                                    }
                                    'B' => {
                                        let len = session_entries_len(executor);
                                        if let Some(i0) = history_index {
                                            if i0 + 1 < len {
                                                history_index = Some(i0 + 1);
                                                if let Some(session) =
                                                    executor.get_session_history()
                                                {
                                                    buffer =
                                                        session.borrow().entries[i0 + 1].clone();
                                                }
                                            } else {
                                                history_index = None;
                                                buffer = saved_line.clone();
                                            }
                                            cursor = buffer.len();
                                        }
                                    }
                                    'C' => cursor = (cursor + 1).min(buffer.len()),
                                    'D' => cursor = cursor.saturating_sub(1),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                c if c >= ' ' => {
                    buffer.insert(cursor, c);
                    cursor += c.len_utf8();
                }
                _ => {}
            }
        }

        if eof && !buffer.is_empty() {
            // readline treats EOF on a nonempty buffer as accept-line.
            let line = std::mem::take(&mut buffer);
            accept_line!(&line);
        }
    }

    if !pending.trim().is_empty() {
        let status = run_source_with_line_offset(
            executor,
            &pending,
            true,
            pending_start_line.saturating_sub(1),
            None,
            None,
        );
        let _ = status;
        pending.clear();
    }

    let status = executor.last_exit_code();
    finish_shell(executor, status, true)
}

pub fn read_unbuffered_line(output: &mut String) -> io::Result<usize> {
    // TODO(input.c): This intentionally avoids BufRead prefetching so a child
    // shell script can inherit unread bytes from the same redirected stdin.
    // Accumulate raw bytes and decode once per line: `byte as char` would
    // Latin-1-encode multibyte script source (e.g. a `中文` literal became
    // `ä¸­æ–‡`). `bytes_to_shell_text` keeps valid UTF-8 and preserves
    // undecodable bytes as raw-byte markers. A `b'\n'` can never sit inside
    // a UTF-8 sequence, so the line split is char-boundary safe.
    //
    // The read goes through the raw OS handle, not io::stdin(): the std
    // stdin owns a process-wide BufReader whose fill_buf would prefetch the
    // whole stream, and GNU input.c reads fd 0 one byte at a time (zread)
    // so a forked sibling sharing the descriptor continues at the exact
    // offset this shell consumed (redir.tests heredoc + `&` children).
    let mut line: Vec<u8> = Vec::new();
    let mut read = 0;
    loop {
        match crate::executor::read_process_stdin_bytes(1)? {
            buf if buf.is_empty() => break,
            buf => {
                read += buf.len();
                line.push(buf[0]);
                if buf[0] == b'\n' {
                    break;
                }
            }
        }
    }
    output.push_str(&crate::executor::bytes_to_shell_text(&line));
    Ok(read)
}

pub fn finish_shell(executor: &mut Executor, status: i32, interactive: bool) -> i32 {
    // shell.c:1007-1009 exit_shell: if remember_on_history is set,
    // maybe_save_shell_history() writes the session's history to $HISTFILE
    // (bashhist.c:488-530): append only this session's lines when the list
    // was not stifled below them, rewrite the file otherwise, then
    // sv_histsize("HISTFILESIZE") truncates the file (variables.c:6088).
    if interactive {
        // shell.c:1008: the save is gated on remember_on_history (the
        // `history` set option), which `set +o history` clears.
        let remember_on_history =
            executor.get_env("__RUBASH_SETOPT_history").as_deref() == Some("1");
        if remember_on_history {
            if let Some(session) = executor.get_session_history() {
                let write_ts = executor.get_env("HISTTIMEFORMAT").is_some();
                let histfile = executor.get_env("HISTFILE").map(String::from);
                let histfilesize = SessionHistory::size_limit(executor.get_env("HISTFILESIZE"));
                let mut shell = session.borrow_mut();
                if shell.lines_this_session > 0 {
                    if let Some(hf) = histfile.as_deref().filter(|hf| !hf.is_empty()) {
                        let path = executor.resolve_shell_path(hf);
                        let path_str = path.to_string_lossy().to_string();
                        if !path.exists() {
                            let _ = fs::write(&path, "");
                        }
                        // bashhist.c:513: force_append_history is the histappend
                        // shopt; it always appends instead of rewriting.
                        let force_append = executor
                            .get_env("__RUBASH_SHOPT_STATE")
                            .is_some_and(|s| s.split('\u{1f}').any(|e| e == "histappend"));
                        if force_append || shell.lines_this_session <= shell.entries.len() {
                            let _ = shell.append_file(&path_str, write_ts);
                        } else {
                            let _ = shell.write_file(&path_str, write_ts);
                        }
                        shell.lines_this_session = 0;
                        if let Some(max) = histfilesize {
                            let _ = shell.truncate_file(&path_str, max, write_ts);
                        }
                    }
                }
            }
        }
    }
    match executor.run_exit_trap_with_status(status) {
        Ok(code) => code,
        Err(ExecuteError::ExitCode(code)) => code,
        Err(e) => {
            if interactive {
                eprintln!("Error: {}", e);
            } else {
                eprintln!("{}", e);
            }
            1
        }
    }
}

/// shell.c:1830-1842 init_interactive + bashhist.c:320-345: create the
/// session history for an interactive (`-i`) shell and load $HISTFILE.
/// Hosts (niu) call this before run_interactive_stdin when a forced
/// interactive shell has no terminal to drive their own REPL on.
pub fn prepare_interactive_history(executor: &mut Executor) {
    if executor.get_session_history().is_none() {
        let session = std::rc::Rc::new(std::cell::RefCell::new(
            crate::history::SessionHistory::new(),
        ));
        executor.set_session_history(Some(session));
    }
    initialize_interactive_history(executor);
    // shell.c:816: directly after the interactive history setup, GNU runs
    // get_tty_state() again — re-binding LINES/COLUMNS from the controlling
    // terminal after the startup files (rubash#300). This runs before the
    // -c string / script / interactive reader, so prompt-time code
    // (PROMPT_COMMAND, PS1 expansions) sees console values under GNU, and
    // now under rubash too.
    bind_interactive_console_winsize(executor);
}

/// Outcome of [`pre_process_interactive_line`] (bashhist.c pre_process_line
/// applied to one interactive line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveExpansion {
    /// Execute this text: the expansion when history expansion modified the
    /// line, otherwise the input verbatim.
    Execute(String),
    /// Do not execute the line (expansion error, or the `:p` print-only
    /// modifier). The diagnostic or the printed expansion has already gone
    /// to stderr and the history list was fixed the way GNU records it.
    Discard,
}

/// bashhist.c:562 pre_process_line (print_changes=1, addit=1) integrated the
/// way the GNU interactive reader does it (y.tab.c:5020): expand history in
/// one accepted interactive line before the host parses it, print the
/// expansion like GNU's `fprintf (stderr, "%s\n", history_value)`, and keep
/// the history recording the EXPANDED text (maybe_add_history).
///
/// For interactive hosts whose line editor owns the accepted line (niubash /
/// reedline). The event designators resolve against the host-installed
/// [`crate::history::HistoryProvider`] list — the live interactive list —
/// falling back to the engine's session history when no provider is set.
/// A host editor that records the accepted line *before* execution (reedline
/// does) sees that trailing raw entry hidden from its own expansion, the
/// same way bashhist.c:576-580 decrements `history_length` to hide the
/// current line, and sees it rewritten to the expansion afterwards (or
/// removed when the expansion failed, which GNU never records).
///
/// Gating mirrors bashhist.c:574: `set +H` (the histexpand option) disables
/// expansion; the caller asserts the interactive context by calling this —
/// y.tab.c:5003 `remember_on_history` corresponds to the host's own
/// recording being active, so the engine's `set -o history` flag is not
/// consulted (interactive hosts keep it off deliberately because the host
/// owns the list). An unset histexpand flag is GNU's interactive
/// HISTEXPAND_DEFAULT (bashhist.c:288): on. Expansion state (histexpand.c
/// statics) persists in the shell state across calls.
pub fn pre_process_interactive_line(executor: &mut Executor, line: &str) -> InteractiveExpansion {
    let posix = executor.get_env("__RUBASH_POSIX_MODE").as_deref() == Some("1");
    let chars = executor.get_env("histchars").unwrap_or("!^#");
    let mut chars = chars.chars();
    let ctx = HistCtx {
        chars: HistChars {
            expand: chars.next().unwrap_or('!'),
            subst: chars.next().unwrap_or('^'),
            comment: chars.next().unwrap_or('#'),
        },
        posix,
    };

    // bashhist.c:541-550 history_expansion_p: the expansion char or the
    // quick-substitution char anywhere in the line.
    let wants_expansion = line.contains(ctx.chars.expand) || line.contains(ctx.chars.subst);
    // bashhist.c:574 gate on history_expansion; `set +H` writes the flag
    // off, unset means the interactive default (bashhist.c:288).
    let histexpand_on = executor.get_env("__RUBASH_SETOPT_histexpand").as_deref() != Some("0");
    if !wants_expansion || !histexpand_on {
        return InteractiveExpansion::Execute(line.to_string());
    }

    // The list events resolve against: the host provider owns the
    // interactive list; the engine session is the fallback.
    let mut scratch = SessionHistory::new();
    let mut raw_preadded = false;
    if let Some(mut entries) = executor.history_provider_snapshot() {
        if entries.last().map(String::as_str) == Some(line) {
            entries.pop();
            raw_preadded = true;
        }
        scratch.entries = entries;
    } else if let Some(session) = executor.get_session_history() {
        scratch.entries = session.borrow().entries.clone();
    }

    // histexpand.c statics persist across expansions; they ride the shell
    // state so later `!?` repeats and `:s` lhs reuse survive.
    scratch.engine = std::mem::take(&mut executor.shell_state_mut().interactive_hist_engine);
    let result = scratch.expand(line, ctx);
    executor.shell_state_mut().interactive_hist_engine = scratch.engine;

    match result.status {
        -1 => {
            // bashhist.c:594 internal_error ("%s", history_value): an
            // interactive shell prefixes only the shell name (error.c
            // report_prolog via get_name_for_error).
            let name = executor
                .get_env("__RUBASH_SHELL_NAME")
                .as_deref()
                .unwrap_or("bash")
                .to_string();
            eprintln!("{name}: {}", result.text);
            // A failed expansion records nothing (GNU returns before
            // maybe_add_history): undo the raw line the host editor added.
            if raw_preadded {
                executor.history_provider_replace_last(line, None);
            }
            InteractiveExpansion::Discard
        }
        2 => {
            // expanded == 2: print-only (`:p`). bashhist.c:598-606 prints
            // the expansion and maybe_add_history records the printed text.
            eprintln!("{}", result.text);
            if raw_preadded && !result.text.is_empty() {
                executor.history_provider_replace_last(line, Some(result.text));
            }
            InteractiveExpansion::Discard
        }
        1 => {
            // Expanded (hist_verify off — no verify mode): print the
            // expansion, then record the EXPANDED line, not the raw text.
            eprintln!("{}", result.text);
            if raw_preadded {
                executor.history_provider_replace_last(line, Some(result.text.clone()));
            }
            InteractiveExpansion::Execute(result.text)
        }
        _ => InteractiveExpansion::Execute(result.text),
    }
}

/// bashhist.c remember_on_history veto for interactive hosts, applied after
/// a line executed: GNU records a read line only when `set -o history` was
/// on when the line was read (maybe_add_history under remember_on_history).
/// The host editor (reedline) records the accepted line *before* execution,
/// so when the caller observed recording off at read time the entry this
/// line left in the provider list (the raw line, or the expansion
/// [`pre_process_interactive_line`] recorded) must be removed again.
/// Unconditional drop of the last entry: in a single-threaded REPL nothing
/// else records between read and this call. No-op without a provider.
pub fn interactive_history_veto_recording(executor: &mut Executor) {
    executor.history_provider_drop_last();
}

/// general.c:718-741 check_binary_file: a script whose first line (two when
/// it starts with a #! interpreter specifier) contains NUL, or an ELF image,
/// is refused with "cannot execute binary file" and EX_BINARY_FILE (126).
pub fn check_binary_file(sample: &[u8]) -> bool {
    if sample.len() >= 4
        && sample[0] == 0x7f
        && sample[1] == b'E'
        && sample[2] == b'L'
        && sample[3] == b'F'
    {
        return true;
    }
    if sample.is_empty() {
        return false;
    }
    let mut lines_left = if sample[0] == b'#' && sample.len() >= 2 && sample[1] == b'!' {
        2
    } else {
        1
    };
    for &byte in sample {
        if byte == b'\n' {
            lines_left -= 1;
            if lines_left == 0 {
                return false;
            }
        } else if byte == 0 {
            return true;
        }
    }
    false
}

/// Script-entry decoding companion to `check_binary_file`: GNU shell.c has
/// no UTF-8 validity gate on script files, so after the binary-file check
/// the raw bytes decode with invalid-sequence bytes carried as raw-byte
/// marker pairs (rubash#132; general.c:718 check_binary_file is the only
/// rejection). See substitution_metadata::bytes_to_script_text.
pub fn bytes_to_script_text(bytes: &[u8]) -> String {
    crate::executor::substitution_metadata::bytes_to_script_text(bytes)
}

/// Read a script-language file the way GNU shell.c does: raw bytes, then
/// `check_binary_file`, then script-text decoding. Errors use the io error
/// message (callers own their diagnostics).
pub fn read_script_bytes(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(crate::executor::substitution_metadata::bytes_to_script_text(&bytes))
}

#[cfg(test)]
mod group_text_scans_tests {
    use super::{
        fnbody_residuals_advance, stdin_source_has_unclosed_function_delimited_body,
        stdin_source_is_function_signature, stdin_source_text_needs_more, FnBodyResidualState,
        GroupTextScans,
    };

    /// Drive one FnBody delimiter scanner exactly like
    /// GroupTextScans::advance_fnbody (raw line + '\n' per prefix) and
    /// assert the answer equals the oracle's at EVERY prefix.
    fn assert_fnbody_matches_full(lines: &[&str], delim: char) {
        let mut chars: Vec<char> = Vec::new();
        let mut pending = String::new();
        let mut resume = 0usize;
        let mut snapshot = FnBodyResidualState::default();
        for (line_index, line) in lines.iter().enumerate() {
            chars.extend(line.chars());
            chars.push('\n');
            pending.push_str(line);
            pending.push('\n');
            let mut state = snapshot.clone();
            let park = fnbody_residuals_advance(&chars, resume, &mut state, delim, &pending);
            assert_eq!(
                state.is_open(),
                stdin_source_has_unclosed_function_delimited_body(&pending, delim),
                "fnbody {delim} answer at prefix {line_index} of {lines:?} ({pending:?})"
            );
            match park {
                Some(park) => {
                    resume = park.pos;
                    snapshot = park.snapshot;
                }
                None => {
                    resume = chars.len();
                    snapshot = state;
                }
            }
        }
    }

    #[test]
    fn fnbody_incremental_matches_full_required_shapes() {
        // Classic function definitions: the body delimiter holds the group
        // open until its closer arrives.
        for delim in ['{', '('] {
            assert_fnbody_matches_full(&["f() {", "  :", "}"], delim);
            assert_fnbody_matches_full(&["function f {", "  :", "}"], delim);
            assert_fnbody_matches_full(&["f()", "{", "}"], delim);
            // Non-signature prefixes: the delimiter is found but the
            // signature check fails — never open.
            assert_fnbody_matches_full(&["echo {", "a", "}"], delim);
            assert_fnbody_matches_full(&["x=(1", "2)", "y"], delim);
            // Quoted/commented delimiters are not openers.
            assert_fnbody_matches_full(&["f() '#{'", "x", "y"], delim);
            // Deeply nested same-delimiter bodies.
            assert_fnbody_matches_full(&["f() {", "{ {", "} }", "}"], delim);
            // Subshell in the signature area (rejected `(` for the paren
            // scanner, plain text for the brace scanner).
            assert_fnbody_matches_full(&["(a", "b)", "f() {", "}"], delim);
        }
    }

    #[test]
    fn fnbody_incremental_matches_full_park_shapes() {
        // The paren scanner's undecided `(`-followed-by-whitespace tail:
        // prefix 1 accepts the `(` as the body delimiter (EOF decides),
        // prefix 2's `)` flips the rejection and the search restarts fresh
        // from just past it.
        assert_fnbody_matches_full(&["f (", ")", "{ :; }"], '(');
        assert_fnbody_matches_full(&["f (  ", "\t)", "{ :; }"], '(');
        assert_fnbody_matches_full(&["f (", ") { :; }", ""], '(');
        assert_fnbody_matches_full(&["a=1 f (", ")"], '(');
        // The brace scanner has no park (monotone search + depth fold) but
        // must still agree on every shape that CAN flip: a `{` arriving on
        // a later line inside a still-open group.
        assert_fnbody_matches_full(&["f() {", "{", "}"], '{');
        // CRLF-shaped input: the '\r' before '\n' is word data.
        assert_fnbody_matches_full(&["f() {\r", "\r}\r"], '{');
    }

    /// Deterministic LCG so a failure reproduces bit for bit.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
    }

    /// Seeded fuzz (>=2000 required; 4000 for parity) over both delimiters.
    #[test]
    fn fnbody_incremental_matches_full_randomized() {
        const FRAGMENTS: &[&str] = &[
            "f() ",
            "function f ",
            "{ ",
            "} ",
            "( ",
            ") ",
            "((",
            ")) ",
            "'q'",
            "\"q",
            "q\"",
            "$( ",
            "$(( ",
            "` ",
            "${x}",
            "#c ",
            "case ",
            "esac",
            " in ",
            "; ",
            "&&",
            "|",
            " a ",
            "echo ",
            "x=( ",
            "\\ ",
            "f1 (",
            ") ",
            "name ()",
            " { ",
            "if ",
            "then",
            "fi",
        ];
        let mut rng = Lcg(0x292_0000_0005);
        for case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_fnbody_matches_full(&lines, if case % 2 == 0 { '{' } else { '(' });
        }
    }

    /// The whole battery: GroupTextScans::needs_more must equal
    /// stdin_source_text_needs_more (the fresh scan it replaces) at EVERY
    /// appended-line prefix, for both POSIX modes — covering the quotes and
    /// comsub machines re-driven over the pending mirror as well as the
    /// perf9 machines and the fresh-tail probes.
    fn assert_battery_matches_fresh(lines: &[&str], posix: bool) {
        let mut scans = GroupTextScans::new(posix);
        let mut pending = String::new();
        for (line_index, line) in lines.iter().enumerate() {
            let raw = format!("{line}\n");
            pending.push_str(&raw);
            scans.push_raw_line(&raw);
            assert_eq!(
                scans.needs_more(&pending, false),
                stdin_source_text_needs_more(&pending, posix),
                "battery at prefix {line_index} of {lines:?} posix={posix} ({pending:?})"
            );
        }
    }

    #[test]
    fn battery_matches_fresh_required_shapes() {
        for posix in [false, true] {
            // Multi-line quotes / comsub / subshell / funsub / subscript.
            assert_battery_matches_fresh(&["echo 'a", "b' c"], posix);
            assert_battery_matches_fresh(&["echo $(a", "b) c"], posix);
            assert_battery_matches_fresh(&["echo ${ a", "; } b"], posix);
            assert_battery_matches_fresh(&["(a", "b)"], posix);
            assert_battery_matches_fresh(&["x=(1", "2)"], posix);
            assert_battery_matches_fresh(&["a[b", "c"], posix);
            // Function bodies keep the group open (the delimiter scans).
            assert_battery_matches_fresh(&["f() {", "  :", "}"], posix);
            assert_battery_matches_fresh(&["f (", ") { :; }", ""], posix);
            assert_battery_matches_fresh(&["function f {", ":"], posix);
            // Backslash continuations.
            assert_battery_matches_fresh(&["echo a \\", "b"], posix);
            assert_battery_matches_fresh(&["f() { \\", ":"], posix);
            // The residual-false-positive override family: the corrected
            // balancer says balanced, the group must NOT stay open on this
            // arm (whatever the token level does with it).
            assert_battery_matches_fresh(&["echo $(case a in a) echo x", "esac)"], posix);
            // POSIX Interp 221: `'` inside `"${...}"` is literal.
            assert_battery_matches_fresh(&["echo \"${IFS+'bar", "}\" x"], posix);
            // Closed everything.
            assert_battery_matches_fresh(&["echo a b", "c d"], posix);
        }
    }

    #[test]
    fn battery_matches_fresh_park_shapes() {
        for posix in [false, true] {
            // Every tail-lookahead park, exercised by a second line that
            // materializes the lookahead.
            assert_battery_matches_fresh(&["echo ${", " x; } b"], posix);
            assert_battery_matches_fresh(&["echo $(", "(1+2))"], posix);
            assert_battery_matches_fresh(&["echo $", "'a'"], posix);
            assert_battery_matches_fresh(&["echo x=\\", "(1 2)"], posix);
            assert_battery_matches_fresh(&["a[$", "(b)]"], posix);
            assert_battery_matches_fresh(&["a[`b", "c`]", "=1"], posix);
            assert_battery_matches_fresh(&["echo $(case a in x) esac)", ";; )"], posix);
            // The fnbody paren park flipping on the next line.
            assert_battery_matches_fresh(&["f (", ")", "{ :; }"], posix);
        }
    }

    /// Seeded fuzz (>=2000 required; 4000, both POSIX modes).
    #[test]
    fn battery_matches_fresh_randomized() {
        const FRAGMENTS: &[&str] = &[
            "$( ",
            ") ",
            "$((",
            ")) ",
            "` ",
            "${",
            "${ ",
            "${x}",
            "}",
            "'a'",
            "\"q",
            "q\"",
            "\\",
            "#c ",
            "case ",
            "esac",
            "esac)",
            " in ",
            "; ",
            ";; ",
            "&&",
            "|",
            "f() ",
            "function f ",
            "{ ",
            "} ",
            "( ",
            "a[",
            "] ",
            "x=(",
            " a ",
            "echo ",
            " $' ",
            "'",
            "\"",
            "$(",
            "`",
            ")",
            "${a:-${b",
            "name ()",
            "if ",
            "then",
            "fi",
        ];
        let mut rng = Lcg(0x292_0000_0006);
        for case in 0..4000u64 {
            let fragment_count = 2 + (rng.next() % 6) as usize;
            let mut fragments = Vec::with_capacity(fragment_count);
            for _ in 0..fragment_count {
                fragments.push(FRAGMENTS[(rng.next() as usize) % FRAGMENTS.len()]);
            }
            let max_lines = 4.min(fragment_count) as u64;
            let line_count = (1 + (rng.next() % max_lines)) as usize;
            let mut lines: Vec<String> = vec![String::new(); line_count];
            for (i, fragment) in fragments.iter().enumerate() {
                let target = if line_count == 1 {
                    0
                } else {
                    (i * line_count / fragment_count).min(line_count - 1)
                };
                lines[target].push_str(fragment);
            }
            let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
            assert_battery_matches_fresh(&lines, case % 2 == 0);
        }
    }

    /// The fresh signature probe the battery keeps (fast-reject) must agree
    /// with the oracle's own shapes at group prefixes.
    #[test]
    fn function_signature_probe_shapes() {
        assert!(stdin_source_is_function_signature("f()\n"));
        assert!(stdin_source_is_function_signature("function f\n"));
        assert!(stdin_source_is_function_signature("'a b c' ()\n"));
        assert!(!stdin_source_is_function_signature("echo hi\n"));
        assert!(!stdin_source_is_function_signature("a=2 (\n"));
    }
}

#[cfg(test)]
mod issue302_tests {
    use super::*;

    #[test]
    fn function_keyword_paren_form_is_a_signature() {
        // parse.y:1056 FUNCTION WORD '(' ')' newline_list function_body:
        // `function f()` awaits its body exactly like `f()` (parse.y:1054)
        // and `function f` (parse.y:1058).
        assert!(stdin_source_is_function_signature("function f()"));
        assert!(stdin_source_is_function_signature("function f ()"));
        assert!(stdin_source_is_function_signature("function f"));
        assert!(stdin_source_is_function_signature("f()"));
        assert!(stdin_source_is_function_signature("f ()"));
        // Complete constructs are not bare signatures.
        assert!(!stdin_source_is_function_signature("function f() { :; }"));
        assert!(!stdin_source_is_function_signature("echo function f()"));
    }

    #[test]
    fn function_keyword_paren_form_keeps_body_open() {
        assert!(stdin_source_text_needs_more("function f()\n", false));
        assert!(stdin_source_text_needs_more("function f()\n{\n", false));
        assert!(!stdin_source_text_needs_more(
            "function f()\n{ :; }\n",
            false
        ));
    }
}

/// perf10 token-reuse differential: the source driver's gather
/// (read_next_source_group) may hand its committed tokens to the group parse
/// in place of a fresh whole-text re-lex, under the three certified
/// conditions (non-alias arm / complete break / no effective extglob
/// toggle — see read_next_source_group's doc). These tests drive the real
/// gather over corpus texts and assert, for every group the PRODUCT
/// actually parses through reuse, that the reused stream (after the
/// caller's tokenize_comsub_body_with_origin trailing-separator pop) is
/// byte-identical — every Token field — to a fresh re-lex of the same
/// group bytes, with the same gather-then-parse interleaving production
/// uses (so the extglob global's timeline matches too).
///
/// Trim-empty groups are outside that contract and skipped by the
/// comparison, mirroring the consumer exactly: run_source_groups gates
/// `if exec_text.trim().is_empty() { continue; }` BEFORE the token stage
/// (builtins/source/execution.rs), and the fresh route short-circuits the
/// same predicate inside tokenize_comsub_body_with_origin — so a group
/// whose text is Rust-trim-empty parses NOTHING through either route and
/// the two representations are never compared or parsed. Their internal
/// difference is real but product-unreachable and platform-dependent: on
/// non-Windows a `\r`-only line keeps its CR (push_main_line's
/// cfg!(windows) strip) and reaches scanner.rs's `'\r'` arm, which lexes
/// it as a line-break Semicolon (GNU keeps `\r` as word data — a CRLF
/// script under GNU prints `$'\r': command not found`), while the fresh
/// route's trim shortcut returns []. The skipped-group assertion below
/// pins the fresh side of that carve-out so the predicate cannot drift
/// silently.
#[cfg(test)]
mod perf10_token_reuse_tests {
    use super::*;

    fn assert_reuse_matches_fresh(text: &str) -> usize {
        crate::lexer::set_parse_extended_glob(false);
        let mut executor = Executor::new();
        executor.shell_state.aliases.clear();
        let raw_lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut index = 0usize;
        let mut reused = 0usize;
        while let Some((pending, _start_line, _group, feeder_tokens)) =
            read_next_source_group(&executor, &raw_lines, &mut index)
        {
            // run_source_groups' own first gate: a trim-empty group is
            // skipped before any tokens exist on either route.
            if pending.trim().is_empty() {
                assert!(
                    tokenize_with_initial_posix(&pending, false).is_empty(),
                    "trim-empty group {:?} must lex to nothing via the fresh route",
                    &pending[..pending.len().min(80)]
                );
                continue;
            }
            let Some(mut tokens) = feeder_tokens else {
                continue;
            };
            if tokens
                .last()
                .is_some_and(|token| token.kind == TokenKind::Semicolon)
            {
                tokens.pop();
            }
            let fresh = tokenize_with_initial_posix(&pending, false);
            let head = &pending[..pending.len().min(160)];
            assert_eq!(
                format!("{tokens:?}"),
                format!("{fresh:?}"),
                "reused vs fresh token streams differ for group starting {head:?}"
            );
            reused += 1;
        }
        assert!(
            reused > 0,
            "no group exercised token reuse for text starting {:?}",
            &text[..text.len().min(80)]
        );
        reused
    }

    #[test]
    fn reuse_matches_fresh_plain_and_function_groups() {
        assert_reuse_matches_fresh("echo hi\necho there\n");
        assert_reuse_matches_fresh("nvm_install() {\n  echo a\n  echo b\n}\necho done\n");
        assert_reuse_matches_fresh("{\n  echo one\n  echo two\n}\nafter\n");
    }

    #[test]
    fn reuse_matches_fresh_brace_group_with_noise() {
        // `}` bytes inside strings, comments, and case patterns while the
        // outer brace group stays open (the nvm.sh shape).
        assert_reuse_matches_fresh(
            "{\n  x='str with } inside'\n  # comment with } brace\n  case $x in\n    a) echo A ;;\n    *) echo B ;;\n  esac\n}\n",
        );
    }

    #[test]
    fn reuse_matches_fresh_heredocs() {
        assert_reuse_matches_fresh("cat <<EOF\nbody line 1\nbody } line\nEOF\necho after\n");
        assert_reuse_matches_fresh("cat <<'EOF'\nquoted $body }\nEOF\necho after\n");
        assert_reuse_matches_fresh("cat <<-EOF\n\tindented body\n\tEOF\necho after\n");
        assert_reuse_matches_fresh("cat <<EOF && cat <<EOF2\nbody1\nEOF\nbody2\nEOF2\n");
    }

    #[test]
    fn reuse_matches_fresh_continuations_and_connectors() {
        assert_reuse_matches_fresh("echo one \\n  two \\n  three\necho next\n");
        assert_reuse_matches_fresh("echo one &&\n  echo two ||\n  echo three\n");
        assert_reuse_matches_fresh("a=b \\n c=d \\n echo $a$c\n");
    }

    #[test]
    fn reuse_matches_fresh_comsub_and_param_spans() {
        assert_reuse_matches_fresh("x=$(echo sub\n  continued)\necho $x\n");
        assert_reuse_matches_fresh("echo ${var:-\n  default}\n");
        assert_reuse_matches_fresh("arr=(\n  one\n  two\n)\necho ${arr[1]}\n");
    }

    #[test]
    fn reuse_matches_fresh_crlf_and_blank_lines() {
        assert_reuse_matches_fresh("echo a\r\necho b\r\n\r\necho c\n");
        assert_reuse_matches_fresh("echo a\n\n\necho b\n");
        assert_reuse_matches_fresh("echo tail-no-newline");
    }

    #[test]
    fn sourced_blank_crlf_line_executes_nothing_on_every_platform() {
        // Product-level guard for the trim-empty carve-out documented on
        // assert_reuse_matches_fresh: a line holding ONLY a CR (a blank CRLF
        // line) must not execute as a command through the source group
        // driver (where token reuse lands). run_source_groups'
        // `exec_text.trim().is_empty()` gate skips the group before tokens
        // exist on either route, and a bare `\r` never forms a word anyway
        // (scanner.rs's '\r' arm lexes a line-start CR as a line-break
        // Semicolon — `\r` only becomes word DATA when glued behind a word
        // character, which is why the commands around the blank line are
        // LF-terminated here). Observable contract, identical pre- and
        // post-reuse on every platform: the assignment AFTER the blank line
        // runs (X == 2) and no stray `$'\r': command not found` leaks into
        // the sourced status (0).
        let mut executor = Executor::new();
        executor.shell_state.aliases.clear();
        crate::builtins::source::execute_text(&mut executor, "X=1\n\r\nX=2\n")
            .expect("source of a three-group text with a blank CRLF line");
        assert_eq!(
            executor.get_env("X").map(str::to_string),
            Some("2".to_string()),
            "the group after the blank CRLF line must run"
        );
        assert_eq!(
            executor.last_exit_code(),
            0,
            "blank CRLF group must be skipped, not executed as a command"
        );
    }

    #[test]
    fn reuse_matches_fresh_posix_flip_lines_inside_group() {
        // `set -o posix` mid-file flips the feeder's internal parse mode per
        // logical line; both the streaming feed and a fresh re-lex replay
        // the same flip at the same line, so reuse stays equivalent. (The
        // EXECUTOR mode only changes at exec time, group by group.)
        assert_reuse_matches_fresh(
            "echo before\nset -o posix\necho '\"$x\"' inside\nset +o posix\necho after\n",
        );
    }

    #[test]
    fn extglob_flip_group_refuses_reuse() {
        crate::lexer::set_parse_extended_glob(false);
        let mut executor = Executor::new();
        executor.shell_state.aliases.clear();
        let text = "echo before-extglob\nshopt -s extglob\ncase x in ?(a)) echo pat ;; esac\n";
        let raw_lines: Vec<&str> = text.split_inclusive('\n').collect();
        let mut index = 0usize;
        let mut refused_seen = false;
        while let Some((_pending, _start, _group, feeder_tokens)) =
            read_next_source_group(&executor, &raw_lines, &mut index)
        {
            if feeder_tokens.is_none() {
                refused_seen = true;
            }
        }
        assert!(
            refused_seen,
            "a group with an effective shopt extglob toggle must refuse reuse"
        );
        crate::lexer::set_parse_extended_glob(false);
    }

    #[test]
    fn reuse_matches_fresh_nvm_corpus() {
        // The vendored nvm.sh v0.40.8 (172906 bytes): the load probe's
        // exact fixture. Exercises the whole gather over ~4500 lines.
        let text = std::fs::read_to_string("benchmarks/corpus/nvm.sh")
            .expect("benchmarks/corpus/nvm.sh vendored in-repo");
        assert_reuse_matches_fresh(&text);
    }
}
