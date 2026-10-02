use crate::lexer::{Token, TokenKind};

use super::*;
use crate::executor::markers::{DATA_DOLLAR, PARSE_ERROR_FIELD_SEP};
use std::rc::Rc;

#[derive(Default)]
pub struct ParseLoopOptions {
    /// Treat a ")" or ";;" at command position as a syntax error that aborts
    /// the remaining input (GNU YYABORT). Enabled for eval reparse.
    pub stray_close_is_error: bool,
    /// The ORIGINAL text being parsed, when the caller has it (eval reparse).
    /// GNU echoes the offending input line verbatim; token reconstruction
    /// cannot recover the original spacing, so the guard slices this text.
    /// Shared by reference count: compound-body re-parses thread the SAME
    /// buffer (nvm.sh -n re-parses ~1500 nested bodies; cloning the String
    /// per body copied 259.5MB and dominated the parse phase — GNU keeps
    /// ONE input string for the whole parse, parse.y shell_input_line).
    pub source_text: Option<Rc<str>>,
    /// Pre-alias-expansion text for the same parse, when the caller spliced
    /// aliases into `source_text` first (grouped driver). GNU's y.error echoes
    /// the input line as read — `math1)`, not its expansion
    /// `echo $( date ))` — so diagnostics slice this text when present.
    /// Lines align with `source_text` because group splices preserve line
    /// structure.
    pub diagnostic_text: Option<Rc<str>>,
    /// How much the caller shifted token positions by before parsing, so the
    /// guard can map a token position back to the source-text byte offset.
    pub source_line_offset: usize,
}

pub(super) struct ParseState {
    pub(super) ast: Ast,
    pub(super) current_cmd: CommandNode,
    pub(super) in_subshell: bool,
    /// ParseLoopOptions::stray_close_is_error, carried so the extracted
    /// command loop (run_command_loop) reads it without the options bag.
    pub(super) stray_close_is_error: bool,
    /// Inline section parsing (GNU single-pass model): while Some, the
    /// command loop stops at the enclosing compound's closer keyword
    /// instead of treating it as an error/unexpected token. See
    /// run_inline_section.
    pub(super) section_stop: SectionStop,
    /// The compound-boundary stack over the tokens the loop iterates on
    /// while a section is open. A successfully parsed nested compound is
    /// skipped wholesale (next_i jump), and its keyword frames are net
    /// balanced, so the stack only stays non-empty for the pathological
    /// not-consumed-compound shapes the slice scanner also shields.
    pub(super) section_stack: Vec<&'static str>,
    /// First token index of the open section (0 for whole parses): bounds
    /// the backward error-helper walks so an error inside a section reads
    /// the same tokens the old body slice contained.
    pub(super) section_token_start: usize,
    /// Unclosed `$(` depth contributed by word tokens seen so far. The
    /// lexer does not fold every multi-line substitution (comsub-posix
    /// tests), so the matching `)` legitimately arrives later as a
    /// top-level token; while this is > 0 a `)` is consumed as that
    /// closer instead of being reported as a stray.
    pub(super) pending_comsub: usize,
    /// Original input text (options.diagnostic_text falling back to
    /// source_text) so a `syntax error near unexpected token 'X'` node can
    /// echo the physical offending line the way parse.y y.error does —
    /// token reconstruction cannot recover the original whitespace.
    /// Rc<str> (see ParseLoopOptions::source_text): the nested compound
    /// parsers share this handle instead of cloning the text per body.
    pub(super) diagnostic_text: Option<Rc<str>>,
    /// options.source_line_offset: how far token positions were shifted,
    /// so a diagnostic computed from `diagnostic_text` line counts maps
    /// back to absolute script lines.
    pub(super) source_line_offset: usize,
}

/// Parse tokens into an AST
pub fn parse(tokens: &[Token]) -> Ast {
    parse_with_options(tokens, ParseLoopOptions::default())
}

/// Options for the parse loop. The stray-close guard reports a top-level
/// ")" that does not close a pending `$(` — a multi-line substitution the
/// lexer did not fold leaves its closer as a top-level token
/// (comsub-posix.tests), and `pending_comsub` lets the guard distinguish
/// that case from a real syntax error such as `echo x)`.
pub fn parse_with_options(tokens: &[Token], options: ParseLoopOptions) -> Ast {
    let mut state = ParseState {
        ast: Ast {
            commands: Vec::new(),
        },
        current_cmd: CommandNode::new(),
        in_subshell: false,
        stray_close_is_error: options.stray_close_is_error,
        section_stop: SectionStop::None,
        section_stack: Vec::new(),
        section_token_start: 0,
        pending_comsub: 0,
        diagnostic_text: options
            .diagnostic_text
            .clone()
            .or_else(|| options.source_text.clone()),
        source_line_offset: options.source_line_offset,
    };

    run_command_loop(tokens, &mut state, 0);

    if !command_is_empty(&state.current_cmd) {
        state.ast.commands.push(state.current_cmd);
    }
    state.ast.commands = fold_command_list(state.ast.commands);
    mark_parse_time_extglob_errors(&mut state.ast, tokens);
    state.ast
}

/// The command-list loop shared by whole parses and inline sections (GNU
/// parse.y:1264 list grammar): walks `tokens` from `start`, pushing
/// completed commands into `state.ast.commands`. Returns the index it
/// stopped at — tokens.len() when the input ran out, or the closer token
/// that ended an open section (state.section_stop). GNU anchor: GNU's
/// parser consumes a compound body's tokens with the SAME yyparse run
/// (parse.y:1037-1054 if_command: the body is a compound_list shifted
/// from the live token stream) — this runner is that shared pass; the
/// old model re-walked every body once to find its closer and again to
/// parse it, which multiplied token work by the nesting depth.
pub(super) fn run_command_loop(tokens: &[Token], state: &mut ParseState, start: usize) -> usize {
    state.section_token_start = start;
    let mut i = start;
    while i < tokens.len() {
        let section_stack_before = state.section_stack.len();
        // Inline-section closer check (GNU grammar position): the slice
        // scanner this replaces (parse_if_section & siblings) stopped at
        // exactly the tokens matching `stack.is_empty() &&
        // boundary-allowed keyword in stop set`; the loop only iterates
        // section-level tokens (nested compounds are skipped via next_i
        // jumps and their keyword frames are net balanced — see the
        // truncate after try_parse_compound_start), so this predicate
        // fires on the same token the scanner chose.
        if state.section_stop != SectionStop::None && section_stop_hit(tokens, i, state) {
            return i;
        }
        if state.section_stop != SectionStop::None {
            update_compound_boundary_stack(tokens, i, &mut state.section_stack);
        }
        // A word token whose text ends inside an unclosed `$(` leaves its
        // `)` closer to arrive as a later top-level token.
        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::CommandSubst | TokenKind::Assignment
        ) {
            // Gate the residual on the corrected balancer (skip.rs): it knows
            // the `case WORD in' chain (GNU special_case_tokens,
            // parse.y:3369-3386 + 3433-3441 — `case W in esac' is the empty
            // case whose `)` closes the substitution), while
            // unclosed_command_substitution_depth (continuation.rs) does not
            // and reports a false residual of 1 for a folded token like
            // `v=$(case x in esac)' (rubash#284). That phantom residual made
            // the guard below swallow the line's real `;;'/`)' tokens as
            // closers and run `... echo hi;; esac)` instead of GNU's
            // `syntax error near unexpected token `;;''.
            // Admission: every `return false` arm of
            // command_substitutions_balanced (skip.rs) opens with an
            // unquoted `$` (unterminated `${`, unterminated `$(`) or a
            // backtick — a token raw with neither byte provably balances,
            // so the scan is skipped (GNU anchor: parse.y:3557 read_token
            // derives comsub state from the token text itself; this gate
            // only skips scans whose answer is provably "balanced").
            let balanced = if tokens[i].raw.contains('$') || tokens[i].raw.contains('`') {
                crate::lexer::command_substitutions_balanced(&tokens[i].raw)
            } else {
                true
            };
            if !balanced {
                state.pending_comsub +=
                    crate::lexer::unclosed_command_substitution_depth(&tokens[i].raw);
            }
        }
        if state.pending_comsub > 0
            && [")", ";;", ";&", ";;;&"]
                .iter()
                .any(|op| super::is_unquoted_operator(&tokens[i], op))
        {
            // Still inside an unfolded `$(` body: `)` is its closer and
            // case terminators belong to the body's own case syntax —
            // neither is a top-level stray. A quoted `')'` is word text
            // (rubash#128), never a closer.
            if super::is_unquoted_operator(&tokens[i], ")") {
                state.pending_comsub -= 1;
            }
            i += 1;
            continue;
        }
        // GNU parse.y: a ')' or a case clause terminator at command position
        // is a syntax error that aborts the remaining input ("case x in
        // esac)" -- the empty case list closes at esac and the ')' is
        // unexpected). The parser used to drop the token silently and run
        // the rest of the line as a simple command.
        // A `)` at top level is always stray: subshell and case-pattern
        // closers are consumed inside their own constructs (in_subshell is
        // set while a `( ... )` body is open), so any `)` reaching the main
        // loop — at command start or mid-command (`echo x)`) — is GNU's
        // `syntax error near unexpected token `)''. Case terminators
        // (`;;', `;&', `;;;&') are likewise consumed by
        // parse_case_command inside a case body, so ANY one reaching the
        // main loop is stray — at command start (`f() { ;; }') or
        // mid-command (`echo ;;'), where GNU's grammar also rejects it.
        if (matches!(tokens[i].raw.as_str(), ";;" | ";&" | ";;;&"))
            || (state.stray_close_is_error
                && super::is_unquoted_operator(&tokens[i], ")")
                && !state.in_subshell)
        {
            push_unexpected_token_error(state, tokens, i);
            break;
        }

        // GNU parse.y:1264-1290 (list grammar): every `;' (and `&')
        // separator must follow a COMPLETED list1 immediately —
        // `list1 ';' newline_list list1', with list0's terminal form
        // `list1 ';' newline_list' allowing only newlines after the final
        // separator. A `;' at list start (`; :'), after another separator
        // (`: ; ;'), or after a newline (`:' NEWLINE `;': the newline
        // already terminated the list1, and no production admits a `;'
        // there) is `syntax error near unexpected token `;''. Newline
        // separator tokens (line_break) are the linebreak production and
        // remain legal wherever they appear. `&' follows the same rules
        // (parse.y:1281-1285). A redirection operator still expecting its
        // word operand also cannot precede a separator (`echo > ;').
        if matches!(tokens[i].kind, TokenKind::Semicolon | TokenKind::Background)
            && !tokens[i].line_break
            && separator_lacks_preceding_command(tokens, i, state.section_token_start)
        {
            push_unexpected_token_error(state, tokens, i);
            break;
        }

        if let Some(next_i) = try_parse_compound_start(tokens, i, state) {
            // GNU parse.y: a complete compound command (including its
            // trailing redirections) must be followed by a command
            // connector — ';', '&', a newline, '|', '&&' or '||'. A token
            // that would start the next command without one is a syntax
            // error: `{ a; } { b; }', `x() { :; } > f { ...; }' and
            // `if ...; fi echo' all report "syntax error near unexpected
            // token `X'".
            let separated = next_i > 0
                && matches!(
                    tokens[next_i - 1].kind,
                    TokenKind::Semicolon
                        | TokenKind::Pipe
                        | TokenKind::PipeErr
                        | TokenKind::And
                        | TokenKind::Or
                        | TokenKind::Background
                );
            let already_error = state
                .ast
                .commands
                .last()
                .is_some_and(|command| command.has_assignment("__RUBASH_PARSE_ERROR__"));
            // rubash#305: a redirection OPERATOR the compound's own
            // trailing-redirect collector stopped at is not "the next
            // command" — in GNU it starts another redirection of the
            // compound (parse.y:672+ `command: ... redirection`), so the
            // grammar error is the missing WORD operand, diagnosed by the
            // shared dangling-operand rules (`done <<EOF>#c` gathers the
            // body at the NEWLINE and reports `newline' on the
            // post-gathering line; `done << ;` reports `;', ...). Only
            // when an operand exists (collector break for another reason)
            // does the unexpected-token fallback keep its old reading.
            if !separated
                && !already_error
                && command_is_empty(&state.current_cmd)
                && tokens.get(next_i).is_some_and(|next| {
                    matches!(
                        next.kind,
                        TokenKind::RedirectIn
                            | TokenKind::RedirectOut
                            | TokenKind::Append
                            | TokenKind::RedirectErr
                            | TokenKind::RedirectErrAppend
                            | TokenKind::HereDoc
                            | TokenKind::HereString
                    )
                })
            {
                if let Some(error) = super::token_actions::missing_redirect_target_node(
                    tokens,
                    next_i,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ) {
                    let pop_line = tokens[next_i].position;
                    while state
                        .ast
                        .commands
                        .last()
                        .is_some_and(|command| command.line == Some(pop_line))
                    {
                        state.ast.commands.pop();
                    }
                    state.current_cmd = error;
                    break;
                }
            }
            if !separated
                && !already_error
                && command_is_empty(&state.current_cmd)
                && tokens.get(next_i).is_some_and(|next| {
                    !matches!(
                        next.kind,
                        TokenKind::Semicolon
                            | TokenKind::Pipe
                            | TokenKind::PipeErr
                            | TokenKind::And
                            | TokenKind::Or
                            | TokenKind::Background
                            | TokenKind::HereDocBody
                            | TokenKind::Eof
                    )
                })
            {
                push_unexpected_token_error(state, tokens, next_i);
                break;
            }
            // A folded compound whose body carries a parse-error node fails
            // the whole logical line: GNU yyparse reads the complete command
            // list before executing any of it, so nothing earlier on the
            // compound's own script lines runs either (`echo one; { :; } }'
            // prints nothing — rubash#285 side-fix). Hoist the inner error
            // node to the top level, drop the commands parsed from the
            // compound's lines, and stop reading the input like yyerror's
            // jump to top level.
            if let Some(error) = state
                .ast
                .commands
                .last()
                .and_then(compound_body_parse_error)
                .cloned()
            {
                let compound_line = tokens.get(i).map(|token| token.position);
                match compound_line {
                    Some(line) => {
                        while state
                            .ast
                            .commands
                            .last()
                            .is_some_and(|command| command.line.is_some_and(|l| l >= line))
                        {
                            state.ast.commands.pop();
                        }
                    }
                    None => {
                        state.ast.commands.pop();
                    }
                }
                state.ast.commands.push(error);
                break;
            }
            // A successfully parsed compound's keyword frames are net
            // balanced (`if..fi`, `case..esac`, `{..}`), exactly like the
            // slice scanner's stack after walking the same region, so the
            // frames this iteration's opener pushed are discarded instead
            // of being left open (which would suppress every later stop).
            state.section_stack.truncate(section_stack_before);
            i = next_i;
            continue;
        }
        match handle_token(tokens, &mut i, state) {
            TokenAction::Advance => i += 1,
            TokenAction::Continue => continue,
            TokenAction::Break => break,
        }
    }

    i
}

