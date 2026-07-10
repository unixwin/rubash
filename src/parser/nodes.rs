use crate::lexer::TokenKind;

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

/// Represents a `select name [in words ...]; do ...; done` compound command.
#[derive(Debug, Clone)]
pub struct SelectCommand {
    pub variable: String,
    pub words: Vec<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `name() { ...; }` shell function definition.
#[derive(Debug, Clone)]
pub struct FunctionCommand {
    pub name: String,
    pub body: Vec<CommandNode>,
}

/// Represents `coproc [NAME] command [args...]` or `coproc [NAME] { body; }`
#[derive(Debug, Clone)]
pub struct CoprocCommand {
    /// Optional name (defaults to COPROC)
    pub name: Option<String>,
    /// The command words (for simple commands)
    pub words: Vec<String>,
    /// Brace group body (for compound commands)
    pub body: Option<Vec<CommandNode>>,
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
    /// `select name [in words ...]; do ...; done`
    pub select_command: Option<SelectCommand>,
    /// `name() { compound_list; }`
    pub function_command: Option<FunctionCommand>,
    /// `{ compound_list; }`
    pub brace_group: Option<Vec<CommandNode>>,
    pub coproc_command: Option<CoprocCommand>,
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
            select_command: None,
            function_command: None,
            brace_group: None,
            coproc_command: None,
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
