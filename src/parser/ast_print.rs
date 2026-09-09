//! GNU bash print_cmd.c port: pretty-print a command tree the way bash
//! prints `declare -f` / `type NAME` function bodies.
//!
//! The formatting rules are the ones implemented in
//! third_party/bash/print_cmd.c: the list connector prints `;` + newline
//! inside function definitions, `semicolon()` suppresses the `;` after a
//! newline or `&`, brace groups add no trailing `;` to their last command,
//! heredoc bodies print after the redirection list (headers in place),
//! `for` puts `do` on its own line while `while` keeps `; do` on the test
//! line, `elif` reprints as a nested `if` inside `else`, and command
//! substitutions stored in words are re-serialized from their parsed trees
//! (parse.y does the same at parse time via print_comsub, so
//! `$( echo x )` prints as `$(echo x)`).

use crate::parser::{
    CommandNode, ConditionalCommand, CoprocCommand, ForCommand, HereDocRedirect, IfCommand,
    LoopCommand, Redirect, RedirectKind, SelectCommand, WordMetadata,
};

const INDENTATION_AMOUNT: i32 = 4;

struct DeferredHeredoc {
    body: String,
}

struct Printer {
    out: String,
    indentation: i32,
    /// print_cmd.c keeps a COUNTER (static int inside_function_def): nested
    /// function defs increment while the enclosing def is still printing, so
    /// the flag stays set for the enclosing list's `;`+newline connectors.
    inside_function_def: usize,
    skip_this_indent: usize,
    was_heredoc: bool,
    printing_connection: usize,
    printing_comsub: bool,
    deferred_heredocs: Vec<DeferredHeredoc>,
}

/// Render `name () { body }` the way `declare -f` / `type NAME` print it
/// (named_function_string with FUNC_MULTILINE, no trailing newline).
pub fn multiline_function_def_text(name: &str, body: &[CommandNode]) -> String {
    multiline_function_def_text_with(name, body, None, &[])
}

/// named_function_string port with the function-definition metadata: the
/// body kind (`( ... )` bodies print inside the braces) and the redirections
/// attached to the definition itself, which print after the closing brace
/// (print_cmd.c:1419-1462: `newline ("} "); print_redirection_list`).
pub fn multiline_function_def_text_with(
    name: &str,
    body: &[CommandNode],
    body_kind: Option<crate::parser::FunctionBodyKind>,
    def_redirects: &[Redirect],
) -> String {
    let mut printer = Printer::new();
    // named_function_string: prefix `function ` when the name is not a valid
    // function name (general.c valid_function_name with pflags=0/4 —
    // POSIX_RESTRICT_FUNCNAME is off in the 5.3 build, so only the
    // assignment-word check fires, e.g. `a=2`).
    if is_assignment_word_name(name) {
        printer.cprintf("function ");
    }
    printer.cprintf(&format!("{name} () \n"));
    printer.indent(printer.indentation);
    printer.cprintf("{ \n");
    printer.inside_function_def += 1;
    printer.indentation += INDENTATION_AMOUNT;
    if body_kind == Some(crate::parser::FunctionBodyKind::Subshell) {
        // cm_subshell branch of make_command_string_internal: the indent
        // lands before `( ` and the inner command follows it directly. The
        // definition's redirects belong to the subshell command, so they
        // print after ` )` INSIDE the braces (GNU `u () { ( echo y ) 2>&1 }`).
        printer.indent(printer.indentation);
        printer.cprintf("( ");
        printer.skip_this_indent += 1;
        printer.print_command_list(body);
        printer.print_deferred_heredocs("");
        printer.cprintf(" )");
        if !def_redirects.is_empty() {
            printer.cprintf(" ");
            printer.print_redirect_slice(def_redirects);
        }
        printer.was_heredoc = false;
    } else {
        printer.print_command_list(body);
        printer.print_deferred_heredocs("");
    }
    printer.indentation -= INDENTATION_AMOUNT;
    printer.inside_function_def = printer.inside_function_def.saturating_sub(1);
    if body_kind != Some(crate::parser::FunctionBodyKind::Subshell) {
        if def_redirects.is_empty() {
            printer.newline("}");
        } else {
            printer.newline("} ");
            printer.print_redirect_slice(def_redirects);
        }
    } else {
        printer.newline("}");
    }
    printer.out
}

/// Redirects of a command node in printed order (the Printer's
/// collect_redirects logic exposed for the exportstr roundtrip).
pub fn collected_redirects(cmd: &CommandNode) -> Vec<Redirect> {
    Printer::new().collect_redirects(cmd)
}

/// Render a redirect list the way print_redirection_list does (headers only
/// for heredocs; the exportstr cannot carry deferred bodies inline).
pub fn redirect_list_text(redirects: &[Redirect]) -> String {
    let mut printer = Printer::new();
    printer.print_redirect_slice(redirects);
    printer.out
}