/// The post-loop fold passes over one command list (whole parse or inline
/// section body): gated exactly as when they lived inline in
/// parse_with_options.
pub(super) fn fold_command_list(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    // Fast-exit admissions: each fold below is a whole-list pass whose map
    // is the identity for commands lacking its trigger field, so a list
    // with no trigger anywhere returns unchanged — skipping the
    // per-command rebuild. (GNU anchor: the folds model grammar
    // productions the token stream already proved absent; a list with no
    // pipe connector has no pipeline production to fold — parse.y:1337ff
    // — and so on for `time'/`!'/`&&'/`&'. nvm.sh -n runs 1578
    // parse_with_options calls for nested bodies whose commands carry no
    // trigger at all; the six unconditional passes cost 468ms of its
    // 848ms parse phase.)

    let has_pipeline = commands.iter().any(|command| command.pipe.is_some());
    let c = if has_pipeline {
        fold_pipeline_commands(commands)
    } else {
        commands
    };
    // time_prefix_from_command fires only when a command's first word is
    // the literal `time'; fold_time_pipeline_commands looks through
    // pipeline_command stages' first words, fold_time_simple_commands at
    // the command's own words — the gate covers both.
    let has_time_prefix = c.iter().any(|command| {
        command.words.first().is_some_and(|word| word == "time")
            || command
                .pipeline_command
                .as_ref()
                .and_then(|pipeline| pipeline.stages.first())
                .and_then(|stage| stage.words.first())
                .is_some_and(|word| word == "time")
    });
    let c = if has_time_prefix {
        fold_time_pipeline_commands(c)
    } else {
        c
    };
    let c = if has_time_prefix {
        fold_time_simple_commands(c)
    } else {
        c
    };
    let has_inverted = c.iter().any(|command| command.inverted);
    let c = if has_inverted {
        fold_inverted_commands(c)
    } else {
        c
    };
    let has_and_or = c.iter().any(|command| command.and_or.is_some());
    let c = if has_and_or {
        fold_and_or_list_commands(c)
    } else {
        c
    };
    let has_background = c.iter().any(|command| command.background);
    if has_background {
        fold_background_commands(c)
    } else {
        c
    }
}

/// The innermost `__RUBASH_PARSE_ERROR__`-marked command inside a folded
/// compound's body (brace group, function body, case clause bodies), or the
/// command itself when it is the error node. The main loop uses this to fail
/// the whole logical line the way GNU's yyparse does.
/// Which closer set an inline section stops at. Mirrors the stop sets of
/// the slice scanners this replaces (find_if_then / parse_if_section).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SectionStop {
    None,
    /// An `if`/`elif` condition: stops at the `then` keyword.
    IfCondition,
    /// A `then`/`elif`/`else` body: stops at `fi`/`done`/`esac` (value
    /// match) or `elif`/`else` (keyword match); `then` marks the
    /// construct-invalid abort the scanner reported as None.
    IfBody,
}

/// Whether `tokens[index]` is the closer the open section stops at. The
/// predicate is the slice scanner's own stop test (stack empty +
/// command_boundary_keyword_allowed + stop set), evaluated on the loop's
/// iteration points — the same tokens the scanner walked.
fn section_stop_hit(tokens: &[Token], index: usize, state: &ParseState) -> bool {
    if !state.section_stack.is_empty() {
        return false;
    }
    if !super::command_boundary_keyword_allowed(tokens, index) {
        return false;
    }
    let token = &tokens[index];
    match state.section_stop {
        SectionStop::None => false,
        SectionStop::IfCondition => is_keyword(tokens, index, "then"),
        SectionStop::IfBody => {
            matches!(token.value.as_str(), "fi" | "done" | "esac")
                || is_keyword(tokens, index, "elif")
                || is_keyword(tokens, index, "else")
                || is_keyword(tokens, index, "then")
        }
    }
}

/// How an inline section ended.
pub(super) enum SectionOutcome {
    /// Stopped at the closer token; `index` IS the closer (not consumed).
    Ended {
        body: Vec<CommandNode>,
        index: usize,
    },
    /// Input ran out before any closer (the scanner's None-by-EOF).
    Eof { body: Vec<CommandNode> },
    /// The loop broke on a parse error before the closer; the body's last
    /// command carries the error node (the scanner still knew the real
    /// boundary — callers recover it with the cold scanner when needed).
    Errored { body: Vec<CommandNode> },
    /// `then` arrived where a body was expected (parse_if_section's
    /// return-None abort).
    AbortedThen {
        body: Vec<CommandNode>,
        index: usize,
    },
}

/// Parse one compound-body section inline in the caller's parse state
/// (GNU parse.y: the body of `if COND; then BODY; fi` is a compound_list
/// consumed by the SAME yyparse run — parse.y:1037-1054 — not a re-scan
/// plus a re-parse of a sliced copy). Swaps in a fresh command list (and
/// the per-body balancer state the old fresh ParseState started with:
/// pending_comsub 0, in_subshell false, stray-close leniency), runs the
/// shared command loop until the closer predicate fires, restores the
/// caller's state, and applies the same fold/extglob tail the slice
/// parse ran over its body slice.
pub(super) fn run_inline_section(
    tokens: &[Token],
    state: &mut ParseState,
    start: usize,
    stop: SectionStop,
) -> SectionOutcome {
    let saved_commands = std::mem::take(&mut state.ast.commands);
    let saved_current = std::mem::take(&mut state.current_cmd);
    let saved_pending_comsub = state.pending_comsub;
    let saved_in_subshell = state.in_subshell;
    let saved_stray = state.stray_close_is_error;
    let saved_stop = state.section_stop;
    let saved_stack = std::mem::take(&mut state.section_stack);
    let saved_start = state.section_token_start;

    state.pending_comsub = 0;
    state.in_subshell = false;
    state.stray_close_is_error = false;
    state.section_stop = stop;

    let i = run_command_loop(tokens, state, start);

    // Classify with the SECTION state still armed (the restored caller
    // state below must not influence the closer predicate).
    let at_eof = i >= tokens.len();
    let aborted_then = !at_eof
        && stop == SectionStop::IfBody
        && state.section_stack.is_empty()
        && super::command_boundary_keyword_allowed(tokens, i)
        && is_keyword(tokens, i, "then");
    let ended = !at_eof
        && !aborted_then
        && state.section_stack.is_empty()
        && super::command_boundary_keyword_allowed(tokens, i)
        && match stop {
            SectionStop::None => false,
            SectionStop::IfCondition => is_keyword(tokens, i, "then"),
            SectionStop::IfBody => {
                matches!(tokens[i].value.as_str(), "fi" | "done" | "esac")
                    || is_keyword(tokens, i, "elif")
                    || is_keyword(tokens, i, "else")
            }
        };

    let mut body = std::mem::take(&mut state.ast.commands);
    if !command_is_empty(&state.current_cmd) {
        body.push(std::mem::take(&mut state.current_cmd));
    }
    // The stopped-at closer token (when present) is exactly the slice
    // boundary the scanner produced, so this sub-slice equals the old
    // body slice for both the fold gates and the extglob marker.
    let body_tokens = &tokens[start..i.min(tokens.len())];
    body = fold_command_list(body);
    let mut body_ast = Ast { commands: body };
    mark_parse_time_extglob_errors(&mut body_ast, body_tokens);
    let body = body_ast.commands;

    state.ast.commands = saved_commands;
    state.current_cmd = saved_current;
    state.pending_comsub = saved_pending_comsub;
    state.in_subshell = saved_in_subshell;
    state.stray_close_is_error = saved_stray;
    state.section_stop = saved_stop;
    state.section_stack = saved_stack;
    state.section_token_start = saved_start;

    if at_eof {
        return SectionOutcome::Eof { body };
    }
    if aborted_then {
        return SectionOutcome::AbortedThen { body, index: i };
    }
    if ended {
        return SectionOutcome::Ended { body, index: i };
    }
    SectionOutcome::Errored { body }
}

