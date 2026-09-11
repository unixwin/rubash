use super::heredoc_scan::skip_heredoc_in_chars_with_closure;

pub(super) fn ends_with_unquoted_backslash(input: &str) -> bool {
    // GNU parse.y shell_getc remove_quoted_newline: backslash-newline is ignored
    // unless the current delimiter is a single quote (qc == '\''). The dstack
    // correctly nests quotes inside command substitutions: a single quote
    // inside $(...) is quoted even when the substitution itself is inside
    // double quotes (quote.tests: echo "$(echo 'foo\↵bar')" must keep the
    // backslash). The old boolean single/double tracker treated '\'' as
    // literal when double==true, so the trailing '\' inside that nested
    // single was misclassified as unquoted and the logical line was joined
    // with the backslash removed (foobar instead of foo\↵bar).
    // Track a delimiter stack mirroring parse.y dstack for the cases that
    // affect this probe: ' " ` and $(.
    let chars: Vec<char> = input.chars().collect();
    let mut stack: Vec<char> = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let top = stack.last().copied();
        if top == Some('\'') {
            if ch == '\'' {
                stack.pop();
            }
            i += 1;
            continue;
        }
        if top == Some('"') {
            if ch == '\\' {
                // Inside double quotes a backslash escapes the next char
                // (parse.y parse_matched_pair LEX_PASSNEXT). Consume both
                // so a trailing '\' inside double is correctly seen as
                // an escaped newline that should be removed.
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if ch == '"' {
                stack.pop();
                i += 1;
                continue;
            }
            if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                stack.push('(');
                i += 2;
                continue;
            }
            if ch == '`' {
                stack.push('`');
                i += 1;
                continue;
            }
            // Single quote inside double quotes is literal (POSIX).
            i += 1;
            continue;
        }
        if top == Some('`') {
            if ch == '\\' {
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            if ch == '`' {
                stack.pop();
            }
            i += 1;
            continue;
        }
        if top == Some('(') {
            // Inside command substitution quoting resets: ' " ` and nested $(
            // are delimiters again (parse.y read_token_word / parse_matched_pair).
            if ch == '\'' {
                stack.push('\'');
                i += 1;
                continue;
            }
            if ch == '"' {
                stack.push('"');
                i += 1;
                continue;
            }
            if ch == '`' {
                stack.push('`');
                i += 1;
                continue;
            }
            if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                stack.push('(');
                i += 2;
                continue;
            }
            if ch == ')' {
                stack.pop();
                i += 1;
                continue;
            }
            if ch == '\\' {
                if i + 1 < chars.len() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        // Top-level (no quote or other)
        if ch == '\'' {
            stack.push('\'');
            i += 1;
            continue;
        }
        if ch == '"' {
            stack.push('"');
            i += 1;
            continue;
        }
        if ch == '`' {
            stack.push('`');
            i += 1;
            continue;
        }
        if ch == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            stack.push('(');
            i += 2;
            continue;
        }
        if ch == '\\' {
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        i += 1;
    }

    if stack.last() == Some(&'\'') {
        return false;
    }
    let trailing_backslashes = input.chars().rev().take_while(|ch| *ch == '\\').count();
    trailing_backslashes % 2 == 1
}

pub(super) fn has_unclosed_quotes(input: &str) -> bool {
    // TODO(parse.y): Bash reads parser input with full quoting state,
    // continuations, command substitutions, arithmetic contexts, and here-doc
    // deferral. This tracks only enough single/double quote state to keep a
    // multi-line alias definition as one parser unit.
    //
    // Crucially, quote characters that live *inside* a parameter expansion
    // (`${...}`), command substitution (`$(...)` / backticks), or ANSI-C string
    // (`$'...'`) must NOT toggle the surrounding word's quote state. GNU Bash
    // scans these nested contexts with their own quote rules; a `'` inside
    // `${IFS+'}'z}` is balanced within the expansion and must not leak a
    // dangling single-quote into the rest of the line.
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }

        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }

        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '\\' && (!single || ansi_single) {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }

        // Skip a parameter expansion `${...}` as a self-contained unit so that
        // quotes inside its word/operator body do not affect outer state.
        // Inside double quotes the unit must still be skipped: the quote
        // toggles below are dolbrace-style (`'` only when !double, `"` only
        // when !single), so body quotes of `"${IFS+"'"x ~ x'}"` would
        // otherwise leak a dangling double-quote state and report the line
        // as unclosed (posixexp2 case 28). The span scan is mode-dependent —
        // POSIX honors the Interp 221 big hammer (`'` literal, first `}`
        // closes), non-POSIX honors quotes — so try POSIX first and fall back
        // to the non-POSIX scan; if neither closes, the line is unclosed.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'{') {
            let body: String = chars[index + 2..].iter().collect();
            if !double {
                let context = crate::lexer::dolbrace::BraceContext {
                    outer_double_quote: false,
                    posix: false,
                    replacement_context: false,
                    initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                };
                if let Some(scan) =
                    crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
                {
                    index += 2 + body[..scan.end].chars().count();
                    comment_start = false;
                    continue;
                }
                // Unterminated/odd expansion: fall through and let the caller
                // treat the input as having unclosed syntax.
            } else {
                let mut closed = false;
                for posix_mode in [true, false] {
                    let context = crate::lexer::dolbrace::BraceContext {
                        outer_double_quote: true,
                        posix: posix_mode,
                        replacement_context: false,
                        initial_state: crate::lexer::dolbrace::DolbraceState::Param,
                    };
                    if let Some(scan) =
                        crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
                    {
                        index += 2 + body[..scan.end].chars().count();
                        comment_start = false;
                        closed = true;
                        break;
                    }
                }
                if closed {
                    continue;
                }
                // Neither scan closes the span: fall through; the trailing
                // quote toggles leave the state as-is and the caller reports
                // unclosed input, matching the lexer's own fallback swallow.
            }
        }

        // Skip a command substitution `$(...)` (and `$((...))` arithmetic) as a
        // self-contained unit.
        // A balanced command substitution owns its nested quote state, even
        // when the substitution itself appears inside an outer double quote.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                continue;
            }
        }

        // Skip a backtick command substitution as a self-contained unit.
        if ch == '`' && !single && !double {
            if let Some(end) = skip_backtick_unit(&chars, index) {
                index = end;
                comment_start = false;
                continue;
            }
        }

        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }

        match ch {
            '\'' if ansi_single => {
                ansi_single = false;
                comment_start = false;
            }
            '\'' if !double && !ansi_single => {
                single = !single;
                comment_start = false;
            }
            '"' if !single && !ansi_single => {
                double = !double;
                comment_start = false;
            }
            _ => {
                if !single && !double && !ansi_single {
                    comment_start = false;
                }
            }
        }
        index += 1;
    }

    single || double || ansi_single
}