/// general.c assignment(): non-zero when NAME is an assignment word — a
/// legal variable name followed by `=` (possibly `name[...]=` / `name+=`).
/// valid_function_name rejects such names, so named_function_string and
/// print_function_def prefix the `function` keyword when printing them.
fn is_assignment_word_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    let bytes = name.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch == '=' {
            return true;
        }
        if ch == '+' && bytes.get(index + 1) == Some(&b'=') {
            return true;
        }
        if ch == '[' {
            // GNU assignment() skips a subscript and requires `]` then `=`.
            let mut depth = 0usize;
            let mut cursor = index;
            while cursor < bytes.len() {
                if bytes[cursor] == b'[' {
                    depth += 1;
                } else if bytes[cursor] == b']' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                cursor += 1;
            }
            if cursor >= bytes.len() {
                return false;
            }
            index = cursor;
        } else if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return false;
        }
        index += 1;
    }
    false
}

/// print_cmd.c print_command for one parsed command: the canonical text the
/// shell prints for --pretty-print (eval.c:215-253 pretty_print_loop prints
/// this plus one newline per command).
pub fn pretty_print_command(command: &CommandNode) -> String {
    let mut printer = Printer::new();
    printer.make_command_string(command);
    printer.print_deferred_heredocs("");
    printer.out
}

impl Printer {
    fn new() -> Self {
        Self {
            out: String::new(),
            indentation: 0,
            inside_function_def: 0,
            skip_this_indent: 0,
            was_heredoc: false,
            printing_connection: 0,
            printing_comsub: false,
            deferred_heredocs: Vec::new(),
        }
    }

    fn cprintf(&mut self, text: &str) {
        self.out.push_str(text);
    }

    fn indent(&mut self, amount: i32) {
        for _ in 0..amount.max(0) {
            self.out.push(' ');
        }
    }

    /// print_cmd.c newline(): newline, re-indent, then an optional string.
    fn newline(&mut self, tail: &str) {
        self.out.push('\n');
        self.indent(self.indentation);
        if !tail.is_empty() {
            self.cprintf(tail);
        }
    }

    /// print_cmd.c semicolon(): suppress after a newline or after " &".
    fn semicolon(&mut self) {
        if self.out.ends_with('\n') || self.out.ends_with(" &") {
            return;
        }
        self.out.push(';');
    }

    /// make_command_string_internal: indent (unless skipped), then dispatch.
    fn make_command_string(&mut self, cmd: &CommandNode) {
        if self.skip_this_indent > 0 {
            self.skip_this_indent -= 1;
        } else {
            self.indent(self.indentation);
        }
        self.command_body(cmd);
    }

    /// The switch() in make_command_string_internal plus the trailing
    /// redirect list for compound commands.
    fn command_body(&mut self, cmd: &CommandNode) {
        // time [-p] [!] command — CMD_TIME_PIPELINE / CMD_TIME_POSIX prefixes.
        if let Some(time) = &cmd.time_command {
            self.cprintf("time ");
            if time.posix_format {
                self.cprintf("-p ");
            }
            if time.inverted {
                self.cprintf("! ");
            }
            self.skip_this_indent += 1;
            self.make_command_string(&time.command);
            return;
        }
        // `! command` — CMD_INVERT_RETURN prefix.
        if let Some(inverted) = &cmd.inverted_command {
            self.cprintf("! ");
            self.skip_this_indent += 1;
            self.make_command_string(&inverted.command);
            return;
        }
        // `command &` — connection with connector '&'; the space before a
        // following command belongs to the enclosing list logic.
        if let Some(background) = &cmd.background_command {
            self.skip_this_indent += 1;
            self.make_command_string(&background.command);
            self.cprintf(" &");
            return;
        }

        let mut rendered_compound = true;
        if let Some(for_command) = &cmd.for_command {
            self.print_for_command(for_command);
        } else if let Some(case_command) = &cmd.case_command {
            self.print_case_command(case_command);
        } else if let Some(loop_command) = &cmd.loop_command {
            self.print_loop_command(loop_command);
        } else if let Some(if_command) = &cmd.if_command {
            self.print_if_command(if_command);
        } else if let Some(arithmetic) = &cmd.arithmetic_command {
            let expression = match arithmetic.expression.trim() {
                trimmed if !trimmed.is_empty() => trimmed.to_string(),
                _ => arithmetic
                    .raw_expression
                    .as_ref()
                    .map(|raw| raw.trim().to_string())
                    .unwrap_or_default(),
            };
            self.cprintf(&format!("(( {expression} ))"));
        } else if let Some(conditional) = &cmd.conditional_command {
            self.print_conditional_command(conditional);
        } else if let Some(subshell) = &cmd.subshell_command {
            self.cprintf("( ");
            self.skip_this_indent += 1;
            self.print_command_list(&subshell.body);
            self.print_deferred_heredocs("");
            self.cprintf(" )");
            self.was_heredoc = false;
        } else if let Some(group) = &cmd.brace_group {
            self.print_group_command(&group.body);
        } else if let Some(coproc) = &cmd.coproc_command {
            self.print_coproc_command(coproc);
        } else if cmd.function_command.is_some() {
            // Nested function definitions print through print_function_def,
            // which owns the definition's own redirect list (printed after
            // the closing brace) — the generic trailing print below must
            // not repeat it.
            self.print_function_def_node(cmd);
            if cmd.background {
                self.cprintf(" &");
            }
            return;
        } else if let Some(select) = &cmd.select_command {
            self.print_select_command(select);
        } else if let Some(pipeline) = &cmd.pipeline_command {
            self.printing_connection += 1;
            self.skip_this_indent += 1; // the pipeline node consumed the indent
            self.print_pipeline_stages(&pipeline.stages, &pipeline.operators);
            if self.printing_connection == 1 {
                self.print_deferred_heredocs("");
            }
            self.printing_connection -= 1;
        } else if let Some(and_or_list) = &cmd.and_or_list {
            self.printing_connection += 1;
            self.skip_this_indent += 1; // the connection node consumed the indent
            for (index, command) in and_or_list.commands.iter().enumerate() {
                if index > 0 {
                    let and = and_or_list
                        .connectors
                        .get(index - 1)
                        .copied()
                        .unwrap_or(true);
                    if and {
                        self.print_deferred_heredocs(" && ");
                    } else {
                        self.print_deferred_heredocs(" || ");
                    }
                    self.skip_this_indent += 1;
                }
                self.make_command_string(command);
            }
            if self.printing_connection == 1 {
                self.print_deferred_heredocs("");
            }
            self.printing_connection -= 1;
        } else {
            rendered_compound = false;
            self.print_simple_command(cmd);
        }

        // Compound commands print their redirects after the body (the switch
        // tail in make_command_string_internal); simple commands printed
        // theirs right after the words.
        if rendered_compound && self.redirects_present(cmd) {
            self.cprintf(" ");
            self.print_redirection_list(cmd);
        }
        if cmd.background {
            self.cprintf(" &");
        }
    }

