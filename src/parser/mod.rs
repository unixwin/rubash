//! Parser Module - Bash Parser
//!
//! Transforms tokens into an AST.

use crate::lexer::{Token, TokenKind};

/// Represents a redirect specification
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub fd: Option<u32>,
    pub target: String,
    pub append: bool,
    pub clobber: bool,
}

/// Represents a here-document redirection.
#[derive(Debug, Clone, PartialEq)]
pub struct HereDocRedirect {
    pub fd: Option<u32>,
    pub delimiter: String,
    pub body: Option<String>,
}

/// Represents a narrow `for` compound command.
#[derive(Debug, Clone)]
pub struct ForCommand {
    pub variable: String,
    pub words: Vec<String>,
    pub default_positional: bool,
    pub arithmetic: Option<ArithmeticForCommand>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `for (( init; test; update ))` compound command.
#[derive(Debug, Clone)]
pub struct ArithmeticForCommand {
    pub init: String,
    pub test: String,
    pub update: String,
}

/// Represents a narrow `case` compound command.
#[derive(Debug, Clone)]
pub struct CaseCommand {
    pub word: String,
    pub clauses: Vec<CaseClause>,
}

#[derive(Debug, Clone)]
pub struct CaseClause {
    pub patterns: Vec<String>,
    pub body: Vec<CommandNode>,
    pub terminator: CaseTerminator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseTerminator {
    Break,
    FallThrough,
    TestNext,
}

/// Represents a narrow `name() { ...; }` shell function definition.
#[derive(Debug, Clone)]
pub struct FunctionCommand {
    pub name: String,
    pub body: Vec<CommandNode>,
}

/// Represents a parsed command
#[derive(Debug, Clone)]
pub struct CommandNode {
    /// The command words (first is the command name)
    pub words: Vec<String>,
    /// Lexer kind for each command word, used for quote-sensitive expansion
    /// decisions while the parser still stores words as strings.
    pub word_kinds: Vec<TokenKind>,
    /// Variable assignments
    pub assignments: std::collections::HashMap<String, String>,
    /// Input redirect
    pub redirect_in: Option<Redirect>,
    /// Output redirect
    pub redirect_out: Option<Redirect>,
    /// Append redirect
    pub append: Option<Redirect>,
    /// Stderr redirect
    pub redirect_err: Option<Redirect>,
    /// Stderr append redirect
    pub redirect_err_append: Option<Redirect>,
    /// Here-document stdin body
    pub heredoc: Option<String>,
    /// Here-document delimiter word, used when reprinting functions.
    pub heredoc_delimiter: Option<String>,
    /// All here-document redirections in parse order.
    pub heredoc_redirects: Vec<HereDocRedirect>,
    /// Here-string stdin word
    pub here_string: Option<String>,
    /// Pipe to next command
    pub pipe: Option<usize>,
    /// Background execution (&)
    pub background: bool,
    /// Connector to the next command: Some(true) for &&, Some(false) for ||.
    pub and_or: Option<bool>,
    /// Return status is inverted by the reserved word `!`.
    pub inverted: bool,
    /// Command is executed inside a subshell grouping `( ... )`.
    pub subshell: bool,
    /// This command closes the current subshell grouping.
    pub subshell_end: bool,
    /// `for name in words; do ...; done`
    pub for_command: Option<ForCommand>,
    /// `case word in pattern) ... ;; esac`
    pub case_command: Option<CaseCommand>,
    /// `name() { compound_list; }`
    pub function_command: Option<FunctionCommand>,
    /// Script line number where this command starts, when known.
    pub line: Option<usize>,
}

impl CommandNode {
    pub fn new() -> Self {
        Self {
            words: Vec::new(),
            word_kinds: Vec::new(),
            assignments: std::collections::HashMap::new(),
            redirect_in: None,
            redirect_out: None,
            append: None,
            redirect_err: None,
            redirect_err_append: None,
            heredoc: None,
            heredoc_delimiter: None,
            heredoc_redirects: Vec::new(),
            here_string: None,
            pipe: None,
            background: false,
            and_or: None,
            inverted: false,
            subshell: false,
            subshell_end: false,
            for_command: None,
            case_command: None,
            function_command: None,
            line: None,
        }
    }