const DECLARATION_COMMAND_WORDS: [&str; 5] =
    ["declare", "typeset", "local", "export", "readonly"];

fn is_identifier_head(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if is_identifier_head(first) => {}
        _ => return false,
    }
    chars.all(is_identifier_char)
}

/// `name=` / `name+=` / `name[subscript]=...` prefix (anything may follow `=`).
fn assignment_prefix_word(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let head = &word[..eq];
    let head = head.strip_suffix('+').unwrap_or(head);
    let Some(open) = head.find('[') else {
        return valid_identifier(head);
    };
    head.ends_with(']') && valid_identifier(&head[..open])
}

/// True when the logical line ends inside an unclosed compound array
/// assignment: a `name=(` / `name+=(` word whose parentheses are still open
/// at end of line. GNU parse.y keeps reading physical lines until the
/// matching `)` closes the compound assignment word, so the line-oriented
/// collector must not finalize the logical line there (ISSUE #78: a
/// multi-line `plugins=(...)` rc assignment used to execute its elements as
/// commands).
///
/// GNU only continues for an assignment word in command position, in the
/// assignment-prefix region of a command (`x=1 a=(...`), or as an operand of
/// a declaration builtin (`declare -a b=(...`). Any other adjacent `name=(`
/// (`echo a=(b`) is an immediate syntax error in GNU, so it must NOT swallow
/// the following lines: the collector finalizes the logical line and the
/// parser reports the error.
pub(super) fn has_unclosed_compound_assignment(input: &str) -> bool {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut compound_depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut in_comment = false;
    let mut comment_start = true;
    let mut word = String::new();
    let mut word_pure = true;
    let mut seen_command_word = false;
    let mut declaration_context = false;

    while index < chars.len() {
        let ch = chars[index];

        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
                if compound_depth == 0 {
                    seen_command_word = false;
                    declaration_context = false;
                }
            }
            index += 1;
            continue;
        }

        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }

        if ch == '\n' && !single && !double && !ansi_single {
            comment_start = true;
            if compound_depth == 0 {
                if !word.is_empty() {
                    classify_top_level_word(
                        &word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    word.clear();
                    word_pure = true;
                }
                seen_command_word = false;
                declaration_context = false;
            }
            index += 1;
            continue;
        }

        if ch == '#' && !single && !double && !ansi_single && comment_start {
            in_comment = true;
            index += 1;
            continue;
        }

        if ch.is_whitespace() && !single && !double && !ansi_single {
            comment_start = true;
            if compound_depth == 0 && !word.is_empty() {
                classify_top_level_word(
                    &word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                word.clear();
                word_pure = true;
            }
            index += 1;
            continue;
        }

        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '$' && !single && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            word_pure = false;
            index += 2;
            continue;
        }

        // A `${...}` parameter expansion is opaque to parenthesis balancing.
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'{') {
            let body: String = chars[index + 2..].iter().collect();
            let context = crate::lexer::dolbrace::BraceContext {
                outer_double_quote: false,
                posix: false,
                replacement_context: false,
                initial_state: crate::lexer::dolbrace::DolbraceState::Param,
            };
            if let Some(scan) = crate::lexer::dolbrace::scan_braced_parameter_body(&body, context)
            {
                index += 2 + body[..scan.end].chars().count();
            } else {
                index += 2;
            }
            comment_start = false;
            word_pure = false;
            continue;
        }

        // A `$(...)` / `$((...))` command substitution is opaque to
        // parenthesis balancing; an unbalanced one is reported by
        // has_unclosed_command_substitution instead.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                word_pure = false;
                continue;
            }
            return false;
        }

        if ch == '`' && !single && !double {
            if let Some(end) = skip_backtick_unit(&chars, index) {
                index = end;
                comment_start = false;
                word_pure = false;
                continue;
            }
            return false;
        }

        if ch == '\'' && !double {
            single = !single;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if ch == '"' && !single {
            double = !double;
            comment_start = false;
            word_pure = false;
            index += 1;
            continue;
        }

        if single || double {
            index += 1;
            continue;
        }

        if ch == '(' && compound_depth == 0 {
            let opens_compound = word_pure
                && word
                    .strip_suffix('=')
                    .map(|head| {
                        let head = head.strip_suffix('+').unwrap_or(head);
                        valid_identifier(head)
                    })
                    .unwrap_or(false)
                && (!seen_command_word || declaration_context);
            if opens_compound {
                compound_depth = 1;
                word.clear();
                word_pure = true;
                // A `#` may start a comment right after the opening paren.
                comment_start = true;
                index += 1;
                continue;
            }
            // A subshell/grouping paren ends the assignment-prefix region:
            // GNU reports `echo a=(b` immediately instead of continuing.
            if !word.is_empty() {
                classify_top_level_word(
                    &word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                word.clear();
                word_pure = true;
            }
            seen_command_word = true;
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == '(' && compound_depth > 0 {
            compound_depth += 1;
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == ')' {
            if compound_depth > 0 {
                compound_depth -= 1;
                word.clear();
                word_pure = true;
            } else {
                if !word.is_empty() {
                    classify_top_level_word(
                        &word,
                        &mut seen_command_word,
                        &mut declaration_context,
                    );
                    word.clear();
                    word_pure = true;
                }
                seen_command_word = true;
            }
            comment_start = true;
            index += 1;
            continue;
        }

        if ch == ';' || ch == '|' || ch == '&' {
            if compound_depth == 0 && !word.is_empty() {
                classify_top_level_word(
                    &word,
                    &mut seen_command_word,
                    &mut declaration_context,
                );
                word.clear();
                word_pure = true;
                seen_command_word = false;
                declaration_context = false;
            }
            comment_start = true;
            index += 1;
            continue;
        }

        word.push(ch);
        comment_start = false;
        index += 1;
    }

    compound_depth > 0
}

fn classify_top_level_word(
    word: &str,
    seen_command_word: &mut bool,
    declaration_context: &mut bool,
) {
    if !*seen_command_word && DECLARATION_COMMAND_WORDS.contains(&word) {
        *declaration_context = true;
        return;
    }
    if assignment_prefix_word(word) {
        // Assignment words keep the command inside its prefix region.
        return;
    }
    if *declaration_context && (word.starts_with('-') || word.starts_with('+')) {
        // Declaration builtin flags (`declare -a`).
        return;
    }
    *seen_command_word = true;
}

/// Skip a balanced `(` ... `)` unit starting at `open` (the index of the `(`),
/// returning the index just past the matching `)`. Returns `None` if unbalanced.
fn skip_parenthesized_unit(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    let mut single = false;
    let mut double = false;
    // A case pattern list owns its closing `)` (parse.y `case_item`), so the
    // pattern `)` must not balance the command-substitution parenthesis. Track
    // case depth the same way `has_unclosed_command_substitution` does; without
    // it `$(case a in a) echo x\nesac)` looks balanced at the pattern `)` and
    // the logical line is finalized before `esac)`.
    let mut case_depth = 0usize;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;
    while index < chars.len() {
        let ch = chars[index];
        if single {
            if ch == '\'' {
                single = false;
            }
            index += 1;
            continue;
        }
        if double {
            // GNU skip_double_quoted (parse.y): inside double quotes a
            // backslash escapes the following character, so `\"` does not
            // close the quote and neither member of the pair can balance a
            // parenthesis. Consume both characters; a backslash before a
            // non-special char is harmless to skip since neither byte is
            // significant to this balancer.
            if ch == '\\' {
                index += 2;
                continue;
            }
            if ch == '"' {
                double = false;
            }
            index += 1;
            continue;
        }
        // Skip a here-document body so its ) and quote bytes stay opaque to
        // parenthesis balancing, mirroring how GNU make_here_document reads
        // the body from the input stream (parse.y gather_here_documents)
        // instead of feeding it back to the token scanner. Matches the heredoc
        // skip already present in has_unclosed_command_substitution below.
        if !single
            && !double
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) != Some(&'<')
        {
            let (next, closes) = skip_heredoc_in_chars_with_closure(chars, index);
            if closes {
                return Some(next);
            }
            index = next;
            continue;
        }
        // Quoted text is a literal word and cannot begin a reserved word.
        if !single && !double {
            update_command_substitution_case_depth(
                chars,
                index,
                ch,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
            );
        }
        match ch {
            '\'' => single = true,
            '"' => double = true,
            '(' if case_depth == 0 => depth += 1,
            ')' if case_depth == 0 => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Skip a backtick command substitution starting at `open` (the index of the
/// opening backtick), returning the index just past the closing backtick.
/// Returns `None` if unbalanced.
fn skip_backtick_unit(chars: &[char], open: usize) -> Option<usize> {
    let mut index = open + 1;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
            continue;
        }
        if chars[index] == '`' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

pub(crate) fn has_unclosed_command_substitution(input: &str) -> bool {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut backtick = false;
    let mut single = false;
    let mut double = false;
    let mut ansi_single = false;
    let mut escaped = false;
    let mut comment_start = true;
    let mut in_comment = false;
    let mut case_depth = 0usize;
    let mut parameter_depth = 0usize;
    let mut parameter_single = false;
    let mut word = String::new();
    let mut word_boundary = true;
    let mut current_word_boundary = true;

    while index < chars.len() {
        let ch = chars[index];
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                comment_start = true;
            }
            index += 1;
            continue;
        }
        if escaped {
            escaped = false;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\n' && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
            index += 1;
            continue;
        }
        if ch == '#'
            && !single
            && !double
            && !ansi_single
            && !backtick
            && depth == 0
            && comment_start
        {
            in_comment = true;
            index += 1;
            continue;
        }
        if ch.is_whitespace() && !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = true;
            index += 1;
            continue;
        }
        if ansi_single {
            if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                ansi_single = false;
            }
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$' && !single && !double && chars.get(index + 1) == Some(&'\'') {
            ansi_single = true;
            comment_start = false;
            index += 2;
            continue;
        }
        if depth > 0 && ch == '`' && !single {
            index = skip_backtick_substitution(&chars, index);
            comment_start = false;
            continue;
        }
        if ch == '\'' && parameter_depth > 0 && !ansi_single {
            parameter_single = !parameter_single;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            comment_start = false;
            index += 1;
            continue;
        }
        if single {
            index += 1;
            continue;
        }
        if ch == '`' && depth == 0 {
            backtick = !backtick;
            comment_start = false;
            index += 1;
            continue;
        }
        if ch == '$'
            && chars.get(index + 1) == Some(&'{')
            && !single
            && !ansi_single
            && !parameter_single
        {
            parameter_depth += 1;
            index += 2;
            continue;
        }
        if ch == '}' && parameter_depth > 0 {
            parameter_depth = parameter_depth.saturating_sub(1);
            parameter_single = false;
            index += 1;
            continue;
        }
        // A balanced command substitution owns its nested quote state even
        // inside an outer double-quoted word. Keep it atomic here as well as
        // in has_unclosed_quotes; an unbalanced unit falls through so this
        // checker still reports the missing closing delimiter.
        if ch == '$' && !single && chars.get(index + 1) == Some(&'(') && !parameter_single {
            if let Some(end) = skip_parenthesized_unit(&chars, index + 1) {
                index = end;
                comment_start = false;
                continue;
            }
            if chars.get(index + 2) == Some(&'(') {
                if let Some(end) = skip_arithmetic_substitution(&chars, index + 3) {
                    index = end;
                    comment_start = false;
                    continue;
                }
                // POSIX permits command substitution when the text after
                // "$((" is not a valid arithmetic expression.
            }
            depth += 1;
            if depth == 1 {
                case_depth = 0;
                word.clear();
                word_boundary = true;
                current_word_boundary = true;
            }
            comment_start = false;
            index += 2;
            continue;
        }
        if depth > 0
            && ch == '#'
            && !single
            && !double
            && !ansi_single
            && !backtick
            && word_boundary
        {
            while index + 1 < chars.len() && chars[index + 1] != '\n' {
                index += 1;
            }
            word.clear();
            word_boundary = true;
            current_word_boundary = true;
            index += 1;
            continue;
        }
        if depth > 0 && !ansi_single && !backtick {
            update_command_substitution_case_depth(
                &chars,
                index,
                ch,
                &mut word,
                &mut case_depth,
                &mut word_boundary,
                &mut current_word_boundary,
            );
        }
        if depth > 0
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if depth > 0 && ch == '<' && chars.get(index + 1) == Some(&'<') {
            let (next, closes) = skip_heredoc_in_chars_with_closure(&chars, index);
            if closes {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return false;
                }
            }
            index = next;
            continue;
        }
        if backtick
            && ch == '<'
            && chars.get(index + 1) == Some(&'<')
            && chars.get(index + 2) == Some(&'<')
        {
            index += 3;
            continue;
        }
        if backtick && ch == '<' && chars.get(index + 1) == Some(&'<') {
            index = skip_heredoc_in_chars_with_closure(&chars, index).0;
            continue;
        }
        if depth > 0 && case_depth == 0 && !ansi_single && ch == '(' {
            depth += 1;
        } else if depth > 0 && case_depth == 0 && !ansi_single && ch == ')' {
            depth -= 1;
        }
        if !single && !double && !ansi_single && !backtick && depth == 0 {
            comment_start = false;
        }
        index += 1;
    }

    depth > 0 || backtick || ansi_single
}

