use crate::lexer::TokenKind;

/// Represents a redirect specification
#[derive(Debug, Clone, PartialEq)]
pub struct Redirect {
    pub fd: Option<u32>,
    pub fd_var: Option<String>,
    pub operator: String,
    pub kind: RedirectKind,
    pub target: String,
    pub append: bool,
    pub clobber: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectKind {
    Input,
    Output,
    Append,
    ReadWrite,
    DuplicateInput,
    DuplicateOutput,
    CloseInput,
    CloseOutput,
    ClobberOutput,
    CombinedOutput,
    CombinedAppend,
    HereString,
    HereDoc,
    Unknown,
}

/// Represents a here-document redirection.
#[derive(Debug, Clone, PartialEq)]
pub struct HereDocRedirect {
    pub fd: Option<u32>,
    pub fd_var: Option<String>,
    pub operator: String,
    pub delimiter: String,
    pub strip_tabs: bool,
    pub quoted_delimiter: bool,
    pub here_string: bool,
    pub body: Option<String>,
}

/// Represents a narrow `for` compound command.
#[derive(Debug, Clone)]
pub struct ForCommand {
    pub keyword: String,
    pub variable: String,
    pub in_keyword: Option<String>,
    pub words: Vec<String>,
    pub word_metadata: Vec<WordMetadata>,
    pub default_positional: bool,
    pub list_terminator: Option<String>,
    pub arithmetic: Option<ArithmeticForCommand>,
    pub body_kind: CommandBodyKind,
    pub body_open_delimiter: Option<String>,
    pub body_close_delimiter: Option<String>,
    pub do_keyword: Option<String>,
    pub end_keyword: Option<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `for (( init; test; update ))` compound command.
#[derive(Debug, Clone)]
pub struct ArithmeticForCommand {
    pub open_delimiter: String,
    pub init: String,
    pub separators: Vec<String>,
    pub test: String,
    pub update: String,
    pub close_delimiter: String,
}

/// Represents a `(( expression ))` arithmetic command.
#[derive(Debug, Clone)]
pub struct ArithmeticCommand {
    pub open_delimiter: String,
    pub expression: String,
    pub close_delimiter: String,
    pub operators: Vec<ArithmeticOperator>,
    pub variables: Vec<String>,
    pub has_assignment: bool,
    pub has_comparison: bool,
    pub has_logical: bool,
    pub has_update: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticOperator {
    pub text: String,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandBodyKind {
    DoDone,
    BraceGroup,
}

/// Represents an `if condition; then ... [elif ...] [else ...] fi` command.
#[derive(Debug, Clone)]
pub struct IfCommand {
    pub keyword: String,
    pub condition: Vec<CommandNode>,
    pub condition_terminator: Option<String>,
    pub then_keyword: String,
    pub then_body: Vec<CommandNode>,
    pub elif_branches: Vec<ElifBranch>,
    pub else_keyword: Option<String>,
    pub else_body: Option<Vec<CommandNode>>,
    pub end_keyword: String,
}

#[derive(Debug, Clone)]
pub struct ElifBranch {
    pub keyword: String,
    pub condition: Vec<CommandNode>,
    pub condition_terminator: Option<String>,
    pub then_keyword: String,
    pub body: Vec<CommandNode>,
}

/// Represents `while condition; do body; done` or `until condition; do body; done`.
#[derive(Debug, Clone)]
pub struct LoopCommand {
    pub keyword: String,
    pub condition: Vec<CommandNode>,
    pub condition_terminator: Option<String>,
    pub do_keyword: String,
    pub body_open_delimiter: String,
    pub body_close_delimiter: String,
    pub body: Vec<CommandNode>,
    pub end_keyword: String,
    pub kind: LoopKind,
    pub until: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopKind {
    While,
    Until,
}

/// Represents a `[[ expression ]]` conditional command.
#[derive(Debug, Clone)]
pub struct ConditionalCommand {
    pub open_delimiter: String,
    pub args: Vec<String>,
    pub arg_metadata: Vec<WordMetadata>,
    pub close_delimiter: String,
    pub expression: ConditionalExpression,
}

/// Represents the parsed expression inside `[[ ... ]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalExpression {
    pub kind: ConditionalExpressionKind,
    pub open_delimiter: Option<String>,
    pub operator: Option<String>,
    pub operands: Vec<String>,
    pub pattern_operand: Option<ConditionalPatternOperand>,
    pub children: Vec<ConditionalExpression>,
    pub close_delimiter: Option<String>,
}

/// Represents a pattern-like right-hand operand in `[[ lhs == pat ]]` or
/// `[[ lhs =~ regex ]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalPatternOperand {
    pub text: String,
    pub kind: ConditionalPatternKind,
    pub operators: Vec<String>,
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub brace_expansions: Vec<BraceExpansion>,
    pub parameter_expansions: Vec<ParameterExpansion>,
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub has_glob: bool,
    pub has_extglob: bool,
}

impl ConditionalPatternOperand {
    pub fn new(text: String, kind: ConditionalPatternKind) -> Self {
        let operators = if kind == ConditionalPatternKind::Glob {
            case_pattern_operators(&text)
        } else {
            Vec::new()
        };
        let extglob_patterns = if kind == ConditionalPatternKind::Glob {
            super::extglob_patterns_in_word(&text)
        } else {
            Vec::new()
        };
        let brace_expansions = if kind == ConditionalPatternKind::Glob {
            super::brace_expansions_in_word(&text)
        } else {
            Vec::new()
        };
        let parameter_expansions = super::parameter_expansions_in_word(&text);
        let arithmetic_expansions = super::arithmetic_expansions_in_word(&text);
        Self {
            has_glob: case_pattern_has_glob(&operators),
            has_extglob: case_pattern_has_extglob(&operators),
            extglob_patterns,
            brace_expansions,
            parameter_expansions,
            arithmetic_expansions,
            text,
            kind,
            operators,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalPatternKind {
    Glob,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalExpressionKind {
    Empty,
    Word,
    Unary,
    Binary,
    Logical,
    Negation,
    Group,
    Unknown,
}

/// Represents a `( compound_list )` subshell command.
#[derive(Debug, Clone)]
pub struct SubshellCommand {
    pub open_delimiter: String,
    pub close_delimiter: String,
    pub body: Vec<CommandNode>,
}

/// Represents a `{ compound_list; }` brace group command.
#[derive(Debug, Clone)]
pub struct BraceGroupCommand {
    pub open_delimiter: String,
    pub close_delimiter: String,
    pub body: Vec<CommandNode>,
}

/// Represents a `command | command` pipeline.
#[derive(Debug, Clone)]
pub struct PipelineCommand {
    pub stages: Vec<CommandNode>,
    pub operators: Vec<String>,
}

/// Represents commands connected by `&&` and `||`.
#[derive(Debug, Clone)]
pub struct AndOrListCommand {
    pub commands: Vec<CommandNode>,
    pub connectors: Vec<bool>,
    pub operators: Vec<String>,
}

/// Represents `time [-p] [!] command`.
#[derive(Debug, Clone)]
pub struct TimeCommand {
    pub keyword: String,
    pub prefix_words: Vec<String>,
    pub prefix_word_metadata: Vec<WordMetadata>,
    pub command: Box<CommandNode>,
    pub posix_format: bool,
    pub inverted: bool,
}

/// Represents `command &`.
#[derive(Debug, Clone)]
pub struct BackgroundCommand {
    pub operator: String,
    pub command: Box<CommandNode>,
}

/// Represents `! command`.
#[derive(Debug, Clone)]
pub struct InvertedCommand {
    pub operator: String,
    pub command: Box<CommandNode>,
}

/// Represents a parsed `name=(...)` or `name+=(...)` compound assignment word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundAssignment {
    pub name: String,
    pub value: String,
    pub operator: String,
    pub append: bool,
    pub word_index: Option<usize>,
    pub elements: Vec<CompoundAssignmentElement>,
}

/// Represents one element inside a `name=(...)` compound assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompoundAssignmentElement {
    pub subscript: Option<String>,
    pub value: String,
    pub operator: Option<String>,
    pub append: bool,
    pub element_index: usize,
    pub subscript_brace_expansions: Vec<BraceExpansion>,
    pub subscript_parameter_expansions: Vec<ParameterExpansion>,
    pub subscript_arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub brace_expansions: Vec<BraceExpansion>,
    pub parameter_expansions: Vec<ParameterExpansion>,
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub pathname_patterns: Vec<PathnamePattern>,
    pub tilde_expansions: Vec<TildeExpansion>,
    pub word_quotes: Vec<WordQuote>,
}

/// Represents a parsed `name[subscript]=value` or `name[subscript]+=value` word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayElementAssignment {
    pub name: String,
    pub subscript: String,
    pub value: String,
    pub operator: String,
    pub append: bool,
    pub word_index: Option<usize>,
    pub subscript_brace_expansions: Vec<BraceExpansion>,
    pub subscript_parameter_expansions: Vec<ParameterExpansion>,
    pub subscript_arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub brace_expansions: Vec<BraceExpansion>,
    pub parameter_expansions: Vec<ParameterExpansion>,
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub pathname_patterns: Vec<PathnamePattern>,
    pub tilde_expansions: Vec<TildeExpansion>,
    pub word_quotes: Vec<WordQuote>,
}

/// Represents a parsed `<(...)` or `>(...)` process substitution.
#[derive(Debug, Clone)]
pub struct ProcessSubstitution {
    pub target: String,
    pub open_delimiter: String,
    pub operator: String,
    pub source: String,
    pub close_delimiter: String,
    pub commands: Vec<CommandNode>,
    pub output: bool,
    pub word_index: Option<usize>,
    pub redirect_fd: Option<u32>,
}

/// Represents a parsed `$()` or backtick command substitution inside a word.
#[derive(Debug, Clone)]
pub struct CommandSubstitutionNode {
    pub text: String,
    pub open_delimiter: String,
    pub operator: String,
    pub source: String,
    pub close_delimiter: String,
    pub commands: Vec<CommandNode>,
    pub backtick: bool,
    pub current_shell: bool,
    pub pipe_output: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed `$(( expression ))` arithmetic expansion inside a word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArithmeticExpansion {
    pub text: String,
    pub open_delimiter: String,
    pub expression: String,
    pub close_delimiter: String,
    pub operators: Vec<ArithmeticOperator>,
    pub variables: Vec<String>,
    pub has_assignment: bool,
    pub has_comparison: bool,
    pub has_logical: bool,
    pub has_update: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed `$name`, `$?`, or `${...}` parameter expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterExpansion {
    pub text: String,
    pub open_delimiter: String,
    pub parameter: String,
    pub close_delimiter: String,
    pub name: String,
    pub operator: Option<String>,
    pub operator_prefix: bool,
    pub word: Option<String>,
    pub braced: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed brace expansion such as `{a,b}` or `{1..3}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BraceExpansion {
    pub text: String,
    pub open_delimiter: String,
    pub body: String,
    pub close_delimiter: String,
    pub operators: Vec<String>,
    pub range: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed extglob pattern such as `@(a|b)` or `!(tmp)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtglobPattern {
    pub text: String,
    pub open_delimiter: String,
    pub operator: char,
    pub pattern: String,
    pub close_delimiter: String,
    pub operators: Vec<String>,
    pub alternatives: Vec<String>,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed tilde prefix such as `~`, `~/x`, `~+`, or `~user`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TildeExpansion {
    pub text: String,
    pub open_delimiter: String,
    pub prefix: String,
    pub close_delimiter: String,
    pub suffix: String,
    pub after_colon: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Represents a parsed pathname expansion pattern such as `*.rs` or `src/[ab]?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathnamePattern {
    pub text: String,
    pub operators: Vec<String>,
    pub has_star: bool,
    pub has_question: bool,
    pub has_bracket: bool,
    pub globstar: bool,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuoteKind {
    Single,
    Double,
    AnsiC,
    Locale,
}

/// Represents a quoted segment in a shell word before quote removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordQuote {
    pub text: String,
    pub open_delimiter: String,
    pub body: String,
    pub kind: QuoteKind,
    pub close_delimiter: String,
    pub word_index: Option<usize>,
    pub assignment_name: Option<String>,
}

/// Structured metadata for words stored outside a simple command word list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordMetadata {
    pub word_index: usize,
    pub value: String,
    pub raw: String,
    pub brace_expansions: Vec<BraceExpansion>,
    pub parameter_expansions: Vec<ParameterExpansion>,
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub tilde_expansions: Vec<TildeExpansion>,
    pub pathname_patterns: Vec<PathnamePattern>,
    pub word_quotes: Vec<WordQuote>,
}

impl WordMetadata {
    pub fn new(word_index: usize, value: String, raw: String) -> Self {
        Self {
            word_index,
            brace_expansions: super::brace_expansions_in_word(&value),
            parameter_expansions: super::parameter_expansions_in_word(&value),
            arithmetic_expansions: super::arithmetic_expansions_in_word(&value),
            extglob_patterns: super::extglob_patterns_in_word(&value),
            tilde_expansions: super::tilde_expansions_in_word(&value),
            pathname_patterns: super::pathname_patterns_in_word(&value, &raw),
            word_quotes: super::word_quotes_in_raw(&raw),
            value,
            raw,
        }
    }
}

/// Represents a narrow `case` compound command.
#[derive(Debug, Clone)]
pub struct CaseCommand {
    pub keyword: String,
    pub word: String,
    pub word_metadata: WordMetadata,
    pub in_keyword: String,
    pub clauses: Vec<CaseClause>,
    pub end_keyword: String,
}

#[derive(Debug, Clone)]
pub struct CaseClause {
    pub pattern_open_delimiter: Option<String>,
    pub patterns: Vec<String>,
    pub pattern_separators: Vec<String>,
    pub pattern_close_delimiter: String,
    pub pattern_nodes: Vec<CasePattern>,
    pub body: Vec<CommandNode>,
    pub terminator: CaseTerminator,
    pub terminator_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasePattern {
    pub text: String,
    pub raw_text: String,
    pub operators: Vec<String>,
    pub extglob_patterns: Vec<ExtglobPattern>,
    pub brace_expansions: Vec<BraceExpansion>,
    pub parameter_expansions: Vec<ParameterExpansion>,
    pub arithmetic_expansions: Vec<ArithmeticExpansion>,
    pub clause_index: usize,
    pub pattern_index: usize,
    pub has_glob: bool,
    pub has_extglob: bool,
    pub negated_extglob: bool,
}

impl CasePattern {
    pub fn new(text: String, clause_index: usize, pattern_index: usize) -> Self {
        Self::new_with_raw(text.clone(), text, clause_index, pattern_index)
    }

    pub fn new_with_raw(
        text: String,
        raw_text: String,
        clause_index: usize,
        pattern_index: usize,
    ) -> Self {
        let operators = case_pattern_operators(&raw_text);
        Self {
            has_glob: case_pattern_has_glob(&operators),
            has_extglob: case_pattern_has_extglob(&operators),
            negated_extglob: operators.iter().any(|operator| operator == "!("),
            operators,
            extglob_patterns: super::extglob_patterns_in_word(&text),
            brace_expansions: super::brace_expansions_in_word(&text),
            parameter_expansions: super::parameter_expansions_in_word(&text),
            arithmetic_expansions: super::arithmetic_expansions_in_word(&text),
            raw_text,
            text,
            clause_index,
            pattern_index,
        }
    }
}

fn case_pattern_has_glob(operators: &[String]) -> bool {
    operators
        .iter()
        .any(|operator| operator == "*" || operator == "?" || operator.starts_with('['))
}

fn case_pattern_has_extglob(operators: &[String]) -> bool {
    operators
        .iter()
        .any(|operator| matches!(operator.as_str(), "@(" | "*(" | "+(" | "?(" | "!("))
}

fn case_pattern_operators(raw_pattern: &str) -> Vec<String> {
    let chars = raw_pattern.chars().collect::<Vec<_>>();
    let mut operators = Vec::new();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '$' && chars.get(index + 1) == Some(&'\'') {
            if let Some(next_index) = skip_case_pattern_quoted(&chars, index + 2, '\'') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '$' && chars.get(index + 1) == Some(&'"') {
            if let Some(next_index) = skip_case_pattern_quoted(&chars, index + 2, '"') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '\'' {
            if let Some(next_index) = skip_case_pattern_quoted(&chars, index + 1, '\'') {
                index = next_index;
                continue;
            }
        }

        if chars[index] == '"' {
            if let Some(next_index) = skip_case_pattern_quoted(&chars, index + 1, '"') {
                index = next_index;
                continue;
            }
        }

        match chars[index] {
            '@' | '!' | '+' | '?' | '*' if chars.get(index + 1) == Some(&'(') => {
                operators.push(chars[index..=index + 1].iter().collect());
                index += 2;
                continue;
            }
            '*' if chars.get(index + 1) == Some(&'*') => {
                operators.push("**".to_string());
                index += 2;
                continue;
            }
            '*' => operators.push("*".to_string()),
            '?' => operators.push("?".to_string()),
            '[' => {
                let mut end = index + 1;
                while end < chars.len() && chars[end] != ']' {
                    if chars[end] == '\\' {
                        end += 1;
                    }
                    end += 1;
                }
                if end < chars.len() {
                    operators.push(chars[index..=end].iter().collect());
                    index = end + 1;
                    continue;
                }
            }
            '|' => operators.push("|".to_string()),
            '\\' => {
                index += 2;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    operators
}

fn skip_case_pattern_quoted(chars: &[char], start: usize, delimiter: char) -> Option<usize> {
    let mut index = start;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if delimiter == '"' && ch == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if ch == delimiter {
            return Some(index + 1);
        }
        index += 1;
    }
    None
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
    pub keyword: String,
    pub variable: String,
    pub in_keyword: Option<String>,
    pub words: Vec<String>,
    pub word_metadata: Vec<WordMetadata>,
    pub default_positional: bool,
    pub list_terminator: Option<String>,
    pub body_kind: CommandBodyKind,
    pub body_open_delimiter: Option<String>,
    pub body_close_delimiter: Option<String>,
    pub do_keyword: Option<String>,
    pub end_keyword: Option<String>,
    pub body: Vec<CommandNode>,
}

/// Represents a narrow `name() { ...; }` shell function definition.
#[derive(Debug, Clone)]
pub struct FunctionCommand {
    pub name: String,
    pub body: Vec<CommandNode>,
    pub keyword: bool,
    pub keyword_text: Option<String>,
    pub has_parentheses: bool,
    pub open_paren: Option<String>,
    pub close_paren: Option<String>,
    pub body_kind: FunctionBodyKind,
    pub body_open_delimiter: Option<String>,
    pub body_close_delimiter: Option<String>,
    pub body_start: Option<usize>,
    pub body_end: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionBodyKind {
    BraceGroup,
    Subshell,
    CommandSequence,
    CompoundCommand,
}

/// Represents `coproc [NAME] command [args...]` or `coproc [NAME] { body; }`
#[derive(Debug, Clone)]
pub struct CoprocCommand {
    pub keyword: String,
    /// Optional name (defaults to COPROC)
    pub name: Option<String>,
    /// The command words (for simple commands)
    pub words: Vec<String>,
    pub word_metadata: Vec<WordMetadata>,
    pub body_kind: CoprocBodyKind,
    pub body_open_delimiter: Option<String>,
    pub body_close_delimiter: Option<String>,
    /// Brace group body (for compound commands)
    pub body: Option<Vec<CommandNode>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoprocBodyKind {
    SimpleCommand,
    BraceGroup,
    Subshell,
    CommandSequence,
    CompoundCommand,
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
    /// Structured array element assignment words parsed from `name[index]=value`.
    pub array_element_assignments: Vec<ArrayElementAssignment>,
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
    /// Structured extglob patterns parsed from `@(a|b)`, `!(x)`, etc.
    pub extglob_patterns: Vec<ExtglobPattern>,
    /// Structured tilde prefixes parsed from words and assignment values.
    pub tilde_expansions: Vec<TildeExpansion>,
    /// Structured pathname expansion patterns parsed from glob words.
    pub pathname_patterns: Vec<PathnamePattern>,
    /// Structured quoted segments parsed from raw shell words.
    pub word_quotes: Vec<WordQuote>,
    /// Redirections in parse order.
    pub redirects: Vec<Redirect>,
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
    pub conditional_command: Option<Box<ConditionalCommand>>,
    /// `( compound_list )`
    pub subshell_command: Option<SubshellCommand>,
    /// `case word in pattern) ... ;; esac`
    pub case_command: Option<Box<CaseCommand>>,
    /// `select name [in words ...]; do ...; done`
    pub select_command: Option<Box<SelectCommand>>,
    /// `name() { compound_list; }`
    pub function_command: Option<Box<FunctionCommand>>,
    /// `{ compound_list; }`
    pub brace_group: Option<BraceGroupCommand>,
    pub coproc_command: Option<Box<CoprocCommand>>,
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
            array_element_assignments: Vec::new(),
            process_substitutions: Vec::new(),
            command_substitutions: Vec::new(),
            arithmetic_expansions: Vec::new(),
            parameter_expansions: Vec::new(),
            brace_expansions: Vec::new(),
            extglob_patterns: Vec::new(),
            tilde_expansions: Vec::new(),
            pathname_patterns: Vec::new(),
            word_quotes: Vec::new(),
            redirects: Vec::new(),
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
