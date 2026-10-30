//! Tier-B syntax views: the owned, span-shaped surface of the
//! Session API's syntax layer (API design Q4, decided 2026-10-25:
//! owned view types, never the internal `Ast`).
//!
//! This module's first shape is deliberately the smallest one a real
//! consumer needs. That consumer is the debugger's breakpoint
//! resolution (debugger design section 5.1, WP-8): to answer
//! "verified or not" for a `(file, line)` breakpoint it needs, per
//! parsed file, the set of lines that carry a command, the subset
//! the DEBUG-trap seam can actually stop on (the section 4.1 seam
//! distribution), and where the file's functions are defined. The
//! view is built from the internal `Ast` at the two places the
//! engine parses a whole reader unit out of a file — the `source`
//! builtin's group runner and the script driver's group loop — and
//! nowhere else: synthetic units (eval strings, alias re-parses,
//! breakpoint-condition evaluations) never register a view, so a
//! view always describes a real file the debuggee is executing.
//!
//! File identity at this layer is the exact string the engine
//! itself puts on the `BASH_SOURCE` stack for the unit: the view
//! records what the engine reported, verbatim. The session
//! core's registration point (PLAN row 11) converges that
//! spelling — and every breakpoint spelling — onto one
//! canonical identity (`debugger::resolution::file_identity`),
//! so a view registered under one spelling serves entries set
//! under any other spelling of the same file.
//!
//! Line classification (the dead-line judgment, WP-8):
//!
//! - [`LineClass::Stoppable`] — a command the seam fires before
//!   starts on this line, in a context attached to this session.
//! - [`LineClass::CommandOnly`] — a command node starts on this
//!   line, but the seam never fires *at* it: a compound head whose
//!   own line carries no simple command (`if`/`while` with the
//!   condition on a following line), or a command inside a detached
//!   context (background body, process substitution, coproc —
//!   section 6.3: those run where this session's seams do not
//!   reach). The verdict text is the design's "DEBUG trap never
//!   fires on this line".
//! - [`LineClass::NoCommand`] — no command node starts on this
//!   line: blank lines, comment lines, continuation lines, heredoc
//!   body lines, grouping openers and closing keywords on their
//!   own line (`(`, `{`, `do`, `then`, `done`, `}`, `)`). The
//!   verdict text is the design's "no command on this line".
//!
//! Command and process *substitutions* are walked in the context
//! they appear in: their bodies execute in a child executor linked
//! to this session's state (design section 6.3), so their lines are
//! seam lines when the surrounding context is attached. Subshell
//! bodies likewise keep the surrounding context (section 4.1: the
//! trap fires per simple command inside them); only the detached
//! contexts above flip the walk into detached mode.

use std::collections::BTreeSet;

use crate::{Ast, CommandNode};

/// One function definition found by the walk: everything pending
/// function-breakpoint resolution needs to prove a name is defined
/// in this file, and where its body opens (the body-open line is
/// the definition identity the engine itself records for the
/// function-entry seam).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSite {
    pub name: String,
    /// Line of the definition command itself (`name() ...`).
    pub definition_line: u32,
    /// Line of the body's opening `{` when the parser recorded it.
    pub body_open_line: Option<u32>,
}

/// The verdict half of the dead-line judgment for one line of one
/// file (debugger design section 5.1). Resolution maps these to
/// `verified: false` plus the design's two reason strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineClass {
    /// A breakpoint here can stop the session.
    Stoppable,
    /// A command starts here but the seam never fires at it.
    CommandOnly,
    /// No command starts here.
    NoCommand,
}

/// The syntax view of one parsed file: line sets plus function
/// sites, accumulated across every reader unit of the file the
/// engine has parsed so far.
///
/// Files are parsed incrementally, one reader unit at a time, so a
/// view is *partial* while its file is still being read: it knows
/// the units it has seen (their line spans are in `spans`) and
/// nothing sound about lines beyond them. `complete` records that
/// the file's reader reached the end — only then is "no command
/// on this line" a sound verdict for a line no unit covered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxView {
    /// The file identity, exactly as the engine's source stack
    /// reports it.
    pub file: String,
    /// Every line a command node starts on.
    pub command_lines: BTreeSet<u32>,
    /// The subset of `command_lines` the seam can stop on.
    pub stoppable_lines: BTreeSet<u32>,
    /// Function definitions, in first-seen order, deduplicated by
    /// (name, definition line).
    pub functions: Vec<FunctionSite>,
    /// The (first, last) command-line span of each parsed unit:
    /// the line ranges this view's knowledge actually covers.
    pub spans: Vec<(u32, u32)>,
    /// The file's reader reached its end: every line has been
    /// attributed to some unit or to none.
    pub complete: bool,
}