fn skip_backtick_substitution(chars: &[char], mut index: usize) -> usize {
    index += 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '`' {
            return index + 1;
        }
        index += 1;
    }
    index
}

/// Skip a `$((...))` expansion while checking its own parenthesis and quote
/// state. Arithmetic `#` is an operator-context character, not a shell
/// comment introducer.
fn skip_arithmetic_substitution(chars: &[char], mut index: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut single = false;
    let mut double = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
            index += 1;
            continue;
        }
        if ch == '"' && !single {
            double = !double;
            index += 1;
            continue;
        }
        if !single && !double {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' && depth > 0 {
                depth -= 1;
            } else if ch == ')' && chars.get(index + 1) == Some(&')') {
                return Some(index + 2);
            }
        }
        index += 1;
    }
    None
}

fn update_command_substitution_case_depth(
    chars: &[char],
    index: usize,
    ch: char,
    word: &mut String,
    case_depth: &mut usize,
    word_boundary: &mut bool,
    current_word_boundary: &mut bool,
) {
    if ch == '_' || ch.is_ascii_alphanumeric() {
        if word.is_empty() {
            *current_word_boundary = *word_boundary;
        }
        word.push(ch);
        return;
    }

    if word.is_empty() {
        if command_substitution_separator_allows_reserved_word(ch) {
            *word_boundary = true;
        } else if !ch.is_whitespace() {
            *word_boundary = false;
        }
        return;
    }

    let reserved_word_allows_next = match word.as_str() {
        "case" if *current_word_boundary => {
            *case_depth += 1;
            false
        }
        "esac" if *current_word_boundary && !case_pattern_starts_with_esac_chars(chars, index) => {
            *case_depth = case_depth.saturating_sub(1);
            true
        }
        "for" | "select" | "while" | "until" | "then" | "do" | "else" | "elif" | "in" | "fi"
        | "done"
            if *current_word_boundary =>
        {
            true
        }
        _ => false,
    };
    word.clear();
    *word_boundary =
        reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
}

