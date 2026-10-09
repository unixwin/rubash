use super::*;
use crate::lexer::{Token, TokenKind};

pub(super) fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
    // The logical line (the reader's command-list boundary) travels beside
    // the physical line so DISCARD-family skips keep working across
    // backslash-newline continuations (rubash#411).
    if cmd.logical_line.is_none() {
        cmd.logical_line = Some(token.logical_line);
    }
}

pub(super) fn push_command_word(cmd: &mut CommandNode, token: &Token) {
    let word_index = cmd.words.len();
    // Word intake runs ONLY the scans whose findings a consumer still reads
    // (process substitutions, extglob, pathname). Command substitutions
    // re-derive at their consumption points (pretty-print rendering, the
    // executor's own comsub execution parse) and word quotes at the
    // conditional `=~` quoting check. GNU
    // parse.y:5305 read_token_word assembles a word in one pass and
    // make_cmd.c make_simple_command stores the WORD_DESC once; GNU runs NO
    // per-word expansion analyses at parse time at all (subst.c analyzes at
    // execution), and rubash's executor likewise re-derives parameter /
    // arithmetic / brace / tilde expansion from the word text at expansion
    // time — the parse-time scans for those four kinds fed store vectors
    // with zero readers and are gone (wt44/parse4; the parameter scan alone
    // was 10.3ms of nvm -n). `WordMetadata::from_scans` applies exactly
    // `new`'s tagging, so both stores stay byte-identical.
    let scans = WordScans::run(&token.value, &token.raw);
    let WordScans {
        extglob_patterns,
        pathname_patterns,
        process_substitutions,
    } = &scans;
    cmd.extglob_patterns
        .extend(extglob_patterns.iter().cloned().map(|mut p| {
            p.word_index = Some(word_index);
            p
        }));
    cmd.pathname_patterns
        .extend(pathname_patterns.iter().cloned().map(|mut p| {
            p.word_index = Some(word_index);
            p
        }));
    let prior_words_are_array_assignments = cmd.words.is_empty()
        || (!cmd.array_element_assignments.is_empty()
            && cmd.array_element_assignments.len() == cmd.words.len());
    if prior_words_are_array_assignments {
        // NOTE: an escaped quote inside an array-element subscript
        // (`a[\" \"]=v`) is NOT a parse-time error: the literal `"` survives
        // word expansion as subscript data and fails inside
        // array_expand_index's evalexp with `expr: arithmetic syntax error:
        // operand expected (error token is "expr")` plus the DISCARD abort
        // (verified GNU 5.3). Routing it through the real subscript evaluator
        // also preserves the associative key case (`foo["bar\"bie"]` is
        // valid data — assoc6.sub:44).
        record_array_element_assignment_for_word(cmd, word_index, &token.value, &token.raw);
    }
    cmd.word_metadata.push(WordMetadata::from_scans(
        word_index,
        token.value.clone(),
        token.raw.clone(),
        scans.process_substitutions,
        scans.extglob_patterns,
        scans.pathname_patterns,
    ));
    cmd.words.push(token.value.clone());
    cmd.word_kinds.push(token.kind.clone());
}

/// The per-word expansion analyses `push_command_word` needs, each run
/// exactly once (see there for the GNU anchor). Only the kinds with live
/// consumers are scanned (wt44/parse4): parameter / arithmetic / brace /
/// tilde expansions, command substitutions and word quotes fed store
/// vectors with zero (or re-derivable) readers — the executor re-derives
/// them from the word text at expansion time, exactly as GNU subst.c
/// does.
pub(super) struct WordScans {
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub pathname_patterns: Vec<PathnamePattern>,
    pub process_substitutions: Vec<ProcessSubstitution>,
}

impl WordScans {
    pub fn run(value: &str, raw: &str) -> Self {
        Self {
            extglob_patterns: extglob_patterns_in_word_with_raw(value, raw),
            pathname_patterns: pathname_patterns_in_word(value, raw),
            process_substitutions: process_substitutions_in_word_with_raw(value, raw),
        }
    }
}

pub(super) fn build_word_metadata(word_index: usize, value: &str, raw: &str) -> WordMetadata {
    WordMetadata::new(word_index, value.to_string(), raw.to_string())
}

pub(super) fn collect_compound_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    if let Some((process_substitution, next_i)) = process_substitution_word_target(tokens, index) {
        return Some((process_substitution.target, next_i + 1));
    }

    if let Some((process_substitution, next_i)) =
        output_process_substitution_word_target(tokens, index)
    {
        return Some((process_substitution.target, next_i + 1));
    }

    let token = tokens.get(index)?;
    if matches!(
        token.kind,
        TokenKind::Word
            | TokenKind::Variable
            | TokenKind::Assignment
            | TokenKind::CommandSubst
            | TokenKind::BraceExpand
    ) {
        return Some((token.value.clone(), index + 1));
    }

    None
}

pub(super) fn collect_compound_or_keyword_word_value(
    tokens: &[Token],
    index: usize,
) -> Option<(String, usize)> {
    collect_compound_word_value(tokens, index).or_else(|| {
        tokens
            .get(index)
            .filter(|token| token.kind == TokenKind::Keyword)
            .map(|token| (token.value.clone(), index + 1))
    })
}

pub(super) fn set_body_line(body: &mut [CommandNode], line: usize) {
    // TODO(parse.y): Bash preserves source locations through compound command
    // parsing. Rubash reparses inline function bodies from text today, so
    // recover the definition line for diagnostics such as readonly errors.
    for command in body {
        command.line = Some(line);
        command.logical_line = command.logical_line.or(Some(line));
    }
}