impl SyntaxView {
    /// Build the view of one parsed reader unit of `file`.
    pub fn from_ast(file: &str, ast: &Ast) -> SyntaxView {
        let mut view = SyntaxView {
            file: file.to_string(),
            ..SyntaxView::default()
        };
        {
            let mut walker = Walker { view: &mut view };
            walker.walk_list(&ast.commands, false);
        }
        if let (Some(first), Some(last)) = (
            view.command_lines.iter().next(),
            view.command_lines.iter().next_back(),
        ) {
            view.spans.push((*first, *last));
        }
        view
    }

    /// Fold another unit's view of the same file into this one:
    /// line sets union, function sites append when new, spans
    /// accumulate, completeness is sticky.
    pub fn merge(&mut self, other: SyntaxView) {
        debug_assert_eq!(self.file, other.file);
        self.command_lines.extend(other.command_lines);
        self.stoppable_lines.extend(other.stoppable_lines);
        for site in other.functions {
            if !self
                .functions
                .iter()
                .any(|s| s.name == site.name && s.definition_line == site.definition_line)
            {
                self.functions.push(site);
            }
        }
        self.spans.extend(other.spans);
        self.complete = self.complete || other.complete;
    }

    /// Does this view's knowledge cover `line` — does it fall
    /// inside some parsed unit's span?
    pub fn covers_line(&self, line: u32) -> bool {
        self.spans
            .iter()
            .any(|(first, last)| *first <= line && line <= *last)
    }

    /// Record that the file's reader reached its end.
    pub fn mark_complete(&mut self) {
        self.complete = true;
    }

    /// The dead-line judgment for one line of this file.
    pub fn classify_line(&self, line: u32) -> LineClass {
        if self.stoppable_lines.contains(&line) {
            LineClass::Stoppable
        } else if self.command_lines.contains(&line) {
            LineClass::CommandOnly
        } else {
            LineClass::NoCommand
        }
    }

    /// The function site proving `name` is defined in this file,
    /// when the walk has seen its definition.
    pub fn function_site(&self, name: &str) -> Option<&FunctionSite> {
        self.functions.iter().find(|site| site.name == name)
    }
}

/// The AST walk. `detached` is the section 6.3 context flag: once a
/// subtree runs detached from the session (background body, process
/// substitution, coproc body), its lines are command lines but
/// never stoppable lines, however deeply attached shapes nest
/// inside it.
struct Walker<'a> {
    view: &'a mut SyntaxView,
}

