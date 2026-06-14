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
}

/// Represents a narrow `for` compound command.
#[derive(Debug, Clone)]
pub struct ForCommand {
    pub variable: String,
    pub words: Vec<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `while/until list; do ...; done` loop.
#[derive(Debug, Clone)]
pub struct LoopCommand {
    pub until: bool,
    pub condition: Vec<CommandNode>,
    pub body: Vec<CommandNode>,
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
    /// `while/until list; do ...; done`
    pub loop_command: Option<LoopCommand>,
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
            assignments: std::collections::HashMap::new(),
            redirect_in: None,
            redirect_out: None,
            append: None,
            redirect_err: None,
            redirect_err_append: None,
            heredoc: None,
            pipe: None,
            background: false,
            and_or: None,
            inverted: false,
            subshell: false,
            subshell_end: false,
            for_command: None,
            loop_command: None,
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

        if token.kind == TokenKind::Keyword
            && (token.value == "while" || token.value == "until")
            && command_is_empty(&current_cmd)
        {
            if let Some((loop_cmd, next_i)) = parse_loop_command(tokens, i) {
                ast.commands.push(loop_cmd);
                current_cmd = CommandNode::new();
                i = next_i;
                continue;
            }
        }

        if token.kind == TokenKind::Word && command_is_empty(&current_cmd) {
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

        match token.kind {
            TokenKind::Word | TokenKind::Variable | TokenKind::CommandSubst => {
                current_cmd.subshell |= in_subshell;
                note_command_line(&mut current_cmd, token);
                current_cmd.words.push(token.value.clone());
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
                                var_value = compound_value;
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
                    }
                }
            }
            TokenKind::Pipe => {
                // Save current command with pipe flag
                current_cmd.subshell |= in_subshell;
                current_cmd.pipe = Some(1);
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::Semicolon => {
                // Command separator
                current_cmd.subshell |= in_subshell;
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
            }
            TokenKind::RedirectIn => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_in = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectOut => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_out = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::Append => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.append = Some(Redirect {
                            fd: None,
                            target: tokens[i + 1].value.clone(),
                            append: true,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectErr => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_err = Some(Redirect {
                            fd: Some(2),
                            target: tokens[i + 1].value.clone(),
                            append: false,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::RedirectErrAppend => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    if matches!(tokens[i + 1].kind, TokenKind::Word | TokenKind::Variable) {
                        current_cmd.redirect_err_append = Some(Redirect {
                            fd: Some(2),
                            target: tokens[i + 1].value.clone(),
                            append: true,
                        });
                        i += 1;
                    }
                }
            }
            TokenKind::HereDoc => {
                note_command_line(&mut current_cmd, token);
                if i + 1 < tokens.len() {
                    i += 1;
                }
            }
            TokenKind::HereDocBody => {
                note_command_line(&mut current_cmd, token);
                current_cmd.heredoc = Some(token.value.clone());
            }
            TokenKind::And | TokenKind::Or => {
                // TODO(parse.y/execute_cmd.c): This preserves the AND-OR
                // list connector on simple commands. Full Bash grammar needs
                // a list AST with compound commands and proper precedence.
                current_cmd.subshell |= in_subshell;
                current_cmd.and_or = Some(token.kind == TokenKind::And);
                ast.commands.push(current_cmd);
                current_cmd = CommandNode::new();
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
                if token.value == "!" && command_is_empty(&current_cmd) {
                    // TODO(parse.y/execute_cmd.c): Bash represents `!` as a
                    // pipeline/list inversion flag. Keep it on the next simple
                    // command until the parser has a real pipeline AST.
                    current_cmd.inverted = true;
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
                    current_cmd.words.push(token.value.clone());
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
    // commands, `"$@"` default words, and reserved-word parsing state. This
    // maps the simple upstream alias test form: `for name in words; do body; done`.
    let variable = tokens.get(start + 1)?.value.clone();
    if !matches!(
        tokens.get(start + 1)?.kind,
        TokenKind::Word | TokenKind::Variable
    ) {
        return None;
    }

    let mut i = start + 2;
    if !is_keyword(tokens, i, "in") {
        return None;
    }
    i += 1;

    let mut words = Vec::new();
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

    let body = parse(&tokens[body_start..i]).commands;
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.for_command = Some(ForCommand {
        variable,
        words,
        body,
    });
    Some((command, i + 1))
}

fn parse_loop_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash parses `while`/`until` as compound
    // commands with arbitrary compound lists, nested loops, redirections, and
    // parser-state-sensitive reserved words. This maps the simple upstream
    // arithmetic test form: `until condition; do body; done`.
    let until = tokens.get(start)?.value == "until";
    let mut i = start + 1;
    let condition_start = i;
    while i < tokens.len() && !is_keyword(tokens, i, "do") {
        i += 1;
    }
    if condition_start == i || !is_keyword(tokens, i, "do") {
        return None;
    }

    let condition = parse(&tokens[condition_start..i]).commands;
    i += 1;

    let body_start = i;
    let mut depth = 0usize;
    while i < tokens.len() {
        if is_keyword(tokens, i, "while") || is_keyword(tokens, i, "until") {
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

    let body = parse(&tokens[body_start..i]).commands;
    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.loop_command = Some(LoopCommand {
        until,
        condition,
        body,
    });
    Some((command, i + 1))
}

fn parse_function_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): Bash has full function_def grammar,
    // including `function name`, redirections, nested compound commands, and
    // parser-state-sensitive reserved words. This maps the upstream builtins
    // `name() { ...; }` form onto a function command node.
    let name = tokens.get(start)?.value.clone();
    if !is_function_name(&name)
        || tokens.get(start + 1)?.value != "("
        || tokens.get(start + 2)?.value != ")"
    {
        return None;
    }

    let mut i = start + 3;
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
        let mut next_i = i + 1;
        while tokens
            .get(next_i)
            .is_some_and(|token| token.kind == TokenKind::Semicolon)
        {
            next_i += 1;
        }

        let mut command = CommandNode::new();
        command.line = tokens.get(start).map(|token| token.position);
        command.function_command = Some(FunctionCommand { name, body });
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

    let mut body = parse(&tokens[body_start..i]).commands;
    if let Some(line) = tokens.get(start).map(|token| token.position) {
        set_body_line(&mut body, line);
    }
    let mut next_i = i + 1;
    while tokens
        .get(next_i)
        .is_some_and(|token| token.kind == TokenKind::Semicolon)
    {
        next_i += 1;
    }

    let mut command = CommandNode::new();
    command.line = tokens.get(start).map(|token| token.position);
    command.function_command = Some(FunctionCommand { name, body });
    Some((command, next_i))
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
        if matches!(
            tokens[i].kind,
            TokenKind::Word | TokenKind::Variable | TokenKind::Assignment
        ) {
            values.push(tokens[i].value.clone());
        }
        i += 1;
    }

    if !is_keyword(tokens, i, ")") {
        return None;
    }

    Some((format!("({})", values.join(" ")), i))
}

fn parse_case_command(tokens: &[Token], start: usize) -> Option<(CommandNode, usize)> {
    // TODO(parse.y/execute_cmd.c): GNU Bash supports `;&`, `;;&`, `|`-joined
    // pattern lists, extglob patterns, nested compound lists, and redirections
    // on the compound command. This covers the simple upstream alias3.sub
    // `case word in pattern) list ;; *) list ;; esac` shape.
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
        while i < tokens.len()
            && !is_keyword(tokens, i, "esac")
            && !(tokens[i].kind == TokenKind::Word && tokens[i].value == ";;")
        {
            i += 1;
        }
        let body = parse(&tokens[body_start..i]).commands;
        clauses.push(CaseClause { patterns, body });

        if i < tokens.len() && tokens[i].kind == TokenKind::Word && tokens[i].value == ";;" {
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

fn note_command_line(cmd: &mut CommandNode, token: &Token) {
    if cmd.line.is_none() {
        cmd.line = Some(token.position);
    }
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
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.for_command.is_none()
        && cmd.case_command.is_none()
        && cmd.function_command.is_none()
}

fn is_function_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
}