fn compound_body_parse_error(cmd: &CommandNode) -> Option<&CommandNode> {
    if cmd.has_assignment("__RUBASH_PARSE_ERROR__")
        || cmd.has_assignment("__RUBASH_PARSE_ERROR_NEAR__")
    {
        return Some(cmd);
    }
    if let Some(group) = &cmd.brace_group {
        for body in &group.body {
            if let Some(error) = compound_body_parse_error(body) {
                return Some(error);
            }
        }
    }
    if let Some(function) = &cmd.function_command {
        for body in &function.body {
            if let Some(error) = compound_body_parse_error(body) {
                return Some(error);
            }
        }
    }
    if let Some(case_cmd) = &cmd.case_command {
        for clause in &case_cmd.clauses {
            for body in &clause.body {
                if let Some(error) = compound_body_parse_error(body) {
                    return Some(error);
                }
            }
        }
    }
    None
}

/// Extglob is enabled while Bash parses a command unit, not after the unit
/// has already been read.  Therefore `shopt -s extglob; echo @(x)` is a
/// syntax error even though a later input line can use the pattern.
fn mark_parse_time_extglob_errors(ast: &mut Ast, tokens: &[Token]) {
    let mut saw_shopt = false;
    let mut saw_enable = false;
    let mut extglob_enabled_in_unit = false;

    for token in tokens {
        if token.kind == TokenKind::Semicolon && !token.line_break {
            if saw_shopt && saw_enable {
                extglob_enabled_in_unit = true;
            }
            saw_shopt = false;
            saw_enable = false;
            continue;
        }
        if token.value == "shopt" {
            saw_shopt = true;
            continue;
        }
        if saw_shopt && token.value == "-s" {
            saw_enable = true;
            continue;
        }
        if saw_enable && token.value == "extglob" {
            continue;
        }
        if extglob_enabled_in_unit && is_unquoted_extglob_word(token) {
            for command in &mut ast.commands {
                if command.words.iter().any(|word| word == &token.value) {
                    command.insert_assignment(
                        "__RUBASH_PARSE_ERROR__".to_string(),
                        "unexpected token `('".to_string(),
                    );
                    break;
                }
            }
        }
        if token.kind != TokenKind::Semicolon {
            saw_shopt = false;
            saw_enable = false;
        }
    }
}

fn is_unquoted_extglob_word(token: &Token) -> bool {
    token.raw == token.value
        && token
            .value
            .chars()
            .any(|operator| matches!(operator, '@' | '!' | '+' | '?' | '*'))
        && token.value.contains("(")
}

fn fold_inverted_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.inverted {
                return command;
            }

            command.inverted = false;
            let line = command.line;
            let and_or = command.and_or.take();
            let background_flag = command.background;
            command.background = false;
            let mut inverted = CommandNode::new();
            inverted.line = line;
            inverted.and_or = and_or;
            inverted.background = background_flag;
            inverted.inverted_command = Some(InvertedCommand {
                operator: "!".to_string(),
                operator_metadata: operator_metadata("!"),
                command: Box::new(command),
            });
            inverted
        })
        .collect()
}

fn fold_background_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command.background {
                return command;
            }

            command.background = false;
            let line = command.line;
            let mut background = CommandNode::new();
            background.line = line;
            background.background_command = Some(BackgroundCommand {
                operator: "&".to_string(),
                operator_metadata: operator_metadata("&"),
                command: Box::new(command),
            });
            background
        })
        .collect()
}

fn operator_metadata(operator: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, operator, operator))
}

fn fold_and_or_list_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut commands = commands;
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        // GNU parse.y's list reduction (parse.y:1264 `list: list ';' list1`)
        // links the ALREADY-BUILT command into the list — make_cmd.c never
        // copies a COMMAND. The index below only ever advances, so moving
        // the element out (the empty replacement is never re-read: every
        // later access is `commands.get(j)` with j > the taken slot)
        // reproduces the old deep-clone result without copying it.
        let command = std::mem::replace(&mut commands[index], CommandNode::new());
        if command.and_or.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut list_commands = vec![command];
        let mut connectors = Vec::new();
        let mut operators = Vec::new();
        index += 1;
        while let Some(connector) = list_commands.last().and_then(|command| command.and_or) {
            connectors.push(connector);
            operators.push(if connector { "&&" } else { "||" }.to_string());
            while commands.get(index).is_some_and(command_is_empty) {
                index += 1;
            }
            if index >= commands.len() {
                break;
            }
            let next = std::mem::replace(&mut commands[index], CommandNode::new());
            list_commands.push(next);
            index += 1;
        }

        if connectors.is_empty() || list_commands.len() != connectors.len() + 1 {
            if list_commands.len() == 1 && !connectors.is_empty() {
                let mut command = list_commands
                    .into_iter()
                    .next()
                    .expect("and-or list has a command");
                command.insert_assignment(
                    "__RUBASH_PARSE_ERROR__".to_string(),
                    "unexpected end of file".to_string(),
                );
                folded.push(command);
                continue;
            }
            folded.extend(list_commands);
            continue;
        }

        let first = list_commands
            .first()
            .expect("and-or list has a first command");
        let last = list_commands
            .last()
            .expect("and-or list has a last command");
        let mut list = CommandNode::new();
        list.line = first.line;
        list.background = last.background;
        list.and_or_list = Some(AndOrListCommand {
            commands: list_commands,
            connectors,
            operator_metadata: operators_metadata(&operators),
            operators,
        });
        folded.push(list);
    }
    folded
}

fn fold_pipeline_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    let mut commands = commands;
    let mut folded = Vec::new();
    let mut index = 0;
    while index < commands.len() {
        // Same move-not-clone discipline as fold_and_or_list_commands: GNU's
        // pipeline production (parse.y:1378 pipeline: pipeline '|' command)
        // links the parsed stages; the taken slot is never re-read (the
        // index only advances, and skip_empty_pipeline_separators starts
        // from the current, untaken index).
        let command = std::mem::replace(&mut commands[index], CommandNode::new());
        if command.pipe.is_none() {
            folded.push(command);
            index += 1;
            continue;
        }

        let mut stages = vec![command];
        let mut operators = Vec::new();
        index += 1;
        skip_empty_pipeline_separators(&commands, &mut index);
        while index < commands.len() {
            if let Some(pipe) = stages.last().and_then(|stage| stage.pipe) {
                operators.push(if pipe == 2 { "|&" } else { "|" }.to_string());
            }
            let command = std::mem::replace(&mut commands[index], CommandNode::new());
            let pipe_done = command.pipe.is_none();
            stages.push(command);
            index += 1;
            if pipe_done {
                break;
            }
            skip_empty_pipeline_separators(&commands, &mut index);
        }

        if stages.len() == 1
            || stages.last().is_some_and(|command| command.pipe.is_some())
            || looks_like_case_pattern_alternate(&stages)
        {
            if stages.len() == 1 || stages.last().is_some_and(|command| command.pipe.is_some()) {
                let mut command = stages
                    .into_iter()
                    .next()
                    .expect("pipeline has a first stage");
                // GNU parse.y yyerror: a `|`/`|&` with no right-hand stage
                // because input ended reports "syntax error: unexpected
                // end of file" — not `near unexpected token `|'' (that
                // wording belongs to `|` mid-list).
                command
                    .insert_assignment("__RUBASH_PARSE_ERROR_EOF__".to_string(), "1".to_string());
                folded.push(command);
                continue;
            }
            folded.extend(stages);
            continue;
        }

        fold_time_pipeline_stage_commands(&mut stages);

        let first = stages.first().expect("pipeline has a first stage");
        let last = stages.last().expect("pipeline has a last stage");
        let mut pipeline = CommandNode::new();
        pipeline.line = first.line;
        pipeline.inverted = first.inverted;
        pipeline.background = last.background;
        pipeline.and_or = last.and_or;
        if let Some(first_stage) = stages.first_mut() {
            first_stage.inverted = false;
        }
        pipeline.pipeline_command = Some(PipelineCommand {
            stages,
            operator_metadata: operators_metadata(&operators),
            operators,
        });
        folded.push(pipeline);
    }
    folded
}

fn skip_empty_pipeline_separators(commands: &[CommandNode], index: &mut usize) {
    while commands.get(*index).is_some_and(command_is_empty) {
        *index += 1;
    }
}

fn operators_metadata(operators: &[String]) -> Vec<WordMetadata> {
    operators
        .iter()
        .enumerate()
        .map(|(index, operator)| build_word_metadata(index, operator, operator))
        .collect()
}

fn looks_like_case_pattern_alternate(stages: &[CommandNode]) -> bool {
    let Some(first) = stages.first() else {
        return false;
    };
    if first.words.get(2).map(String::as_str) != Some("in") {
        return false;
    }
    first.words.len() >= 4 && stages.len() >= 2
}

fn fold_time_pipeline_stage_commands(stages: &mut [CommandNode]) {
    for stage in stages.iter_mut().skip(1) {
        let folded = fold_time_pipeline_stage_command(std::mem::take(stage));
        *stage = folded;
    }
}

fn fold_time_pipeline_stage_command(mut command: CommandNode) -> CommandNode {
    if !command_is_time_simple_candidate(&command) {
        return command;
    }
    let Some(prefix) = time_prefix_from_command(&mut command) else {
        return command;
    };

    let mut timed = CommandNode::new();
    timed.line = command.line;
    timed.inverted = command.inverted;
    timed.pipe = command.pipe.take();
    timed.redirect_in = command.redirect_in.clone();
    timed.redirect_out = command.redirect_out.clone();
    timed.append = command.append.clone();
    timed.redirect_err = command.redirect_err.clone();
    timed.redirect_err_append = command.redirect_err_append.clone();
    command.inverted = false;
    timed.time_command = Some(TimeCommand {
        keyword: prefix.keyword,
        keyword_metadata: prefix.keyword_metadata,
        prefix_words: prefix.prefix_words,
        prefix_word_metadata: prefix.prefix_word_metadata,
        command: Box::new(command),
        posix_format: prefix.posix_format,
        inverted: prefix.inverted,
    });
    timed
}