pub(super) fn is_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    tokens.get(index).is_some_and(|token| {
        // rubash#461: a fragment of an open `=~' RHS regexp word is WORD
        // DATA, never a reserved word — GNU read_token jumps straight to
        // tokword under PST_REGEXP (parse.y:3663), so reserved-word
        // recognition never happens inside the regex (`while [[ x =~
        // x|case ]]; do' must not see a `case' keyword after the `|').
        !token.regexp_rhs_fragment && token.kind == TokenKind::Keyword && token.value == value
    })
}

/// Whether `token` IS the operator/terminator spelled `value` in the source —
/// not merely a word whose quote removal produced that text. GNU read_token
/// (parse.y:5305 read_token_word) consumes a quoted or escaped `)` / `;;` /
/// `)` as WORD TEXT that never becomes an operator token, so
/// `echo ')'` must print `)`. An operator token's raw spelling is exactly its
/// own text; any quoting (`')'`, `"')"`, `\;`) or other word content makes
/// raw differ from the de-quoted value. Fixes rubash#128.
pub(super) fn is_unquoted_operator(token: &Token, value: &str) -> bool {
    token.value == value && token.raw == value
}

pub(super) fn is_case_end_keyword(tokens: &[Token], index: usize) -> bool {
    if !is_keyword(tokens, index, "esac") {
        return false;
    }
    // parse.y:3433-3441 special_case_tokens: with a case open (the caller's
    // case frame / esacs_needed_count), `esac` DIRECTLY after the case's
    // `in` is the ESAC closer of the empty-case production (parse.y:1037) —
    // GNU never reads it as pattern text from that position, so the rule-4
    // refusals below must not second-guess it. This is what lets an
    // enclosing group scan see through `( case x in esac ) ;; esac`
    // (rubash#336): the inner `)` is a subshell closer, not a pattern
    // delimiter, and the `;;` belongs to the OUTER case.
    if follows_case_in_keyword(tokens, index) {
        return true;
    }
    // parse.y:3181-3184 (CHECK_FOR_RESERVED_WORD under PST_CASEPAT): a bare
    // `esac' in the pattern-list state is the terminator ESAC — only a
    // directly preceding `|' (Posix grammar rule 4) or `(' (the phantom
    // rule, an open pattern group) keeps it pattern TEXT. No
    // `;;`-lookahead applies: `case x in a) :;; esac) echo hi;; esac'
    // closes the case at the inter-clause `esac' and the `)' is the outer
    // grammar's error (rubash#390 q3/q4 — verified vs WSL GNU 5.3.0).
    !matches!(
        index.checked_sub(1).and_then(|i| tokens.get(i)),
        Some(token)
            if (token.kind == TokenKind::Pipe && token.value == "|")
                || (token.kind == TokenKind::PipeErr)
                || (token.kind == TokenKind::Keyword && token.value == "(")
    )
}

/// GNU parse.y:3428-3441 special_case_tokens(): `esac` DIRECTLY after the
/// case's `in` is returned as the ESAC token (the `case WORD newline_list IN
/// newline_list ESAC` empty-case production, parse.y:1037), even though `in`
/// is NOT a reserved-word boundary — reserved_word_acceptable (parse.y:5898)
/// has no IN, so this is the one special case that arms the recognition
/// (guarded there by esacs_needed_count, i.e. an open case, which at the
/// token-scanner call sites means the caller's case_depth is nonzero; at
/// depth 0 the saturating decrement is a no-op, keeping `for i in esac` a
/// word list). Group scanners whose case tracking is gated on
/// `command_boundary_keyword_allowed` must consult this alongside the
/// boundary check or they never close `( case x in esac )` (rubash#336).
pub(super) fn follows_case_in_keyword(tokens: &[Token], index: usize) -> bool {
    // GNU parse.y:1037 `case_command: ... IN newline_list ESAC': the empty
    // case's `esac' may be separated from its `in' by a newline_list (the
    // physical newlines are Semicolon tokens with line_break here), so the
    // `in' witness must look back past them — `( case x in\nesac )` closes
    // the case at that `esac' and the `)' is the subshell closer
    // (rubash#390 q3; without this, the pattern heuristic in
    // case_pattern_starts_with_esac reclassified the terminator `esac' as
    // a pattern when a later `;;' existed on the line).
    let mut previous = index;
    while previous > 0 {
        previous -= 1;
        let token = &tokens[previous];
        if token.kind == TokenKind::Semicolon && token.line_break {
            continue;
        }
        return is_keyword(tokens, previous, "in");
    }
    false
}

fn case_pattern_starts_with_esac(tokens: &[Token], index: usize) -> bool {
    // A clause terminator proves this is the case command's closing esac,
    // even when the following subshell close is adjacent (esac)).
    if index > 0 && is_case_clause_terminator_token(&tokens[index - 1]) {
        return false;
    }

    if !matches!(
        tokens.get(index + 1).map(|token| token.value.as_str()),
        Some(")" | "|")
    ) {
        return false;
    }

    let mut close = index + 1;
    while close < tokens.len() {
        if is_keyword(tokens, close, ")") {
            break;
        }
        if tokens[close].kind == TokenKind::Semicolon {
            return false;
        }
        close += 1;
    }
    if !is_keyword(tokens, close, ")") {
        return false;
    }

    let mut scan = close + 1;
    while scan < tokens.len() {
        if is_case_clause_terminator_token(&tokens[scan]) || is_keyword(tokens, scan, "esac") {
            return true;
        }
        if is_keyword(tokens, scan, ")") && command_boundary_keyword_allowed(tokens, scan) {
            return false;
        }
        scan += 1;
    }

    false
}