    fn redirects_present(&self, cmd: &CommandNode) -> bool {
        !cmd.redirects.is_empty()
            || !cmd.heredoc_redirects.is_empty()
            || cmd.here_string.is_some()
    }

    /// A `;`/newline-separated command list (compound_list). A previous
    /// `command &` turns the connector into `&` (list1 AMPERSAND list1).
    fn print_command_list(&mut self, commands: &[CommandNode]) {
        for (index, command) in commands.iter().enumerate() {
            if index > 0 {
                let previous = &commands[index - 1];
                if previous.background || previous.background_command.is_some() {
                    // The " &" suffix was already printed by the command
                    // itself; the connector only adds the separating space.
                    self.cprintf(" ");
                    self.skip_this_indent += 1;
                    self.make_command_string(command);
                    continue;
                }
                // Connection with connector ';'/newline: the
                // inside_function_def branch of print_cmd.c.
                if self.deferred_heredocs.is_empty() {
                    if self.was_heredoc {
                        self.was_heredoc = false;
                    } else {
                        self.cprintf(";");
                    }
                } else {
                    let connector = if self.inside_function_def > 0 { "" } else { ";" };
                    self.print_deferred_heredocs(connector);
                }
                if self.inside_function_def > 0 {
                    self.cprintf("\n");
                } else if self.printing_comsub {
                    self.cprintf("\n");
                } else {
                    self.cprintf(" ");
                    self.skip_this_indent += 1;
                }
            }
            self.make_command_string(command);
        }
    }

    fn print_pipeline_stages(&mut self, stages: &[CommandNode], operators: &[String]) {
        for (index, stage) in stages.iter().enumerate() {
            if index > 0 {
                let operator = operators.get(index - 1).map(String::as_str).unwrap_or("|");
                let connector = if operator == "|&" { " |&" } else { " |" };
                self.print_deferred_heredocs(connector);
                self.cprintf(" ");
                self.skip_this_indent += 1;
            }
            self.make_command_string(stage);
        }
    }

    fn print_simple_command(&mut self, cmd: &CommandNode) {
        let mut parts: Vec<String> = cmd
            .assignments
            .iter()
            .map(|(name, value)| format!("{name}={}", render_assignment_value(value)))
            .collect();
        // `{name}` words consumed as fd-variable redirect prefixes
        // (`exec {v}>>file`) belong to the redirection, not the word list.
        let fd_vars: Vec<String> = self
            .collect_redirects(cmd)
            .iter()
            .filter_map(|redirect| redirect.fd_var.clone())
            .collect();
        for _index in 0..cmd.words.len() {
            let raw = cmd
                .word_metadata
                .get(_index)
                .filter(|metadata| !metadata.raw.is_empty())
                .map(|metadata| metadata.raw.as_str())
                .unwrap_or_else(|| cmd.words[_index].as_str());
            if let Some(name) = raw
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                if fd_vars.iter().any(|v| v == name) {
                    continue;
                }
            }
            parts.push(self.render_word(cmd, _index));
        }
        self.cprintf(&parts.join(" "));