fn fold_time_pipeline_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            let Some(pipeline) = command.pipeline_command.as_mut() else {
                return command;
            };
            let Some(prefix) = time_prefix_from_pipeline(pipeline) else {
                return command;
            };

            let line = command.line;
            let inverted = command.inverted;
            let and_or = command.and_or.take();
            let background = command.background;
            command.inverted = false;
            command.background = false;
            command.and_or = None;
            let mut timed = CommandNode::new();
            timed.line = line;
            timed.inverted = inverted;
            timed.and_or = and_or;
            timed.background = background;
            timed.time_command = Some(TimeCommand {
                keyword: prefix.keyword,
                keyword_metadata: prefix.keyword_metadata,
                prefix_words: prefix.prefix_words,
                prefix_word_metadata: prefix.prefix_word_metadata,
                command: Box::new(command),
                posix_format: prefix.posix_format,
                inverted: prefix.inverted,
            });
            timed
        })
        .collect()
}

struct TimePipelinePrefix {
    keyword: String,
    keyword_metadata: Box<WordMetadata>,
    prefix_words: Vec<String>,
    prefix_word_metadata: Vec<WordMetadata>,
    posix_format: bool,
    inverted: bool,
}

fn time_prefix_from_pipeline(pipeline: &mut PipelineCommand) -> Option<TimePipelinePrefix> {
    let first = pipeline.stages.first_mut()?;
    time_prefix_from_command(first)
}

fn fold_time_simple_commands(commands: Vec<CommandNode>) -> Vec<CommandNode> {
    commands
        .into_iter()
        .map(|mut command| {
            if !command_is_time_simple_candidate(&command) {
                return command;
            }
            let Some(prefix) = time_prefix_from_command(&mut command) else {
                return command;
            };

            let line = command.line;
            let inverted = command.inverted;
            let and_or = command.and_or.take();
            let background = command.background;
            command.inverted = false;
            command.background = false;
            let mut timed = CommandNode::new();
            timed.line = line;
            timed.inverted = inverted;
            timed.and_or = and_or;
            timed.background = background;
            timed.time_command = Some(TimeCommand {
                keyword: prefix.keyword,
                keyword_metadata: prefix.keyword_metadata,
                prefix_words: prefix.prefix_words,
                prefix_word_metadata: prefix.prefix_word_metadata,
                command: Box::new(command),
                posix_format: prefix.posix_format,
                inverted: prefix.inverted,
            });
            timed
        })
        .collect()
}

fn command_is_time_simple_candidate(command: &CommandNode) -> bool {
    command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none()
}

fn time_prefix_from_command(command: &mut CommandNode) -> Option<TimePipelinePrefix> {
    if command.words.first().map(String::as_str) != Some("time") {
        return None;
    }

    let mut index = 1;
    let mut posix_format = false;
    let mut inverted = false;
    let mut prefix_words = Vec::new();
    let mut prefix_word_metadata = Vec::new();
    while let Some(word) = command.words.get(index).map(String::as_str) {
        match word {
            "-p" | "--" | "!" => {
                prefix_word_metadata.push(build_word_metadata(prefix_words.len(), word, word));
                prefix_words.push(word.to_string());
                if word == "-p" {
                    posix_format = true;
                } else if word == "!" {
                    inverted = !inverted;
                }
                index += 1;
            }
            _ => break,
        }
    }
    let keyword = command.words[0].clone();
    let keyword_metadata = Box::new(build_word_metadata(0, &keyword, &keyword));
    let old_word_len = command.words.len();
    command.words = command.words[index..].to_vec();
    if command.word_kinds.len() == old_word_len {
        command.word_kinds = command.word_kinds[index..].to_vec();
    }
    if command.word_metadata.len() == old_word_len {
        command.word_metadata = command.word_metadata[index..].to_vec();
    }
    Some(TimePipelinePrefix {
        keyword,
        keyword_metadata,
        prefix_words,
        prefix_word_metadata,
        posix_format,
        inverted,
    })
}