    /// Returns Some(true) for &&, Some(false) for ||, None otherwise
    pub fn and_or(&self) -> Option<bool> {
        self.and_or
    }
}

impl Default for CommandNode {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents a parsed AST
#[derive(Debug, Clone)]
pub struct Ast {
    /// List of commands
    pub commands: Vec<CommandNode>,
}

/// Parse tokens into an AST
pub fn parse(tokens: &[Token]) -> Ast {
    let mut ast = Ast {
        commands: Vec::new(),
    };
    let mut current_cmd = CommandNode::new();
    let mut in_subshell = false;

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token.kind == TokenKind::Keyword
            && token.value == "for"
            && command_is_empty(&current_cmd)
        {
            if let Some((for_cmd, next_i)) = parse_for_command(tokens, i) {
                ast.commands.push(for_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if ((token.kind == TokenKind::Word)
            || (token.kind == TokenKind::Keyword && token.value == "function"))
            && command_is_empty(&current_cmd)
        {
            if let Some((function_cmd, next_i)) = parse_function_command(tokens, i) {
                ast.commands.push(function_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if token.kind == TokenKind::Keyword
            && token.value == "case"
            && command_is_empty(&current_cmd)
        {
            if let Some((case_cmd, next_i)) = parse_case_command(tokens, i) {
                ast.commands.push(case_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if command_is_empty(&current_cmd)
            && ((token.kind == TokenKind::Keyword && token.value == "(")
                || token.value.starts_with("(("))
        {
            if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
                ast.commands.push(arith_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if command_accepts_embedded_arithmetic_command(&current_cmd)
            && ((token.kind == TokenKind::Keyword && token.value == "(")
                || token.value.starts_with("(("))
        {
            if let Some((arith_cmd, next_i)) = parse_arithmetic_command(tokens, i) {
                note_command_line(&mut current_cmd, token);
                current_cmd.words.extend(arith_cmd.words);
                current_cmd.and_or = arith_cmd.and_or;
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        match token.kind {
            TokenKind::Word | TokenKind::Variable | TokenKind::CommandSubst => {
                current_cmd.subshell |= in_subshell;
                note_command_line(&mut current_cmd, token);
                push_command_word(&mut current_cmd, token);
            }
            TokenKind::Assignment => {
                current_cmd.subshell |= in_subshell;
                note_command_line(&mut current_cmd, token);
                if let Some(pos) = token.value.find('=') {
                    if current_cmd.words.is_empty() {
                        let var_name = token.value[..pos].to_string();
                        let mut var_value = token.value[pos + 1..].to_string();
                        if var_value.is_empty() {
                            if let Some((compound_value, next_i)) =
                                collect_compound_assignment(tokens, i)
                            {
                                var_value = format!("\x1e{compound_value}");
                                i = next_i;
                            }
                        }
                        current_cmd.assignments.insert(var_name, var_value);
                    } else {
                        let mut word = token.value.clone();
                        if word.ends_with('=') {
                            if let Some((compound_value, next_i)) =
                                collect_compound_assignment(tokens, i)
                            {
                                word.push('\x1e');
                                word.push_str(&compound_value);
                                i = next_i;
                            }
                        }
                        current_cmd.words.push(word);
                        current_cmd.word_kinds.push(TokenKind::Word);
                    }
                }
            }
            TokenKind::Pipe => {
                if command_is_open_conditional(&current_cmd) {
                    push_command_word(&mut current_cmd, token);
                } else {
                    // Save current command with pipe flag
                    current_cmd.subshell |= in_subshell;
                    current_cmd.pipe = Some(1);
                    ast.commands.push(current_cmd);
                    current_cmd = CommandNode::new();
                }
            }
            TokenKind::Semicolon => {
                // Command separator
                current_cmd.subshell |= in_subshell;
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::RedirectIn => {
                if command_is_open_conditional(&current_cmd) {
                    push_command_word(&mut current_cmd, token);
                } else {
                    note_command_line(&mut current_cmd, token);
                    if i + 1 < tokens.len()
                        && matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable)
                    {
                        current_cmd.redirect_in = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                            clobber: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectOut => {
                if command_is_open_conditional(&current_cmd) {
                    push_command_word(&mut current_cmd, token);
                } else {
                    note_command_line(&mut current_cmd, token);
                    if i + 1 < tokens.len()
                        && matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable)
                    {
                        current_cmd.redirect_out = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                            clobber: token.value == ">|",
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::Append => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len()
                    && matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable)
                {
                    current_cmd.append = Some(Redirect {
                        fd: None,
                        target: tokens[i + 1].value.clone(),
                        append: true,
                        clobber: false,
                    });
                    i += 1;
                }
            }
            TokenKind::RedirectErr => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len()
                    && matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable)
                {
                    current_cmd.redirect_err = Some(Redirect {
                        fd: Some(2),
                        target: tokens[i + 1].value.clone(),
                        append: false,
                        clobber: token.value == "2>|",
                    });
                    i += 1;
                }
            }
            TokenKind::RedirectErrAppend => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len()
                    && matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable)
                {
                    current_cmd.redirect_err_append = Some(Redirect {
                        fd: Some(2),
                        target: tokens[i + 1].value.clone(),
                        append: true,
                        clobber: false,
                    });
                    i += 1;
                }
            }
            TokenKind::HereDoc => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    let fd = take_heredoc_fd_prefix(&mut current_cmd);
                    let delimiter = tokens[i + 1].value.clone();
                    current_cmd.heredoc_redirects.push(HereDocRedirect {
                        fd,
                        delimiter: delimiter.clone(),
                        body: None,
                    });
                    if fd.is_none() {
                        current_cmd.heredoc_delimiter = Some(delimiter);
                    }
                    i += 1;
                }
            }
            TokenKind::HereString => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len()
                    && matches!(
                        tokens[i + 1].kind,
                        TokenKind::Word
                            | TokenKind::Variable
                            | TokenKind::CommandSubst
                            | TokenKind::Assignment
                    )
                {
                    current_cmd.here_string = Some(tokens[i + 1].value.clone());
                    i += 1;
                }
            }
            TokenKind::HereDocBody => {
                note_command_line(&mut current_cmd, token);
                assign_heredoc_body(&mut current_cmd, &mut ast, token.value.clone());
            }
            TokenKind::And | TokenKind::Or => {
                if command_is_open_conditional(&current_cmd) {
                    push_command_word(&mut current_cmd, token);
                } else {
                    // TODO(parse.y/execute_cmd.c): This preserves the AND-OR
                    // list connector on simple commands. Full Bash grammar needs
                    // a list AST with compound commands and proper precedence.
                    current_cmd.subshell |= in_subshell;
                    current_cmd.and_or = Some(token.kind == TokenKind::And);
                    ast.commands.push(current_cmd);
                    current_cmd = CommandNode::new();
                }
            }
            TokenKind::Background => {
                // TODO(parse.y/jobs.c): Bash starts the preceding pipeline
                // asynchronously and returns immediately. Until job control is
                // represented, keep `&` as a command terminator so redirections
                // apply to the command instead of treating `&` as an argument.
                current_cmd.subshell |= in_subshell;
                current_cmd.background = true;
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::Keyword => {
                if command_is_open_conditional(&current_cmd)
                    && matches!(token.value.as_str(), "(" | ")")
                {
                    push_command_word(&mut current_cmd, token);
                    i += 1;
                    continue;
                }

                if token.value == "!" && command_is_empty(&current_cmd) {
                    // TODO(parse.y/execute_cmd.c): Bash represents `!` as a
                    // pipeline/list inversion flag. Keep it on the next simple
                    // command until the parser has a real pipeline AST.
                    current_cmd.inverted = !current_cmd.inverted;
                    note_command_line(&mut current_cmd, token);
                    i += 1;
                    continue;
                }

                if token.value == "(" && command_is_empty(&current_cmd) {
                    in_subshell = true;
                    i += 1;
                    continue;
                }

                if token.value == ")" && in_subshell {
                    if command_is_empty(&current_cmd) {
                        if let Some(command) = ast.commands.last_mut() {
                            command.subshell_end = true;
                        }
                    } else {
                        current_cmd.subshell = true;
                        current_cmd.subshell_end = true;
                    }
                    in_subshell = false;
                    i += 1;
                    continue;
                }

                // TODO(parse.y): Reserved words are only reserved in specific
                // parser states. If an ordinary command has already started,
                // keep the token text so alias expansion can reparse it later.
                if !matches!(token.value.as_str(), "(" | ")" | "{" | "}") {
                    note_command_line(&mut current_cmd, token);
                    push_command_word(&mut current_cmd, token);
                }
            }
            TokenKind::Eof => {
                break;
            }
            _ => {
                // Skip other token types (keywords, variables, etc.)
            }
        }

        i += 1;
    }

    // Don't forget the last command
    if !command_is_empty(&current_cmd) {
        ast.commands.push(current_cmd);
    }

    ast
}

fn parse_for_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports all `for_command`
    // grammar alternatives, nested compound lists, redirections on compound
    // commands and reserved-word parsing state. This maps common
    // `for name [in words]; do body; done` forms.
    if let Some((command, next_i)) = parse_arithmetic_for_command(tokens, start) {
        return Some((command, next_i));
    }

    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    let mut words = Vec::new();
    let default_positional = if is_keyword(tokens, i, "in") {
        i += 1;
        while i < tokens.len() && !is_keyword(tokens, i, "do") {
            if tokens[i].kind == TokenKind::Semicolon {
                i += 1;
                continue;
            }
            if matches!(
                tokens[i].kind,
                TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
            ) {
                words.push(tokens[i].value.clone());
            }
            i += 1;
        }
        false
    } else {
        while tokens
            .get(i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            i += 1;
        }
        true
    };

    if !is_keyword(tokens, i, "do") {
        return None;
    }
    i += 1;

    let body_start = i;
    let mut depth = 0usize;
    while i < tokens.len() {
        if is_keyword(tokens, i, "for") {
            depth += 1;
        } else if is_keyword(tokens, i, "done") {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        i += 1;
    }

    if !is_keyword(tokens, i, "done") {
        return None;
    }

    let body = parse(&tokens[body_start..i])
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect();
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        variable,
        words,
        default_positional,
        arithmetic: None,
        body,
    });
    Some((command, i + 1))
}

fn parse_arithmetic_for_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    let mut i = if tokens.get(start + 1)?.value == "((" {
        start + 2
    } else if is_keyword(tokens, start + 1, "(") && is_keyword(tokens, start + 2, "(") {
        start + 3
    } else {
        return None;
    };

    let mut parts = vec![Vec::new(), Vec::new(), Vec::new()];
    let mut part_index = 0usize;
    let mut paren_depth = 0usize;
    while i + 1 < tokens.len() {
        if paren_depth == 0 && tokens[i].value == "))" {
            i += 1;
            break;
        }

        if paren_depth == 0 && is_keyword(tokens, i, ")") && is_keyword(tokens, i + 1, ")") {
            i += 2;
            break;
        }

        if paren_depth == 0 && tokens[i].kind == TokenKind::Semicolon {
            part_index += 1;
            if part_index > 2 {
                return None;
            }
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, "(") {
            paren_depth += 1;
            parts[part_index].push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            parts[part_index].push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            parts[part_index].push(combined);
            i += 2;
            continue;
        }

        parts[part_index].push(tokens[i].value.clone());
        i += 1;
    }

    if part_index != 2 {
        return None;
    }

    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
    }

    if !is_keyword(tokens, i, "do") {
        return None;
    }
    i += 1;

    let body_start = i;
    let mut depth = 0usize;
    while i < tokens.len() {
        if is_keyword(tokens, i, "for") {
            depth += 1;
        } else if is_keyword(tokens, i, "done") {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        i += 1;
    }

    if !is_keyword(tokens, i, "done") {
        return None;
    }

    let body = parse(&tokens[body_start..i])
        .commands
        .into_iter()
        .filter(|command| !command_is_empty(command))
        .collect();
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        variable: String::new(),
        words: Vec::new(),
        default_positional: false,
        arithmetic: Some(ArithmeticForCommand {
            init: parts[0].join(" "),
            test: parts[1].join(" "),
            update: parts[2].join(" "),
        }),
        body,
    });
    Some((command, i + 1))
}

fn parse_function_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): Bash has full function_def grammar,
    // including `function name`, redirections, nested compound commands, and
    // parser-state-sensitive reserved words. This maps the upstream builtins
    // `name() { ...; }` and `function name { ...; }` forms onto a function
    // command node.
    let keyword_form = is_keyword(tokens, start, "function");
    let (name_index, mut i) = if keyword_form {
        (start + 1, start + 2)
    } else {
        (start, start + 1)
    };
    let name = tokens.get(name_index)?.value.clone();
    if !(is_function_name(&name) || (keyword_form && is_function_keyword_name(&name))) {
        return None;
    }

    if tokens.get(i).is_some_and(|token| token.value == "(") {
        if tokens.get(i + 1)?.value != ")" {
            return None;
        }
        i += 2;
    } else if !keyword_form {
        return None;
    }

    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
    }
    if let Some(group) = tokens
        .get(i)
        .map(|token| token.value.as_str())
        .filter(|value| value.starts_with('{') && value.ends_with('}'))
    {
        // TODO(parse.y): The lexer can currently preserve a full brace group
        // as one token. Recognize it as a function body for `name() { ...; }`
        // until the parser owns brace groups structurally.
        let inner = group.trim_start_matches('{').trim_end_matches('}').trim();
        let body_tokens = crate::lexer::tokenize(inner);
        let mut body = parse(&body_tokens).commands;
        if let Some(line) = tokens.get(start).map(|token| token.position) {
            set_body_line(&mut body, line);
        }
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.function_command = Some(FunctionCommand { name, body });
        let mut next_i = i + 1;
        collect_trailing_redirections(tokens, &mut next_i, &mut command);
        while tokens
            .get(next_i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            next_i += 1;
        }
        return Some((command, next_i));
    }
    if tokens.get(i)?.value != "{" {
        return None;
    }
    i += 1;
    while tokens
        .get(i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        i += 1;
    }

    let body_start = i;
    let mut depth = 1usize;
    while i < tokens.len() {
        if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "{" {
            depth += 1;
        } else if tokens[i].kind == TokenKind::Keyword && tokens[i].value == "}" {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        i += 1;
    }
    if i >= tokens.len() {
        return None;
    }

    let body = parse(&tokens[body_start..i]).commands;
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.function_command = Some(FunctionCommand { name, body });
    let mut next_i = i + 1;
    collect_trailing_redirections(tokens, &mut next_i, &mut command);
    while tokens
        .get(next_i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }
    Some((command, next_i))
}

fn collect_trailing_redirections(tokens: &[Token], index: &mut usize, command: &mut CommandNode) {
    loop {
        let Some(token) = tokens.get(*index) else {
            break;
        };
        let Some(target) = tokens.get(*index + 1).filter(|next| {
            matches!(
                next.kind,
                TokenKind::Word | TokenKind::Variable | TokenKind::CommandSubst
            )
        }) else {
            break;
        };

        match token.kind {
            TokenKind::RedirectIn => {
                command.redirect_in = Some(Redirect {
                    fd: None,
                    target: target.value.clone(),
                    append: false,
                    clobber: false,
                });
            }
            TokenKind::RedirectOut => {
                command.redirect_out = Some(Redirect {
                    fd: None,
                    target: target.value.clone(),
                    append: false,
                    clobber: token.value == ">|",
                });
            }
            TokenKind::Append => {
                command.append = Some(Redirect {
                    fd: None,
                    target: target.value.clone(),
                    append: true,
                    clobber: false,
                });
            }
            TokenKind::RedirectErr => {
                command.redirect_err = Some(Redirect {
                    fd: Some(2),
                    target: target.value.clone(),
                    append: false,
                    clobber: token.value == "2>|",
                });
            }
            TokenKind::RedirectErrAppend => {
                command.redirect_err_append = Some(Redirect {
                    fd: Some(2),
                    target: target.value.clone(),
                    append: true,
                    clobber: false,
                });
            }
            _ => break,
        }

        *index += 2;
    }
}

fn set_body_line(body: &mut [CommandNode], line: usize) {
    // TODO(parse.y): Bash preserves source locations through compound command
    // parsing. Rubash reparses inline function bodies from text today, so
    // recover the definition line for diagnostics such as readonly errors.
    for command in body {
        command.line = Some(line);
    }
}

fn collect_compound_assignment(tokens: &[Token], start: usize) -> Option<(String, usize)> {
    // TODO(parse.y/arrayfunc.c): Bash parses `name=(...)` as a compound array
    // assignment WORD and later expands it with `assign_array_var_from_string`.
    // This preserves the simple parenthesized value shape used by alias.tests.
    if !is_keyword(tokens, start + 1, "(") {
        return None;
    }

    let mut i = start + 2;
    let mut values = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, ")") {
        if let Some((left, rhs)) = compound_subscript_assignment(&tokens[i].value) {
            values.push(format!("{}{}", left, quote_compound_assignment_word(rhs)));
            i += 1;
            continue;
        }

        if tokens[i].value == "[" || tokens[i].value.starts_with('[') {
            let mut subscript = String::new();
            let mut j = i;
            if tokens[j].value == "[" {
                j += 1;
                while j < tokens.len() && tokens[j].value != "]" {
                    subscript.push_str(&tokens[j].value);
                    j += 1;
                }
                if j >= tokens.len() || tokens[j].value != "]" {
                    return None;
                }
                j += 1;
            } else if let Some(inner) = tokens[j]
                .value
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
            {
                subscript.push_str(inner);
                j += 1;
            }

            if j + 1 < tokens.len() && matches!(tokens[j].value.as_str(), "=" | "+=") {
                values.push(format!(
                    "[{}]{}{}",
                    quote_compound_assignment_word(&subscript),
                    tokens[j].value,
                    quote_compound_assignment_word(&tokens[j + 1].value)
                ));
                i = j + 2;
                continue;
            }
        }

        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
        ) {
            values.push(quote_compound_assignment_word(&tokens[i].value));
        }
        i += 1;
    }

    if !is_keyword(tokens, i, ")") {
        return None;
    }

    Some((format!("({})", values.join(" ")), i))
}

fn compound_subscript_assignment(value: &str) -> Option<(String, &str)> {
    if !value.starts_with('[') {
        return None;
    }

    for operator in ["]+=", "]="] {
        let Some(pos) = value.find(operator) else {
            continue;
        };
        let split = pos + operator.len();
        let subscript = &value[1..pos];
        let assignment = if operator == "]=" { "=" } else { "+=" };
        return Some((
            format!(
                "[{}]{}",
                quote_compound_assignment_word(subscript),
                assignment
            ),
            &value[split..],
        ));
    }

    None
}

fn quote_compound_assignment_word(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_ascii_whitespace() || matches!(ch, '"' | '\\'))
    {
        return value.to_string();
    }

