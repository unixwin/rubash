use super::parse_loop::{
    mismatched_closer_node, run_inline_section, unclosed_keyword_eof_node, ParseState,
    SectionOutcome, SectionStop,
};
use super::*;
use crate::lexer::Token;

pub(super) fn parse_if_command(
    tokens: &[Token],
    start: usize,
    state: &mut ParseState,
) -> Option<(CommandNode, usize)> {
    if !is_keyword(tokens, start, "if") {
        return None;
    }

    // GNU parse.y:1037-1054 if_command: the condition list, `then', the
    // body lists and `fi' are all shifted by the SAME yyparse run. The
    // condition is parsed by the shared command loop stopped at `then'
    // (the old model re-walked the region once in find_if_then and again
    // in the slice parse — the depth-amplified double walk).
    let condition = match run_inline_section(tokens, state, start + 1, SectionStop::IfCondition) {
        SectionOutcome::Ended {
            body,
            index: then_index,
        } => {
            let condition = non_empty_body(body);
            let keyword_metadata = build_keyword_metadata(&tokens[start]);
            let then_keyword = tokens[then_index].value.clone();
            let then_keyword_metadata = build_keyword_metadata(&tokens[then_index]);
            return parse_if_command_after_condition(
                tokens,
                start,
                state,
                condition,
                keyword_metadata,
                then_index,
                then_keyword,
                then_keyword_metadata,
            );
        }
        // No `then` at any section level: find_if_then's None.
        SectionOutcome::Eof { .. } => return None,
        // The condition errored before its `then`; the cold scanner
        // recovers the boundary the slice parse would have used.
        SectionOutcome::Errored { body } => {
            let then_index = find_if_then(tokens, start + 1)?;
            let condition = non_empty_body(body);
            let keyword_metadata = build_keyword_metadata(&tokens[start]);
            let then_keyword = tokens[then_index].value.clone();
            let then_keyword_metadata = build_keyword_metadata(&tokens[then_index]);
            return parse_if_command_after_condition(
                tokens,
                start,
                state,
                condition,
                keyword_metadata,
                then_index,
                then_keyword,
                then_keyword_metadata,
            );
        }
        // `then` cannot be an abort token for a condition stop set.
        SectionOutcome::AbortedThen { .. } => return None,
    };
}

/// Satellite callers (function/case compound bodies, time-prefixed
/// compounds) keep the standalone contract: the same inline engine over a
/// scratch state initialized exactly like the fresh state
/// parse_body_with_diagnostics built for the old body slice.
pub(super) fn parse_if_command_standalone(
    tokens: &[Token],
    start: usize,
    source: Option<&std::rc::Rc<str>>,
    line_offset: usize,
) -> Option<(CommandNode, usize)> {
    let mut state = ParseState {
        ast: Ast {
            commands: Vec::new(),
        },
        current_cmd: CommandNode::new(),
        in_subshell: false,
        stray_close_is_error: false,
        section_stop: SectionStop::None,
        section_stack: Vec::new(),
        section_token_start: 0,
        pending_comsub: 0,
        diagnostic_text: source.cloned(),
        source_line_offset: line_offset,
    };
    parse_if_command(tokens, start, &mut state)
}

