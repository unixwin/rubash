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

/// Represents a `(( expression ))` arithmetic command.
#[derive(Debug, Clone)]
pub struct ArithmeticCommand {
    pub expression: String,
}

/// Represents an `if condition; then ... [elif ...] [else ...] fi` command.
#[derive(Debug, Clone)]
pub struct IfCommand {
    pub condition: Vec<CommandNode>,
    pub then_body: Vec<CommandNode>,
    pub elif_branches: Vec<ElifBranch>,
    pub else_body: Option<Vec<CommandNode>>,
}

#[derive(Debug, Clone)]
pub struct ElifBranch {
    pub condition: Vec<CommandNode>,
    pub body: Vec<CommandNode>,
}

/// Represents `while condition; do body; done` or `until condition; do body; done`.
#[derive(Debug, Clone)]
pub struct LoopCommand {
    pub condition: Vec<CommandNode>,
    pub body: Vec<CommandNode>,
    pub until: bool,
}

/// Represents a `[[ expression ]]` conditional command.
#[derive(Debug, Clone)]
pub struct ConditionalCommand {
    pub args: Vec<String>,
}

/// Represents a `( compound_list )` subshell command.
#[derive(Debug, Clone)]
pub struct SubshellCommand {
    pub body: Vec<CommandNode>,
}

/// Represents a `{ compound_list; }` brace group command.
#[derive(Debug, Clone)]
pub struct BraceGroupCommand {
    pub body: Vec<CommandNode>,
}

/// Represents a `command | command` pipeline.
#[derive(Debug, Clone)]
pub struct PipelineCommand {
    pub stages: Vec<CommandNode>,
}

/// Represents commands connected by `&&` and `||`.
#[derive(Debug, Clone)]
pub struct AndOrListCommand {
    pub commands: Vec<CommandNode>,
    pub connectors: Vec<bool>,
}

/// Represents `time [-p] [!] command`.
#[derive(Debug, Clone)]
pub struct TimeCommand {
    pub command: Box<CommandNode>,
    pub posix_format: bool,
    pub inverted: bool,
}

/// Represents `command &`.
#[derive(Debug, Clone)]
pub struct BackgroundCommand {
    pub command: Box<CommandNode>,
}

/// Represents `! command`.
#[derive(Debug, Clone)]
pub struct InvertedCommand {
    pub command: Box<CommandNode>,
}

/// Represents a parsed `name=(...)` or `name+=(...)` compound assignment word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundAssignment {
    pub name: String,
    pub value: String,
    pub append: bool,
    pub word_index: Option<usize>,
}

/// Represents a parsed `<(...)` or `>(...)` process substitution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSubstitution {
    pub target: String,
    pub source: String,
    pub output: bool,
    pub word_index: Option<usize>,
    pub redirect_fd: Option<u32>,
}

/// Represents a parsed `$()` or backtick command substitution inside a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSubstitutionNode {
    pub text: String,
    pub source: String,
    pub backtick: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed `$(( expression ))` arithmetic expansion inside a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticExpansion {
    pub text: String,
    pub expression: String,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed `$name`, `$?`, or `${...}` parameter expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterExpansion {
    pub text: String,
    pub parameter: String,
    pub braced: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed brace expansion such as `{a,b}` or `{1..3}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraceExpansion {
    pub text: String,
    pub body: String,
    pub range: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
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
    pub default_positional: bool,
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
    /// Structured compound array assignment words parsed from `name=(...)`.
    pub compound_assignments: Vec<CompoundAssignment>,
    /// Structured process substitutions parsed from `<(...)` and `>(...)`.
    pub process_substitutions: Vec<ProcessSubstitution>,
    /// Structured command substitutions parsed from `$()` and backticks.
    pub command_substitutions: Vec<CommandSubstitutionNode>,
    /// Structured arithmetic expansions parsed from `$((...))`.
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    /// Structured parameter expansions parsed from `$name`, `$?`, and `${...}`.
    pub parameter_expansions: Vec<ParameterExpansion>,
    /// Structured brace expansions parsed from `{a,b}` and `{1..3}` words.
    pub brace_expansions: Vec<BraceExpansion>,
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
    /// Pipeline of commands connected by `|`.
    pub pipeline_command: Option<PipelineCommand>,
    /// Commands connected by `&&` and `||`.
    pub and_or_list: Option<AndOrListCommand>,
    /// `time [-p] [!] command`.
    pub time_command: Option<TimeCommand>,
    /// `command &`.
    pub background_command: Option<BackgroundCommand>,
    /// `! command`.
    pub inverted_command: Option<InvertedCommand>,
    /// Command is executed inside a subshell grouping `( ... )`.
    pub subshell: bool,
    /// This command closes the current subshell grouping.
    pub subshell_end: bool,
    /// `for name in words; do ...; done`
    pub for_command: Option<ForCommand>,
    /// `(( expression ))`
    pub arithmetic_command: Option<ArithmeticCommand>,
    /// `if condition; then body; fi`
    pub if_command: Option<IfCommand>,
    /// `while/until condition; do body; done`
    pub loop_command: Option<LoopCommand>,
    /// `[[ expression ]]`
    pub conditional_command: Option<ConditionalCommand>,
    /// `( compound_list )`
    pub subshell_command: Option<SubshellCommand>,
    /// `case word in pattern) ... ;; esac`
    pub case_command: Option<CaseCommand>,
    /// `select name [in words ...]; do ...; done`
    pub select_command: Option<SelectCommand>,
    /// `name() { compound_list; }`
    pub function_command: Option<FunctionCommand>,
    /// `{ compound_list; }`
    pub brace_group: Option<BraceGroupCommand>,
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
            compound_assignments: Vec::new(),
            process_substitutions: Vec::new(),
            command_substitutions: Vec::new(),
            arithmetic_expansions: Vec::new(),
            parameter_expansions: Vec::new(),
            brace_expansions: Vec::new(),
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
            pipeline_command: None,
            and_or_list: None,
            time_command: None,
            background_command: None,
            inverted_command: None,
            subshell: false,
            subshell_end: false,
            for_command: None,
            arithmetic_command: None,
            if_command: None,
            loop_command: None,
            conditional_command: None,
            subshell_command: None,
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