fn try_parse_compound_start(tokens: &[Token], i: usize, state: &mut ParseState) -> Option<usize> {
    let token = &tokens[i];

    if token.kind == TokenKind::Keyword
        && token.value == "time"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((time_cmd, next_i)) = parse_time_prefixed_compound_command(tokens, i) {
            push_compound_command(state, time_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "if"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((if_cmd, next_i)) = parse_if_command(tokens, i, state) {
            push_compound_command(state, if_cmd);
            return Some(next_i);
        }

        // GNU parse.y if-compound grammar: the offender is the first
        // keyword the grammar rejects at its position (`if then; fi` →
        // `then`, `if x; fi` → `fi`, `if x; then y; done` → `done`); absent
        // that, input ended inside the command ("from `if' command on
        // line N").
        let mut command = match if_frame_offender(&tokens[i + 1..]) {
            Some(rel) => mismatched_closer_node(
                tokens,
                i + 1 + rel,
                state.diagnostic_text.as_deref(),
                state.source_line_offset,
            ),
            None => unclosed_keyword_eof_node(tokens, i, "if"),
        };
        // GNU parse.y: line_number at EOF is PHYSICAL — every newline
        // read advances it, including comment-only and blank trailing
        // lines that produce no tokens (rubash#135: `if {[catch {` plus
        // comment lines reports the last physical line + 1, not the last
        // token line + 1). GNU reads a final newline even when the input
        // lacks one (an EOL-less `if {` still reports line 2), so the
        // EOF line is newline-count + 1 unconditionally.
        if let Some(text) = state.diagnostic_text.as_deref() {
            let physical_eof = state.source_line_offset
                + text.chars().filter(|c| *c == '\n').count()
                + usize::from(!text.ends_with('\n'))
                + 1;
            for (name, value) in command.assignments.iter_mut() {
                if name == "__RUBASH_PARSE_ERROR_EOF_COMPOUND__" {
                    let mut fields = value.split(PARSE_ERROR_FIELD_SEP);
                    let compound = fields.next().unwrap_or("if").to_string();
                    let open_line = fields.next().unwrap_or("1").to_string();
                    *value = format!(
                        "{compound}{PARSE_ERROR_FIELD_SEP}{open_line}{PARSE_ERROR_FIELD_SEP}{physical_eof}"
                    );
                }
            }
        }
        // Keep the original token stream available to the executor.  Bash
        // expands aliases while parsing, so an alias such as `f=fi` can
        // close this compound command even though the first parse did not
        // see `fi`. The parse-error node still makes genuinely malformed
        // input fail after the alias reparse misses.
        command.insert_assignment(
            "__RUBASH_PARSE_SOURCE_SPAN__".to_string(),
            tokens[i..]
                .iter()
                .map(|token| token.raw.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        state.current_cmd = command;
        state
            .ast
            .commands
            .push(std::mem::take(&mut state.current_cmd));
        return Some(tokens.len());
    }

    if token.kind == TokenKind::Keyword
        && matches!(token.value.as_str(), "while" | "until")
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((loop_cmd, next_i)) = parse_loop_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, loop_cmd);
            return Some(next_i);
        }

        let opener: &'static str = if token.value == "while" {
            "while"
        } else {
            "until"
        };
        // No `done` anywhere in the remaining stream: input either ended
        // inside the loop ("from `while' command on line N") or a foreign
        // closer aborted it ("near unexpected token `X'").
        if !tokens[i + 1..]
            .iter()
            .any(|t| t.kind == TokenKind::Keyword && t.value == "done")
        {
            let command = match first_mismatched_closer(&tokens[i + 1..], opener) {
                None => unclosed_keyword_eof_node(tokens, i, opener),
                Some(rel) => mismatched_closer_node(
                    tokens,
                    i + 1 + rel,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ),
            };
            state.current_cmd = command;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(tokens.len());
        }

        state.current_cmd.insert_assignment(
            "__RUBASH_PARSE_ERROR__".to_string(),
            "unexpected token `do'".to_string(),
        );
        let mut next_i = i + 1;
        while tokens.get(next_i).is_some() {
            let is_done = is_keyword(tokens, next_i, "done");
            next_i += 1;
            if is_done {
                break;
            }
        }
        state
            .ast
            .commands
            .push(std::mem::take(&mut state.current_cmd));
        return Some(next_i);
    }

    if token.kind == TokenKind::Keyword
        && token.value == "for"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((for_cmd, next_i)) = parse_for_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, for_cmd);
            return Some(next_i);
        }
        let arithmetic_for_marker = tokens.get(i + 1).is_some_and(|next| next.value == "((")
            || (tokens.get(i + 1).is_some_and(|next| next.value == "(")
                && tokens.get(i + 2).is_some_and(|next| next.value == "("));
        if arithmetic_for_marker {
            let open = i + if tokens.get(i + 1).is_some_and(|t| t.value == "((") {
                2
            } else {
                3
            };
            let mut semicolons = 0u32;
            let mut depth = 0i32;
            let mut close = open;
            for j in open..tokens.len() {
                let t = &tokens[j];
                let op = |v: &str| super::is_unquoted_operator(t, v);
                let next_op = |v: &str| {
                    tokens
                        .get(j + 1)
                        .is_some_and(|n| super::is_unquoted_operator(n, v))
                };
                if op("((") || (op("(") && next_op("(")) {
                    depth += 1;
                } else if op("))") || (op(")") && next_op(")")) {
                    if depth == 0 {
                        close = j;
                        break;
                    }
                    depth -= 1;
                } else if depth == 0 && t.kind == TokenKind::Semicolon {
                    semicolons += 1;
                }
            }
            let expr_raw: String = {
                let slice = &tokens[open..close];
                if slice.is_empty() {
                    String::new()
                } else {
                    let mut parts = Vec::with_capacity(slice.len());
                    for (idx, t) in slice.iter().enumerate() {
                        if idx == 0 {
                            parts.push(t.raw.clone());
                        } else {
                            let prev = &slice[idx - 1];
                            let prev_end = prev.column + prev.raw.len();
                            if t.column > prev_end {
                                parts.push(" ".to_string());
                            }
                            parts.push(t.raw.clone());
                        }
                    }
                    parts.concat()
                }
            };
            let error_msg = if semicolons < 2 {
                "syntax error: arithmetic expression required".to_string()
            } else {
                "syntax error: `;' unexpected".to_string()
            };
            state
                .current_cmd
                .insert_assignment("__RUBASH_PARSE_ERROR__".to_string(), error_msg);
            state.current_cmd.insert_assignment(
                "__RUBASH_PARSE_SOURCE__".to_string(),
                format!("(( {} ))", expr_raw.trim()),
            );
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(i + 1);
        }

        // No `done` in the remaining stream: input either ended inside
        // the `for` body ("from `for' command on line N") or a foreign
        // closer aborted it ("near unexpected token `X'").
        if !tokens[i + 1..]
            .iter()
            .any(|t| t.kind == TokenKind::Keyword && t.value == "done")
        {
            let command = match first_mismatched_closer(&tokens[i + 1..], "for") {
                None => unclosed_keyword_eof_node(tokens, i, "for"),
                Some(rel) => mismatched_closer_node(
                    tokens,
                    i + 1 + rel,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ),
            };
            state.current_cmd = command;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(tokens.len());
        }
    }

    if ((token.kind == TokenKind::Word)
        || (token.kind == TokenKind::Keyword && token.value == "function")
        || (token.kind == TokenKind::RedirectIn && matches!(token.value.as_str(), "<" | ">"))
        || (token.kind == TokenKind::Keyword && token.value == "!"))
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((function_cmd, next_i)) = parse_function_command_with_diagnostic(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, function_cmd);
            return Some(next_i);
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "case"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((case_cmd, next_i)) = parse_case_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, case_cmd);
            return Some(next_i);
        }
        // GNU parse.y case grammar: when `esac` never arrives the failure
        // splits by WHERE the token stream stopped. Input ending inside a
        // clause list (`case x in a) q;;`, or the empty list `case x in`)
        // reports "unexpected end of file from `case' command on line N".
        // A word that starts a pattern but is followed by something that
        // is not `)` reports that token (`case x in a b` -> `b`), or
        // `newline` when the line ends inside the pattern (`case x in a`).
        let rest = &tokens[i + 1..];
        let esac_absent = !rest
            .iter()
            .any(|t| t.kind == TokenKind::Keyword && t.value == "esac");
        if esac_absent {
            let tail_start = rest
                .iter()
                .position(|t| t.value == "in")
                .map(|p| p + 1)
                .unwrap_or(0);
            // A line break after `in` is legal case syntax (GNU's
            // linebreak production) — the lexer emits it as a Semicolon,
            // which is not a clause-position error.
            let semicolons = rest[tail_start..]
                .iter()
                .take_while(|t| t.kind == TokenKind::Semicolon)
                .count();
            let tail_start = tail_start + semicolons;
            let tail = &rest[tail_start..];
            let clause_prefix = tail
                .iter()
                .any(|t| matches!(t.value.as_str(), ")" | ";;" | ";&" | ";;&"));
            let command = match tail.first() {
                // `case x in` + EOF: the empty clause list is legal - only
                // `esac` is missing.
                None => Some(unclosed_keyword_eof_node(tokens, i, "case")),
                Some(first) if first.kind == TokenKind::Word && !clause_prefix => {
                    match tail.get(1) {
                        // `case x in a` EOF - GNU blames the virtual
                        // `newline` token where `)` was expected.
                        None => {
                            let mut command = CommandNode::new();
                            command.line = Some(first.position);
                            command.insert_assignment(
                                "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
                                format!("newline{PARSE_ERROR_FIELD_SEP}{}", first.position),
                            );
                            command.insert_assignment(
                                "__RUBASH_PARSE_SOURCE__".to_string(),
                                offending_line_text(
                                    tokens,
                                    i + 1 + tail_start,
                                    state.diagnostic_text.as_deref(),
                                    state.source_line_offset,
                                ),
                            );
                            Some(command)
                        }
                        // `a |` continues the pattern - still inside the
                        // clause list at EOF.
                        Some(next) if next.value == "|" => {
                            Some(unclosed_keyword_eof_node(tokens, i, "case"))
                        }
                        Some(_) => Some(mismatched_closer_node(
                            tokens,
                            i + 1 + tail_start + 1,
                            state.diagnostic_text.as_deref(),
                            state.source_line_offset,
                        )),
                    }
                }
                Some(_) if clause_prefix => Some(match first_mismatched_closer(rest, "case") {
                    None => unclosed_keyword_eof_node(tokens, i, "case"),
                    Some(rel) => mismatched_closer_node(
                        tokens,
                        i + 1 + rel,
                        state.diagnostic_text.as_deref(),
                        state.source_line_offset,
                    ),
                }),
                _ => None,
            };
            if let Some(command) = command {
                state.current_cmd = command;
                state
                    .ast
                    .commands
                    .push(std::mem::take(&mut state.current_cmd));
                return Some(tokens.len());
            }
        }
        return Some(push_parse_error_until(
            state,
            tokens,
            i,
            "esac",
            case_parse_error_message(tokens, i),
        ));
    }

    if token.kind == TokenKind::Keyword
        && token.value == "select"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((select_cmd, next_i)) = parse_select_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, select_cmd);
            return Some(next_i);
        }

        // Same unclosed-at-EOF rule as `for`: no `done` remaining means
        // input ended inside the `select` body or a foreign closer ended it.
        if !tokens[i + 1..]
            .iter()
            .any(|t| t.kind == TokenKind::Keyword && t.value == "done")
        {
            let command = match first_mismatched_closer(&tokens[i + 1..], "select") {
                None => unclosed_keyword_eof_node(tokens, i, "select"),
                Some(rel) => mismatched_closer_node(
                    tokens,
                    i + 1 + rel,
                    state.diagnostic_text.as_deref(),
                    state.source_line_offset,
                ),
            };
            state.current_cmd = command;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(tokens.len());
        }
    }

    if token.kind == TokenKind::Keyword
        && token.value == "coproc"
        && command_allows_compound_start(&state.current_cmd)
    {
        if let Some((coproc_cmd, next_i)) = parse_coproc_command(tokens, i) {
            push_compound_command(state, coproc_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            push_compound_command(state, arith_cmd);
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd) && token.value == "[[" {
        if let Some((conditional_cmd, next_i)) = parse_conditional_command(tokens, i) {
            push_compound_command(state, conditional_cmd);
            return Some(next_i);
        }
        // Unclosed `[[`: GNU's cond parser (parse.y cond_term/cond_error)
        // consumes the remaining tokens and reports the first offending
        // token or the `unexpected EOF while looking for `]]'' diagnostic.
        let (command, next_i) = conditional_eof_error_command(tokens, i);
        state.current_cmd = command;
        state
            .ast
            .commands
            .push(std::mem::take(&mut state.current_cmd));
        return Some(next_i);
    }

    if command_allows_compound_start(&state.current_cmd)
        && token.kind == TokenKind::Keyword
        && token.value == "("
    {
        if let Some((subshell_cmd, next_i)) = parse_subshell_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, subshell_cmd);
            return Some(next_i);
        }
        return Some(push_unclosed_paren_error(state, tokens, i));
    }

    if command_accepts_embedded_arithmetic_command(&state.current_cmd)
        && ((token.kind == TokenKind::Keyword && token.value == "(")
            || token.value.starts_with("(("))
    {
        if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
            note_command_line(&mut state.current_cmd, token);
            state.current_cmd.words.extend(arith_cmd.words);
            state.current_cmd.and_or = arith_cmd.and_or;
            state
                .ast
                .commands
                .push(std::mem::take(&mut state.current_cmd));
            return Some(next_i);
        }
    }

    if command_allows_compound_start(&state.current_cmd) {
        if let Some((brace_cmd, next_i)) = parse_brace_group_command(
            tokens,
            i,
            state.diagnostic_text.as_ref(),
            state.source_line_offset,
        ) {
            push_compound_command(state, brace_cmd);
            return Some(next_i);
        }
        if token.value == "{"
            || (token.value.starts_with('{')
                && token.value.contains(';')
                && !token.value.contains('}'))
        {
            return Some(push_parse_error_until(
                state,
                tokens,
                i,
                "}",
                "unexpected end of file",
            ));
        }
    }

    None
}

fn command_allows_compound_start(command: &CommandNode) -> bool {
    command_is_empty(command) || command_is_pending_inversion(command)
}

/// Compound-body re-parse that keeps the enclosing parse's diagnostic
/// context. Body token slices carry absolute script-line positions, and
/// GNU yyerror echoes the whole physical input line (parse.y:6813-6824
/// print_offending_line over shell_input_line), so a syntax error raised
/// inside a compound body must slice the ORIGINAL source text, not join
/// the body fragment (`{ :; ; }' is echoed in full, not as `: ; ;').
/// Callers without source context (alias/coproc reparses) pass None/0
/// and keep the fragment-join fallback.
pub(super) fn parse_body_with_diagnostics(
    tokens: &[Token],
    source: Option<&Rc<str>>,
    source_line_offset: usize,
) -> Vec<CommandNode> {
    parse_with_options(
        tokens,
        ParseLoopOptions {
            // Rc clone: refcount bump, not a text copy (see
            // ParseLoopOptions::diagnostic_text docs).
            diagnostic_text: source.cloned(),
            source_line_offset,
            ..ParseLoopOptions::default()
        },
    )
    .commands
}

/// True when the separator at `index` (a `;'/`&' token with
/// `line_break == false`) has no completed command immediately before it:
/// list start (`index == 0`, also the start of any compound-body token
/// slice), or the previous token is itself a separator/connector, or a
/// redirection operator still expecting its word operand. Mirrors the
/// GNU list grammar (parse.y:1264-1290), where a separator is only
/// grammatical directly after a completed `list1`.
fn separator_lacks_preceding_command(tokens: &[Token], index: usize, floor: usize) -> bool {
    if index <= floor {
        return true;
    }
    match index.checked_sub(1) {
        None => true,
        Some(prev) => matches!(
            tokens[prev].kind,
            TokenKind::Semicolon
                | TokenKind::Background
                | TokenKind::Pipe
                | TokenKind::PipeErr
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::RedirectIn
                | TokenKind::RedirectOut
                | TokenKind::RedirectErr
                | TokenKind::RedirectErrAppend
                | TokenKind::Append
                | TokenKind::HereDoc
                | TokenKind::HereString
        ),
    }
}

/// GNU parse.y reports `syntax error near unexpected token `X'' on the
/// offending token and echoes only that input line (yyerror + the current
/// input line), aborting the rest of the input.
fn push_unexpected_token_error(state: &mut ParseState, tokens: &[Token], i: usize) {
    push_unexpected_token_error_named(state, tokens, i, None);
}