#[allow(clippy::too_many_arguments)]
fn parse_if_command_after_condition(
    tokens: &[Token],
    start: usize,
    state: &mut ParseState,
    condition: Vec<CommandNode>,
    keyword_metadata: Box<WordMetadata>,
    then_index: usize,
    then_keyword: String,
    then_keyword_metadata: Box<WordMetadata>,
) -> Option<(CommandNode, usize)> {
    if condition.is_empty() {
        return None;
    }
    let condition_terminator = condition_terminator_before(tokens, then_index);
    let condition_terminator_metadata = condition_terminator_metadata_before(tokens, then_index);
    let mut index = then_index + 1;

    let (then_body, boundary) = parse_if_section_inline(tokens, index, state)?;
    // GNU parse.y: `then`/`elif`/`else` bodies are non-empty command lists.
    if then_body.is_empty() {
        return None;
    }
    index = boundary;

    let mut elif_branches = Vec::new();
    while is_keyword(tokens, index, "elif") {
        let elif_keyword = tokens[index].value.clone();
        let elif_keyword_metadata = build_keyword_metadata(&tokens[index]);
        let elif_condition =
            match run_inline_section(tokens, state, index + 1, SectionStop::IfCondition) {
                SectionOutcome::Ended {
                    body,
                    index: elif_then,
                } => Some((non_empty_body(body), elif_then)),
                SectionOutcome::Errored { body } => find_if_then(tokens, index + 1)
                    .map(|elif_then| (non_empty_body(body), elif_then)),
                SectionOutcome::Eof { .. } | SectionOutcome::AbortedThen { .. } => None,
            };
        let Some((condition, elif_then)) = elif_condition else {
            return None;
        };
        let elif_then_keyword = tokens[elif_then].value.clone();
        let elif_then_keyword_metadata = build_keyword_metadata(&tokens[elif_then]);
        let condition_terminator = condition_terminator_before(tokens, elif_then);
        let condition_terminator_metadata = condition_terminator_metadata_before(tokens, elif_then);
        let (body, next_boundary) = parse_if_section_inline(tokens, elif_then + 1, state)?;
        if body.is_empty() {
            return None;
        }
        elif_branches.push(ElifBranch {
            keyword: elif_keyword,
            keyword_metadata: elif_keyword_metadata,
            condition,
            condition_terminator,
            condition_terminator_metadata,
            then_keyword: elif_then_keyword,
            then_keyword_metadata: elif_then_keyword_metadata,
            body,
        });
        index = next_boundary;
    }

    let (else_keyword, else_keyword_metadata, else_body) = if is_keyword(tokens, index, "else") {
        let else_keyword = tokens[index].value.clone();
        let else_keyword_metadata = build_keyword_metadata(&tokens[index]);
        let (body, next_boundary) = parse_if_section_inline(tokens, index + 1, state)?;
        if body.is_empty() {
            return None;
        }
        index = next_boundary;
        (Some(else_keyword), Some(else_keyword_metadata), Some(body))
    } else {
        (None, None, None)
    };

    if !is_keyword(tokens, index, "fi") {
        if index >= tokens.len() {
            // GNU parse.y yyerror EOF path: `fi` never arrived —
            // "unexpected end of file from `if' command on line N".
            let command = unclosed_keyword_eof_node(tokens, start, "if");
            return Some((command, tokens.len()));
        }
        // GNU reports the token that actually arrived where `fi` was
        // expected, at that token's line (`if x; then y; done` →
        // `near unexpected token `done'` at done's line).
        let command = mismatched_closer_node(
            tokens,
            index,
            state.diagnostic_text.as_deref(),
            state.source_line_offset,
        );
        return Some((command, tokens.len()));
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.if_command = Some(IfCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata,
        condition,
        condition_terminator,
        condition_terminator_metadata,
        then_keyword,
        then_keyword_metadata,
        then_body,
        elif_branches,
        else_keyword,
        else_keyword_metadata,
        else_body,
        end_keyword: tokens[index].value.clone(),
        end_keyword_metadata: build_keyword_metadata(&tokens[index]),
    });

    Some(finish_compound_command(command, tokens, index + 1))
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn condition_terminator_before(tokens: &[Token], then_index: usize) -> Option<String> {
    tokens
        .get(then_index.saturating_sub(1))
        .filter(|token| token.kind == crate::lexer::TokenKind::Semicolon)
        .map(|token| token.value.clone())
}

fn condition_terminator_metadata_before(
    tokens: &[Token],
    then_index: usize,
) -> Option<Box<WordMetadata>> {
    tokens
        .get(then_index.saturating_sub(1))
        .filter(|token| token.kind == crate::lexer::TokenKind::Semicolon)
        .map(build_keyword_metadata)
}

fn find_if_then(tokens: &[Token], start: usize) -> Option<usize> {
    let mut stack = Vec::new();
    let mut index = start;
    while index < tokens.len() {
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && is_keyword(tokens, index, "then")
        {
            return Some(index);
        }
        update_compound_boundary_stack(tokens, index, &mut stack);
        index += 1;
    }
    None
}

/// One `then`/`elif`/`else` body, parsed inline by the shared command
/// loop stopped at the section closers (`fi`/`done`/`esac` by value,
/// `elif`/`else` by keyword; `then` aborts the construct). GNU anchor:
/// parse.y:1037-1054 — the body is a compound_list shifted from the live
/// token stream, never re-scanned.
fn parse_if_section_inline(
    tokens: &[Token],
    start: usize,
    state: &mut ParseState,
) -> Option<(Vec<CommandNode>, usize)> {
    match run_inline_section(tokens, state, start, SectionStop::IfBody) {
        SectionOutcome::Ended { body, index } => Some((non_empty_body(body), index)),
        // `then` where a body command was expected: the scanner's
        // return-None abort.
        SectionOutcome::AbortedThen { .. } => None,
        // EOF inside the section: the scanner's tokens.get(index)? -> None.
        SectionOutcome::Eof { .. } => None,
        // The body errored before its closer; recover the boundary the
        // slice scanner found (cold path — error sections are rare, and
        // the enclosing parse discards the construct via
        // compound_body_parse_error anyway).
        SectionOutcome::Errored { body } => {
            let index = scan_if_section_end(tokens, start)?;
            Some((non_empty_body(body), index))
        }
    }
}

/// The old parse_if_section boundary scan, kept only for the errored-body
/// fallback: finds where the section's closer sits regardless of the
/// parse error inside.
fn scan_if_section_end(tokens: &[Token], start: usize) -> Option<usize> {
    let mut stack = Vec::new();
    let mut index = start;
    while index < tokens.len() {
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && matches!(tokens[index].value.as_str(), "fi" | "done" | "esac")
        {
            break;
        }
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && is_keyword(tokens, index, "then")
        {
            return None;
        }
        if stack.is_empty()
            && command_boundary_keyword_allowed(tokens, index)
            && (is_keyword(tokens, index, "elif") || is_keyword(tokens, index, "else"))
        {
            break;
        }
        update_compound_boundary_stack(tokens, index, &mut stack);
        index += 1;
    }

    tokens.get(index)?;
    Some(index)
}

fn non_empty_body(body: Vec<CommandNode>) -> Vec<CommandNode> {
    body.into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}