fn is_case_clause_terminator_token(token: &Token) -> bool {
    token.kind == TokenKind::Word && matches!(token.raw.as_str(), ";;" | ";&" | ";;&")
}

/// GNU parse.y for_command/select_command/while_command/until_command all
/// accept either "do list done" or the brace-group form ("{ list }") as the
/// loop body, so the matching terminator may be "done" or "}".
const LOOP_BODY_TERMINATOR: &str = "done-or-brace";

/// Case-pattern region sentinels for `update_compound_boundary_stack`,
/// mirroring GNU's PST_CASEPAT reader state (parser.h:29, set at parse.y
/// 3379/3396 when the `in` of a case is read, cleared at the clause's `)`
/// at 3788). While the reader is inside a pattern list, CHECK_FOR_RESERVED
/// WORD (parse.y:3177-3186) refuses to recognize reserved words — `while'
/// in `case x in (while|break))' is PATTERN TEXT, never a loop opener
/// (only `esac' can match, and only when the previous token is not `|',
/// parse.y:3181 Posix grammar rule 4). The NUL prefix cannot collide with
/// any token value the stack otherwise holds.
const CASE_PATTERN_EXPECT: &str = "\u{0}case-pattern";
/// Entered right after `in' or a clause terminator: the NEXT token may be
/// the optional clause-opening `(' (parse.y:1231), which is grammar, not
/// extglob nesting.
const CASE_PATTERN_MAY_OPEN: &str = "\u{0}case-pattern-may-open";
/// An extglob `(' opened inside the pattern text (parse.y:5466 absorbs
/// these into the pattern word in GNU; rubash's tokenizer emits them).
const CASE_PATTERN_PAREN: &str = "\u{0}case-pattern-paren";

/// The clause `)' that closes a pattern region: expected only when a
/// pattern sentinel sits on top of the stack.
fn case_pattern_state_step(tokens: &[Token], index: usize, stack: &mut Vec<&'static str>) -> bool {
    let top = stack.last().copied();
    if !matches!(
        top,
        Some(CASE_PATTERN_EXPECT) | Some(CASE_PATTERN_MAY_OPEN) | Some(CASE_PATTERN_PAREN)
    ) {
        return false;
    }
    let token = &tokens[index];
    // newline_list is legal before the clause's first pattern token and
    // around clause boundaries (parse.y:1225 pattern_list): a physical
    // newline never settles the may-open state and never counts as
    // pattern text (rubash#308: `case x in <newline> ( a ) ...' made the
    // newline consume the may-open slot, so the clause `(' became extglob
    // nesting and desynchronized the whole scan).
    if token.kind == TokenKind::Semicolon && token.line_break {
        return true;
    }
    if top == Some(CASE_PATTERN_MAY_OPEN) {
        // The optional clause-opening `(' (or anything else) settles the
        // state; re-dispatch below against CASE_PATTERN_EXPECT.
        stack.pop();
        stack.push(CASE_PATTERN_EXPECT);
        if is_unquoted_operator(token, "(") && token.kind == TokenKind::Keyword {
            // Consumed as the clause open; pattern text begins after it.
            return true;
        }
    }
    if is_unquoted_operator(token, "(") && token.kind == TokenKind::Keyword {
        stack.push(CASE_PATTERN_PAREN);
        return true;
    }
    if is_unquoted_operator(token, ")") && token.kind == TokenKind::Keyword {
        if stack.last().copied() == Some(CASE_PATTERN_PAREN) {
            stack.pop();
        } else {
            // The clause-closing `)': leaves the pattern region (GNU clears
            // PST_CASEPAT here, parse.y:3788).
            stack.pop();
        }
        return true;
    }
    if token.kind == TokenKind::Keyword
        && token.value == "esac"
        && !matches!(index.checked_sub(1).and_then(|i| tokens.get(i)), Some(prev) if prev.value == "|")
    {
        // `esac' after `in'/terminator is recognized even in pattern state
        // (`case x in esac' empty case); pop the pattern sentinel and let
        // the caller's expect matching run for the "esac" below it.
        stack.pop();
        return false;
    }
    // Pattern text: words, `|' alternatives, and every reserved word stay
    // inert — no compound opener is pushed (GNU parse.y:3177-3178).
    true
}
/// GNU parse.y:879-880 select_command/for_command have TWO body forms —
/// `... do list done` and `... '{' list '}'`. Once the `do` of the do-form
/// has been read, the only legal closer is `done`; a `}` inside the body
/// closes the innermost open brace group (a nested group or a function
/// definition body), never the loop. The frame is transformed at `do` so
/// the LOOP_BODY_TERMINATOR `}` shortcut keeps applying only to the
/// brace-form production.
const LOOP_DO_TERMINATOR: &str = "done";