/// Core of `push_unexpected_token_error`, reading the diagnostic context
/// from `ParseState` (`diagnostic_text`/`source_line_offset` are copied
/// from `ParseLoopOptions` at state init and never mutated), with an
/// explicit display name for the offending token — GNU names the virtual
/// `newline' token where its grammar expected `)' right after a shifted
/// function-head `(' (parse.y:1054 `WORD '(' ')'; `echo (' at end of input
/// reports `newline', not `(').
pub(super) fn push_unexpected_token_error_named(
    state: &mut ParseState,
    tokens: &[Token],
    i: usize,
    name_override: Option<&str>,
) {
    // GNU reports the yacc token: the lexer keeps a whole `{ ...; }' brace
    // group in one token, but the grammar token is just `{'.
    let token_text = name_override.unwrap_or_else(|| {
        if tokens[i].value.starts_with('{') {
            "{"
        } else {
            tokens[i].value.as_str()
        }
    });
    state.current_cmd.insert_assignment(
        "__RUBASH_PARSE_ERROR__".to_string(),
        format!("unexpected token `{token_text}'"),
    );
    // GNU's parser aborts the input line at the error, so commands already
    // parsed from the same line never run (`{ a; } { b; }' does not run
    // `a').  Drop the trailing commands parsed from the offending line.
    let error_line = tokens[i].position;
    while state.ast.commands.last().is_some_and(|command| {
        command.line == Some(error_line)
            // rubash#385: the failed command may SPAN lines — the pipeline
            // whose first stage opened on an earlier line (`case x in\n
            // esac|y) ...`: yyerror discards the whole in-progress command,
            // so a stage still awaiting its continuation (pipe/and_or
            // linkage flag set) belongs to the discarded command whatever
            // its line. The flag is only ever set on a NON-final stage
            // (the stage after `|'/`&&' has not arrived), so completed
            // pipelines and and-or lists are never popped by this arm.
            || command.pipe.is_some()
            || command.and_or.is_some()
    }) {
        state.ast.commands.pop();
    }
    // GNU echoes only the offending input line, from its first
    // token, not the rest of the file (parse.y:6813-6824
    // print_offending_line prints the current `shell_input_line').
    // With the original text available (script driver, eval reparse)
    // echo that line verbatim; token reconstruction cannot recover the
    // original spacing. Token positions are absolute script LINES and
    // `source_line_offset` is the line shift the caller applied
    // (script_driver.rs `token.position += line_offset`), so the line
    // within the text is `position - offset`, 1-based — never a byte
    // offset (rubash#204: the old byte-offset read reported line N's
    // number with line 1's text).
    let verbatim = state.diagnostic_text.as_ref().and_then(|text| {
        let line_in_text = tokens[i].position.checked_sub(state.source_line_offset)?;
        source_line_by_number(text, line_in_text)
    });
    let source = verbatim.unwrap_or_else(|| {
        let line_number = tokens[i].position;
        let mut line_start = i;
        while line_start > state.section_token_start
            && tokens[line_start - 1].position == line_number
        {
            line_start -= 1;
        }
        let mut joined = tokens[line_start].raw.clone();
        for token in tokens[line_start + 1..].iter() {
            if token.position != line_number {
                break;
            }
            joined.push(' ');
            joined.push_str(&token.raw);
        }
        joined
    });
    state
        .current_cmd
        .insert_assignment("__RUBASH_PARSE_SOURCE__".to_string(), source);
    state.current_cmd.line = Some(error_line);
    state
        .ast
        .commands
        .push(std::mem::take(&mut state.current_cmd));
}

fn push_parse_error_until(
    state: &mut ParseState,
    tokens: &[Token],
    start: usize,
    terminator: &str,
    message: &str,
) -> usize {
    state
        .current_cmd
        .insert_assignment("__RUBASH_PARSE_ERROR__".to_string(), message.to_string());
    let mut next_i = start + 1;
    while tokens.get(next_i).is_some() {
        let is_terminator = is_keyword(tokens, next_i, terminator);
        next_i += 1;
        if is_terminator {
            break;
        }
    }
    state
        .ast
        .commands
        .push(std::mem::take(&mut state.current_cmd));
    next_i
}

/// GNU parse.y:6890-6901: reaching EOF inside an unclosed `(` compound
/// command reports "unexpected end of file from `(' command on line N" from
/// the compoundcmd_lineno stack, not the generic near-EOF message.  Heredocs
/// still pending inside the failed region already issued their
/// "delimited by end-of-file" warnings during the parse (make_cmd.c:626), so
/// the details are carried to the executor on the error node.
fn push_unclosed_paren_error(state: &mut ParseState, tokens: &[Token], start: usize) -> usize {
    let paren_line = tokens.get(start).map(|token| token.position).unwrap_or(1);
    // Pair each HereDocBody token with its `<<` delimiter word in emission
    // order: each `<<` pushes the following word and each body pops it.
    // The queue is fed from the whole token stream so a `<<` seen before
    // the `(` (e.g. `cat <<EOF && (`) still pairs its deferred body.
    let mut pending_delimiters: std::collections::VecDeque<String> =
        std::collections::VecDeque::new();
    let mut warned: Vec<(String, usize, usize)> = Vec::new();
    let mut last_line = paren_line;
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::HereDoc {
            let delimiter = tokens
                .get(index + 1)
                .map(|next| next.value.clone())
                .unwrap_or_default();
            pending_delimiters.push_back(delimiter);
            continue;
        }
        if token.kind != TokenKind::HereDocBody {
            if index > start {
                last_line = last_line.max(token.position);
            }
            continue;
        }
        let delimiter = pending_delimiters.pop_front().unwrap_or_default();
        if index <= start {
            continue;
        }
        let gather_line = token.position;
        let body = token
            .value
            .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
            .unwrap_or(token.value.as_str());
        let unterminated = body.starts_with(DATA_DOLLAR);
        let body = body
            .strip_prefix(DATA_DOLLAR)
            .or_else(|| body.strip_prefix(crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX))
            .unwrap_or(body);
        let body_lines = body.lines().count();
        // A terminated or delimiter-prefixed `)` body also consumed the
        // delimiter line; an unterminated body ended at EOF on its last
        // body line.
        let consumed_last = gather_line + body_lines + usize::from(!unterminated);
        last_line = last_line.max(consumed_last);
        if unterminated {
            warned.push((delimiter, gather_line, consumed_last));
        }
    }
    let eof_line = last_line + 1;
    state.current_cmd.line = state.current_cmd.line.or(Some(paren_line));
    state.current_cmd.insert_assignment(
        "__RUBASH_PARSE_ERROR__".to_string(),
        "unexpected end of file".to_string(),
    );
    state.current_cmd.insert_assignment(
        "__RUBASH_PARSE_ERROR_EOF_SUBSHELL__".to_string(),
        format!("{paren_line}{PARSE_ERROR_FIELD_SEP}{eof_line}"),
    );
    for (warn_index, (delimiter, at_line, warn_line)) in warned.iter().enumerate() {
        state.current_cmd.insert_assignment(
            format!("__RUBASH_PARSE_ERROR_HD_WARN_{warn_index}__"),
            format!(
                "{delimiter}{PARSE_ERROR_FIELD_SEP}{at_line}{PARSE_ERROR_FIELD_SEP}{warn_line}"
            ),
        );
    }
    state
        .ast
        .commands
        .push(std::mem::take(&mut state.current_cmd));
    tokens.len()
}