    let mut quoted = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(ch);
    }
    quoted.push('"');
    quoted
}

fn parse_arithmetic_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    let first = tokens.get(start)?.value.as_str();

    if let Some(inner) = first
        .strip_prefix("((")
        .and_then(|value| value.strip_suffix("))"))
    {
        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.words.push("((".to_string());
        command.words.push(inner.to_string());
        command.words.push("))".to_string());
        return Some(finish_arithmetic_command(command, tokens, start + 1));
    }

    let mut i;
    let mut parts = Vec::new();
    let mut paren_depth = 0usize;
    if first == "((" {
        i = start + 1;
    } else if is_keyword(tokens, start, "(") && is_keyword(tokens, start + 1, "(") {
        i = start + 2;
    } else {
        return None;
    }

    while i + 1 < tokens.len() {
        if paren_depth == 0 && tokens[i].value == "))" {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            command.words.push("((".to_string());
            command.words.push(parts.join(" "));
            command.words.push("))".to_string());
            return Some(finish_arithmetic_command(command, tokens, i + 1));
        }

        if paren_depth == 0 && is_keyword(tokens, i, ")") && is_keyword(tokens, i + 1, ")") {
            let mut command = CommandNode::new();
            command.line = tokens.get(start).map(|token| token.position);
            command.words.push("((".to_string());
            command.words.push(parts.join(" "));
            command.words.push("))".to_string());
            return Some(finish_arithmetic_command(command, tokens, i + 2));
        }

        if is_keyword(tokens, i, "(") {
            paren_depth += 1;
            parts.push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if is_keyword(tokens, i, ")") && paren_depth > 0 {
            paren_depth -= 1;
            parts.push(tokens[i].value.clone());
            i += 1;
            continue;
        }

        if let Some(combined) = arithmetic_combined_operator(&tokens[i], tokens.get(i + 1)) {
            parts.push(combined);
            i += 2;
            continue;
        }

        if tokens[i].kind == TokenKind::Semicolon {
            i += 1;
            continue;
        }

        parts.push(tokens[i].value.clone());
        i += 1;
    }

    None
}

fn finish_arithmetic_command(
    mut command: CommandNode,
    tokens: &[Token],
    index: usize,
) -> (CommandNode, usize) {
    match tokens.get(index).map(|token| &token.kind) {
        Some(TokenKind::And) => {
            command.and_or = Some(true);
            (command, index + 1)
        }
        Some(TokenKind::Or) => {
            command.and_or = Some(false);
            (command, index + 1)
        }
        Some(TokenKind::Semicolon) => (command, index + 1),
        _ => (command, index),
    }
}

fn arithmetic_combined_operator(token: &Token, next: Option<&Token>) -> Option<String> {
    let op = token.value.as_str();
    if !matches!(op, ">" | "<" | "!" | "&" | "|" | "<<" | ">>") {
        return None;
    }

    let next = next?;
    if next.value == "=" {
        return Some(format!("{op}="));
    }

    next.value
        .strip_prefix('=')
        .map(|rhs| format!("{op}={rhs}"))
}

fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports extglob patterns, nested
    // compound lists, and redirections on the compound command. This covers the
    // common `case word in pattern) list terminator` shape.
    let word = tokens.get(start + 1)?.value.clone();
    let mut i = start + 2;
    while i < tokens.len() && !is_keyword(tokens, i, "in") {
        i += 1;
    }
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    i += 1;