pub(super) fn update_compound_boundary_stack(
    tokens: &[Token],
    index: usize,
    stack: &mut Vec<&'static str>,
) {
    // rubash#461: a fragment of an open `=~' RHS regexp word is one GNU
    // word's interior (PST_REGEXP, parse.y:3663 goto tokword) — it can
    // neither open a compound frame (`case' after the regex `|') nor
    // close one (`done' as regex data), so the whole boundary grammar is
    // inert for it.
    if tokens
        .get(index)
        .is_some_and(|token| token.regexp_rhs_fragment)
    {
        return;
    }

    // Case-pattern region first: keywords inside `in ... )' are pattern
    // text (GNU PST_CASEPAT), never compound openers (rubash#308:
    // `(while|break)' inside a function body opened a phantom loop that
    // swallowed the function's `}' and corrupted every enclosing scan).
    if case_pattern_state_step(tokens, index, stack) {
        return;
    }

    if let Some(expected) = stack.last().copied() {
        let expected_matches = if expected == "esac" {
            is_case_end_keyword(tokens, index)
        } else if expected == LOOP_BODY_TERMINATOR {
            is_keyword(tokens, index, "done")
                || is_boundary_keyword(tokens, index, "}")
                || folded_brace_group_closes_loop(tokens, index)
        } else if expected == LOOP_DO_TERMINATOR {
            is_keyword(tokens, index, "done")
        } else if expected == "}" {
            is_boundary_keyword(tokens, index, "}")
        } else {
            is_keyword(tokens, index, expected)
        };
        if expected_matches {
            stack.pop();
            // GNU parse.y for_command/select_command brace-form body: the
            // `'}' that closes the '{' list '}' body also ends the loop —
            // one token, one grammar role. A do-form loop sits on
            // LOOP_DO_TERMINATOR here, so this only fires for the
            // brace-form frame directly beneath a just-closed brace group.
            if expected == "}" && stack.last().copied() == Some(LOOP_BODY_TERMINATOR) {
                stack.pop();
            }
            return;
        }
    }

    // The case's own `in' (parse.y:3379/3396 sets PST_CASEPAT) and the
    // clause terminators that restart the pattern list enter the pattern
    // state regardless of command-boundary position: `in' follows the case
    // WORD, and `;;' follows the clause body's last command.
    if is_keyword(tokens, index, "in") && stack.last().copied() == Some("esac") {
        stack.push(CASE_PATTERN_MAY_OPEN);
        return;
    }
    if is_case_clause_terminator_token(&tokens[index]) && stack.last().copied() == Some("esac") {
        stack.push(CASE_PATTERN_MAY_OPEN);
        return;
    }

    if !command_boundary_keyword_allowed(tokens, index) {
        return;
    }

    if is_keyword(tokens, index, "if") {
        stack.push("fi");
    } else if matches!(
        tokens.get(index).map(|token| token.value.as_str()),
        Some("for" | "select" | "while" | "until")
    ) {
        stack.push(LOOP_BODY_TERMINATOR);
    } else if is_keyword(tokens, index, "case") {
        stack.push("esac");
    } else if is_keyword(tokens, index, "do") && stack.last().copied() == Some(LOOP_BODY_TERMINATOR)
    {
        // parse.y:882-884 `do newline_list list done`: with the `do` read,
        // `done` is the only terminator (see LOOP_DO_TERMINATOR).
        stack.pop();
        stack.push(LOOP_DO_TERMINATOR);
    } else if is_boundary_keyword(tokens, index, "{") || word_command_open_brace(tokens, index) {
        // A brace group opened inside any open keyword frame is its own
        // nesting level (parse.y:936 simple_command '{' list '}'): its '}'
        // must not serve as an enclosing loop's terminator or an outer
        // group's closer (rubash#314: a multi-line `g() { ... }` as a
        // direct statement of a loop body inside a function lost its
        // closing brace to the enclosing function's group scan).
        stack.push("}");
    }
}

/// Whether `tokens[index]` is a folded complete `{ ...; }' group that
/// terminates an open brace-form loop frame (LOOP_BODY_TERMINATOR). GNU
/// parse.y:879-884/936 `... '{' compound_list '}'` — a loop whose
/// brace-form body arrives as one folded keyword token is COMPLETE after
/// it, exactly like the unfolded `}' closer, so the body scan of an
/// ENCLOSING construct must not keep the loop frame open and steal the
/// next `}' (invocation3.sub: `for i in 1 2 3; { select var in a; { :; }
/// <<<a; }` — the select's folded `{ :; }' body left its frame open and
/// the for's `}' closed the select instead, failing the whole script).
/// A folded group followed by `do' is a loop CONDITION (`while { :; };
/// do :; done', parse.y:886 `while_command: WHILE compound_list DO ...`),
/// not the body — keep the frame so the do-form transform still applies.
fn folded_brace_group_closes_loop(tokens: &[Token], index: usize) -> bool {
    let Some(token) = tokens.get(index) else {
        return false;
    };
    if token.kind != TokenKind::Keyword
        || !token.value.starts_with('{')
        || !token.value.ends_with('}')
        || token.value.len() < 2
    {
        return false;
    }
    // `while { :; }; do' / `while { :; }\ndo' keep LOOP_BODY open: skip
    // separators (`;', newlines) and let the FIRST significant token
    // decide — `do' means the group was the loop's condition list.
    !tokens[index + 1..]
        .iter()
        .find(|next| next.kind != TokenKind::Semicolon)
        .is_some_and(|next| next.kind == TokenKind::Keyword && next.value == "do")
}