/// The innermost still-open compound in `region`, modelling GNU's
/// compoundcmd_lineno stack (parse.y:344-358): `(` `{` if for while until
/// select case push; their closers pop only a matching top, so a `)` inside
/// an unclosed `case` pattern list never pops a subshell.
fn innermost_unclosed_compound(region: &[Token]) -> Option<(String, usize)> {
    let mut stack: Vec<(&'static str, usize)> = Vec::new();
    for token in region {
        if token.kind != TokenKind::Keyword {
            continue;
        }
        let value = token.value.trim_end();
        // A collapsed `{...}` keyword token is a complete group; a `{`-token
        // that does not end with `}` swallowed the rest of the input and is
        // by construction the innermost opener.
        if value.starts_with('{') {
            if !value.ends_with('}') {
                stack.push(("{", token.position));
            }
            continue;
        }
        match value {
            "(" => stack.push(("(", token.position)),
            ")" => {
                if matches!(stack.last(), Some(("(", _))) {
                    stack.pop();
                }
            }
            "if" => stack.push(("if", token.position)),
            "fi" => {
                if matches!(stack.last(), Some(("if", _))) {
                    stack.pop();
                }
            }
            "for" | "while" | "until" | "select" => {
                stack.push((
                    match value {
                        "for" => "for",
                        "while" => "while",
                        "until" => "until",
                        _ => "select",
                    },
                    token.position,
                ));
            }
            "done" => {
                if matches!(
                    stack.last(),
                    Some(("for" | "while" | "until" | "select", _))
                ) {
                    stack.pop();
                }
            }
            "case" => stack.push(("case", token.position)),
            "esac" => {
                if matches!(stack.last(), Some(("case", _))) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    stack
        .last()
        .map(|(name, line)| ((*name).to_string(), *line))
}

/// GNU parse.y:6890-6901 EOF reporting for an unclosed `{` group (plain
/// `{ cmd` or a function body `name() { cmd`): "unexpected end of file from
/// `X' command on line N" names the INNERMOST unclosed compound, and pending
/// heredocs already warned during the parse (make_cmd.c:626).
///
/// `brace_index` points at either a bare `{` keyword token (body is the
/// following tokens) or a collapsed `{ ...` keyword token the lexer emitted
/// when an unclosed group swallowed the rest of the input — the swallowed
/// text is retokenized so inner compounds and heredocs are still seen.
pub(super) fn unclosed_brace_eof_node(tokens: &[Token], brace_index: usize) -> CommandNode {
    let brace = &tokens[brace_index];
    let brace_line = brace.position;
    let inner_text = brace
        .value
        .strip_prefix('{')
        .filter(|_| brace.value.trim_end() != "{")
        .unwrap_or("");

    // Region tokens for the innermost-compound and heredoc scans, mapped to
    // absolute script lines.
    let region_owned;
    let region: &[Token] = if inner_text.is_empty() {
        &tokens[brace_index + 1..]
    } else {
        let mut inner = crate::lexer::tokenize(inner_text);
        for token in inner.iter_mut() {
            token.position = token.position + brace_line - 1;
        }
        region_owned = inner;
        &region_owned
    };

    // GNU reports the error at line_number once EOF is reached: one past the
    // last physical line, plus one more when a trailing unquoted backslash
    // forced a continuation read that hit EOF (eval `X() { (a)>\'`).
    let last_line = if inner_text.is_empty() {
        region
            .iter()
            .map(|token| token.position)
            .max()
            .unwrap_or(brace_line)
    } else {
        brace_line + inner_text.matches('\n').count()
    };
    let continuation = if inner_text.is_empty() {
        false
    } else {
        inner_text.trim_end_matches([' ', '\t']).ends_with('\\')
    };
    compound_eof_error_node(
        "{",
        brace_line,
        region,
        last_line,
        eof_line_extra(last_line, continuation),
    )
}

/// `name() (` with no closing `)`: the same EOF reporting names the `(`.
pub(super) fn unclosed_paren_eof_node(tokens: &[Token], paren_index: usize) -> CommandNode {
    let paren_line = tokens[paren_index].position;
    let region = &tokens[paren_index + 1..];
    let last_line = region
        .iter()
        .map(|token| token.position)
        .max()
        .unwrap_or(paren_line);
    compound_eof_error_node(
        "(",
        paren_line,
        region,
        last_line,
        eof_line_extra(last_line, false),
    )
}

fn eof_line_extra(last_line: usize, continuation: bool) -> usize {
    last_line + 1 + usize::from(continuation)
}

/// `if`/`while`/`until`/`for`/`select`/`case` opener whose closer never
/// arrives before end of input. GNU parse.y (yyerror EOF path,
/// parse.y:6890-6901) reports the innermost open compound via
/// compoundcmd_lineno: "unexpected end of file from `X' command on line N".
/// Callers have already confirmed the matching closer is absent from the
/// remaining stream, so `opener` is the outermost unclosed frame and
/// `region` still exposes any deeper nested opener.
pub(super) fn unclosed_keyword_eof_node(
    tokens: &[Token],
    opener_index: usize,
    opener: &'static str,
) -> CommandNode {
    let opener_line = tokens[opener_index].position;
    let region = &tokens[opener_index + 1..];
    let last_line = region
        .iter()
        .map(|token| token.position)
        .max()
        .unwrap_or(opener_line);
    compound_eof_error_node(
        opener,
        opener_line,
        region,
        last_line,
        eof_line_extra(last_line, false),
    )
}

/// GNU parse.y reserved-word grammar: while a keyword compound frame is
/// open, the only legal closer is that frame's own terminator. A
/// `fi`/`done`/`esac`/`}`/`)`/`;;`-family token arriving for a different
/// frame is a `syntax error near unexpected token 'X'` at that token, not
/// an EOF diagnostic (`if x; then y; done` reports `done`, not a missing
/// `fi`). Seeded with the opener being reported; returns the first
/// mismatched closer in `region`, or None when input truly ran out.
fn first_mismatched_closer(region: &[Token], opener: &'static str) -> Option<usize> {
    let mut stack: Vec<&'static str> = vec![opener];
    for (region_index, token) in region.iter().enumerate() {
        if token.kind != TokenKind::Keyword {
            continue;
        }
        let value = token.value.trim_end();
        // Same collapsed-`{` rule as innermost_unclosed_compound.
        if value.starts_with('{') {
            if !value.ends_with('}') {
                stack.push("{");
            }
            continue;
        }
        match value {
            "(" => stack.push("("),
            "if" => stack.push("if"),
            "for" | "while" | "until" | "select" => stack.push(match value {
                "for" => "for",
                "while" => "while",
                "until" => "until",
                _ => "select",
            }),
            "case" => stack.push("case"),
            ")" => {
                if matches!(stack.last(), Some(&"(")) {
                    stack.pop();
                } else if !matches!(stack.last(), Some(&"case")) {
                    // Inside `case`, `)` is legal pattern syntax.
                    return Some(region_index);
                }
            }
            "fi" => {
                if matches!(stack.last(), Some(&"if")) {
                    stack.pop();
                } else {
                    return Some(region_index);
                }
            }
            "done" => {
                if matches!(stack.last(), Some(&"for" | &"while" | &"until" | &"select")) {
                    stack.pop();
                } else {
                    return Some(region_index);
                }
            }
            "esac" => {
                if matches!(stack.last(), Some(&"case")) {
                    stack.pop();
                } else {
                    return Some(region_index);
                }
            }
            "}" => {
                if matches!(stack.last(), Some(&"{")) {
                    stack.pop();
                } else {
                    return Some(region_index);
                }
            }
            ";;" | ";&" | ";;&" => {
                if !matches!(stack.last(), Some(&"case")) {
                    return Some(region_index);
                }
            }
            _ => {}
        }
    }
    None
}

/// GNU parse.y if-compound grammar over a *failed* `if` parse: tracks the
/// `if` frame's position and returns the region index of the first token
/// the grammar rejects at its position (`if then; fi` → `then`, `if x; fi`
/// → `fi`, `if x; then y; done` → `done`), skipping nested compound
/// interiors. Phase keywords (`then`/`elif`/`else`/`fi`) are legal only
/// after a `;` separator and only when the current phase already saw a
/// command — that is exactly the condition GNU's list/term rules require.
/// None ⇒ input truly ended inside the `if` (an EOF error).
fn if_frame_offender(region: &[Token]) -> Option<usize> {
    let mut stack: Vec<&'static str> = vec!["if"];
    let mut in_condition = true;
    let mut expect_command = true;
    let mut phase_has_command = false;
    for (index, token) in region.iter().enumerate() {
        if stack.len() == 1 && token.kind == TokenKind::Semicolon {
            if expect_command {
                // GNU parse.y linebreak: newline_list folds any run of
                // newlines between list terms — `if x\n\ny\nthen` is
                // legal, and a comment-only line contributes nothing but
                // its newline. Only a literal `;` with no command before
                // it offends (`if ;`, `if x; ;`).
                if token.line_break {
                    continue;
                }
                return Some(index); // `if ;`, `if x; ;`, `if x; then ;`
            }
            expect_command = true;
            continue;
        }
        if token.kind == TokenKind::Keyword {
            let value = token.value.trim_end();
            // A compound command opener at our own frame level starts a
            // command (`if (x); fi` ⇒ the missing `then` blames `fi`).
            if stack.len() == 1 {
                match value {
                    "(" | "if" | "for" | "while" | "until" | "select" | "case" => {
                        expect_command = false;
                        phase_has_command = true;
                    }
                    _ => {}
                }
            }
            if value.starts_with('{') {
                if stack.len() == 1 {
                    expect_command = false;
                    phase_has_command = true;
                }
                if !value.ends_with('}') {
                    stack.push("{");
                }
                continue;
            }
            match value {
                "(" => {
                    stack.push("(");
                    continue;
                }
                "if" => {
                    stack.push("if");
                    continue;
                }
                "for" | "while" | "until" | "select" => {
                    stack.push(match value {
                        "for" => "for",
                        "while" => "while",
                        "until" => "until",
                        _ => "select",
                    });
                    continue;
                }
                "case" => {
                    stack.push("case");
                    continue;
                }
                _ => {}
            }
            // Closers pop matching nested frames; our own `fi` is itself
            // the offender because this parse already failed (GNU rejects
            // the `fi` that arrives while the structure is still broken).
            match value {
                ")" => {
                    if matches!(stack.last(), Some(&"(")) {
                        stack.pop();
                    } else if !matches!(stack.last(), Some(&"case")) {
                        return Some(index);
                    }
                    continue;
                }
                "fi" => {
                    if stack.len() > 1 && matches!(stack.last(), Some(&"if")) {
                        stack.pop();
                    } else {
                        return Some(index);
                    }
                    continue;
                }
                "done" => {
                    if matches!(stack.last(), Some(&"for" | &"while" | &"until" | &"select")) {
                        stack.pop();
                    } else {
                        return Some(index);
                    }
                    continue;
                }
                "esac" => {
                    if matches!(stack.last(), Some(&"case")) {
                        stack.pop();
                    } else {
                        return Some(index);
                    }
                    continue;
                }
                "}" => {
                    if matches!(stack.last(), Some(&"{")) {
                        stack.pop();
                    } else {
                        return Some(index);
                    }
                    continue;
                }
                ";;" | ";&" | ";;&" => {
                    if !matches!(stack.last(), Some(&"case")) {
                        return Some(index);
                    }
                    continue;
                }
                _ => {}
            }
            if stack.len() != 1 {
                continue;
            }
            // Command-position modifiers GNU accepts before a command word.
            if matches!(value, "time" | "!") {
                expect_command = false;
                phase_has_command = true;
                continue;
            }
            if expect_command {
                // A phase keyword is legal after `;` only when the phase
                // already contains a command; every other keyword here is
                // the offender.
                let legal = match value {
                    "then" => in_condition && phase_has_command,
                    "elif" | "else" => !in_condition && phase_has_command,
                    _ => false,
                };
                if !legal {
                    return Some(index);
                }
            } else {
                // Mid-command: any keyword is out of place (`if x then` →
                // GNU reports `then`).
                return Some(index);
            }
            match value {
                "then" => in_condition = false,
                "elif" => in_condition = true,
                _ => {}
            }
            expect_command = true;
            phase_has_command = false;
            continue;
        }
        if stack.len() == 1 {
            expect_command = false;
            phase_has_command = true;
        }
    }
    None
}

/// Physical input line containing `tokens[index]` — verbatim from the
/// original source when available (GNU parse.y y.error echoes the line as
/// read), else reconstructed from same-line token raws. `line_offset` is
/// the caller's line shift (`ParseLoopOptions::source_line_offset`), so
/// the line within `source` is `position - line_offset`.
pub(super) fn offending_line_text(
    tokens: &[Token],
    index: usize,
    source: Option<&str>,
    line_offset: usize,
) -> String {
    if let Some(text) = source {
        if let Some(line) = tokens[index]
            .position
            .checked_sub(line_offset)
            .and_then(|line_in_text| source_line_by_number(text, line_in_text))
        {
            return line;
        }
    }
    let line = tokens[index].position;
    let mut start = index;
    while start > 0 && tokens[start - 1].position == line {
        start -= 1;
    }
    let mut text = tokens[start].raw.clone();
    let mut prev_end = tokens[start].column + tokens[start].raw.len();
    for token in &tokens[start + 1..] {
        if token.position != line {
            break;
        }
        // A line-break Semicolon is the physical NEWLINE (GNU read_token
        // returns a newline token), never source text on this line — echoing its
        // synthesized `;' raw appended a stray semicolon to the offending
        // line (`done;' for a bare `done' line, rubash#327 wording note).
        if token.kind == TokenKind::Semicolon && token.line_break {
            break;
        }
        if token.column > prev_end {
            text.push(' ');
        }
        text.push_str(&token.raw);
        prev_end = token.column + token.raw.len();
    }
    text
}

/// `syntax error near unexpected token 'X'` reported AT the line of X
/// with that physical input line echoed — the shape GNU's yyerror produces
/// when a closer arrives inside the wrong compound frame (`if x; then y;
/// done` reports `done` at done's line).
pub(super) fn mismatched_closer_node(
    tokens: &[Token],
    bad_index: usize,
    diagnostic_text: Option<&str>,
    line_offset: usize,
) -> CommandNode {
    let bad = &tokens[bad_index];
    let mut command = CommandNode::new();
    command.line = Some(bad.position);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_NEAR__".to_string(),
        format!(
            "{}{}{}",
            bad.value.trim_end(),
            PARSE_ERROR_FIELD_SEP,
            bad.position
        ),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_SOURCE__".to_string(),
        offending_line_text(tokens, bad_index, diagnostic_text, line_offset),
    );
    command
}

fn compound_eof_error_node(
    opener: &str,
    open_line: usize,
    region: &[Token],
    last_line: usize,
    eof_line: usize,
) -> CommandNode {
    let (name, name_line) =
        innermost_unclosed_compound(region).unwrap_or_else(|| (opener.to_string(), open_line));

    // Heredocs still pending when EOF hit already warned during the GNU
    // parse: pair `<<` delimiters with body tokens exactly like the subshell
    // path, and warn for a `<<` whose body read never even produced a token.
    let mut pending_delimiters: std::collections::VecDeque<(String, usize)> =
        std::collections::VecDeque::new();
    let mut warned: Vec<(String, usize, usize)> = Vec::new();
    for (index, token) in region.iter().enumerate() {
        if token.kind == TokenKind::HereDoc {
            let delimiter = region
                .get(index + 1)
                .map(|next| next.value.clone())
                .unwrap_or_default();
            pending_delimiters.push_back((delimiter, token.position));
            continue;
        }
        if token.kind != TokenKind::HereDocBody {
            continue;
        }
        let (delimiter, _here_line) = pending_delimiters.pop_front().unwrap_or_default();
        let gather_line = token.position;
        let body = token
            .value
            .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
            .unwrap_or(token.value.as_str());
        let unterminated = body.starts_with(DATA_DOLLAR);
        let body = body
            .strip_prefix(DATA_DOLLAR)
            .or_else(|| body.strip_prefix(crate::executor::markers::HEREDOC_WARNED_BODY_PREFIX))
            .unwrap_or(body);
        let body_lines = body.lines().count();
        let consumed_last = gather_line + body_lines + usize::from(!unterminated);
        if unterminated {
            warned.push((delimiter, gather_line, consumed_last));
        }
    }
    // A `<<` with no body token at all gathered at the last input line and
    // warned there (GNU make_cmd.c gather at EOF: `f() { cat <<EOF` warns
    // "at line 4" at prefix line 4).
    for (delimiter, here_line) in pending_delimiters {
        warned.push((
            delimiter,
            last_line.max(here_line),
            last_line.max(here_line),
        ));
    }

    let mut command = CommandNode::new();
    command.line = Some(open_line);
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR__".to_string(),
        "unexpected end of file".to_string(),
    );
    command.insert_assignment(
        "__RUBASH_PARSE_ERROR_EOF_COMPOUND__".to_string(),
        format!("{name}{PARSE_ERROR_FIELD_SEP}{name_line}{PARSE_ERROR_FIELD_SEP}{eof_line}"),
    );
    for (warn_index, (delimiter, at_line, warn_line)) in warned.iter().enumerate() {
        command.insert_assignment(
            format!("__RUBASH_PARSE_ERROR_HD_WARN_{warn_index}__"),
            format!(
                "{delimiter}{PARSE_ERROR_FIELD_SEP}{at_line}{PARSE_ERROR_FIELD_SEP}{warn_line}"
            ),
        );
    }
    command
}

pub(super) fn command_is_pending_inversion(command: &CommandNode) -> bool {
    if !command.inverted {
        return false;
    }
    let mut without_inversion = command.clone();
    without_inversion.inverted = false;
    command_is_empty(&without_inversion)
}

fn push_compound_command(state: &mut ParseState, mut command: CommandNode) {
    if command_is_pending_inversion(&state.current_cmd) {
        command.inverted = !command.inverted;
        command.line = command.line.or(state.current_cmd.line);
    }
    state.ast.commands.push(command);
    state.current_cmd = CommandNode::new();
}

pub(super) fn parse_time_prefixed_compound_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    tokens.get(start)?;
    let mut posix_format = false;
    let mut inverted = false;
    let mut prefix_words = Vec::new();
    let mut prefix_word_metadata = Vec::new();
    let mut i = start + 1;
    while tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        prefix_word_metadata.push(build_word_metadata(
            prefix_words.len(),
            &tokens[i].value,
            &tokens[i].raw,
        ));
        prefix_words.push(tokens[i].value.clone());
        match tokens[i].value.as_str() {
            "-p" => posix_format = true,
            "!" => inverted = !inverted,
            _ => {}
        }
        i += 1;
    }

    let (mut command, next_i) = if is_keyword(tokens, i, "for") {
        parse_for_command(tokens, i, None, 0)?
    } else if is_keyword(tokens, i, "if") {
        parse_if_command_standalone(tokens, i, None, 0)?
    } else if tokens
        .get(i)
        .is_some_and(|token| matches!(token.value.as_str(), "while" | "until"))
    {
        parse_loop_command(tokens, i, None, 0)?
    } else if is_keyword(tokens, i, "case") {
        parse_case_command(tokens, i, None, 0)?
    } else if is_keyword(tokens, i, "select") {
        parse_select_command(tokens, i, None, 0)?
    } else if is_keyword(tokens, i, "coproc") {
        parse_coproc_command(tokens, i)?
    } else if let Some(parsed) = parse_function_command(tokens, i) {
        parsed
    } else if tokens.get(i).is_some_and(|token| token.value == "[[") {
        parse_conditional_command(tokens, i)?
    } else if is_keyword(tokens, i, "{")
        || tokens.get(i).is_some_and(|token| {
            token.kind == TokenKind::Keyword
                && token.value.starts_with('{')
                && token.value.ends_with('}')
        })
    {
        parse_brace_group_command(tokens, i, None, 0)?
    } else if let Some(parsed) = parse_arithmetic_command(tokens, i) {
        parsed
    } else if is_keyword(tokens, i, "(") {
        parse_subshell_command(tokens, i, None, 0)?
    } else {
        return None;
    };

    let pipe = command.pipe.take();
    let and_or = command.and_or.take();
    let background = command.background;
    command.background = false;
    let mut timed = CommandNode::new();
    timed.line = tokens.get(start).map(|token| token.position);
    timed.pipe = pipe;
    timed.and_or = and_or;
    timed.background = background;
    timed.time_command = Some(TimeCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: Box::new(build_word_metadata(
            0,
            &tokens[start].value,
            &tokens[start].raw,
        )),
        prefix_words,
        prefix_word_metadata,
        command: Box::new(command),
        posix_format,
        inverted,
    });
    Some((timed, next_i))
}