impl Walker<'_> {
    fn record(&mut self, node: &CommandNode, stoppable: bool, detached: bool) {
        if let Some(line) = node_line(node) {
            self.view.command_lines.insert(line);
            if stoppable && !detached {
                self.view.stoppable_lines.insert(line);
            }
        }
    }

    fn walk_list(&mut self, nodes: &[CommandNode], detached: bool) {
        for node in nodes {
            self.walk_node(node, detached);
        }
    }

    fn walk_node(&mut self, node: &CommandNode, detached: bool) {
        // A `&` on the node itself launches the whole command
        // detached (section 6.3), whatever shape it has.
        let detached = detached || node.background;
        // Substitutions inside this node's own words execute as
        // part of the node, in the node's context: process
        // substitutions run in separate processes (detached).
        // (Command substitutions are not stored on the node — the
        // wt44/parse4 record vectors were retired; see nodes.rs.)
        for substitution in &node.process_substitutions {
            self.walk_list(&substitution.commands, true);
        }

        if let Some(pipeline) = &node.pipeline_command {
            self.record(node, false, detached);
            self.walk_list(&pipeline.stages, detached);
            return;
        }
        if let Some(list) = &node.and_or_list {
            self.record(node, false, detached);
            self.walk_list(&list.commands, detached);
            return;
        }
        if let Some(time) = &node.time_command {
            self.record(node, false, detached);
            self.walk_node(&time.command, detached);
            return;
        }
        if let Some(background) = &node.background_command {
            self.record(node, false, detached);
            self.walk_node(&background.command, true);
            return;
        }
        if let Some(inverted) = &node.inverted_command {
            self.record(node, false, detached);
            self.walk_node(&inverted.command, detached);
            return;
        }
        if let Some(for_command) = &node.for_command {
            // The for head carries a seam (section 4.1: once per
            // iteration, before the body) at the head's own line.
            self.record(node, true, detached);
            self.walk_list(&for_command.body, detached);
            return;
        }
        if node.arithmetic_command.is_some() {
            self.record(node, true, detached);
            return;
        }
        if let Some(if_command) = &node.if_command {
            // No seam at the `if` head itself: the seams are the
            // condition and body commands (section 4.1). A head
            // line shared with the first condition command is
            // stoppable through that command, not through the head.
            self.record(node, false, detached);
            self.walk_list(&if_command.condition, detached);
            self.walk_list(&if_command.then_body, detached);
            for branch in &if_command.elif_branches {
                self.walk_list(&branch.condition, detached);
                self.walk_list(&branch.body, detached);
            }
            if let Some(else_body) = &if_command.else_body {
                self.walk_list(else_body, detached);
            }
            return;
        }
        if let Some(loop_command) = &node.loop_command {
            // Same head rule as `if`: the condition commands carry
            // the loop's seam (once per check, section 4.1).
            self.record(node, false, detached);
            self.walk_list(&loop_command.condition, detached);
            self.walk_list(&loop_command.body, detached);
            return;
        }
        if node.conditional_command.is_some() {
            self.record(node, true, detached);
            return;
        }
        if let Some(subshell) = &node.subshell_command {
            // No line of the head's own is recorded: the parser
            // attributes a subshell node's line to its *closing*
            // line, and neither the `(` line nor the `)` line has a
            // command starting on it — the body's commands record
            // their own lines.
            self.walk_list(&subshell.body, detached);
            return;
        }
        if let Some(case) = &node.case_command {
            // The case head carries the seam that evaluates the
            // word and dispatches (section 4.1); clause bodies are
            // ordinary commands at their own lines.
            self.record(node, true, detached);
            for clause in &case.clauses {
                self.walk_list(&clause.body, detached);
            }
            return;
        }
        if let Some(select) = &node.select_command {
            // The select head is a seam like the for head (it is
            // the for family's other member, section 4.1).
            self.record(node, true, detached);
            self.walk_list(&select.body, detached);
            return;
        }
        if let Some(function) = &node.function_command {
            // The definition line is stoppable, but not because a
            // seam fires before the definition executes — none
            // does (the executor's seam admission skips definition
            // commands outright, matching GNU's dispatch). It is
            // stoppable because the function-entry seam reports
            // this line: at entry the engine stamps the current
            // line with the definition line, so a line breakpoint
            // here binds and fires at each call's entry seam.
            self.record(node, true, detached);
            if let Some(line) = node_line(node) {
                let site = FunctionSite {
                    name: function.name.clone(),
                    definition_line: line,
                    body_open_line: function.body_open_line.and_then(line_u32),
                };
                if !self
                    .view
                    .functions
                    .iter()
                    .any(|s| s.name == site.name && s.definition_line == site.definition_line)
                {
                    self.view.functions.push(site);
                }
            }
            self.walk_list(&function.body, detached);
            return;
        }
        if let Some(group) = &node.brace_group {
            // Grouping head, same rule as the subshell: a bare `{`
            // line starts no command; the body records its lines.
            self.walk_list(&group.body, detached);
            return;
        }
        if let Some(coproc) = &node.coproc_command {
            // A coproc body runs in its own process: command lines,
            // never stoppable (section 6.3's detached rule, applied
            // to the one compound shape the table leaves implicit).
            // The simple form (`coproc cmd`) is itself the command,
            // so its line counts; the compound form's node line is
            // attributed like a subshell's and is not recorded.
            if coproc.body.is_none() {
                self.record(node, false, detached);
            }
            if let Some(body) = &coproc.body {
                self.walk_list(body, true);
            }
            return;
        }
        // What remains is a simple command — words, assignments,
        // redirects only, or nothing the walk must descend into.
        // Section 4.1: the seam fires before every simple command,
        // assignments-only commands included.
        self.record(node, true, detached);
    }
}

fn line_u32(line: usize) -> Option<u32> {
    u32::try_from(line).ok().filter(|line| *line > 0)
}

fn node_line(node: &CommandNode) -> Option<u32> {
    node.line.and_then(line_u32)
}

#[cfg(test)]
mod tests {
    use super::{LineClass, SyntaxView};

    fn view_of(source: &str) -> SyntaxView {
        let tokens = crate::lexer::tokenize(source);
        let ast = crate::parser::parse(&tokens);
        SyntaxView::from_ast("fixture.sh", &ast)
    }