pub(super) fn command_boundary_keyword_allowed(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return true;
    };

    if previous.kind == TokenKind::Keyword
        && ((previous.value.starts_with('{') && previous.value.ends_with('}'))
            // A folded `(( ... ))' arithmetic group is GNU's ARITH_CMD
            // token, which reserved_word_acceptable (parse.y:5899) also
            // accepts as the predecessor of a reserved word — `if (( x
            // == y )) then' / `while (( x )) do' carry no `;' terminator
            // (rubash#460; the unfolded `))' closer is handled by
            // compound_close_precedes below).
            || (previous.value.starts_with("((")
                && previous.value.ends_with("))")
                && previous.value.len() >= 4))
    {
        return true;
    }

    // GNU parse.y reads reserved words at any position where a command is
    // complete: separators, `)`/`}`-style closers, and the `]]`/`))` that
    // end `[[ ]]`/`(( ))` (cond.tests:230 `if [[ str ]] then [[ str ]] fi`
    // — no `;` before `then`). A `]]`/`))` that never closed a matching
    // opener is a plain word argument (`if echo ]] then` is a GNU syntax
    // error, not a boundary), so verify the pairing backward.
    matches!(
        previous.kind,
        TokenKind::Semicolon
            | TokenKind::HereDocBody
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::Background
    ) || (previous.kind == TokenKind::Keyword
        && matches!(
            previous.value.as_str(),
            "{" | "(" | ")" | "then" | "do" | "else" | "elif" | "fi" | "done" | "esac" | "}"
                // GNU parse.y:5899-5940 reserved_word_acceptable also
                // accepts BANG, TIME, TIMEOPT, TIMEIGN, IF, WHILE, UNTIL
                // and COPROC as the preceding token: a compound command
                // is a legal pipeline element after `!' (rubash#257
                // `if ...; then ! case x in ...) ;; esac; fi') and after
                // `time'/`if'/`while'/`until'/`coproc'. Without these the
                // if/loop body-boundary scanner never pushes the inner
                // compound and truncates the body at its own closer
                // (`esac' arrives "where `fi' was expected").
                | "!" | "time" | "if" | "while" | "until" | "coproc"
        ))
        || (previous.kind == TokenKind::Word
            && matches!(previous.raw.as_str(), ";;" | ";&" | ";;&"))
        // GNU parse.y:5906-5908 TIMEOPT/TIMEIGN: `-p' after `time' and
        // `--' after `time'/`time -p' also leave a command boundary.
        || time_option_precedes(tokens, index)
        // GNU parse.y:5934-5939: a WORD directly after `coproc' or
        // `function' (the NAME of `coproc NAME cmd' / `function f {')
        // is followed by an acceptable reserved word.
        || (previous.kind == TokenKind::Word
            && matches!(
                index.checked_sub(2).and_then(|i| tokens.get(i)).map(|t| {
                    (t.kind.clone(), t.value.clone())
                }),
                Some((TokenKind::Keyword, ref v)) if v == "coproc" || v == "function"
            ))
        || compound_close_precedes(tokens, index)
}

/// Whether the token at `index - 1` is a TIMEOPT/TIMEIGN option word —
/// `-p` right after the `time` keyword, or `--` after `time`/`time -p`
/// (GNU parse.y:3470-3479 tokenizes exactly these two spellings).
fn time_option_precedes(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return false;
    };
    if previous.kind != TokenKind::Word {
        return false;
    }
    let before_that = index
        .checked_sub(2)
        .and_then(|i| tokens.get(i))
        .map(|t| (t.kind.clone(), t.value.clone()));
    match previous.raw.as_str() {
        "-p" => {
            matches!(&before_that, Some((TokenKind::Keyword, v)) if v == "time")
        }
        "--" => match &before_that {
            Some((TokenKind::Keyword, v)) if v == "time" => true,
            Some((TokenKind::Word, w)) if w == "-p" => {
                matches!(
                    index.checked_sub(3).and_then(|i| tokens.get(i)).map(|t| {
                        (t.kind.clone(), t.value.clone())
                    }),
                    Some((TokenKind::Keyword, v)) if v == "time"
                )
            }
            _ => false,
        },
        _ => false,
    }
}

/// Whether the token at `index - 1` is a `]]` or `))` that actually
/// closed a matching `[[`/`((` opener (GNU: the closer ends the compound
/// command, so the next token sits at a command boundary). Scans backward
/// counting closer/opener balance on raw spelling so quoted `']]'` and
/// word-argument `]]` never count.
fn compound_close_precedes(tokens: &[Token], index: usize) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|i| tokens.get(i)) else {
        return false;
    };
    let (closer, opener) = match previous.raw.as_str() {
        "]]" => ("]]", "[["),
        "))" => ("))", "(("),
        _ => return false,
    };
    let mut depth = 0i32;
    for token in tokens[..index].iter().rev() {
        if token.raw == closer {
            depth += 1;
        } else if token.raw == opener {
            depth -= 1;
            if depth == 0 {
                return true;
            }
        }
    }
    false
}