    let mut clauses = Vec::new();
    while i < tokens.len() && !is_keyword(tokens, i, "esac") {
        while i < tokens.len() && tokens[i].kind == TokenKind::Semicolon {
            i += 1;
        }
        if is_keyword(tokens, i, "esac") {
            break;
        }

        let mut patterns = Vec::new();
        while i < tokens.len() && !is_keyword(tokens, i, ")") {
            if matches!(
                tokens[i].kind,
                TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
            ) {
                patterns.push(tokens[i].value.clone());
            }
            i += 1;
        }
        if !is_keyword(tokens, i, ")") {
            return None;
        }
        i += 1;

        let body_start = i;
        i = case_body_end(tokens, i);
        let body = parse(&tokens[body_start..i]).commands;
        let terminator = case_terminator(tokens, i).unwrap_or(CaseTerminator::Break);
        clauses.push(CaseClause {
            patterns,
            body,
            terminator,
        });

        if is_case_terminator(tokens, i) {
            i += 1;
        }
    }

    if !is_keyword(tokens, i, "esac") {
        return None;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.case_command = Some(CaseCommand { word, clauses });
    Some((command, i + 1))
}

fn case_body_end(tokens: &[Token], mut index: usize) -> usize {
    let mut nested_case_depth = 0usize;

    while index < tokens.len() {
        if is_keyword(tokens, index, "case") {
            nested_case_depth += 1;
            index += 1;
            continue;
        }

        if is_keyword(tokens, index, "esac") {
            if nested_case_depth == 0 {
                break;
            }
            nested_case_depth -= 1;
            index += 1;
            continue;
        }

        if nested_case_depth == 0 && is_case_terminator(tokens, index) {
            break;
        }

        index += 1;
    }

    index
}

fn is_case_terminator(tokens: &[Token], index: usize) -> bool {
    case_terminator(tokens, index).is_some()
}

fn case_terminator(tokens: &[Token], index: usize) -> Option<CaseTerminator> {
    let token = tokens.get(index)?;
    if token.kind != TokenKind::Word {
        return None;
    }

    match token.value.as_str() {
        ";;" => Some(CaseTerminator::Break),
        ";&" => Some(CaseTerminator::FallThrough),
        ";;&" => Some(CaseTerminator::TestNext),
        _ => None,
    }
}

fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
}