        if self.redirects_present(cmd) && !parts.is_empty() {
            self.cprintf(" ");
        }
        if self.redirects_present(cmd) {
            self.print_redirection_list(cmd);
        }
    }

    /// One word: prefer the raw source text, with command substitutions
    /// re-serialized from their parsed trees (GNU normalizes them at parse
    /// time; `$( echo x )` must print as `$(echo x)`).
    fn render_word(&mut self, cmd: &CommandNode, index: usize) -> String {
        let metadata = cmd.word_metadata.get(index);
        let word = cmd.words.get(index).cloned().unwrap_or_default();
        self.render_standalone_word(&word, metadata)
    }

    fn render_standalone_word(&mut self, word: &str, metadata: Option<&WordMetadata>) -> String {
        let raw = metadata
            .filter(|metadata| metadata.value == word && !metadata.raw.is_empty())
            .map(|metadata| metadata.raw.clone())
            .unwrap_or_else(|| word.to_string());
        let mut rendered = raw;
        if let Some(metadata) = metadata {
            for node in &metadata.command_substitutions {
                if node.backtick || node.current_shell {
                    continue;
                }
                if node.text.is_empty() || !rendered.contains(&node.text) {
                    continue;
                }
                let replacement = self.comsub_text(node);
                rendered = rendered.replacen(&node.text, &replacement, 1);
            }
        }
        rendered
    }

    /// Re-serialize a command substitution body (print_comsub): newlines in
    /// the source list are preserved, and the body renders with a fresh
    /// printer so surrounding indentation does not leak in.
    fn comsub_text(&mut self, node: &crate::parser::CommandSubstitutionNode) -> String {
        let mut printer = Printer::new();
        printer.printing_comsub = true;
        printer.skip_this_indent += 1; // no leading indent after `$(`
        printer.print_command_list(&node.commands);
        printer.print_deferred_heredocs("");
        let mut text = printer.out;
        while text.starts_with(' ') {
            text.remove(0);
        }
        format!("$({text})")
    }

    fn print_for_command(&mut self, for_command: &ForCommand) {
        if let Some(arithmetic) = &for_command.arithmetic {
            // print_cmd.c print_arith_for_command: cprintf ("for (("), each
            // section's raw text with its leading blanks skipped (expr.c
            // echoes the section text as written between the (( )) markers),
            // "; " separators, cprintf ("))") and no semicolon before the
            // do-line (newline ("do\n") follows directly).
            self.cprintf("for ((");
            self.cprintf(arithmetic.init_metadata.expression.trim_start());
            self.cprintf("; ");
            self.cprintf(arithmetic.test_metadata.expression.trim_start());
            self.cprintf("; ");
            self.cprintf(arithmetic.update_metadata.expression.trim_start());
            self.cprintf("))");
            self.print_do_done_body(&for_command.body);
            return;
        }
        self.cprintf(&format!("for {} in ", for_command.variable));
        for (index, word) in for_command.words.iter().enumerate() {
            if index > 0 {
                self.cprintf(" ");
            }
            let rendered = self.render_standalone_word(word, for_command.word_metadata.get(index));
            self.cprintf(&rendered);
        }
        self.print_loop_body(&for_command.body);
    }

    /// Shared for/select tail: `;`, `do` on its own line, body, `done`.
    /// Shared for/select tail: header semicolon, then the do/done body.
    fn print_loop_body(&mut self, body: &[CommandNode]) {
        self.cprintf(";");
        self.print_do_done_body(body);
    }

    /// print_for_command tail after the header: do-line, body, semicolon,
    /// done (print_cmd.c prints do, the body, a semicolon, then done).
    fn print_do_done_body(&mut self, body: &[CommandNode]) {
        self.newline("do\n");
        self.indentation += INDENTATION_AMOUNT;
        self.print_command_list(body);
        self.print_deferred_heredocs("");
        self.semicolon();
        self.indentation -= INDENTATION_AMOUNT;
        self.newline("done");
    }

    fn print_select_command(&mut self, select: &SelectCommand) {
        self.cprintf(&format!("select {} in ", select.variable));
        for (index, word) in select.words.iter().enumerate() {
            if index > 0 {
                self.cprintf(" ");
            }
            let rendered = self.render_standalone_word(word, select.word_metadata.get(index));
            self.cprintf(&rendered);
        }
        self.print_loop_body(&select.body);
    }

    fn print_loop_command(&mut self, loop_command: &LoopCommand) {
        let keyword = if loop_command.until || loop_command.kind == crate::parser::LoopKind::Until {
            "until"
        } else {
            "while"
        };
        self.cprintf(&format!("{keyword} "));
        self.skip_this_indent += 1;
        self.print_condition_list(&loop_command.condition);
        self.print_deferred_heredocs("");
        self.semicolon();
        if self.was_heredoc {
            self.indent(self.indentation);
            self.cprintf("do\n");
            self.was_heredoc = false;
        } else {
            self.cprintf(" do\n");
        }
        self.indentation += INDENTATION_AMOUNT;
        self.print_command_list(&loop_command.body);
        self.print_deferred_heredocs("");
        self.indentation -= INDENTATION_AMOUNT;
        self.semicolon();
        self.newline("done");
    }

    fn print_if_command(&mut self, if_command: &IfCommand) {
        self.cprintf("if ");
        self.skip_this_indent += 1;
        self.print_condition_list(&if_command.condition);
        self.print_deferred_heredocs("");
        self.semicolon();
        if self.was_heredoc {
            self.indent(INDENTATION_AMOUNT);
            self.cprintf("then\n");
            self.was_heredoc = false;
        } else {
            self.cprintf(" then\n");
        }
        self.indentation += INDENTATION_AMOUNT;
        self.print_command_list(&if_command.then_body);
        self.print_deferred_heredocs("");
        self.indentation -= INDENTATION_AMOUNT;

        self.print_if_tail(if_command);

        self.semicolon();
        self.newline("fi");
    }

    /// `elif` chains reprint the way bash parses them: a complete nested
    /// `if` inside the `else` branch, one indentation level deeper.
    fn print_if_tail(&mut self, if_command: &IfCommand) {
        if let Some((first, rest)) = if_command.elif_branches.split_first() {
            self.semicolon();
            self.newline("else\n");
            self.indentation += INDENTATION_AMOUNT;
            let nested = IfCommand {
                keyword: first.keyword.clone(),
                keyword_metadata: first.keyword_metadata.clone(),
                condition: first.condition.clone(),
                condition_terminator: first.condition_terminator.clone(),
                condition_terminator_metadata: first.condition_terminator_metadata.clone(),
                then_keyword: first.then_keyword.clone(),
                then_keyword_metadata: first.then_keyword_metadata.clone(),
                then_body: first.body.clone(),
                elif_branches: rest.to_vec(),
                else_keyword: if_command.else_keyword.clone(),
                else_keyword_metadata: if_command.else_keyword_metadata.clone(),
                else_body: if_command.else_body.clone(),
                end_keyword: if_command.end_keyword.clone(),
                end_keyword_metadata: if_command.end_keyword_metadata.clone(),
            };
            let nested_node = CommandNode {
                if_command: Some(nested),
                ..CommandNode::new()
            };
            self.make_command_string(&nested_node);
            self.indentation -= INDENTATION_AMOUNT;
        } else if let Some(else_body) = &if_command.else_body {
            self.semicolon();
            self.newline("else\n");
            self.indentation += INDENTATION_AMOUNT;
            self.print_command_list(else_body);
            self.print_deferred_heredocs("");
            self.indentation -= INDENTATION_AMOUNT;
        }
    }

    fn print_case_command(&mut self, case_command: &crate::parser::CaseCommand) {
        let word = if case_command.word_metadata.raw.is_empty() {
            case_command.word.clone()
        } else {
            case_command.word_metadata.raw.clone()
        };
        self.cprintf(&format!("case {word} in "));
        self.print_case_clauses(&case_command.clauses);
        self.newline("esac");
    }

    fn print_case_clauses(&mut self, clauses: &[crate::parser::CaseClause]) {
        self.indentation += INDENTATION_AMOUNT;
        let mut first = true;
        for clause in clauses {
            if !self.printing_comsub || !first {
                self.newline("");
            }
            first = false;
            for (index, pattern) in clause.patterns.iter().enumerate() {
                if index > 0 {
                    self.cprintf(" | ");
                }
                let raw = clause
                    .pattern_nodes
                    .get(index)
                    .filter(|node| !node.raw_text.is_empty())
                    .map(|node| node.raw_text.clone())
                    .unwrap_or_else(|| pattern.clone());
                self.cprintf(&raw);
            }
            self.cprintf(")\n");
            self.indentation += INDENTATION_AMOUNT;
            self.print_command_list(&clause.body);
            self.indentation -= INDENTATION_AMOUNT;
            self.print_deferred_heredocs("");
            self.newline(match clause.terminator {
                crate::parser::CaseTerminator::FallThrough => ";&",
                crate::parser::CaseTerminator::TestNext => ";;&",
                crate::parser::CaseTerminator::Break => ";;",
            });
        }
        self.indentation -= INDENTATION_AMOUNT;
    }

    /// print_group_command: `{ ` ... ` }`. Inside a function definition the
    /// group prints multiline; the body's last command gets no `;`.
    fn print_group_command(&mut self, body: &[CommandNode]) {
        self.cprintf("{ ");
        if self.inside_function_def > 0 {
            self.cprintf("\n");
            self.indentation += INDENTATION_AMOUNT;
        } else {
            self.skip_this_indent += 1;
        }
        self.print_command_list(body);
        self.print_deferred_heredocs("");
        if self.inside_function_def > 0 {
            self.cprintf("\n");
            self.indentation -= INDENTATION_AMOUNT;
            self.indent(self.indentation);
        } else {
            self.semicolon();
            self.cprintf(" ");
        }
        self.cprintf("}");
        self.was_heredoc = false;
    }

    fn print_coproc_command(&mut self, coproc: &CoprocCommand) {
        self.cprintf("coproc ");
        if coproc.body.is_some() {
            if let Some(name) = &coproc.name {
                self.cprintf(&format!("{name} "));
            }
        }
        self.skip_this_indent += 1;
        if let Some(body) = &coproc.body {
            match coproc.body_kind {
                crate::parser::CoprocBodyKind::Subshell => {
                    self.cprintf("( ");
                    self.skip_this_indent += 1;
                    self.print_command_list(body);
                    self.print_deferred_heredocs("");
                    self.cprintf(" )");
                }
                _ => self.print_group_command(body),
            }
        } else {
            for (index, word) in coproc.words.iter().enumerate() {
                if index > 0 {
                    self.cprintf(" ");
                }
                let rendered = self.render_standalone_word(word, coproc.word_metadata.get(index));
                self.cprintf(&rendered);
            }
        }
    }

    fn print_conditional_command(&mut self, conditional: &ConditionalCommand) {
        self.cprintf("[[ ");
        for (index, arg) in conditional.args.iter().enumerate() {
            if index > 0 {
                self.cprintf(" ");
            }
            let rendered = self.render_standalone_word(arg, conditional.arg_metadata.get(index));
            self.cprintf(&rendered);
        }
        // The parser keeps the closing `]]` in args (command_text.rs renders
        // `[[ {args}` verbatim); only add the delimiter when it is absent.
        if conditional.args.last().map(String::as_str) != Some("]]") {
            self.cprintf(" ]]");
        }
    }

    /// Multi-command conditions (`if a; b; then`): GNU stores one
    /// connection chain; join the way `;` connectors render mid-line.
    fn print_condition_list(&mut self, conditions: &[CommandNode]) {
        for (index, condition) in conditions.iter().enumerate() {
            if index > 0 {
                self.cprintf("; ");
            }
            self.make_command_string(condition);
        }
    }

    // ---- redirections ----

    /// Render a redirect slice in order (def-redirect lists and exportstr
    /// roundtrips). Heredoc headers print inline; their bodies cannot be
    /// deferred across a `}` boundary the way GNU's global printer does, so
    /// only the header is emitted.
    fn print_redirect_slice(&mut self, redirects: &[Redirect]) {
        let count = redirects.len();
        for (index, redirect) in redirects.iter().enumerate() {
            if redirect.kind == RedirectKind::HereDoc {
                let info = HereDocRedirect {
                    fd: redirect.fd,
                    fd_var: redirect.fd_var.clone(),
                    operator: redirect.operator.clone(),
                    operator_metadata: redirect.operator_metadata.clone(),
                    delimiter: redirect.target.clone(),
                    delimiter_metadata: redirect.target_metadata.clone(),
                    strip_tabs: redirect.operator.ends_with("<<-"),
                    quoted_delimiter: false,
                    here_string: false,
                    body: None,
                };
                self.cprintf(&heredoc_header(&info));
            } else {
                self.print_redirection(redirect);
            }
            if index + 1 < count {
                self.cprintf(" ");
            }
        }
    }

    /// print_function_def (print_cmd.c:1323): nested `function NAME () ...`
    /// printing. Non-posix mode always prefixes the `function` keyword; the
    /// body indents relative to the enclosing printer state, and the
    /// definition's redirections print after the closing brace.
    fn print_function_def_node(&mut self, cmd: &CommandNode) {
        let Some(function) = cmd.function_command.as_ref() else {
            return;
        };
        self.cprintf(&format!("function {} () \n", function.name));
        self.indent(self.indentation);
        self.cprintf("{ \n");
        self.inside_function_def += 1;
        self.indentation += INDENTATION_AMOUNT;
        if function.body_kind == crate::parser::FunctionBodyKind::Subshell {
            self.indent(self.indentation);
            self.cprintf("( ");
            self.skip_this_indent += 1;
            self.print_command_list(&function.body);
            self.print_deferred_heredocs("");
            self.cprintf(" )");
            let def_redirects = self.collect_redirects(cmd);
            if !def_redirects.is_empty() {
                self.cprintf(" ");
                self.print_redirect_slice(&def_redirects);
            }
            self.was_heredoc = false;
        } else {
            self.print_command_list(&function.body);
            self.print_deferred_heredocs("");
        }
        self.indentation -= INDENTATION_AMOUNT;
        self.inside_function_def = self.inside_function_def.saturating_sub(1);
        if function.body_kind != crate::parser::FunctionBodyKind::Subshell {
            let def_redirects = self.collect_redirects(cmd);
            if def_redirects.is_empty() {
                self.newline("}");
            } else {
                self.newline("} ");
                self.print_redirect_slice(&def_redirects);
            }
        } else {
            self.newline("}");
        }
    }

    fn print_redirection_list(&mut self, cmd: &CommandNode) {
        self.was_heredoc = false;
        let redirects = self.collect_redirects(cmd);
        let count = redirects.len();
        let mut heredoc_index = 0usize;
        for (index, redirect) in redirects.iter().enumerate() {
            if redirect.kind == RedirectKind::HereDoc {
                let info = self.heredoc_info(cmd, redirect, heredoc_index);
                heredoc_index += 1;
                self.cprintf(&heredoc_header(&info));
                let body = heredoc_body_text(cmd, &info);
                self.deferred_heredocs.push(DeferredHeredoc { body });
            } else {
                self.print_redirection(redirect);
            }
            if index + 1 < count {
                self.cprintf(" ");
            }
        }

        if self.deferred_heredocs.is_empty() {
            return;
        }
        if self.printing_connection > 0 {
            return; // bodies flush at the connector
        }
        self.print_heredoc_bodies();
    }

    fn heredoc_info(
        &self,
        cmd: &CommandNode,
        redirect: &Redirect,
        heredoc_index: usize,
    ) -> HereDocRedirect {
        if let Some(info) = cmd.heredoc_redirects.get(heredoc_index) {
            return info.clone();
        }
        HereDocRedirect {
            fd: redirect.fd,
            fd_var: redirect.fd_var.clone(),
            operator: redirect.operator.clone(),
            operator_metadata: redirect.operator_metadata.clone(),
            delimiter: redirect.target.clone(),
            delimiter_metadata: redirect.target_metadata.clone(),
            strip_tabs: redirect.operator.ends_with("<<-"),
            quoted_delimiter: false,
            here_string: false,
            body: cmd.heredoc.clone(),
        }
    }

    fn collect_redirects(&self, cmd: &CommandNode) -> Vec<Redirect> {
        if !cmd.redirects.is_empty() {
            return cmd.redirects.clone();
        }
        let mut redirects = Vec::new();
        if let Some(redirect) = &cmd.redirect_in {
            redirects.push(redirect.clone());
        }
        if let Some(redirect) = &cmd.redirect_out {
            redirects.push(redirect.clone());
        }
        if let Some(redirect) = &cmd.append {
            redirects.push(redirect.clone());
        }
        if let Some(redirect) = &cmd.redirect_err {
            redirects.push(redirect.clone());
        }
        if let Some(redirect) = &cmd.redirect_err_append {
            redirects.push(redirect.clone());
        }
        if let Some(here_string) = &cmd.here_string {
            if !redirects
                .iter()
                .any(|redirect| redirect.kind == RedirectKind::HereString)
            {
                redirects.push(Redirect {
                    fd: None,
                    fd_var: None,
                    operator: "<<<".to_string(),
                    operator_metadata: Box::new(crate::parser::WordMetadata::literal(
                        0,
                        "<<<".to_string(),
                        "<<<".to_string(),
                    )),
                    kind: RedirectKind::HereString,
                    target: here_string.clone(),
                    target_metadata: Box::new(crate::parser::WordMetadata::literal(
                        0,
                        here_string.clone(),
                        here_string.clone(),
                    )),
                    append: false,
                    clobber: false,
                });
            }
        }
        redirects
    }

    /// print_redirection: byte-exact operator rendering per instruction.
    fn print_redirection(&mut self, redirect: &Redirect) {
        let fd_prefix = |redirect: &Redirect, default_fd: u32| -> String {
            if let Some(var) = &redirect.fd_var {
                format!("{{{var}}}")
            } else if let Some(fd) = redirect.fd {
                if fd != default_fd {
                    format!("{fd}")
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        };
        let target = if redirect.target_metadata.raw.is_empty() {
            redirect.target.as_str()
        } else {
            redirect.target_metadata.raw.as_str()
        };
        match redirect.kind {
            RedirectKind::Input => {
                let prefix = fd_prefix(redirect, 0);
                self.cprintf(&format!("{prefix}< {target}"));
            }
            RedirectKind::Output => {
                let prefix = fd_prefix(redirect, 1);
                self.cprintf(&format!("{prefix}> {target}"));
            }
            RedirectKind::ClobberOutput => {
                let prefix = fd_prefix(redirect, 1);
                self.cprintf(&format!("{prefix}>| {target}"));
            }
            RedirectKind::Append => {
                let prefix = fd_prefix(redirect, 1);
                self.cprintf(&format!("{prefix}>> {target}"));
            }
            RedirectKind::ReadWrite => {
                let prefix = fd_prefix(redirect, 0);
                self.cprintf(&format!("{prefix}<> {target}"));
            }
            RedirectKind::HereString => {
                let prefix = fd_prefix(redirect, 0);
                self.cprintf(&format!("{prefix}<<< {target}"));
            }
            RedirectKind::DuplicateInput => {
                if let Some(var) = &redirect.fd_var {
                    let fd = redirect
                        .fd
                        .map(|fd| fd.to_string())
                        .unwrap_or_else(|| target.trim_start_matches('&').to_string());
                    self.cprintf(&format!("{{{var}}}<&{fd}"));
                } else {
                    let redirectee = target.trim_start_matches('&');
                    if redirectee.chars().all(|ch| ch.is_ascii_digit()) {
                        // print_cmd.c r_duplicating_input prints both fds
                        // unconditionally: `<&3` prints as `0<&3`.
                        let fd = redirect.fd.unwrap_or(0);
                        self.cprintf(&format!("{fd}<&{redirectee}"));
                    } else if redirect.fd == Some(0) || redirect.fd.is_none() {
                        // r_duplicating_input_word omits a zero redirector.
                        self.cprintf(&format!("<&{redirectee}"));
                    } else {
                        let fd = redirect.fd.map(|fd| fd.to_string()).unwrap_or_default();
                        self.cprintf(&format!("{fd}<&{redirectee}"));
                    }
                }
            }
            RedirectKind::DuplicateOutput => {
                if let Some(var) = &redirect.fd_var {
                    let fd = redirect
                        .fd
                        .map(|fd| fd.to_string())
                        .unwrap_or_else(|| target.trim_start_matches('&').to_string());
                    self.cprintf(&format!("{{{var}}}>&{fd}"));
                } else {
                    let redirectee = target.trim_start_matches('&');
                    if redirectee.chars().all(|ch| ch.is_ascii_digit()) {
                        // print_cmd.c r_duplicating_output prints both fds
                        // unconditionally: `>&2` prints as `1>&2`.
                        let fd = redirect.fd.unwrap_or(1);
                        self.cprintf(&format!("{fd}>&{redirectee}"));
                    } else if redirect.fd == Some(1) || redirect.fd.is_none() {
                        // r_duplicating_output_word omits a unit redirector.
                        self.cprintf(&format!(">&{redirectee}"));
                    } else {
                        let fd = redirect.fd.map(|fd| fd.to_string()).unwrap_or_default();
                        self.cprintf(&format!("{fd}>&{redirectee}"));
                    }
                }
            }
            RedirectKind::CloseInput => {
                if let Some(var) = &redirect.fd_var {
                    // print_cmd.c r_close_this + REDIR_VARASSIGN always prints
                    // `{var}>&-`, even when the source operator was `<&-`.
                    self.cprintf(&format!("{{{var}}}>&-"));
                } else {
                    let fd = redirect.fd.map(|fd| fd.to_string()).unwrap_or_default();
                    self.cprintf(&format!("{fd}<&-"));
                }
            }
            RedirectKind::CloseOutput => {
                if let Some(var) = &redirect.fd_var {
                    self.cprintf(&format!("{{{var}}}>&-"));
                } else {
                    let fd = redirect.fd.map(|fd| fd.to_string()).unwrap_or_default();
                    self.cprintf(&format!("{fd}>&-"));
                }
            }
            RedirectKind::CombinedOutput => {
                self.cprintf(&format!("&> {target}"));
            }
            RedirectKind::CombinedAppend => {
                self.cprintf(&format!("&>> {target}"));
            }
            RedirectKind::Unknown => {
                // Keep the operator verbatim rather than losing it.
                let fd = redirect.fd.map(|fd| fd.to_string()).unwrap_or_default();
                self.cprintf(&format!("{fd}{} {target}", redirect.operator));
            }
            RedirectKind::HereDoc => {
                // Handled by the caller (headers) and deferred bodies.
            }
        }
    }

    fn print_heredoc_bodies(&mut self) {
        self.cprintf("\n");
        let deferred = std::mem::take(&mut self.deferred_heredocs);
        for heredoc in &deferred {
            self.cprintf(&heredoc.body);
            self.cprintf("\n");
        }
        self.was_heredoc = true;
    }

    /// print_deferred_heredocs(connector): print the connector string, then
    /// any deferred heredoc bodies. `;`-only connectors are swallowed.
    fn print_deferred_heredocs(&mut self, connector: &str) {
        let print_connector = !connector.is_empty()
            && (connector.as_bytes()[0] != b';' || connector.len() > 1);
        if print_connector {
            self.cprintf(connector);
        }
        if !self.deferred_heredocs.is_empty() {
            self.print_heredoc_bodies();
            if print_connector {
                self.cprintf(" ");
            }
        }
        self.deferred_heredocs.clear();
    }
}

fn heredoc_header(info: &HereDocRedirect) -> String {
    let mut prefix = String::new();
    if let Some(var) = &info.fd_var {
        prefix.push_str(&format!("{{{var}}}"));
    } else if let Some(fd) = info.fd {
        prefix.push_str(&format!("{fd}"));
    }
    let dash = if info.strip_tabs { "-" } else { "" };
    if info.quoted_delimiter {
        format!("{prefix}<<{dash}'{}'", info.delimiter)
    } else {
        format!("{prefix}<<{dash}{}", info.delimiter)
    }
}

fn heredoc_body_text(cmd: &CommandNode, info: &HereDocRedirect) -> String {
    let body = info
        .body
        .clone()
        .or_else(|| cmd.heredoc.clone())
        .unwrap_or_default();
    let body = body
        .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
        .unwrap_or(&body)
        .to_string();
    // print_heredoc_body prints the stored body word plus the delimiter.
    format!("{}{}", body, info.delimiter)
}

/// Reconstruct the printed form of an assignment value. The lexer stores a
/// private `\x1c` marker on quoted right-hand sides plus the decoded value;
/// upstream bash prints the word verbatim, so re-quote the way bash would
/// have printed it: double quotes for values containing `$`/backtick,
/// single quotes (with `'"'"'` escaping) for shell metacharacters and
/// control characters, verbatim otherwise.
fn render_assignment_value(value: &str) -> String {
    let Some(quoted_value) = value.strip_prefix('\x1c') else {
        return value.to_string();
    };
    if quoted_value.chars().any(|ch| ch.is_control()) {
        return format!("'{quoted_value}'");
    }
    if quoted_value.contains('$') || quoted_value.contains('`') {
        let escaped = quoted_value
            .replace('"', "\\\"")
            .replace('`', "\\`");
        return format!("\"{escaped}\"");
    }
    let needs_quoting = quoted_value.is_empty()
        || quoted_value.chars().any(|ch| {
            matches!(
                ch,
                ' ' | '\t'
                    | '\n'
                    | '\''
                    | '"'
                    | '\\'
                    | '|'
                    | '&'
                    | ';'
                    | '('
                    | ')'
                    | '<'
                    | '>'
                    | '!'
                    | '{'
                    | '}'
                    | '*'
                    | '['
                    | '?'
                    | ']'
                    | '^'
                    | '~'
                    | '#'
            )
        });
    if needs_quoting {
        format!("'{}'", quoted_value.replace('\'', "'\"'\"'"))
    } else {
        format!("'{quoted_value}'")
    }
}