/// Deepest-first search for a parse-error marker inside a command subtree.
/// Mirrors GNU's unit-of-parse: a compound command (function definition,
/// brace group, subshell, if/while/for/case/select body) is parsed as one
/// command, and a syntax error anywhere inside it aborts the reader at
/// that command (parse.y parse_command -> report_syntax_error; verified:
/// GNU 5.3.0 rejects `f() { case x in ?(a)) :;; esac; }` with rc 2 at the
/// definition, and `bash -n` on bash-completion fails at the pattern line).
/// Rubash parks body errors as marker assignments on the inner node, so
/// this walks the child command containers and reports the innermost
/// marked node. Word-embedded command substitutions are deliberately not
/// walked: their failure is contained per GNU's child parse (rubash#131).
pub(super) fn subtree_parse_error_node(command: &CommandNode) -> Option<&CommandNode> {
    fn walk<'a>(command: &'a CommandNode, found: &mut Option<&'a CommandNode>) {
        if found.is_some() {
            return;
        }
        if command
            .assignments
            .iter()
            .any(|(name, _)| name.starts_with("__RUBASH_PARSE_ERROR"))
        {
            *found = Some(command);
            return;
        }
        let mut bodies: Vec<&[CommandNode]> = Vec::new();
        if let Some(list) = &command.and_or_list {
            bodies.push(&list.commands);
        }
        if let Some(pipeline) = &command.pipeline_command {
            bodies.push(&pipeline.stages);
        }
        if let Some(compound) = &command.for_command {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.select_command {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.if_command {
            bodies.push(&compound.then_body);
            if let Some(else_body) = &compound.else_body {
                bodies.push(else_body);
            }
        }
        if let Some(compound) = &command.loop_command {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.subshell_command {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.case_command {
            for clause in &compound.clauses {
                bodies.push(&clause.body);
            }
        }
        if let Some(compound) = &command.function_command {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.brace_group {
            bodies.push(&compound.body);
        }
        if let Some(compound) = &command.coproc_command {
            if let Some(body) = &compound.body {
                bodies.push(body);
            }
        }
        let singles: Vec<&CommandNode> = [
            command.time_command.as_ref().map(|c| &*c.command),
            command.background_command.as_ref().map(|c| &*c.command),
            command.inverted_command.as_ref().map(|c| &*c.command),
        ]
        .into_iter()
        .flatten()
        .collect();
        for body in bodies {
            for child in body {
                walk(child, found);
                if found.is_some() {
                    return;
                }
            }
        }
        for child in singles {
            walk(child, found);
            if found.is_some() {
                return;
            }
        }
    }
    let mut found = None;
    walk(command, &mut found);
    found
}

/// Bubble an inner parse-error marker up to the compound command's own
/// node so the executor's top-level marker check (which fires under `-n`
/// too, via the reader-loop diagnostics) reports the failure where GNU's
/// parser would have aborted (rubash#131).
pub(super) fn propagate_subtree_parse_error(command: &mut CommandNode) {
    if command
        .assignments
        .iter()
        .any(|(name, _)| name.starts_with("__RUBASH_PARSE_ERROR"))
    {
        return;
    }
    // GNU yyerror's parser_error cites the parser's line_number at the
    // FAILURE (parse.y:6839 via error.c:300-316), which is the offending
    // token's line — an enclosing compound that adopts the subtree's error
    // must report it there, not at the compound's own opening line
    // (`while :; do case x in\nesac) ...` names the `esac)' line,
    // rubash#390 q3).
    let mut error_line = None;
    let markers: Vec<(String, String)> = match subtree_parse_error_node(command) {
        Some(inner) => {
            error_line = inner.line;
            inner
                .assignments
                .iter()
                .filter(|(name, _)| name.starts_with("__RUBASH_PARSE"))
                .cloned()
                .collect()
        }
        None => return,
    };
    command.line = error_line.or(command.line);
    for (name, value) in markers {
        command.insert_assignment(name, value);
    }
}

pub(super) fn is_boundary_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    command_boundary_keyword_allowed(tokens, index) && is_keyword(tokens, index, value)
}

pub(super) fn token_completes_brace_group_command(token: &Token) -> bool {
    // GNU parse.y `compound_list: newline_list list0` and
    // `list0: list1 '\n' newline_list | list1 '&' newline_list |
    // list1 ';' newline_list` (parse.y:1262-1279): a `{ }` group closes
    // after a command terminated by `;', a newline, or `&'. `&&'/`||'
    // (And/Or) are connectors, not terminators — `{ a && }` stays an
    // error as in GNU.
    if matches!(token.kind, TokenKind::Semicolon | TokenKind::Background) {
        return true;
    }
    // GNU parse.y:1265 compound_list: `newline_list list1' — the list may
    // end in a bare completed command; a closed `[[ ]]' conditional is one
    // (COND_END is in reserved_word_acceptable's list, parse.y:5920, so a
    // following `}' is the group's close: `{ [[ a ]] }' parses). The `]]'
    // arrives as a word-shaped token (the conditional parser matches it by
    // raw text); the unquoted-raw check keeps quoted `"]]"' out.
    if token.value == "]]" && token.raw == "]]" {
        return true;
    }
    // An `(( ... ))' arithmetic-command token is likewise a completed
    // compound command (`{ ((1+2)) }' parses; ARITH_CMD is in the
    // acceptable list, parse.y:5912).
    if token.value.starts_with("((") && token.value.ends_with("))") {
        return true;
    }
    if token.kind != TokenKind::Keyword {
        return false;
    }
    if matches!(token.value.as_str(), "}" | ")" | "fi" | "done" | "esac") {
        return true;
    }
    if token.value.starts_with('{') && token.value.ends_with('}') && token.value.len() >= 2 {
        // A collapsed `{ ... }' group token is itself a completed compound
        // command (GNU parse.y:1196 group_command; a closed group satisfies
        // compound_list without a trailing separator). Whether ITS body ends
        // in a completed command is enforced when the group's own recursive
        // parse runs below — the innermost incomplete group reports the
        // error at its own level, where GNU's grammar checks it. The old
        // re-tokenize recursion here summed to O(depth²·input) on nested
        // single-line groups (rubash#176).
        return true;
    }
    false
}

pub(super) fn matching_brace_group_end(tokens: &[Token], start: usize) -> Option<usize> {
    // The lexer keeps trailing blanks inside a '{' keyword token when the
    // brace is followed by blanks before the physical newline ("{\t"), so
    // recognize the opening brace the same way parse_function_command's
    // value.trim() == "{" gate does. An exact is_keyword(tokens, start,
    // "{") here rejects "{\t", the function-body scan returned None, and
    // the caller's last-'}' fallback silently absorbed the rest of the
    // script into a dead function body (GNU parse.y reads the reserved
    // word '{' and treats the blanks as a token separator).
    if !tokens
        .get(start)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value.trim() == "{")
    {
        return None;
    }

    let mut depth = 1usize;
    let mut stack = Vec::new();
    let mut index = start + 1;
    while index < tokens.len() {
        // A token that popped (or pushed) a keyword/brace frame this pass
        // has already served its grammar role for this scan (e.g. a `}`
        // that closed a nested brace group inside an open loop frame) —
        // it must not ALSO adjust the brace depth, or an inner group's
        // closer terminates the outer group (rubash#314).
        let stack_depth_before = stack.len();
        update_compound_boundary_stack(tokens, index, &mut stack);
        if stack.len() != stack_depth_before || !stack.is_empty() {
            index += 1;
            continue;
        }

        if is_boundary_keyword(tokens, index, "{") || word_command_open_brace(tokens, index) {
            depth += 1;
        } else if is_boundary_keyword(tokens, index, "}") {
            // GNU parse.y requires a completed command before a brace-group
            // close. `{ command }` is malformed; `{ command; }`, a physical
            // newline, and a completed compound command before `}` are valid.
            let has_completed_command = index > start + 1
                && tokens
                    .get(index - 1)
                    .is_some_and(token_completes_brace_group_command);
            if !has_completed_command {
                return None;
            }
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

/// GNU's grammar puts `{` in command position after constructs whose
/// rubash token stream shows it following a WORD or a non-boundary
/// keyword, so `command_boundary_keyword_allowed` alone misses it:
/// `coproc [NAME] {` (parse.y:1125-1174 `coproc: COPROC [WORD]
/// shell_command`), `function NAME {` (function_def `FUNCTION WORD
/// command`), and the pipeline prefixes `!`, `time`, `time -p`,
/// `time --` (parse.y `time_pipeline`, `BANG`). Without this, the `{` is
/// invisible to the depth scan and that group's `}` is mistaken for the
/// enclosing brace's close (type4.sub `mkcoprocs`: `coproc a { cat
/// <<EOF1 ... }` aborted the whole function parse).
/// The walk leftwards consumes only genuine command prefixes, so
/// `echo coproc a {` and `foo bar {` keep `{` as a plain word.
fn word_command_open_brace(tokens: &[Token], index: usize) -> bool {
    if !is_keyword(tokens, index, "{") {
        return false;
    }
    let mut j = index;
    let mut saw_prefix = false;
    while j > 0 {
        let prev = &tokens[j - 1];
        match prev.kind {
            TokenKind::Keyword => match prev.value.as_str() {
                // BANG negation, TIME, and `coproc {`/`function {`
                // (the latter malformed in GNU but harmless to accept).
                "!" | "time" | "coproc" | "function" => {
                    saw_prefix = true;
                    j -= 1;
                }
                _ => return false,
            },
            TokenKind::Word => match prev.value.as_str() {
                // `time -p` / `time --` options (TIMEOPT/TIMEIGN).
                "-p" | "--" => j -= 1,
                _ => {
                    // A WORD directly before `{` is a command argument
                    // (`echo a {`) unless it is a coproc/function name,
                    // which must directly follow its keyword.
                    if j >= 2
                        && matches!(tokens[j - 2].kind, TokenKind::Keyword)
                        && matches!(tokens[j - 2].value.as_str(), "coproc" | "function")
                    {
                        saw_prefix = true;
                        j -= 2;
                        continue;
                    }
                    return false;
                }
            },
            // A separator or other non-word token: `{` opens a group only
            // when the walk consumed at least one command prefix.
            _ => return saw_prefix,
        }
    }
    saw_prefix
}

/// GNU parse.y:1054 `function_def: WORD '(' ')' newline_list function_body'.
/// The LALR parser shifts a `(' that arrives after EXACTLY ONE WORD at
/// command position as a function-head candidate: a pending `!' inversion
/// still allows the shift (GNU `! +(a|b)' errors at `a'), while anything
/// that already committed the parse to `simple_command' — a second word, an
/// assignment prefix (`x=1 y=2 +(a|b)' errors at `('), or a redirection
/// prefix (`>f (a)' errors at `(') — does not. Word-DERIVED records
/// (expansions, quotes, extglob patterns) ride on the single word and never
/// disqualify it; `__RUBASH_*' marker pairs share the assignments list with
/// real assignments and are likewise ignored. The structural fields are
/// exactly what `command_is_empty` checks once the word itself is stripped.
pub(super) fn lone_word_function_head_candidate(cmd: &CommandNode) -> bool {
    if cmd.words.len() != 1 {
        return false;
    }
    let mut bare = cmd.clone();
    bare.inverted = false;
    bare.words.clear();
    bare.word_kinds.clear();
    bare.assignments
        .retain(|(name, _)| !name.starts_with("__RUBASH_"));
    bare.process_substitutions.clear();
    bare.command_substitutions.clear();
    bare.arithmetic_expansions.clear();
    bare.parameter_expansions.clear();
    bare.brace_expansions.clear();
    bare.extglob_patterns.clear();
    bare.tilde_expansions.clear();
    bare.pathname_patterns.clear();
    bare.word_quotes.clear();
    command_is_empty(&bare)
}

pub(super) fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.compound_assignments.is_empty()
        && cmd.array_element_assignments.is_empty()
        && cmd.process_substitutions.is_empty()
        && cmd.command_substitutions.is_empty()
        && cmd.arithmetic_expansions.is_empty()
        && cmd.parameter_expansions.is_empty()
        && cmd.brace_expansions.is_empty()
        && cmd.extglob_patterns.is_empty()
        && cmd.tilde_expansions.is_empty()
        && cmd.pathname_patterns.is_empty()
        && cmd.word_quotes.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.redirects.is_empty()
        && cmd.pipe.is_none()
        && !cmd.background
        && cmd.and_or.is_none()
        && !cmd.inverted
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.for_command.is_none()
        && cmd.arithmetic_command.is_none()
        && cmd.if_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.conditional_command.is_none()
        && cmd.subshell_command.is_none()
        && cmd.case_command.is_none()
        && cmd.select_command.is_none()
        && cmd.function_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.coproc_command.is_none()
}

pub(super) fn command_is_open_conditional(cmd: &CommandNode) -> bool {
    (cmd.words.first().map(String::as_str) == Some("[[")
        || (matches!(cmd.words.first().map(String::as_str), Some("if" | "elif"))
            && cmd.words.get(1).map(String::as_str) == Some("[[")))
        && !cmd.words.iter().any(|word| word == "]]")
}

pub(super) fn command_accepts_embedded_arithmetic_command(cmd: &CommandNode) -> bool {
    matches!(
        cmd.words.first().map(String::as_str),
        Some("if" | "elif" | "while" | "until" | "do" | "then" | "else")
    ) && cmd.words.len() == 1
}

/// GNU parse.y:1054-1061 function_def: `WORD '(' ')' newline_list
/// function_body` — the grammar accepts ANY word as the candidate name; `{`
/// and `}` are ordinary word characters (syntax.h:29-30), so
/// `ble/x:{s}/d` and `{x}' parse as function definitions (rubash#244).
/// Character rejection happens only at definition time
/// (execute_cmd.c:6289 execute_intern_function -> general.c:445
/// valid_function_word), which the executor models separately
/// (executor/function_calls.rs define_function). What remains here is only
/// what can never be an unquoted word's text: real shell break characters
/// and operator bytes (these only reach a word value inside `$(...)` /
/// `${...}` spans — the `$' family, owned by the executor's W_HASDOLLAR
/// check).
pub(super) fn is_function_name(name: &str) -> bool {
    if name.is_empty() || name.contains('=') {
        return false;
    }

    !name
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | ';' | '&' | '|'))
}

/// Same grammar class for the `function name { ... }' keyword form
/// (parse.y:1056-1061): any WORD, with `{`/`}` legal name characters
/// (rubash#244). See is_function_name for the executor-level split.
pub(super) fn is_function_keyword_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | ';' | '&' | '|'))
}