fn push_command_word(cmd: &mut CommandNode, token: &Token) {
    cmd.words.push(token.value.clone());
    cmd.word_kinds.push(token.kind.clone());
}

fn take_heredoc_fd_prefix(cmd: &mut CommandNode) -> Option<u32> {
    let fd = cmd
        .words
        .last()
        .filter(|word| !word.is_empty() && word.chars().all(|ch| ch.is_ascii_digit()))?
        .parse::<u32>()
        .ok()?;
    cmd.words.pop();
    cmd.word_kinds.pop();
    Some(fd)
}

fn assign_heredoc_body(current_cmd: &mut CommandNode, ast: &mut Ast, body: String) {
    if fill_pending_heredoc_body(current_cmd, &body) {
        return;
    }
    for command in ast.commands.iter_mut().rev() {
        if fill_pending_heredoc_body(command, &body) {
            return;
        }
    }
    current_cmd.heredoc = Some(body);
}

fn fill_pending_heredoc_body(cmd: &mut CommandNode, body: &str) -> bool {
    let Some(redirect) = cmd
        .heredoc_redirects
        .iter_mut()
        .find(|redirect| redirect.body.is_none())
    else {
        return false;
    };

    redirect.body = Some(body.to_string());
    if redirect.fd.is_none() {
        cmd.heredoc = Some(body.to_string());
        cmd.heredoc_delimiter = Some(redirect.delimiter.clone());
    }
    true
}