pub(super) fn parse_time_prefixed_shell_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    if !time_prefixed_shell_command_allows_simple_pipeline(tokens, start) {
        return None;
    }

    let end = time_prefixed_shell_command_end(tokens, start + 1);

    let mut commands = parse(&tokens[start..end]).commands;
    if commands.len() != 1 {
        return None;
    }

    Some((commands.remove(0), end))
}

/// The 1-based `line` of `text`, verbatim. Token positions are line
/// numbers (parse.y `line_number`), so diagnostic line echo is a line
/// lookup, not a byte-offset slice (rubash#204).
pub(super) fn source_line_by_number(text: &str, line: usize) -> Option<String> {
    let line = line.checked_sub(1)?;
    text.lines().nth(line).map(str::to_string)
}

fn time_prefixed_shell_command_end(tokens: &[Token], mut index: usize) -> usize {
    let mut stack = Vec::new();
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;

    while let Some(token) = tokens.get(index) {
        if stack.is_empty()
            && paren_depth == 0
            && brace_depth == 0
            && token.kind == TokenKind::Semicolon
            && previous_significant_token_is_pipe(tokens, index)
        {
            index += 1;
            continue;
        }

        if stack.is_empty()
            && paren_depth == 0
            && brace_depth == 0
            && matches!(
                token.kind,
                TokenKind::Semicolon | TokenKind::And | TokenKind::Or | TokenKind::Background
            )
        {
            break;
        }

        update_compound_boundary_stack(tokens, index, &mut stack);
        if stack.last().copied() != Some("esac") && command_boundary_keyword_allowed(tokens, index)
        {
            if is_keyword(tokens, index, "(") {
                paren_depth += 1;
            } else if is_keyword(tokens, index, ")") {
                paren_depth = paren_depth.saturating_sub(1);
            } else if is_keyword(tokens, index, "{") {
                brace_depth += 1;
            } else if is_keyword(tokens, index, "}") {
                brace_depth = brace_depth.saturating_sub(1);
            }
        }

        index += 1;
    }

    index
}

fn previous_significant_token_is_pipe(tokens: &[Token], index: usize) -> bool {
    let mut previous = index;
    while let Some(next) = previous.checked_sub(1) {
        previous = next;
        let token = &tokens[previous];
        if token.kind == TokenKind::Semicolon {
            continue;
        }
        return matches!(token.kind, TokenKind::Pipe | TokenKind::PipeErr);
    }
    false
}

pub(super) fn time_prefixed_shell_command_allows_simple_pipeline(
    tokens: &[Token],
    start: usize,
) -> bool {
    if !is_keyword(tokens, start, "time") {
        return false;
    }

    let mut index = start + 1;
    while tokens
        .get(index)
        .is_some_and(|token| matches!(token.value.as_str(), "-p" | "--" | "!"))
    {
        index += 1;
    }

    let Some(token) = tokens.get(index) else {
        return true;
    };
    if matches!(
        token.kind,
        TokenKind::Semicolon | TokenKind::And | TokenKind::Or | TokenKind::Background
    ) {
        return true;
    }

    !time_prefixed_shell_command_starts_with_compound(tokens, index)
}

fn time_prefixed_shell_command_starts_with_compound(tokens: &[Token], index: usize) -> bool {
    if parse_function_command(tokens, index).is_some() {
        return true;
    }

    if tokens.get(index).is_some_and(|token| {
        matches!(
            token.value.as_str(),
            "for" | "case" | "select" | "coproc" | "if" | "while" | "until" | "[[" | "function"
        ) || token.value.starts_with("((")
    }) {
        return true;
    }

    if is_keyword(tokens, index, "{") || is_keyword(tokens, index, "(") {
        return true;
    }

    if tokens.get(index).is_some_and(|token| {
        token.kind == TokenKind::Keyword
            && token.value.starts_with('{')
            && token.value.ends_with('}')
    }) {
        return true;
    }

    tokens.get(index).is_some_and(|token| {
        token.kind == TokenKind::Word
            && tokens
                .get(index + 1)
                .is_some_and(|next| super::is_unquoted_operator(next, "("))
            && tokens
                .get(index + 2)
                .is_some_and(|next| super::is_unquoted_operator(next, ")"))
    })
}

#[cfg(test)]
mod stray_close_tests {
    use super::*;

    fn marker_source(ast: &Ast) -> String {
        ast.commands
            .iter()
            .find_map(|command| {
                command
                    .get_assignment("__RUBASH_PARSE_SOURCE__")
                    .map(|value| value.clone())
            })
            .unwrap_or_default()
    }

    #[test]
    fn eval_style_verbatim_line_echo_preserves_original_spacing() {
        // The eval string: one line, caller at script line 199 -> offset 198.
        let source_text = String::from("case esac in esac) ;; *) echo \"x\";; esac");
        let tokens = crate::lexer::tokenize(&source_text);
        let mut tokens = tokens;
        for token in tokens.iter_mut() {
            token.position += 198;
        }
        let ast = parse_with_options(
            &tokens,
            ParseLoopOptions {
                stray_close_is_error: true,
                source_text: Some(source_text.clone().into()),
                source_line_offset: 198,
                ..ParseLoopOptions::default()
            },
        );
        assert_eq!(marker_source(&ast), source_text);
    }

    #[test]
    fn without_source_text_falls_back_to_token_join() {
        let source_text = ") echo never ;; esac";
        let tokens = crate::lexer::tokenize(source_text);
        let ast = parse_with_options(
            &tokens,
            ParseLoopOptions {
                stray_close_is_error: true,
                ..ParseLoopOptions::default()
            },
        );
        assert_eq!(marker_source(&ast), ") echo never ;; esac");
    }
}
