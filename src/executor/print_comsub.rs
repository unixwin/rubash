//! GNU print_cmd.c canonical serializer for command-substitution bodies
//! (rubash#274).
//!
//! GNU never keeps the raw `$()` body text in the word: parse.y:4451
//! `parse_comsub` yyparse()s the body and REPLACES it with `print_comsub`'s
//! serialization (parse.y:4632 -> print_cmd.c:152 `make_command_string`
//! with `printing_comsub` set), and every later text exposure —
//! `$BASH_COMMAND` for the enclosing command, `declare -f`, xtrace of the
//! word — carries that canonical form. Rubash executes the raw body (the
//! comsub line model reproduces the canonical numbering instead), so the
//! canonical text is produced here on demand for the text-exposure sites.
//!
//! The layout rules port each print_cmd.c printer verbatim:
//!
//! - `cprintf`/`indent(amount)`/`newline(s)`/`semicolon()` are
//!   print_cmd.c:1495-1528: `newline` prints `\n` + current indentation +
//!   `s`; `semicolon` prints `;` unless the buffer already ends in `\n`
//!   (or ` &`).
//! - A `;` list connector prints `;` + one space; a newline connector
//!   prints the newline itself in comsub mode (`s[0] = printing_comsub ?
//!   c : ';'`, print_cmd.c:290-319) and the next command starts with the
//!   current indentation. Inside a function definition every separator
//!   additionally ends with `\n` (print_cmd.c:309), so `;`-joined
//!   statements each get a line and newline separators leave a blank
//!   line. Which connector applies is the same consume-once newline-run
//!   boundary set the comsub line model uses
//!   (`canonical_newline_boundaries`): a run of physical line breaks only
//!   becomes a newline connector when the statement did not already end
//!   with an explicit `;`/`&`/`&&`/`||`/`|`.
//! - for/select (print_cmd.c:618): `for %s in words;` `\n` `do` `\n`
//!   indent+4 body `;` `\n` `done`. Arith-for (print_cmd.c:635):
//!   `for ((init; test; step))` with the same tail.
//! - while/until (print_cmd.c:812): `while test; do` `\n` indent+4 body
//!   `\n` `done`.
//! - if (print_cmd.c:836): `if test; then` `\n` body; every `elif` prints
//!   as a nested `else` + `if` (the parser has no elif node — the false
//!   case holds a nested if); `fi` per nesting level.
//! - case (print_cmd.c:750): `case word in ` + clauses; in comsub the
//!   first clause's patterns stay on the `in` line (print_cmd.c:771
//!   suppresses the newline so a later re-parse never reads a reserved
//!   word after `in`), the action at indent+4 inside the clause's own
//!   +4, `;;`/`;&`/`;;&` on their own line, `esac` at the end.
//! - function definitions (print_cmd.c:1323): `function name () ` `\n`
//!   `{ ` `\n` indent+4 body `\n` `}` — the `function` prefix is printed
//!   even for `name() { ... }` sources when not posixly_correct
//!   (print_cmd.c:1346-1351; verified against WSL GNU Bash 5.3.0,
//!   target/resid1/p274k.sh).
//! - groups (print_cmd.c:693): `{ body; }` on one line outside function
//!   definitions, braces on their own lines inside one. Subshells
//!   (print_cmd.c:348): `( body )`. Words print their raw token text
//!   (quotes intact, print_cmd.c:586 `command_print_word_list` joined with
//!   single spaces), which is also why run-together source whitespace
//!   collapses (`$(echo    a   b)` -> `$(echo a b)`).
//!
//! Verified against WSL GNU Bash 5.3.0 (2026-09-28,
//! target/resid1/p274{d..l}.sh): simple/for/while/if/elif/case/function/
//! group/subshell/pipeline/redirect shapes byte-identical through ERR- and
//! DEBUG-trap `$BASH_COMMAND` exposure.

use super::*;
use crate::parser::ElifBranch;