fn is_keyword(tokens: &[Token], index: usize, value: &str) -> bool {
    tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Keyword && token.value == value)
}

fn command_is_empty(cmd: &CommandNode) -> bool {
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.heredoc_redirects.is_empty()
        && cmd.here_string.is_none()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.for_command.is_none()
        && cmd.case_command.is_none()
        && cmd.function_command.is_none()
}

fn command_is_open_conditional(cmd: &CommandNode) -> bool {
    (cmd.words.first().map(String::as_str) == Some("[[")
        || (matches!(cmd.words.first().map(String::as_str), Some("if" | "elif"))
            && cmd.words.get(1).map(String::as_str) == Some("[[")))
        && !cmd.words.iter().any(|word| word == "]]")
}

fn command_accepts_embedded_arithmetic_command(cmd: &CommandNode) -> bool {
    matches!(
        cmd.words.first().map(String::as_str),
        Some("if" | "elif" | "while" | "until" | "do" | "then" | "else")
    ) && cmd.words.len() == 1
}

fn is_function_name(name: &str) -> bool {
    if name.is_empty() || name.contains('=') {
        return false;
    }

    !name
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

fn is_function_keyword_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '{' | '}' | ';' | '&' | '|'))
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::lexer::tokenize;

    #[test]
    fn test_parse_simple() {
        let tokens = tokenize("ls -la");
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words.len(), 2);
    }

    #[test]
    fn test_parse_pipeline() {
        let tokens = tokenize("ls | grep foo");
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 2);
    }

    #[test]
    fn test_parse_empty() {
        let tokens: Vec<Token> = vec![];
        let ast = parse(&tokens);
        assert_eq!(ast.commands.len(), 0);
    }

    #[test]
    fn test_parse_heredoc_delimiter() {
        let tokens = tokenize("cat <<EOF\nbody\nEOF");
        let ast = parse(&tokens);

        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].heredoc_delimiter.as_deref(), Some("EOF"));
        assert_eq!(ast.commands[0].heredoc.as_deref(), Some("body\n"));
    }

    #[test]
    fn test_parse_multiple_heredoc_redirects_with_fd() {
        let tokens = tokenize("done <<EOF1 3<<EOF2\none\nEOF1\ntwo\nEOF2");
        let ast = parse(&tokens);

        assert_eq!(ast.commands.len(), 1);
        assert_eq!(ast.commands[0].words, vec!["done"]);
        assert_eq!(ast.commands[0].heredoc_redirects.len(), 2);
        assert_eq!(ast.commands[0].heredoc_redirects[0].fd, None);
        assert_eq!(
            ast.commands[0].heredoc_redirects[0].body.as_deref(),
            Some("one\n")
        );
        assert_eq!(ast.commands[0].heredoc_redirects[1].fd, Some(3));
        assert_eq!(
            ast.commands[0].heredoc_redirects[1].body.as_deref(),
            Some("two\n")
        );
    }

    #[test]
    fn test_parse_piped_heredoc_body_belongs_to_left_command() {
        let tokens = tokenize("cat <<EOF | sort -u\nbody\nEOF");
        let ast = parse(&tokens);

        assert_eq!(ast.commands.len(), 2);
        assert_eq!(ast.commands[0].words, vec!["cat"]);
        assert_eq!(ast.commands[0].heredoc.as_deref(), Some("body\n"));
        assert_eq!(ast.commands[1].words, vec!["sort", "-u"]);
        assert!(ast.commands[1].heredoc.is_none());
    }

    #[test]
    fn test_parse_arithmetic_loop_conditions_as_condition_words() {
        let tokens = tokenize(
            "while (( n < 3 )); do (( n++ )); done; until (( n == 5 )); do (( n++ )); done",
        );
        let ast = parse(&tokens);
        let words: Vec<Vec<String>> = ast
            .commands
            .iter()
            .map(|command| command.words.clone())
            .collect();

        assert_eq!(
            words,
            vec![
                vec!["while", "((", "n < 3", "))"],
                vec!["do", "((", "n++", "))"],
                vec!["done"],
                vec!["until", "((", "n == 5", "))"],
                vec!["do", "((", "n++", "))"],
                vec!["done"],
            ]
        );
    }

    #[test]
    fn test_parse_arithmetic_bitwise_assignment_operators() {
        let tokens = tokenize("(( n &= 10 )); (( n |= 1 )); (( n <<= 2 )); (( n >>= 1 ))");
        let ast = parse(&tokens);
        let words: Vec<Vec<String>> = ast
            .commands
            .iter()
            .filter(|command| !command.words.is_empty())
            .map(|command| command.words.clone())
            .collect();

        assert_eq!(
            words,
            vec![
                vec!["((", "n &= 10", "))"],
                vec!["((", "n |= 1", "))"],
                vec!["((", "n <<= 2", "))"],
                vec!["((", "n >>= 1", "))"],
            ]
        );
    }

    #[test]
    fn test_parse_grouped_arithmetic_command_expression() {
        let tokens = tokenize("(( (n = 3) )); (( ((m = 0)) ))");
        let ast = parse(&tokens);
        let words: Vec<Vec<String>> = ast
            .commands
            .iter()
            .filter(|command| !command.words.is_empty())
            .map(|command| command.words.clone())
            .collect();

        assert_eq!(
            words,
            vec![
                vec!["((", "( n = 3 )", "))"],
                vec!["((", "( ( m = 0 ) )", "))"],
            ]
        );
    }

    #[test]
    fn test_parse_arithmetic_for_command() {
        let tokens = tokenize("for (( i = 0; i < 3; i++ )); do echo $i; done");
        let ast = parse(&tokens);
        let for_command = ast.commands[0].for_command.as_ref().unwrap();
        let arithmetic = for_command.arithmetic.as_ref().unwrap();

        assert_eq!(arithmetic.init, "i = 0");
        assert_eq!(arithmetic.test, "i < 3");
        assert_eq!(arithmetic.update, "i++");
        assert_eq!(for_command.body[0].words, ["echo", "$i"]);
    }
}