fn command_substitution_separator_allows_reserved_word(ch: char) -> bool {
    matches!(ch, ';' | '&' | '|' | '(' | ')' | '\n')
}

fn case_pattern_starts_with_esac_chars(chars: &[char], delimiter_index: usize) -> bool {
    if !matches!(chars.get(delimiter_index), Some(')' | '|')) {
        return false;
    }

    let mut close = delimiter_index;
    while close < chars.len() {
        match chars[close] {
            ')' => break,
            ';' | '\n' => return false,
            _ => close += 1,
        }
    }
    if chars.get(close) != Some(&')') {
        return false;
    }

    let mut scan = close + 1;
    let mut word = String::new();
    let mut word_boundary = true;
    while scan < chars.len() {
        let ch = chars[scan];
        if ch == ';' && chars.get(scan + 1) == Some(&';') {
            return true;
        }
        if ch == '_' || ch.is_ascii_alphanumeric() {
            word.push(ch);
            scan += 1;
            continue;
        }
        if word == "esac" && word_boundary {
            return true;
        }
        if ch == ')' {
            return false;
        }
        if word.is_empty() {
            if command_substitution_separator_allows_reserved_word(ch) {
                word_boundary = true;
            } else if !ch.is_whitespace() {
                word_boundary = false;
            }
            scan += 1;
            continue;
        }
        let reserved_word_allows_next =
            word_boundary && command_substitution_reserved_word_allows_next(&word);
        word.clear();
        word_boundary =
            reserved_word_allows_next || command_substitution_separator_allows_reserved_word(ch);
        scan += 1;
    }

    word == "esac" && word_boundary
}

fn command_substitution_reserved_word_allows_next(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "select"
            | "while"
            | "until"
            | "then"
            | "do"
            | "else"
            | "elif"
            | "in"
            | "fi"
            | "done"
            | "esac"
    )
}

#[cfg(test)]
mod tests {
    use super::has_unclosed_quotes;

    #[test]
    fn command_substitution_quotes_do_not_leak_from_outer_double_quote() {
        let input = r#"echo \"$(echo \"\${IFS+'}'z}\")\""#;
        assert!(!has_unclosed_quotes(input));
    }
}