/// Canonical `print_comsub` serialization of a `$(...)` body. Re-parses the
/// raw body with the comsub-body tokenizer (the same parse
/// `command_substitution_node` performs) and prints it with
/// [`ComsubPrinter`]. Parse failures degrade to the trimmed raw text — the
/// caller is producing a diagnostic string, and GNU's replacement only
/// exists for bodies that parsed.
pub(in crate::executor) fn print_comsub_text(
    source: &str,
    posix: bool,
    start_line: usize,
) -> String {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tokens = crate::lexer::tokenize_comsub_body(trimmed, posix, start_line.max(1), true);
    let ast = crate::parser::parse(&tokens);
    let boundaries = super::command_substitution::canonical_newline_boundaries(&tokens);
    let mut printer = ComsubPrinter {
        out: String::new(),
        indentation: 0,
        skip_this_indent: 0,
        inside_function_def: 0,
        boundaries,
    };
    printer.print_command_list(&ast.commands);
    printer.out
}

/// Replace every `$()` span of `text` with its canonical print_comsub form,
/// the way parse_comsub rewrites the word at parse time (parse.y:4632).
/// Backquote and `${ ... }` (funsub/valsub) spans keep their raw text — GNU
/// extracts those verbatim (parse.y:3877 parse_matched_pair), never through
/// print_comsub.
pub(in crate::executor) fn canonicalize_comsub_spans(text: &str) -> String {
    if !text.contains("$(") {
        return text.to_string();
    }
    let substitutions = crate::parser::command_substitutions_in_word_public(text);
    if substitutions.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for substitution in &substitutions {
        if substitution.backtick || substitution.current_shell {
            continue;
        }
        let canonical = print_comsub_text(&substitution.source, false, 1);
        if canonical.is_empty() {
            continue;
        }
        out = out.replacen(&substitution.text, &format!("$({canonical})"), 1);
    }
    out
}

struct ComsubPrinter {
    out: String,
    indentation: usize,
    skip_this_indent: usize,
    inside_function_def: usize,
    boundaries: std::collections::HashSet<usize>,
}

impl ComsubPrinter {
    fn cprintf(&mut self, text: &str) {
        self.out.push_str(text);
    }

    /// print_cmd.c:1507 `indent(amount)` — `amount` spaces.
    fn indent(&mut self) {
        let spaces = " ".repeat(self.indentation);
        self.cprintf(&spaces);
    }

    /// print_cmd.c:1495 `newline(string)` — `\n` + indentation + string.
    fn newline(&mut self, text: &str) {
        self.cprintf("\n");
        self.indent();
        self.cprintf(text);
    }

    /// print_cmd.c:1520 `semicolon()` — `;` unless the buffer already ends
    /// in `\n` or ` &`.
    fn semicolon(&mut self) {
        if self.out.ends_with('\n') || self.out.ends_with(" &") {
            return;
        }
        self.cprintf(";");
    }

    fn word_source_text(word: &str, metadata: Option<&WordMetadata>) -> String {
        metadata
            .filter(|metadata| metadata.value == *word && !metadata.raw.is_empty())
            .map(|metadata| metadata.raw.clone())
            .unwrap_or_else(|| word.to_string())
    }

