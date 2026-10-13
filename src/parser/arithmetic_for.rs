use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn parse_arithmetic_for_command(
    tokens: &[Token],
    start: usize,
) -> Option<(CommandNode, usize)> {
    let mut i = if tokens.get(start + 1)?.value == "((" {
        start + 2
    } else if is_keyword(tokens, start + 1, "(") && is_keyword(tokens, start + 2, "(") {
        start + 3
    } else {
        return None;
    };

    // GNU expr.c echoes the section text exactly as it appears between the
    // (( )) delimiters (leading blanks skipped at display time, trailing
    // blanks kept), so capture the raw whitespace-carrying text per section
    // alongside the normalized token join.
    let mut raw_parts: Vec<String> = vec![String::new(), String::new(), String::new()];
    let mut part_index = 0usize;
    let mut paren_depth = 0usize;
    let mut closed = false;
    while i + 1 < tokens.len() {
        if paren_depth == 0 && tokens[i].value == "))" {
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            i += 1;
            closed = true;
            break;
        }

        if paren_depth == 0 && is_keyword(tokens, i, ")") && is_keyword(tokens, i + 1, ")") {
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            i += 2;
            closed = true;
            break;
        }

        if tokens[i].kind == TokenKind::Semicolon && tokens[i].line_break {
            i += 1;
            continue;
        }

        if paren_depth == 0 && tokens[i].kind == TokenKind::Semicolon {
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            part_index += 1;
            if part_index > 2 {
                return None;
            }
            i += 1;
            continue;
        }

        if paren_depth == 0 && tokens[i].value == ";;" {
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            part_index += 2;
            if part_index > 2 {
                return None;
            }
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, "(") {
            paren_depth += 1;
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            raw_parts[part_index].push_str(&tokens[i].raw);
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            raw_parts[part_index].push_str(&tokens[i].raw);
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            raw_parts[part_index].push_str(&tokens[i].leading_ws);
            raw_parts[part_index].push_str(&combined);
            i += 2;
            continue;
        }

        raw_parts[part_index].push_str(&tokens[i].leading_ws);
        raw_parts[part_index].push_str(&tokens[i].raw);
        i += 1;
    }

    if !closed || part_index != 2 {
        return None;
    }

    let mut list_terminator = None;
    let mut list_terminator_metadata = None;
    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        if list_terminator.is_none() {
            list_terminator = Some(tokens[i].value.clone());
            list_terminator_metadata = Some(build_keyword_metadata(&tokens[i]));
        }
        i += 1;
    }

    let (
        body,
        body_end,
        body_kind,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
    ) = if let Some((body, next_i)) = parse_arithmetic_for_brace_body(tokens, i) {
        (
            body,
            next_i,
            CommandBodyKind::BraceGroup,
            None,
            None,
            None,
            None,
        )
    } else {
        if !is_keyword(tokens, i, "do") {
            return None;
        }
        let do_keyword = Some(tokens[i].value.clone());
        let do_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));
        i += 1;

        let body_start = i;
        let mut stack = Vec::new();
        while i < tokens.len() {
            if stack.is_empty()
                && command_boundary_keyword_allowed(tokens, i)
                && is_keyword(tokens, i, "done")
            {
                break;
            }
            update_compound_boundary_stack(tokens, i, &mut stack);
            i += 1;
        }

        if !is_keyword(tokens, i, "done") {
            return None;
        }
        let end_keyword = Some(tokens[i].value.clone());
        let end_keyword_metadata = Some(build_keyword_metadata(&tokens[i]));

        (
            parse_arithmetic_for_body_commands(&tokens[body_start..i]),
            i + 1,
            CommandBodyKind::DoDone,
            do_keyword,
            do_keyword_metadata,
            end_keyword,
            end_keyword_metadata,
        )
    };
    let (body_open_delimiter, body_close_delimiter) =
        command_body_delimiters(body_kind, do_keyword.as_deref(), end_keyword.as_deref());
    let (body_open_delimiter_metadata, body_close_delimiter_metadata) =
        command_body_delimiter_metadata(
            body_kind,
            do_keyword_metadata.as_ref(),
            end_keyword_metadata.as_ref(),
            body_open_delimiter.as_deref(),
            body_close_delimiter.as_deref(),
        );
    // GNU make_cmd.c:254-261 splits the (( )) text into three sections by
    // scanning with skip_to_delim and keeping each section's text verbatim
    // (leading whitespace skipped, quotes intact); execute_cmd.c:3171
    // eval_arith_for_expr then strips quotes (expand_arith_string) before
    // evalexp, so the evaluator never sees a quote byte and evalerror echoes
    // the dequoted section. A token-value join cannot serve as the eval
    // input: a quoted section travels through token values as carrier-
    // marked text, and a carrier byte reaching the arithmetic evaluator is
    // a lexical failure — `for (( i="3" ; ... ))` dies with `operand
    // expected` and the evalerror echo leaks the raw 0x1c carrier (issue
    // #548). The whitespace-carrying capture is the verbatim section text,
    // and the arithmetic evaluator performs the same in-place quote removal
    // as the GNU composition, so feeding it the cleaned capture restores
    // the contract byte for byte.
    let init = arith_for_section_text(&raw_parts[0]);
    let test = arith_for_section_text(&raw_parts[1]);
    let update = arith_for_section_text(&raw_parts[2]);

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.logical_line = tokens.get(start).map(|token| token.logical_line);
    command.for_command = Some(Box::new(ForCommand {
        keyword: tokens[start].value.clone(),
        keyword_metadata: build_keyword_metadata(&tokens[start]),
        variable: String::new(),
        variable_metadata: Box::new(build_word_metadata(0, "", "")),
        in_keyword: None,
        in_keyword_metadata: None,
        words: Vec::new(),
        word_metadata: Vec::new(),
        default_positional: false,
        list_terminator,
        list_terminator_metadata,
        arithmetic: Some(ArithmeticForCommand {
            open_delimiter: "((".to_string(),
            open_delimiter_metadata: delimiter_metadata("(("),
            init: init.clone(),
            init_metadata: ArithmeticExpressionMetadata::new(raw_parts[0].clone()),
            separators: vec![";".to_string(), ";".to_string()],
            separator_metadata: vec![separator_metadata(0, ";"), separator_metadata(1, ";")],
            test: test.clone(),
            test_metadata: ArithmeticExpressionMetadata::new(raw_parts[1].clone()),
            update: update.clone(),
            update_metadata: ArithmeticExpressionMetadata::new(raw_parts[2].clone()),
            close_delimiter: "))".to_string(),
            close_delimiter_metadata: delimiter_metadata("))"),
        }),
        body_kind,
        body_open_delimiter,
        body_open_delimiter_metadata,
        body_close_delimiter,
        body_close_delimiter_metadata,
        do_keyword,
        do_keyword_metadata,
        end_keyword,
        end_keyword_metadata,
        body,
    }));
    Some(finish_compound_command(command, tokens, body_end))
}