/// GNU parse.y:1097 `subshell: '(' compound_list ')'` and the
/// `DO compound_list DONE` loop/select rules (parse.y:867-872, 893-993):
/// compound_list is `newline_list list0` (parse.y:1252-1279) — newlines may
/// precede the command list, but the list itself must contain a command.
/// When the body slice between the delimiters holds nothing that can start
/// a command, the yacc lookahead at the failure point is the reported
/// token: `( )` => near `)', `( ; )` => near `;',
/// `while false; do ; done` => near `;', `while false; do done` => near
/// `done' (rubash#221). A case arm before `;;` is NOT this rule (an arm
/// body may be empty) and stays accepted.
pub(super) fn empty_compound_body_error_node(
    tokens: &[Token],
    body_start: usize,
    close_index: usize,
) -> Option<CommandNode> {
    let close = close_index.min(tokens.len());
    let mut index = body_start.min(close);
    while index < close {
        let token = &tokens[index];
        if token.kind == TokenKind::Semicolon && token.line_break {
            // newline_list: a physical newline before the list is fine.
            index += 1;
            continue;
        }
        // The first non-newline token decides: only tokens that can never
        // begin list0's list1 leave the body empty.
        let cannot_start_command = match token.kind {
            TokenKind::Semicolon
            | TokenKind::Background
            | TokenKind::Pipe
            | TokenKind::PipeErr
            | TokenKind::And
            | TokenKind::Or => true,
            TokenKind::Word => matches!(token.value.as_str(), ";;" | ";&" | ";;&"),
            TokenKind::Keyword => {
                matches!(token.value.as_str(), "done" | "fi" | "esac" | "}" | ")")
            }
            _ => false,
        };
        if cannot_start_command {
            return Some(super::parse_loop::mismatched_closer_node(
                tokens, index, None, 0,
            ));
        }
        return None;
    }
    // Only newlines (or nothing): the closer itself is the lookahead.
    Some(super::parse_loop::mismatched_closer_node(
        tokens, close, None, 0,
    ))
}

/// GNU's function_def grammar accepts any single WORD as a candidate function
/// name; a word whose raw text differs from its dequoted value was quoted or
/// escaped (W_QUOTED), which the executor later rejects via err_invalidid.
/// The parser must still parse `'a b c' () { ...; }' as a function definition
/// so the executor can report the name error (parse.y: function_def).
pub(super) fn is_quoted_function_name(name: &str, name_raw: &str) -> bool {
    !name_raw.is_empty() && name_raw != name
}