    fn words_source_text(words: &[String], metadata: &[WordMetadata]) -> String {
        words
            .iter()
            .enumerate()
            .map(|(index, word)| Self::word_source_text(word, metadata.get(index)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A command list with print_cmd.c's `;`/newline connector rules (the
    /// `case ';'`/`case '\n'` arms of make_command_string_internal,
    /// print_cmd.c:288-319). The first command's connector belongs to the
    /// caller (the construct that opened the list printed its header).
    fn print_command_list(&mut self, commands: &[CommandNode]) {
        for (index, command) in commands.iter().enumerate() {
            if index > 0 {
                // Consume-once: the boundary belongs to the first command
                // starting on that raw line; a `;`-joined follower on the
                // same line stays on the canonical line (probe g1 of the
                // comsub line model: `a <nl> b1; b2` prints `b1; b2`).
                let newline_connector = command
                    .line
                    .is_some_and(|line| self.boundaries.remove(&line));
                self.print_connector(newline_connector);
            }
            let start = command.line;
            let end = command.end_line;
            self.print_command(command);
            // Drain every boundary inside this command's raw span, exactly
            // like the line model's span draining: a newline-run pointing at
            // the construct's own `do`/`then`/`esac` was consumed by the
            // construct's internal layout and must not become the next
            // sibling's connector (probe g3: `done; follower` stays on the
            // closer's line).
            if let (Some(start), Some(end)) = (start, end.or(start)) {
                self.boundaries.retain(|raw| *raw < start || *raw > end);
            }
        }
    }

    /// The `;` / `\n` connector between two list members (print_cmd.c:288-319):
    /// comsub printing keeps newline connectors as newlines, `;` connectors
    /// print `;` + one space, and inside a function definition every
    /// separator additionally ends with its own newline.
    fn print_connector(&mut self, newline_connector: bool) {
        if newline_connector {
            self.cprintf("\n");
        } else {
            self.cprintf(";");
        }
        if self.inside_function_def > 0 {
            self.cprintf("\n");
        } else if !newline_connector {
            self.cprintf(" ");
        }
    }

    fn print_command(&mut self, command: &CommandNode) {
        if self.skip_this_indent > 0 {
            self.skip_this_indent -= 1;
        } else {
            self.indent();
        }

        if let Some(time_command) = &command.time_command {
            self.cprintf("time ");
            if time_command.posix_format {
                self.cprintf("-p ");
            }
            if time_command.inverted {
                self.cprintf("! ");
            }
            self.skip_this_indent += 1;
            self.print_command(&time_command.command);
            self.print_redirects_of(command);
            return;
        }
        if let Some(inverted) = &command.inverted_command {
            self.cprintf("! ");
            self.skip_this_indent += 1;
            self.print_command(&inverted.command);
            self.print_redirects_of(command);
            return;
        }
        if let Some(background) = &command.background_command {
            self.print_command(&background.command);
            // print_cmd.c `&` connector: ` &` then one space before the next
            // command (`cprintf(" ")` under `c != '&' || second`).
            self.cprintf(" & ");
            self.print_redirects_of(command);
            return;
        }
        if let Some(pipeline) = &command.pipeline_command {
            for (index, stage) in pipeline.stages.iter().enumerate() {
                if index > 0 {
                    self.cprintf(" ");
                    self.skip_this_indent += 1;
                }
                self.print_command(stage);
            }
            self.print_redirects_of(command);
            return;
        }
        if let Some(and_or) = &command.and_or_list {
            for (index, member) in and_or.commands.iter().enumerate() {
                if index > 0 {
                    let connector = and_or.operators.get(index - 1).map(String::as_str);
                    self.cprintf(match connector {
                        Some("||") => " || ",
                        _ => " && ",
                    });
                    self.skip_this_indent += 1;
                }
                self.print_command(member);
            }
            self.print_redirects_of(command);
            return;
        }
        if let Some(for_command) = &command.for_command {
            self.print_for_or_select(
                for_command.variable.as_str(),
                &for_command.words,
                &for_command.word_metadata,
                for_command.default_positional,
                for_command.arithmetic.as_ref(),
                &for_command.body,
            );
            self.print_redirects_of(command);
            return;
        }
        if let Some(select_command) = &command.select_command {
            self.print_for_or_select(
                select_command.variable.as_str(),
                &select_command.words,
                &select_command.word_metadata,
                select_command.default_positional,
                None,
                &select_command.body,
            );
            self.print_redirects_of(command);
            return;
        }
        if let Some(loop_command) = &command.loop_command {
            // print_cmd.c:812 print_until_or_while.
            self.cprintf(if loop_command.until {
                "until "
            } else {
                "while "
            });
            self.skip_this_indent += 1;
            self.print_command_list(&loop_command.condition);
            self.semicolon();
            self.cprintf(" do\n");
            self.indentation += 4;
            self.print_command_list(&loop_command.body);
            self.indentation -= 4;
            self.semicolon();
            self.newline("done");
            self.print_redirects_of(command);
            return;
        }
        if let Some(if_command) = &command.if_command {
            self.print_if_command(if_command);
            self.print_redirects_of(command);
            return;
        }
        if let Some(case_command) = &command.case_command {
            self.print_case_command(case_command);
            self.print_redirects_of(command);
            return;
        }
        if let Some(function_command) = &command.function_command {
            self.print_function_def(function_command);
            self.print_redirects_of(command);
            return;
        }
        if let Some(group) = &command.brace_group {
            // print_cmd.c:693 print_group_command.
            self.cprintf("{ ");
            if self.inside_function_def == 0 {
                self.skip_this_indent += 1;
            } else {
                self.cprintf("\n");
                self.indentation += 4;
            }
            self.print_command_list(&group.body);
            if self.inside_function_def > 0 {
                self.cprintf("\n");
                self.indentation -= 4;
                self.indent();
            } else {
                self.semicolon();
                self.cprintf(" ");
            }
            self.cprintf("}");
            self.print_redirects_of(command);
            return;
        }
        if let Some(subshell) = &command.subshell_command {
            // print_cmd.c:348: `( body )`.
            self.cprintf("( ");
            self.skip_this_indent += 1;
            self.print_command_list(&subshell.body);
            self.cprintf(" )");
            self.print_redirects_of(command);
            return;
        }
        if let Some(coproc) = &command.coproc_command {
            // print_cmd.c:349-356: `coproc ` + NAME (unless the body is a
            // simple command whose words carry it) + body.
            self.cprintf("coproc ");
            let body_is_simple = coproc.body.is_none();
            if !body_is_simple {
                if let Some(name) = &coproc.name {
                    self.cprintf(&format!("{name} "));
                }
            }
            self.skip_this_indent += 1;
            match &coproc.body {
                Some(body) => self.print_command_list(body),
                None => {
                    let text = Self::words_source_text(&coproc.words, &coproc.word_metadata);
                    self.cprintf(&text);
                }
            }
            self.print_redirects_of(command);
            return;
        }
        if let Some(arithmetic) = &command.arithmetic_command {
            // print_cmd.c:879 print_arith_command: `((expression))`.
            let expression = arithmetic
                .raw_expression
                .as_deref()
                .unwrap_or(&arithmetic.expression);
            self.cprintf(&format!("(({expression}))"));
            self.print_redirects_of(command);
            return;
        }
        if let Some(conditional) = &command.conditional_command {
            // print_cmd.c print_cond_command prints the [[ ]] expression
            // tree; the raw arg sequence carries the operator text.
            let text = conditional
                .arg_metadata
                .iter()
                .enumerate()
                .map(|(index, metadata)| {
                    Self::word_source_text(
                        conditional
                            .args
                            .get(index)
                            .map(String::as_str)
                            .unwrap_or(""),
                        Some(metadata),
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            self.cprintf(&format!("[[ {text} ]]"));
            self.print_redirects_of(command);
            return;
        }

        // cm_simple (print_cmd.c:578 print_simple_command): GNU's word list
        // carries the leading assignment words; emit them first, then the
        // command words, then the redirection list.
        let mut parts = Vec::new();
        for (index, (name, value)) in command.assignments.iter().enumerate() {
            let raw = command
                .assignment_raws
                .get(index)
                .filter(|raw| !raw.is_empty())
                .unwrap_or(value);
            let text = format!("{name}={raw}");
            parts.push(canonicalize_comsub_spans(&text));
        }
        for (index, word) in command.words.iter().enumerate() {
            let text = Self::word_source_text(word, command.word_metadata.get(index));
            parts.push(canonicalize_comsub_spans(&text));
        }
        self.cprintf(&parts.join(" "));
        self.print_redirects_of(command);
    }

    /// print_cmd.c:618 print_for_command / print_select_command /
    /// print_cmd.c:635 print_arith_for_command.
    fn print_for_or_select(
        &mut self,
        variable: &str,
        words: &[String],
        metadata: &[WordMetadata],
        default_positional: bool,
        arithmetic: Option<&ArithmeticForCommand>,
        body: &[CommandNode],
    ) {
        let keyword = if arithmetic.is_some() { "for" } else { "for " };
        let _ = keyword; // both spellings are `for`
        match arithmetic {
            Some(arithmetic) => {
                self.cprintf(&format!(
                    "for (({}; {}; {}))",
                    arithmetic.init, arithmetic.test, arithmetic.update
                ));
            }
            None => {
                let list = if default_positional {
                    "\"$@\"".to_string()
                } else {
                    Self::words_source_text(words, metadata)
                };
                self.cprintf(&format!("for {variable} in {list}"));
                self.cprintf(";");
            }
        }
        self.newline("do\n");
        self.indentation += 4;
        self.print_command_list(body);
        self.semicolon();
        self.indentation -= 4;
        self.newline("done");
    }

    /// print_cmd.c:836 print_if_command. `elif` chains print as nested
    /// `else` + `if` nodes — the parser's false_case holds the nested if,
    /// with the trailing `else` body at the innermost level.
    fn print_if_command(&mut self, if_command: &IfCommand) {
        self.cprintf("if ");
        self.skip_this_indent += 1;
        self.print_command_list(&if_command.condition);
        self.semicolon();
        self.cprintf(" then\n");
        self.indentation += 4;
        self.print_command_list(&if_command.then_body);
        self.indentation -= 4;
        // print_cmd.c:869-878: the before-else `semicolon()` lives INSIDE
        // the false_case branch, and the trailing `semicolon(); newline
        // ("fi")` always runs — a no-else `if` therefore has exactly one
        // `;` (before `fi`), and a nested elif chain gets one before the
        // `else` and one after the inner `fi` (it is the else body's list
        // member).
        if !if_command.elif_branches.is_empty() {
            self.print_elif_chain(if_command);
        } else if let Some(else_body) = &if_command.else_body {
            self.semicolon();
            self.newline("else\n");
            self.indentation += 4;
            self.print_command_list(else_body);
            self.indentation -= 4;
        }
        self.semicolon();
        self.newline("fi");
    }

    fn print_elif_chain(&mut self, if_command: &IfCommand) {
        let Some(first) = if_command.elif_branches.first() else {
            return;
        };
        self.semicolon();
        self.newline("else\n");
        self.indentation += 4;
        // The nested `if` is the else body's command: it starts with the
        // current indentation (make_command_string_internal's indent()).
        self.indent();
        self.cprintf("if ");
        self.skip_this_indent += 1;
        self.print_command_list(&first.condition);
        self.semicolon();
        self.cprintf(" then\n");
        self.indentation += 4;
        self.print_command_list(&first.body);
        self.indentation -= 4;
        let rest = ElifChainRest {
            elifs: &if_command.elif_branches[1..],
            else_body: if_command.else_body.as_deref(),
        };
        self.print_elif_rest(&rest);
        self.semicolon();
        self.newline("fi");
        self.indentation -= 4;
    }

    fn print_elif_rest(&mut self, rest: &ElifChainRest<'_>) {
        if let Some(next) = rest.elifs.first() {
            self.semicolon();
            self.newline("else\n");
            self.indentation += 4;
            self.indent();
            self.cprintf("if ");
            self.skip_this_indent += 1;
            self.print_command_list(&next.condition);
            self.semicolon();
            self.cprintf(" then\n");
            self.indentation += 4;
            self.print_command_list(&next.body);
            self.indentation -= 4;
            let inner = ElifChainRest {
                elifs: &rest.elifs[1..],
                else_body: rest.else_body,
            };
            self.print_elif_rest(&inner);
            self.semicolon();
            self.newline("fi");
            self.indentation -= 4;
        } else if let Some(else_body) = rest.else_body {
            self.semicolon();
            self.newline("else\n");
            self.indentation += 4;
            self.print_command_list(else_body);
            self.indentation -= 4;
        }
    }

    /// print_cmd.c:750 print_case_command + print_case_clauses.
    fn print_case_command(&mut self, case_command: &CaseCommand) {
        let word = Self::word_source_text(&case_command.word, Some(&case_command.word_metadata));
        self.cprintf(&format!("case {word} in "));
        let mut first = true;
        self.indentation += 4;
        for clause in &case_command.clauses {
            // print_cmd.c:771: in comsub printing the first clause's
            // patterns stay on the `case w in ` line — a newline there
            // would let the re-parse misread a reserved word after `in`.
            if !first {
                self.newline("");
            }
            first = false;
            let patterns = clause
                .patterns
                .iter()
                .enumerate()
                .map(|(index, pattern)| {
                    // Patterns keep their raw token text (quotes intact),
                    // the same word contract as every other printer.
                    clause
                        .pattern_nodes
                        .get(index)
                        .map(|node| node.raw_text.trim().to_string())
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| pattern.clone())
                })
                .collect::<Vec<_>>()
                .join(" | ");
            // An `esac` pattern only parses behind an opening paren
            // (print_cmd.c:782-787).
            let paren = if clause.patterns.first().is_some_and(|p| p == "esac") {
                "("
            } else {
                ""
            };
            self.cprintf(&format!("{paren}{patterns})\n"));
            self.indentation += 4;
            self.print_command_list(&clause.body);
            self.indentation -= 4;
            match clause.terminator {
                CaseTerminator::FallThrough => self.newline(";&"),
                CaseTerminator::TestNext => self.newline(";;&"),
                CaseTerminator::Break => self.newline(";;"),
            }
        }
        self.indentation -= 4;
        self.newline("esac");
    }

    /// print_cmd.c:1323 print_function_def.
    fn print_function_def(&mut self, function_command: &FunctionCommand) {
        // print_cmd.c:1346-1351: without posixly_correct GNU ALWAYS prints
        // the `function` prefix, even for `name() { ... }` sources.
        self.cprintf(&format!("function {} () \n", function_command.name));
        self.indent();
        self.cprintf("{ \n");
        self.inside_function_def += 1;
        self.indentation += 4;
        self.print_command_list(&function_command.body);
        self.indentation -= 4;
        self.inside_function_def -= 1;
        self.newline("}");
    }

    fn print_redirects_of(&mut self, command: &CommandNode) {
        if command.redirects.is_empty() && command.heredoc_redirects.is_empty() {
            return;
        }
        self.cprintf(" ");
        self.print_redirect_list(command);
    }

    /// print_cmd.c:1070 print_redirection_list — redirections joined by
    /// single spaces; here-documents print their header in place
    /// (print_cmd.c:1118 print_heredoc_header, `<<'DELIM'` with no space)
    /// and their body block right after (print_cmd.c:1035
    /// print_heredoc_bodies: `\n` + body lines + terminator + `\n`).
    fn print_redirect_list(&mut self, command: &CommandNode) {
        let mut parts = Vec::new();
        for redirect in &command.redirects {
            // The parser mirrors each here-document in the ordered
            // redirects list; heredoc_redirects is the richer record (it
            // carries the gathered body), so print here-documents from
            // there only.
            if matches!(redirect.kind, crate::parser::RedirectKind::HereDoc)
                && !command.heredoc_redirects.is_empty()
            {
                continue;
            }
            parts.push(self.redirect_text(redirect));
        }
        for heredoc in &command.heredoc_redirects {
            let mut text = String::new();
            if let Some(fd) = heredoc.fd.filter(|fd| *fd != 0) {
                text.push_str(&fd.to_string());
            }
            text.push_str(heredoc.operator.trim_end());
            if heredoc.quoted_delimiter {
                text.push('\'');
                text.push_str(&heredoc.delimiter);
                text.push('\'');
            } else {
                text.push_str(&heredoc.delimiter);
            }
            parts.push(text);
        }
        self.cprintf(&parts.join(" "));
        // Here-document bodies print after the redirection line
        // (print_cmd.c:1035-1043): body lines then the terminator on its
        // own line.
        for heredoc in &command.heredoc_redirects {
            let Some(body) = &heredoc.body else {
                continue;
            };
            let decoded = crate::executor::execution_misc::decode_stdin_body_enq(body);
            let mut body_text = decoded.as_str();
            // Strip the transport markers the gathered body carries: the
            // quoted-delimiter tag and the unterminated-EOF flag.
            loop {
                let stripped = body_text
                    .strip_prefix(crate::lexer::QUOTED_HEREDOC_MARKER)
                    .or_else(|| body_text.strip_prefix(crate::executor::markers::DATA_DOLLAR_STR));
                match stripped {
                    Some(rest) => body_text = rest,
                    None => break,
                }
            }
            // The gathered body carries the terminator line; GNU prints
            // body + here_doc_eof separately (print_cmd.c:1143-1147).
            let body_text = body_text
                .strip_suffix(&format!("\n{}", heredoc.delimiter))
                .unwrap_or(body_text);
            self.cprintf("\n");
            self.cprintf(body_text);
            self.cprintf(&heredoc.delimiter);
            self.cprintf("\n");
        }
    }

    fn redirect_text(&mut self, redirect: &Redirect) -> String {
        // The ordered `redirects` list carries the fd INSIDE the operator
        // (`2>`, `2>&`, `<`), exactly as print_cmd.c prints it; only a
        // `{var}` dynamic redirector lives in its own field
        // (print_cmd.c:1150-1260 `{%s}` prefixes).
        let mut prefix = String::new();
        if let Some(fd_var) = &redirect.fd_var {
            if !redirect.operator.contains('{') {
                prefix.push_str(&format!("{{{fd_var}}}"));
            }
        }
        let target = Self::word_source_text(&redirect.target, Some(&redirect.target_metadata));
        // Word-taking redirections print `op target`; fd duplications print
        // the operand glued to the operator (print_cmd.c:1150-1260). The
        // stored operator carries the fd (`2>&`) and the target carries the
        // leading `&` (`&1`); print exactly one of each.
        let glued_target = target.strip_prefix('&').unwrap_or(&target).to_string();
        let fd_digit = if redirect.operator.starts_with(|c: char| c.is_ascii_digit()) {
            String::new()
        } else {
            redirect.fd.map(|fd| fd.to_string()).unwrap_or_default()
        };
        match redirect.kind {
            crate::parser::RedirectKind::DuplicateInput
            | crate::parser::RedirectKind::DuplicateOutput => {
                format!("{prefix}{fd_digit}{}{glued_target}", redirect.operator)
            }
            crate::parser::RedirectKind::CloseInput | crate::parser::RedirectKind::CloseOutput => {
                format!("{prefix}{}", redirect.operator)
            }
            crate::parser::RedirectKind::CombinedOutput
            | crate::parser::RedirectKind::CombinedAppend => {
                format!("{prefix}{}", redirect.operator)
            }
            _ => format!("{prefix}{} {target}", redirect.operator),
        }
    }
}

/// The not-first slice of an elif chain plus the trailing else body, for the
/// recursive nested-else printing.
struct ElifChainRest<'a> {
    elifs: &'a [ElifBranch],
    else_body: Option<&'a [CommandNode]>,
}