fn delimiter_metadata(delimiter: &str) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, delimiter, delimiter))
}

/// The eval input for one arithmetic-for section: the captured section text
/// with leading whitespace skipped (GNU make_cmd.c:254) and any in-band
/// carrier codepoint dropped. The carrier strip is the diagnostic-boundary
/// defense for issue #548: evalerror echoes this text verbatim (expr.c
/// evalerror format), and no registry codepoint may reach user-visible
/// output. Source-verbatim captures never carry carriers; a captured
/// carrier byte would otherwise flow both into the evaluator (a spurious
/// lexical failure) and into the rendered diagnostic.
fn arith_for_section_text(raw: &str) -> String {
    let trimmed = raw.trim_start();
    let is_carrier = |c: char| {
        crate::executor::markers::MARKERS
            .iter()
            .any(|m| m.code == c as u32)
    };
    if !trimmed.chars().any(is_carrier) {
        return trimmed.to_string();
    }
    trimmed.chars().filter(|c| !is_carrier(*c)).collect()
}

fn separator_metadata(index: usize, separator: &str) -> WordMetadata {
    build_word_metadata(index, separator, separator)
}

fn parse_arithmetic_for_body_commands(tokens: &[Token]) -> Vec<CommandNode> {
    parse(tokens)
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect()
}

fn build_keyword_metadata(token: &Token) -> Box<WordMetadata> {
    Box::new(build_word_metadata(0, &token.value, &token.raw))
}

fn parse_arithmetic_for_brace_body(
    tokens: &[Token],
    index: usize,
) -> Option<(Vec<CommandNode>, usize)> {
    let token = tokens.get(index)?;
    if token.kind == TokenKind::Keyword
        && token.value.starts_with('{')
        && token.value.ends_with('}')
        && token.value.len() >= 2
    {
        let inner = token
            .value
            .trim_start_matches('{')
            .trim_end_matches('}')
            .trim();
        let body_tokens = crate::lexer::tokenize(inner);
        return Some((parse_arithmetic_for_body_commands(&body_tokens), index + 1));
    }

    if !is_keyword(tokens, index, "{") {
        return None;
    }

    let i = matching_brace_group_end(tokens, index)?;

    Some((
        parse_arithmetic_for_body_commands(&tokens[index + 1..i]),
        i + 1,
    ))
}