    #[test]
    fn simple_commands_are_stoppable_at_their_lines() {
        let view = view_of("echo one\necho two\nX=3\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(3),
            LineClass::Stoppable,
            "assignments-only commands carry a seam (design 4.1)"
        );
    }

    #[test]
    fn blank_comment_and_closing_lines_have_no_command() {
        let view = view_of("# a comment\n\necho hi\n");
        assert_eq!(view.classify_line(1), LineClass::NoCommand);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(99),
            LineClass::NoCommand,
            "a line past the end of the file has no command either"
        );
    }

    #[test]
    fn heredoc_body_lines_have_no_command() {
        let view = view_of("cat <<EOF\nbody one\nbody two\nEOF\necho after\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        assert_eq!(view.classify_line(4), LineClass::NoCommand);
        assert_eq!(view.classify_line(5), LineClass::Stoppable);
    }

    #[test]
    fn continuation_lines_have_no_command_of_their_own() {
        let view = view_of("echo one \\\n    two\necho next\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
    }

    #[test]
    fn if_head_line_is_stoppable_only_through_its_condition() {
        // Condition on the head line: the line stops (through the
        // condition command's own seam).
        let view = view_of("if test -f x; then\n  echo yes\nfi\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        // Condition on the following line: the bare `if` line is a
        // command line the trap never fires at.
        let view = view_of("if\n  test -f x\nthen\n  echo yes\nfi\n");
        assert_eq!(view.classify_line(1), LineClass::CommandOnly);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
    }

    #[test]
    fn do_on_its_own_line_has_no_command() {
        let view = view_of("while read -r x\ndo\n  echo \"$x\"\ndone\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::Stoppable,
            "the condition command shares the head line"
        );
        assert_eq!(view.classify_line(2), LineClass::NoCommand);
        assert_eq!(view.classify_line(3), LineClass::Stoppable);
        assert_eq!(view.classify_line(4), LineClass::NoCommand);
    }

    #[test]
    fn function_definition_registers_site_and_body_lines() {
        let view = view_of("f() {\n  echo in-f\n}\nf\n");
        let site = view.function_site("f").expect("definition recorded");
        assert_eq!(site.definition_line, 1);
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(
            view.classify_line(3),
            LineClass::NoCommand,
            "the closing brace line carries no command node"
        );
        assert_eq!(view.classify_line(4), LineClass::Stoppable);
        assert!(view.function_site("missing").is_none());
    }

    #[test]
    fn detached_contexts_are_command_lines_but_never_stoppable() {
        let view = view_of("sleep 1 &\necho main\n");
        assert_eq!(view.classify_line(1), LineClass::CommandOnly);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        let view = view_of("echo >(cat > /dev/null)\necho main\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::Stoppable,
            "the owning command itself is attached; only the \
             substitution body is detached"
        );
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn subshell_and_comsub_bodies_keep_their_context() {
        let view = view_of("(\n  echo inner\n)\n");
        assert_eq!(
            view.classify_line(1),
            LineClass::NoCommand,
            "a bare `(` line starts no command; the body does"
        );
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
        assert_eq!(view.classify_line(3), LineClass::NoCommand);
        let view = view_of("X=$(echo inner)\necho outer\n");
        assert_eq!(view.classify_line(1), LineClass::Stoppable);
        assert_eq!(view.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn merge_unions_units_of_one_file() {
        let mut first = view_of("f() {\n  echo a\n}\n");
        let second = view_of("f\necho done\n");
        first.merge(second);
        assert!(first.function_site("f").is_some());
        assert_eq!(first.classify_line(1), LineClass::Stoppable);
        assert_eq!(first.classify_line(2), LineClass::Stoppable);
    }

    #[test]
    fn coverage_spans_track_parsed_units_and_completion_is_sticky() {
        let mut view = view_of("echo one\n# mid\necho three\n");
        assert!(view.covers_line(1));
        assert!(
            view.covers_line(2),
            "a line between two commands of one unit is covered"
        );
        assert!(view.covers_line(3));
        assert!(
            !view.covers_line(9),
            "a line past every parsed unit is not covered"
        );
        assert!(!view.complete);
        let other = view_of("echo late\n");
        view.merge(other);
        assert!(!view.complete, "merging units does not complete a file");
        view.mark_complete();
        assert!(view.complete);
        let mut fresh = SyntaxView {
            file: "fixture.sh".to_string(),
            ..SyntaxView::default()
        };
        fresh.merge(view);
        assert!(fresh.complete, "completeness survives a merge");
    }
}
