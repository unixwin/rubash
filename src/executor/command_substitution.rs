use super::*;
use crate::executor::markers::DATA_DOLLAR;

// Whether the command-substitution body currently being parsed came from a
// backquote. GNU extracts `` `...` `` bodies verbatim (parse.y:3877
// parse_matched_pair — the backquote matcher) instead of re-serializing
// them through print_comsub (which only `$(`, `${` and `<(` bodies get,
// parse.y:4451 parse_comsub), so a backquote body's diagnostics number its
// RAW physical lines under the same (S - 1) base — probed as b1/b2:
// `` echo "`for f in 1 2; do\n nosuchcmd\ndone`" `` reports the raw
// nosuchcmd line, not the canonical one.
thread_local! {
    static COMSUB_BODY_RAW_TEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with backquote body semantics: command-substitution bodies
/// parsed inside keep their raw line numbering (no canonical layout).
pub(in crate::executor) fn with_comsub_raw_text<T>(f: impl FnOnce() -> T) -> T {
    let saved = COMSUB_BODY_RAW_TEXT.replace(true);
    let result = f();
    COMSUB_BODY_RAW_TEXT.set(saved);
    result
}

/// Canonical line layout for a parsed command-substitution body so every
/// diagnostic and `$LINENO` inside the body lands on GNU's line (rubash#136,
/// rubash#201 comsub-string half).
///
/// GNU does not execute the raw body text: at parse time `parse_comsub`
/// (parse.y:4451) fully yyparse()s the body and REPLACES it with the
/// canonical serialization of `print_comsub` (parse.y:4632 ->
/// print_cmd.c:164 make_command_string). At expansion time
/// `command_substitute` (subst.c:7143) hands that canonical text to
/// `parse_and_execute` (builtins/evalstring.c:315), which pushes the string
/// stream (evalstring.c:340 `push_stream(0)`, saving the enclosing command's
/// `line_number` stamped by execute_cmd.c:936 `SET_LINE_NUMBER(Simple->line)`)
/// and compensates `line_number--` (evalstring.c:345-346) so the string's
/// first line re-uses the enclosing command's line. The string reader then
/// counts one line per canonical newline (parse.y:2504 shell_getc line
/// fetch). Net attribution for anything inside the body:
///
///   reported line = (S - 1) + canonical_line
///
/// where S is the enclosing command's `Simple->line` — the line where its
/// FIRST WORD finished reading (parse.y:5851 `simplecmd_lineno =
/// line_number` at read_token_word end), i.e. the first word's end line for
/// multi-line assignment/quoted first words — and canonical_line is the
/// position in the print_cmd.c serialization:
///
/// - for/select (`print_for_command` print_cmd.c:618): `for v in w;` on the
///   head line, `do` on head+1, body from head+2, `done` at body_end+1;
///   arith-for (print_cmd.c:635) has the same shape.
/// - while/until (`print_until_or_while` print_cmd.c:812): `while test; do`
///   — `do` stays on the condition's last line, body at +1, `done` at
///   body_end+1.
/// - if (`print_if_command` print_cmd.c:836): `if test; then` — `then` on
///   the condition's last line, body at +1; every `elif` prints as a nested
///   `else\nif cond; then` (one elif puts the outer `fi` at body_end+2),
///   `else` at prev_body_end+1 with its body at +1, one `fi` line per
///   nesting level.
/// - case (`print_case_command` print_cmd.c:750 + print_case_clauses
///   print_cmd.c:760): the first clause's patterns share the `case w in `
///   line (comsub printing suppresses the leading newline, print_cmd.c:771),
///   action at patterns+1, `;;` at action_end+1, later clause patterns at
///   prev `;;`+1, `esac` at last terminator+1.
/// - function definitions (`print_function_def` print_cmd.c:1323): `name ()`
///   on line L, `{` on L+1, body from L+2, closing `}` at body_end+1; the
///   function-body environment line is the `{` line (execute_cmd.c:5351
///   `line_number = function_line_number = tc->line`). INSIDE the body
///   print_cmd.c sets inside_function_def (print_cmd.c:1362), and the
///   connector case then ends EVERY statement separator with `cprintf
///   ("\n")` (print_cmd.c:309): a `;`-joined statement gets its own line
///   and a newline separator leaves a blank line (probes f1/f2 — GNU
///   reports the second `;`-joined statement one line further and a
///   newline-separated one two lines further than outside a function).
///   This applies to every list nested in the body, including for/if/case
///   bodies, and `{ ... }` groups print their braces on separate lines
///   (print_cmd.c:702-712), while `( ... )` subshells keep the one-line
///   form (print_cmd.c:348 has no funcdef branch).
/// - `{ ...; }` groups (`print_group_command` print_cmd.c:693) and `( ... )`
///   subshells (print_cmd.c:348) print braces and body on ONE canonical line.
/// - statement separators: an explicit `;`/`&`/`&&`/`||`/`|` connector joins
///   the next command onto the same canonical line; a bare newline moves to
///   the next line (print_cmd.c:288-319 connector printing); runs of blank
///   lines collapse because blank lines are not in the parse tree. A
///   newline boundary is consumed by the first command starting on its raw
///   line; a `;`-joined follower on the same raw line stays put (probe g1:
///   `a <nl> b1; b2` reports b1 and b2 on the same line), and boundaries
///   pointing inside a construct's own span (its `do`/`done`, a redirect
///   target after a newline) never bump the next sibling (probe g3:
///   `done; follower` stays on the closer's line).
/// - here-documents (print_cmd.c:120 PRINT_DEFERRED_HEREDOCS,
///   print_cmd.c:1035-1043 print_heredoc_bodies): the body block prints
///   after the command as a leading `\n`, the B body lines and the
///   terminator on its own line, so the next statement lands at
///   terminator+1 (`;` join) or terminator+2 with a blank line (newline
///   join) — probes h1-h3: GNU reports cat+B+3 for a newline-separated
///   follower.
///
/// Verified against WSL GNU Bash 5.3.0 (2026-09-27,
/// target/deep201/probes/{p*,q*,v*,x*,y*,z*,e*,f*,g*,h*}.sh): for-body errors report
/// the canonical body line (p04: GNU 3; raw text numbering says 2), a
/// comsub opened on a later line of its word still numbers from the
/// ENCLOSING command's line (p11: GNU 3), `;`+newline list separators
/// collapse (v5 / issue #201 repro A: GNU 5), assignments take the
/// first-word END line as base (q4 A2/B3, q7 P5, q9 line 2), elif chains
/// follow the nested-else layout (e1 Q7, e2 Q9, e6 E8/Q11), case
/// clauses follow the first-pattern-on-case-line layout (e4 C2/Q4, e5
/// C2/Q7), function-definition bodies follow the per-statement-newline
/// layout (f1/f2/f3/f6: `;`-join +1, newline +2), and same-line joins
/// after a newline boundary keep the boundary's line (g1/g2/g5).
pub(in crate::executor) fn normalize_comsub_body_statement_lines(
    ast: &mut crate::parser::Ast,
    tokens: &[crate::lexer::Token],
) {
    // Backquote bodies keep GNU's raw line numbering (see
    // COMSUB_BODY_RAW_TEXT above); the seed-based raw numbering the callers
    // already applied is GNU's answer for them.
    if COMSUB_BODY_RAW_TEXT.get() {
        return;
    }
    let Some(first) = tokens.first() else {
        return;
    };
    let seed = first.position.max(1);
    let mut boundaries = canonical_newline_boundaries(tokens);
    let mut line = 1usize;
    // The body's first command is canonical line 1 regardless of any
    // leading blank lines in the raw text (blank lines have no node).
    layout_command_list(&mut ast.commands, seed, &mut boundaries, &mut line, false);
}

/// Raw source lines that a genuine newline-run statement boundary crosses
/// INTO (the raw line of the first token after the run). Mirrors the
/// connector printing of print_cmd.c:294-319: a run of physical line breaks
/// only becomes a canonical newline when the statement did not already end
/// with an explicit connector (`;` `&` `&&` `||` `|`) — `a; <newline> b`
/// re-prints as `a; b` on one line — and a run of blank lines collapses to
/// one boundary because blank lines carry no parse-tree node. Shared by the
/// comsub line model and the print_comsub text serializer (rubash#274).
pub(in crate::executor) fn canonical_newline_boundaries(
    tokens: &[crate::lexer::Token],
) -> std::collections::HashSet<usize> {
    use crate::lexer::TokenKind;
    let is_connector = |kind: &TokenKind| {
        matches!(
            kind,
            TokenKind::Pipe
                | TokenKind::PipeErr
                | TokenKind::And
                | TokenKind::Or
                | TokenKind::Background
                | TokenKind::Semicolon
        )
    };
    let is_line_break_separator =
        |kind: &TokenKind, line_break: bool| kind == &TokenKind::Semicolon && line_break;

    let mut boundaries = std::collections::HashSet::new();
    let mut previous: Option<&crate::lexer::Token> = None;
    let mut pending = false;
    for token in tokens {
        if token.kind == TokenKind::HereDocBody {
            // The here-document body is part of the PRECEDING command's
            // canonical footprint (print_cmd.c:1035-1043
            // print_heredoc_bodies), not a statement boundary; the newline
            // after the terminator closes it and starts the next run.
            previous = Some(token);
            continue;
        }
        if is_line_break_separator(&token.kind, token.line_break) {
            let starts_run = previous.is_some_and(|prev| {
                !is_connector(&prev.kind) && !is_line_break_separator(&prev.kind, prev.line_break)
            });
            if starts_run {
                pending = true;
            }
            previous = Some(token);
            continue;
        }
        if pending {
            boundaries.insert(token.position);
            pending = false;
        }
        previous = Some(token);
    }
    boundaries
}

/// Assign the canonical line to every command of a (possibly compound-body)
/// list. `line` is the 1-based canonical line where the list's first
/// command starts (the caller has applied the construct's fixed offset);
/// the first command never bumps (its position is fixed by the construct's
/// shape, print_cmd.c prints it right after the construct's header).
///
/// `funcdef` is print_cmd.c's inside_function_def state (set while printing
/// a function-definition body, print_cmd.c:1362/1380): inside it every
/// statement separator — even an explicit `;` — is followed by `cprintf
/// ("\n")` (print_cmd.c:309), so `;`-joined statements each get their own
/// canonical line and newline separators leave a blank line.
fn layout_command_list(
    commands: &mut [crate::parser::CommandNode],
    seed: usize,
    boundaries: &mut std::collections::HashSet<usize>,
    line: &mut usize,
    funcdef: bool,
) {
    let mut prev_heredoc = false;
    for (index, command) in commands.iter_mut().enumerate() {
        if index > 0 {
            // Consume-once: the boundary crossing into this command's raw
            // line belongs to the first command on that line; a `;`-joined
            // follower on the same raw line stays on the canonical line
            // (probe g1: `a <nl> b1; b2` prints b1,b2 on one line).
            let had_newline = command.line.is_some_and(|raw| boundaries.remove(&raw));
            // Here-document follower (print_cmd.c:120-137
            // print_deferred_heredocs + print_cmd.c:294-319 connector): the
            // body block ends with its own `\n` and was_heredoc suppresses
            // the connector's own separator, so the follower lands one line
            // past the terminator line, plus one more only through the
            // non-funcdef preserve-newline branch (print_cmd.c:315) — the
            // funcdef branch (print_cmd.c:309) already printed its newline.
            let bump = if prev_heredoc {
                1 + funcdef as usize + (!funcdef && had_newline) as usize
            } else {
                funcdef as usize + had_newline as usize
            };
            *line += bump;
        }
        let raw_start = command.line;
        let raw_end = command.end_line;
        prev_heredoc = command
            .heredoc_redirects
            .iter()
            .any(|redirect| redirect.body.is_some());
        layout_command(command, seed, boundaries, line, funcdef);
        // Drain every boundary inside this command's raw span: boundaries
        // that point at internal keywords (`do`, `done`, `then`, `fi`,
        // `esac`) or at continuation words (a redirect target after a
        // newline, probe g7) were consumed by the construct's own layout
        // and must not bump the next sibling (probe g3: `done; follower`
        // stays on the closer's canonical line).
        if let (Some(start), Some(end)) = (raw_start, raw_end.or(raw_start)) {
            boundaries.retain(|raw| *raw < start || *raw > end);
        }
    }
}

/// Number of newlines embedded in a simple command's printed text:
/// print_cmd.c prints words, assignments and redirects on one canonical
/// line, so a quoted multi-line word advances the layout by its newlines.
/// Here-document bodies are accounted separately (they print as a block
/// after the connector, print_cmd.c:1035-1043).
fn simple_command_embedded_newlines(command: &crate::parser::CommandNode) -> usize {
    let mut count = 0usize;
    for word in &command.words {
        count += word.matches('\n').count();
    }
    for (index, (_, value)) in command.assignments.iter().enumerate() {
        let raw = command
            .assignment_raws
            .get(index)
            .filter(|raw| !raw.is_empty())
            .unwrap_or(value);
        count += raw.matches('\n').count();
    }
    count
}

fn assign(line_field: &mut Option<usize>, seed: usize, canonical: usize) {
    *line_field = Some(seed - 1 + canonical);
}

/// Layout one command node at canonical line `*line`, leaving `*line` at
/// the construct's closing line (where `done`/`fi`/`esac`/`}`/`)` or the
/// command's last word lands). `funcdef` is print_cmd.c's
/// inside_function_def state, active for the whole subtree of a function
/// definition's body (print_cmd.c:1362/1380).
fn layout_command(
    command: &mut crate::parser::CommandNode,
    seed: usize,
    boundaries: &mut std::collections::HashSet<usize>,
    line: &mut usize,
    funcdef: bool,
) {
    let start = *line;
    assign(&mut command.line, seed, start);

    if let Some(for_command) = command.for_command.as_deref_mut() {
        // Head words may carry quoted newlines; `do` sits on head_end+1 and
        // the body starts on the line after `do` (print_cmd.c:618-631).
        // A brace-group for-body (`for v in x { body; }`) prints its braces
        // on that same body line, so the fixed offset covers both forms.
        let head_newlines = for_command
            .words
            .iter()
            .map(|word| word.matches('\n').count())
            .sum::<usize>();
        *line = start + head_newlines + 2;
        layout_command_list(&mut for_command.body, seed, boundaries, line, funcdef);
        *line += 1; // `done` at body_end + 1
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(select_command) = command.select_command.as_deref_mut() {
        let head_newlines = select_command
            .words
            .iter()
            .map(|word| word.matches('\n').count())
            .sum::<usize>();
        *line = start + head_newlines + 2;
        layout_command_list(&mut select_command.body, seed, boundaries, line, funcdef);
        *line += 1; // `done`
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(loop_command) = command.loop_command.as_mut() {
        // `while test; do` — `do` shares the condition's last line
        // (print_cmd.c:826 ` do\n`).
        layout_command_list(&mut loop_command.condition, seed, boundaries, line, funcdef);
        *line += 1; // body at condition end + 1
        layout_command_list(&mut loop_command.body, seed, boundaries, line, funcdef);
        *line += 1; // `done`
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(if_command) = command.if_command.as_mut() {
        // `if test; then` — `then` shares the condition's last line
        // (print_cmd.c:850 ` then\n`).
        layout_command_list(&mut if_command.condition, seed, boundaries, line, funcdef);
        *line += 1; // then-body at condition end + 1
        layout_command_list(&mut if_command.then_body, seed, boundaries, line, funcdef);
        for elif in &mut if_command.elif_branches {
            // An `elif` prints as a nested `else` + `if cond; then`
            // (print_cmd.c:856-864 with the false case being an if node).
            *line += 2;
            layout_command_list(&mut elif.condition, seed, boundaries, line, funcdef);
            *line += 1;
            layout_command_list(&mut elif.body, seed, boundaries, line, funcdef);
        }
        if let Some(else_body) = &mut if_command.else_body {
            *line += 2; // `else` then body at else line + 1
            layout_command_list(else_body, seed, boundaries, line, funcdef);
        }
        // One `fi` per nesting level: innermost at body_end+1, one more
        // line per elif level above it (e1/e2/e6 probes).
        *line += 1 + if_command.elif_branches.len();
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(case_command) = command.case_command.as_deref_mut() {
        let case_head_newlines = case_command.word.matches('\n').count();
        // First clause patterns share the `case w in ` line
        // (print_cmd.c:771 suppresses the newline when printing a comsub).
        let mut patterns_line = start + case_head_newlines;
        let mut terminator_line = patterns_line;
        for clause in &mut case_command.clauses {
            *line = patterns_line + 1; // `)\n` ends the patterns line
            layout_command_list(&mut clause.body, seed, boundaries, line, funcdef);
            *line += 1; // `;;` (or `;&`/`;;&`) at action end + 1
            terminator_line = *line;
            patterns_line = terminator_line + 1;
        }
        // `esac` at last terminator + 1 (or case line + 1 with no clauses).
        *line = if case_command.clauses.is_empty() {
            start + case_head_newlines + 1
        } else {
            terminator_line + 1
        };
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(function_command) = command.function_command.as_deref_mut() {
        // `name ()` at L, `{` at L+1, body from L+2, `}` at body_end+1
        // (print_cmd.c:1347-1386). execute_cmd.c:5351 stamps the function
        // environment with the `{` line (tc->line == function_bstart).
        // Everything in the body prints with inside_function_def set
        // (print_cmd.c:1362), so its list separators always end with a
        // newline (print_cmd.c:309).
        let brace_line = start + 1;
        assign(&mut function_command.body_open_line, seed, brace_line);
        *line = brace_line + 1;
        layout_command_list(&mut function_command.body, seed, boundaries, line, true);
        *line += 1; // closing `}`
        assign(&mut function_command.body_end_line, seed, *line);
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(group) = command.brace_group.as_deref_mut() {
        // Outside a function definition `{ ` and the first body command
        // share the line (print_cmd.c:698-699 skip_this_indent), `; }`
        // stays on the body's last line. Inside one the braces take their
        // own lines (print_cmd.c:702-712: `cprintf ("\n")` before the body
        // and before the `}`).
        if funcdef {
            *line += 1; // `{` on its own line
            layout_command_list(&mut group.body, seed, boundaries, line, funcdef);
            *line += 1; // closing `}` on its own line
        } else {
            layout_command_list(&mut group.body, seed, boundaries, line, funcdef);
        }
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(subshell) = command.subshell_command.as_deref_mut() {
        // `( ... )` keeps the body on the braces' line even inside a
        // function definition (print_cmd.c:348-355 has no funcdef branch);
        // only the body's own list separators funcdef-advance.
        layout_command_list(&mut subshell.body, seed, boundaries, line, funcdef);
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(coproc) = command.coproc_command.as_deref_mut() {
        *line = start
            + coproc
                .words
                .iter()
                .map(|word| word.matches('\n').count())
                .sum::<usize>();
        if let Some(body) = &mut coproc.body {
            layout_command_list(body, seed, boundaries, line, funcdef);
        }
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(pipeline) = command.pipeline_command.as_mut() {
        // Stages join with ` | ` on one line (print_cmd.c:257-273).
        for stage in &mut pipeline.stages {
            assign(&mut stage.line, seed, *line);
            *line += simple_command_embedded_newlines(stage);
        }
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(and_or) = command.and_or_list.as_mut() {
        // ` && `/` || ` join members on one line (print_cmd.c:276-286).
        for member in &mut and_or.commands {
            layout_command(member, seed, boundaries, line, funcdef);
        }
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(background) = command.background_command.as_mut() {
        layout_command(&mut background.command, seed, boundaries, line, funcdef);
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(inverted) = command.inverted_command.as_mut() {
        layout_command(&mut inverted.command, seed, boundaries, line, funcdef);
        assign(&mut command.end_line, seed, *line);
        return;
    }
    if let Some(timed) = command.time_command.as_mut() {
        layout_command(&mut timed.command, seed, boundaries, line, funcdef);
        assign(&mut command.end_line, seed, *line);
        return;
    }
    // Plain simple command: words, assignments and redirects print on one
    // canonical line, advanced only by newlines embedded in the text. A
    // here-document defers its body past the connector (print_cmd.c:120
    // PRINT_DEFERRED_HEREDOCS): the canonical text is `cmd <<X`, then
    // print_heredoc_bodies' leading `\n` (print_cmd.c:1035-1043), then each
    // body's lines and its terminator on its own line — leaving `*line` ON
    // the last terminator line; the follower's bump (layout_command_list)
    // accounts the terminator's trailing newline.
    *line += simple_command_embedded_newlines(command);
    let heredocs = command
        .heredoc_redirects
        .iter()
        .filter(|redirect| redirect.body.is_some())
        .count();
    if heredocs > 0 {
        *line += 1; // print_heredoc_bodies' leading `\n`
        let mut body_lines = 0usize;
        for redirect in &command.heredoc_redirects {
            if let Some(body) = &redirect.body {
                body_lines += body.matches('\n').count();
            }
        }
        // Every heredoc contributes its body lines plus its terminator
        // line; the leading `\n` already moved past the first body's start.
        *line += body_lines + heredocs - 1;
    }
    assign(&mut command.end_line, seed, *line);
}

impl Executor {
    /// Expands a command-substitution argument word. When the word was
    /// quoted in the source and starts with `~`, prefix the quote-protection
    /// marker so tilde expansion is skipped (Bash: `$(printf '%s' "~/repo")`
    /// prints `~/repo`, not the home directory).
    /// GNU bash runs brace expansion on each command word before any other
    /// expansion (subst.c expand_words -> brace expansion on the raw word).
    /// The single-command substitution shortcuts expand the raw words
    /// directly, so splice brace-expansion results in place, preserving the
    /// parent word's quote flag for the remaining per-word passes.
    fn brace_expanded_substitution_args(
        &self,
        words: &[String],
        word_parts: &[(String, bool)],
    ) -> Vec<String> {
        let mut expanded_args = Vec::new();
        for (index, word) in words[1..].iter().enumerate() {
            let quote = word_parts.get(index + 1).map(|(_, q)| *q);
            let unquoted = quote != Some(true);
            let braced = crate::expand::braces::expand_braces(word);
            if braced.len() > 1 {
                for item in braced {
                    let expanded = self.expand_protected_tilde(&item, quote);
                    // GNU subst.c expand_words runs pathname expansion on
                    // each brace-expanded word when the original was unquoted
                    // (`$(echo {a,b}*)` expands `a*` and `b*` separately).
                    if unquoted {
                        expanded_args.extend(
                            self.expand_command_substitution_arg_values_quoted(&item, false),
                        );
                    } else {
                        expanded_args.push(expanded);
                    }
                }
            } else {
                // GNU expand_words field-splits an unquoted expansion word on
                // $IFS (subst.c), so the substitution body's `echo $a` hands
                // echo one arg per IFS field, not the raw joined value
                // (nquote5.tests: `$(echo $a)` with IFS=$'\001'). A fully
                // quoted word (`echo "$a"`) is one field, never split.
                let expanded = self.expand_protected_tilde(word, quote);
                if unquoted && for_word_has_unquoted_expansion(word, None) {
                    let split = self.field_split_values(&expanded);
                    // Pathname expansion after field splitting (subst.c
                    // expand_words -> pathname expansion): each field is
                    // expanded independently. `$(echo *)` yields the
                    // directory listing, `$(echo $a)` yields the IFS fields.
                    for value in split {
                        expanded_args
                            .extend(self.apply_command_substitution_pathname_expansion(&value));
                    }
                } else if unquoted {
                    // No unquoted parameter expansion, but the word may still
                    // be a literal glob pattern (`echo *`, `echo *.sh`).
                    expanded_args
                        .extend(self.apply_command_substitution_pathname_expansion(&expanded));
                } else {
                    expanded_args.push(expanded);
                }
            }
        }
        expanded_args
    }

    fn expand_protected_tilde(&self, word: &str, was_quoted: Option<bool>) -> String {
        let expanded = if was_quoted == Some(true) && word.starts_with('~') {
            self.expand_word(&format!(
                "{}{word}",
                crate::executor::markers::QUOTED_WORD_PREFIX_STR
            ))
        } else {
            self.expand_word(word)
        };
        let unescaped = unescape_remaining_shell_escapes(&expanded);
        let protected = if command_substitution_value_needs_payload_protection(word, &unescaped) {
            protect_command_substitution_output(&unescaped)
        } else {
            unescaped
        };
        decode_command_substitution_payload(&restore_old_style_backtick_markers(&protected))
    }

    pub(in crate::executor) fn expand_command_substitution(&self, source: &str) -> String {
        self.expand_command_substitution_with_context(source, SubstitutionQuoteContext::Unquoted)
    }

    pub(in crate::executor) fn expand_command_substitution_with_context(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        self.last_command_substitution_status.set(Some(0));
        self.last_command_substitution_parse_error.set(false);
        // A command substitution is a subshell boundary: GNU runs the body
        // in a forked child (subst.c:7143 command_substitute ->
        // execute_cmd.c:1576 execute_in_subshell), so no mutation of shell
        // state — the arithmetic/bad-substitution latches (subst.c:10277),
        // subshell_level, or the DEBUG-trap command text — can reach the
        // parent. Fast-path builtins expand on this shared executor under
        // `&self`, so the boundary is expressed as the typed interior
        // snapshot/restore (issue #67: `x=$(echo $((b)))` under `set -u`
        // prints the diagnostic, leaves x empty, keeps running).
        let saved_state = self.shell_state.snapshot_interior();
        self.shell_state
            .subshell_depth
            .set(saved_state.subshell_depth() + 1);
        // execute_cmd.c:1576 execute_in_subshell marks SUBSHELL_COMSUB —
        // start_job (jobs.c:3837) refuses fg/bg inside it.
        self.shell_state.in_command_substitution.set(true);
        self.shell_state.parameter_bad_substitution.set(false);
        // Bash evaluates BASH_COMMAND in a command substitution against the
        // substitution's own command source, rather than the outer word.
        *self.shell_state.debug_trap_command.borrow_mut() = Some(source.trim().to_string());
        let result = self.expand_command_substitution_inner(source, context);
        self.shell_state.restore_interior(&saved_state);
        result
    }

    pub(in crate::executor) fn expand_command_substitution_readback_with_context(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> SubstitutionOutput {
        let output = self.expand_command_substitution_with_context(source, context);
        let status = self.last_command_substitution_status.get().unwrap_or(0);
        // expand_command_substitution_with_context returns shell TEXT (marker
        // pairs for carrier/raw bytes). readback takes raw capture bytes, so
        // decode the markers once — into_bytes() would re-encode them as
        // literal PUA glyphs at the next bytes_to_shell_text boundary.
        SubstitutionOutput::readback(
            crate::executor::substitution_metadata::shell_text_to_raw_bytes(&output),
            status,
            context,
        )
    }

    pub(in crate::executor) fn expand_command_substitution_inner(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> String {
        // TODO(subst.c/parse.y/execute_cmd.c): Bash command substitution runs a
        // subshell, captures stdout, removes trailing newlines, and performs
        // full parsing/execution. This handles the alias4.sub form
        // `$(eval echo b)` so alias-expanded command substitutions participate
        // in word expansion.
        let source = source.trim();
        if source.is_empty() {
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        // NOTE: an earlier revision stripped a leading `eval ` here and ran
        // the remainder as a plain command. That breaks eval semantics for
        // multi-word strings: `x=$(eval "echo hi")` must re-parse the string
        // into two commands words, not execute a command named "echo hi"
        // (issue #69). Command substitutions that begin with `eval` now fall
        // through to the real parser/executor fallback below, which runs the
        // eval builtin with full re-parse semantics (alias4.sub
        // `$(eval echo b)` included).
        if let Some(inner) = strip_wrapping_subshell_group(source) {
            return self.expand_command_substitution_inner(inner, context);
        }
        // GNU execute_cmd.c:4648-4649: every simple command — `true`,
        // `false`, `:` included — prints its xtrace head line inside the
        // substitution child. The hardcoded status shortcuts cannot express
        // that side effect, so under `set -x` they are disqualified (the
        // rubash#117 whitelist discipline: a shortcut must be provably
        // equivalent, and it is not when tracing) and the body falls to the
        // real parser/executor below, which traces it (rubash#254).
        if source == "false" && !self.xtrace_enabled() {
            self.last_command_substitution_status.set(Some(1));
            return String::new();
        }
        if matches!(source, "true" | ":") && !self.xtrace_enabled() {
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        if let Some(rest) = source.strip_prefix('<') {
            // GNU subst.c:7162-7179 command_substitute: a body whose first
            // non-blank character is `<` (not followed by `<`, `>`, or `&`)
            // is parsed with parse_string_to_command (y.tab.c:7191,
            // SEVAL_ONECMD, whole-string consumption) and admitted by
            // can_optimize_cat_file (builtins/evalstring.c:199): exactly one
            // simple command with no words and exactly one fd-0
            // r_input_direction. Then optimize_cat_file (subst.c:6665) opens
            // the operand via open_redir_file (builtins/evalstring.c:759) —
            // a failed open reports
            // internal_error ("%s: %s", fn, strerror (errno)) there and the
            // substitution yields empty output with EXECUTION_FAILURE
            // (subst.c:7169-7173); an operand that redirection_expand
            // (redir.c:298) resolves to zero or several words is
            // AMBIGUOUS_REDIRECT (redir.c:200). Any other body — trailing
            // words, extra redirections, lists, parse errors — is an
            // ordinary subshell body that must reach the real
            // parser/executor below, never be read as one long filename
            // (rubash#196). Admission is decided by the real parser (the
            // rubash#117 whitelist discipline), not by a text heuristic.
            let heredoc_or_fdop_prefix =
                matches!(rest.chars().next(), Some('<' | '>' | '&') | None);
            let cat_file_target = if heredoc_or_fdop_prefix {
                None
            } else {
                self.comsub_cat_file_target(source)
            };
            if let Some((target, raw_word)) = cat_file_target {
                return self.comsub_cat_file_substitute(&target, &raw_word);
            }
        }
        // GNU trap.c reset_or_restore_signal_handlers (~1588): a command
        // substitution child keeps the DEBUG trap only when
        // function_trace_mode is set. When it is inherited, every inner
        // command's run_debug_trap must fire — with the trap output captured
        // into the substitution result — which none of the word-level
        // shortcuts below can express. Disqualify the whole shortcut family
        // and route the body through the real parser/executor, matching
        // subst.c:7143 command_substitute -> parse_and_execute.
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "functrace")
            && crate::builtins::trap::get_trap_action(&self.shell_state.env_vars, "DEBUG")
                .is_some_and(|action| !action.is_empty())
        {
            if let Some(output) = self.command_list_substitution_output(source, context) {
                return output;
            }
            return String::new();
        }
        // cd/pwd and heredoc word shortcuts predate the triviality gate and
        // likewise bypass the execute_cmd.c:4649 head-line trace; under
        // `set -x` they are disqualified so the multi-command body (`cd X &&
        // pwd` traces TWO lines, `++ cd X` then `++ pwd`) reaches the real
        // executor (rubash#254).
        if !self.xtrace_enabled() {
            if let Some(output) = self.command_substitution_cd_pwd_output(source) {
                return output;
            }
            if let Some(output) = self.command_substitution_heredoc_output(source) {
                return output;
            }
        }
        // GNU applies alias expansion while reading the substitution body
        // (parse.y alias_expand_token + push_string): expand the body text
        // at stream level once here so the whitelist below judges the same
        // words the real parser would see — an alias body can inject
        // operators or quotes the raw source did not carry.
        let word_source = strip_command_substitution_comments(source);
        let word_source = self.comsub_body_alias_splice_extracted(&word_source);

        // rubash#117 whitelist admission: GNU subst.c:7143
        // command_substitute routes every body through parse_and_execute —
        // there are no word-level shortcuts upstream. A shortcut here is
        // only equivalent when the body is provably a single simple command
        // of literal words (no quoting, no expansion, no operators), where
        // parsing cannot change the outcome. Everything else falls through
        // to the real parser/executor: a false positive only costs speed,
        // while the blacklist guards this replaced kept producing silent
        // semantic bugs for the next uncovered character class.
        // `set -x` also disqualifies the whole shortcut family: every word
        // below would skip the simple-command head line that GNU prints at
        // execute_cmd.c:4649 (`++ echo hi` for `v=$(echo hi)`), and the
        // shortcut cannot express it. Routing to the real executor costs
        // speed only (rubash#254).
        if !command_substitution_body_is_trivial(&word_source) || self.xtrace_enabled() {
            if let Some(output) = self.command_list_substitution_output(source, context) {
                return output;
            }
            return String::new();
        }

        let word_parts = split_shell_words_with_quote_info(&word_source);
        let words: Vec<String> = word_parts.iter().map(|(word, _)| word.clone()).collect();

        if let Some(output) = self.timed_command_substitution_output(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("echo") {
            let expanded_args = self.brace_expanded_substitution_args(&words, &word_parts);
            return echo_command_substitution_output(&expanded_args);
        }

        if words.first().map(String::as_str) == Some("printf") {
            let expanded_args: Vec<String> = words[1..]
                .iter()
                .enumerate()
                .flat_map(|(index, word)| {
                    if let Some(values) = self.array_at_word_values(word) {
                        return values;
                    }
                    if let Some(values) = self.quoted_positional_at_word_values(word, None) {
                        return values;
                    }
                    let was_quoted = word_parts.get(index + 1).map(|(_, q)| *q);
                    let unquoted = was_quoted != Some(true);
                    let expanded =
                        strip_matching_quotes(&self.expand_protected_tilde(word, was_quoted))
                            .to_string();
                    // Same expand_words semantics as the echo/recho/zecho
                    // paths: unquoted expansion words split on $IFS, fully
                    // quoted words stay one field.
                    let values = if unquoted && for_word_has_unquoted_expansion(word, None) {
                        self.field_split_values(&expanded)
                    } else {
                        vec![expanded]
                    };
                    // Pathname expansion (subst.c expand_words): each
                    // unquoted field is expanded independently.
                    if unquoted {
                        values
                            .into_iter()
                            .flat_map(|v| self.apply_command_substitution_pathname_expansion(&v))
                            .collect::<Vec<_>>()
                    } else {
                        values
                    }
                })
                .collect();
            let mut env_vars = self.shell_state.env_vars.clone();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let status = crate::builtins::printf::execute_with_io(
                expanded_args.iter().map(String::as_str),
                &mut env_vars,
                &mut stdout,
                &mut stderr,
            )
            .unwrap_or(1);
            self.last_command_substitution_status.set(Some(status));
            return bytes_to_shell_text(&stdout)
                .trim_capture_terminator()
                .to_string();
        }

        // The file-operand fast path must not claim a bare `cat` (or flag/
        // `-` operands): those read fd 0, which lives in FUNCTION_STDIN and
        // shares the caller's cursor — external_cat owns that semantics.
        if words.first().map(String::as_str) == Some("cat")
            && words[1..].iter().all(|word| !word.starts_with('-'))
            && words.len() > 1
            // A `/dev/fd/N` operand needs real execution when it names an
            // fd this shortcut cannot model: N>0 needs the descriptor
            // table (external_cat owns it), and fd 0 is only the
            // FUNCTION_STDIN cursor when that channel exists — otherwise
            // fd 0 is the inherited process stdin, which full execution
            // drains (subst.c:7143 shared offset).
            && !words[1..].iter().any(|word| {
                match crate::executor::dev_fd_operands::dev_operand_fd(word) {
                    Some(0) => !self.shell_state.env_vars.contains_key(FUNCTION_STDIN),
                    Some(_) => true,
                    None => false,
                }
            })
        {
            let mut output = String::new();
            let mut status = 0;
            for word in &words[1..] {
                // Process substitution `<(...)`: Bash materializes it to a
                // temporary file holding the command's output, so `cat` reads
                // that output. Run the inner command directly instead of
                // treating the literal `<(...)` text as a file path.
                if let Some(source) = word
                    .strip_prefix("<(")
                    .and_then(|rest| rest.strip_suffix(')'))
                {
                    let mut executor = self.command_substitution_executor();
                    crate::builtins::trap::reset_for_subshell(&mut executor.shell_state.env_vars);
                    output.push_str(&executor.expand_command_substitution(source));
                    continue;
                }
                let path = self.expand_word(word);
                // `/dev/stdin`/`/dev/fd/0`/`/proc/self/fd/0` read fd 0 —
                // the FUNCTION_STDIN cursor (the gate above already sent
                // N>0 descriptors to full execution). Consuming fd 0 here
                // writes the cursor back the way the external fast path
                // does (comsub_stdin_writeback).
                if crate::executor::dev_fd_operands::dev_operand_fd(&path) == Some(0) {
                    if let Some(text) = self.shell_state.env_vars.get(FUNCTION_STDIN) {
                        self.comsub_stdin_writeback
                            .set(Some((text.len(), Self::function_stdin_fingerprint(text))));
                    }
                    output.push_str(&self.function_stdin_remaining().unwrap_or_default());
                    continue;
                }
                match fs::read_to_string(shell_path_to_windows(&path, &self.shell_state.env_vars)) {
                    Ok(value) => output.push_str(&value),
                    Err(_) => {
                        status = 1;
                        eprintln!("cat: '{}': No such file or directory", path);
                    }
                }
            }
            self.last_command_substitution_status.set(Some(status));
            return output.trim_capture_terminator().to_string();
        }

        if words.first().map(String::as_str) == Some("basename") {
            let Some(path) = words.get(1).map(|word| self.expand_word(word)) else {
                self.last_command_substitution_status.set(Some(1));
                return String::new();
            };
            let trimmed = path.trim_end_matches(['/', '\\']);
            let name = trimmed
                .rsplit(['/', '\\'])
                .next()
                .filter(|name| !name.is_empty())
                .unwrap_or(trimmed);
            let suffix = words.get(2).map(|word| self.expand_word(word));
            let output = suffix
                .as_deref()
                .and_then(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name);
            self.last_command_substitution_status.set(Some(0));
            return output.to_string();
        }

        if let Some(output) = self.command_describe_substitution_output(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("umask") {
            return self
                .shell_state
                .env_vars
                .get("__RUBASH_UMASK")
                .cloned()
                .unwrap_or_else(|| "0022".to_string());
        }

        if words.first().map(String::as_str) == Some("ulimit") {
            // Run the real builtin engine (getrlimit/setrlimit on unix, the
            // emulated table on Windows) instead of a canned lookup, so
            // `$(ulimit -n)` cannot drift from `ulimit -n` (rubash#261).
            // The env map is copied because a comsub is a fork: on Windows
            // the emulated limit table lives in env_vars and a set inside
            // `$(ulimit -n 2048)` must not leak into the parent (GNU
            // setrlimit in a forked child never does). Diagnostics go to
            // the shell's stderr; the comsub status is the builtin's own.
            let mut comsub_env = self.shell_state.env_vars.clone();
            let mut out = Vec::new();
            let mut err = Vec::new();
            let status = crate::builtins::ulimit::execute_with_io(
                &words[1..],
                &mut comsub_env,
                &mut out,
                &mut err,
            )
            .unwrap_or_else(|_| 1); // Vec<u8> writes are infallible
            let _ = std::io::stderr().write_all(&err);
            self.last_command_substitution_status.set(Some(status));
            return String::from_utf8_lossy(&out)
                .trim_capture_terminator()
                .to_string();
        }

        if words.first().map(String::as_str) == Some("pwd") {
            if words.get(1).map(String::as_str) == Some("-P") {
                return std::env::current_dir()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
            }
            return self
                .shell_state
                .env_vars
                .get("PWD")
                .cloned()
                .unwrap_or_default();
        }

        if words.first().map(String::as_str) == Some("type")
            && words.get(1).map(String::as_str) == Some("-t")
            && words.len() >= 3
        {
            let mut subshell = self.command_substitution_executor();
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Ok(status) = subshell.execute_type_with_io(&words[1..], &mut stdout, &mut stderr)
            {
                self.last_command_substitution_status.set(Some(status));
                return String::from_utf8_lossy(&stdout)
                    .trim_capture_terminator()
                    .to_string();
            }
        }

        if words.first().map(String::as_str) == Some("kill")
            && words.get(1).map(String::as_str) == Some("-l")
        {
            if let Some(word) = words.get(2) {
                // The spec may reference variables set by a running trap
                // action ($(kill -l $BASH_TRAPSIG) in trap9.sub), so expand
                // before translating; a literal that names no signal keeps
                // the historical empty-output behavior.
                let expanded = self.expand_word(word);
                if let Some(signal) = crate::builtins::kill::translate_signal(&expanded) {
                    self.last_command_substitution_status.set(Some(0));
                    return signal.to_string();
                }
                self.last_command_substitution_status.set(Some(1));
                return String::new();
            }
        }

        if words.first().map(String::as_str) == Some("mktemp") {
            if let Some(path) = self.mktemp_command_substitution(&words) {
                return path;
            }
        }

        if let Some(output) = self.run_external_command_substitution(&words) {
            return output;
        }

        if words.first().map(String::as_str) == Some("mktemp") {
            if let Some(path) = self.mktemp_command_substitution(&words) {
                return path;
            }
        }

        // Fallback: run the source through the real parser/executor in a
        // subshell and capture stdout (function calls, pipelines, compound
        // commands that the special-case dispatch above does not cover).
        if let Some(output) = self.command_list_substitution_output(source, context) {
            return output;
        }

        String::new()
    }

    pub(in crate::executor) fn command_substitution_cd_pwd_output(
        &self,
        source: &str,
    ) -> Option<String> {
        let (left, right) =
            split_unquoted_and_and(source).or_else(|| split_unquoted_semicolon(source))?;
        let right_words = split_shell_words(right.trim());
        if !matches!(right_words.as_slice(), [cmd] if cmd == "pwd")
            && !matches!(right_words.as_slice(), [cmd, option] if cmd == "pwd" && option == "-P")
        {
            return None;
        }

        let left_words = split_shell_words(left.trim());
        if left_words.first().map(String::as_str) != Some("cd") || left_words.len() > 2 {
            return None;
        }
        let target = if let Some(word) = left_words.get(1) {
            // GNU subst.c expands the cd target with expand_string (parameter,
            // tilde) but not pathname expansion: `cd` takes a single directory,
            // so a glob pattern would be a literal path, not a match list.
            self.expand_word(word)
        } else {
            self.home_value()
        };
        let target = shell_path_to_windows(&target, &self.shell_state.env_vars);
        let Ok(path) = fs::canonicalize(target) else {
            self.last_command_substitution_status.set(Some(1));
            return Some(String::new());
        };
        if !path.is_dir() {
            self.last_command_substitution_status.set(Some(1));
            return Some(String::new());
        }

        self.last_command_substitution_status.set(Some(0));
        let display = path.to_string_lossy().replace('\\', "/");
        Some(display.strip_prefix("//?/").unwrap_or(&display).to_string())
    }

    fn command_list_substitution_output_typed(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<SubstitutionOutput> {
        // The body was extracted from the current word; keep GNU's in-place
        // line counter so body diagnostics report the original script line
        // instead of restarting at 1 (subst.c comsub handling).
        let body_start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let source = &self.comsub_body_alias_splice_extracted(source);
        let tokens = crate::lexer::tokenize_comsub_body(
            source,
            self.posix_mode_enabled(),
            body_start_line,
            true,
        );
        let mut ast = crate::parser::parse(&tokens);
        normalize_comsub_body_statement_lines(&mut ast, &tokens);

        if ast.commands.iter().any(command_has_parse_error) {
            // GNU reports the body's syntax error (parse.y yyerror) even
            // though the enclosing command is what dies — emit the stored
            // diagnostic instead of silently swallowing it (case-pattern
            // `$(esac;x)` in comsub-posix6.sub).
            if let Some(command) = ast.commands.iter().find_map(command_parse_error_node) {
                self.report_command_parse_error(command);
            }
            self.last_command_substitution_parse_error.set(true);
            self.last_command_substitution_status.set(Some(2));
            return Some(SubstitutionOutput::readback(Vec::new(), 2, context));
        }

        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // GNU subst.c:7413 runs the body through parse_and_execute
        // ("command substitution"), whose reader bumps indirection_level
        // (builtins/evalstring.c:348) — every trace inside the substitution
        // child renders one PS4 level deeper (`++ echo hi` for
        // `v=$(echo hi; true)`, and the head line execute_cmd.c:4649 prints
        // for each simple command of the body). Pipeline-stage clones must
        // NOT inherit this bump, so it lives here at the body-reader
        // boundary, not in command_substitution_executor (rubash#254).
        subshell
            .shell_state
            .xtrace_indirection_level
            .set(subshell.shell_state.xtrace_indirection_level.get() + 1);
        // The body was alias-expanded at stream level by
        // comsub_body_alias_splice above (parse.y alias_expand_token on the
        // fresh input); the child must not expand those words a second time.
        if self.alias_expansion_enabled() || self.posix_mode_enabled() {
            subshell
                .shell_state
                .env_vars
                .insert("__RUBASH_ALIAS_STREAMED".to_string(), "1".to_string());
        }
        // GNU subst.c:7143 command_substitute: the forked child is a POSIX
        // subshell — every inherited non-ignored trap is reset (signals.c
        // restore/original dispositions via the fork; an `trap '' SIG`
        // ignore action survives) — and its exit path ALWAYS runs
        // run_exit_trap (subst.c:7340-7341 `rc = run_exit_trap ();
        // exit (rc);`), so an EXIT trap installed by the body runs there:
        // set directly, or indirectly through eval, a function, or a
        // sourced file (modernish fatal.sh installs `trap 'echo $PPID' 0`
        // via `command . "$MSH_AUX/fatal.sh"`, rubash#252). The previous
        // word-level gate ran the reset and the exit trap only when the
        // body text contained a literal `trap` word, so an EXIT trap set
        // by a sourced file never fired and the substitution read empty.
        crate::builtins::trap::reset_for_subshell(&mut subshell.shell_state.env_vars);
        // Keep the command source visible to BASH_COMMAND while the parsed
        // substitution body runs, including DEBUG trap actions.
        *subshell.shell_state.debug_trap_command.borrow_mut() = Some(source.trim().to_string());
        subshell.stdout_capture = Some(Vec::new());
        // GNU subst.c:7143 command_substitute: the substitution child's
        // stdout is the capture pipe, never the caller's fd 1. The forked
        // executor clones the parent's fd table, and since the compound
        // redirect binds landed there (4fe2a48f) an enclosing for/group
        // redirect surfaces as an fd-1 file binding that
        // apply_external_stdout_redirect delivered to external children —
        // their output bypassed the capture and landed in the outer
        // redirect target while the substitution read an empty pipe
        // (niubash shell-quirks Q16: inside
        // `for …; do out=$(cargo test 2>&1); …; done > summary.txt` the
        // failing round's output reached summary.txt and $out stayed
        // empty). Rebind fd 1 to the default Stdout endpoint — it resolves
        // to the active stdout_capture in write_fd_endpoint — rather than
        // removing the entry: in-shell writes consult fd_table[1] first and
        // a missing entry silently drops output. External children then hit
        // the stdout_capture pipe branch; body-level redirects rebind fd 1
        // on the child's own table. fd 2 stays inherited — $( ) does not
        // capture stderr (GNU subst.c:7149).
        subshell.fd_table.entries.insert(
            1,
            crate::executor::fd_table::FdEntry {
                read: None,
                write: Some(FdWriteEndpoint::Stdout),
                closed: false,
                dynamic: false,
            },
        );

        // GNU subst.c:7356-7359 command_substitute: without inherit_errexit
        // the substitution child runs `builtin_ignoring_errexit = 0` and
        // `change_flag ('e', FLAG_OFF)` — it clears the -e *flag itself*, so
        // an explicit `set -e` inside the body re-enables it (set-e.tests
        // `x=$(set -e; false; echo bad)` prints nothing). A suppression
        // counter would keep -e dead even after `set -e`. POSIX mode
        // enables inherit_errexit (set-e1.sub).
        let posix_mode = subshell
            .shell_state
            .env_vars
            .get("__RUBASH_POSIX_MODE")
            .map(String::as_str)
            == Some("1");
        let inherit_errexit = crate::builtins::shopt::option_enabled(
            &subshell.shell_state.env_vars,
            "inherit_errexit",
        );
        // Builtins inside the body that write the process stdout directly
        // consult the thread-local capture — which belongs to an enclosing
        // pipeline stage when this substitution runs inside one, leaking
        // the substitution's output into the stage's pipe. Give the body
        // its own thread-local capture and merge both buffers.
        let (captured, status) = crate::executor::shell_options::capture_stdout(|| {
            if std::env::var_os("RUBASH_DBG_TRAP").is_some() {
                eprintln!(
                    "[CS] body-pre trapEXIT={:?} reset={:?}",
                    subshell.shell_state.env_vars.get("__RUBASH_TRAP_EXIT"),
                    subshell.shell_state.env_vars.get("__RUBASH_TRAP_RESET")
                );
            }
            let result = if posix_mode || inherit_errexit {
                subshell.execute_ast(&ast)
            } else {
                subshell.suppress_errexit = 0;
                subshell.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                crate::builtins::set::set_shell_option(
                    &mut subshell.shell_state.env_vars,
                    "errexit",
                    false,
                );
                subshell.execute_ast(&ast)
            };
            if std::env::var_os("RUBASH_DBG_TRAP").is_some() {
                eprintln!(
                    "[CS] body-post trapEXIT={:?} reset={:?}",
                    subshell.shell_state.env_vars.get("__RUBASH_TRAP_EXIT"),
                    subshell.shell_state.env_vars.get("__RUBASH_TRAP_RESET")
                );
            }
            // GNU parse.y: a syntax error inside the substitution body is a
            // read-time failure of the ENCLOSING command — after this command
            // finishes the reader stops (`$( esac ; ...)` in a case pattern:
            // the `*)` arm prints, `echo we should not see this` is skipped).
            // The body ast carried a __RUBASH_PARSE_ERROR__ node past the
            // early command_has_parse_error screen, so propagate the child's
            // parse_error latch onto the parent's abort flag here.
            if subshell.parse_error_occurred {
                self.last_command_substitution_parse_error.set(true);
            }
            let mut status = command_substitution_result_status(result, subshell.exit_code);
            // Bash runs EXIT in the command-substitution child, so an
            // EXIT trap installed by the body contributes its output to
            // captured stdout (subst.c:7340 — unconditional run_exit_trap).
            if let Ok(exit_status) = subshell.run_exit_trap_for_status(status) {
                status = exit_status;
            }
            status
        });
        let mut output = subshell.stdout_capture.take().unwrap_or_default();
        output.extend_from_slice(&captured);

        // GNU subst.c:7143 command_substitute forks sharing the parent's
        // fd 0 — input the body consumed is gone for the caller too. The
        // child's cursor lives in its env clone; fold it back when both
        // sides still name the same FUNCTION_STDIN buffer.
        if let (Some(parent_input), Some(child_input)) = (
            self.shell_state.env_vars.get(FUNCTION_STDIN).cloned(),
            subshell.shell_state.env_vars.get(FUNCTION_STDIN).cloned(),
        ) {
            if parent_input == child_input {
                if let Some(child_offset) = subshell
                    .shell_state
                    .env_vars
                    .get(FUNCTION_STDIN_OFFSET)
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    self.comsub_stdin_writeback.set(Some((
                        child_offset,
                        Self::function_stdin_fingerprint(&parent_input),
                    )));
                }
            }
        }

        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }

        let readback = SubstitutionOutput::readback(output, status, context);
        self.last_command_substitution_status
            .set(Some(readback.status));
        Some(readback)
    }

    /// Legacy String boundary for callers that still build AST words as text.
    fn command_list_substitution_output(
        &self,
        source: &str,
        context: SubstitutionQuoteContext,
    ) -> Option<String> {
        self.command_list_substitution_output_typed(source, context)
            .map(|output| output.text_lossy())
    }

    pub(in crate::executor) fn command_substitution_executor(&self) -> Executor {
        // The fork copy: every mutable shell datum arrives through the
        // cloned state boundary (execute_cmd.c:1576 execute_in_subshell /
        // subst.c:7143 command_substitute), so new semantic fields are
        // isolated automatically — no per-field list to maintain.
        let mut shell_state = self.shell_state.clone();
        // GNU forks carry the parent's loop_level, but a `break` in a real
        // child can only end the child. In-process the loop-break flag
        // would reach the parent's live loop, so the boundary resets it —
        // the same rule the flat `( )` region applies at entry.
        shell_state.loop_depth = 0;
        shell_state
            .subshell_depth
            .set(self.shell_state.subshell_depth.get() + 1);
        shell_state.in_command_substitution.set(true);
        // Mid-expansion error latches belong to the parent's in-flight word
        // expansion; the forked child starts with a clean slate (same as a
        // real fork, where no such rubash-internal latch exists).
        shell_state.arithmetic_expansion_error.set(false);
        shell_state.arithmetic_nonfatal_error.set(false);
        shell_state.arithmetic_fatal_error.set(false);
        shell_state.arithmetic_nounset_error.set(false);
        shell_state.arithmetic_last_error_category.set(None);
        shell_state.parameter_bad_substitution.set(false);
        // The substitution body is fresh parser input (subst.c:7143
        // command_substitute re-reads the collected text): GNU expands its
        // aliases at that read, so the driver's streamed-batch marker must
        // not reach the child's executor-level expansion.
        shell_state.env_vars.remove("__RUBASH_ALIAS_STREAMED");
        let mut fd_table = self.fd_table.clone();
        // GNU subst.c:7320 command_substitute: after the fork the child
        // installs the capture pipe with `dup2 (fildes[1], 1)` — fd 1
        // becomes a NEW open file description and inherits none of the
        // parent fd 1's dup2 aliases. Every caller of this constructor runs
        // its body against a fresh capture as the child's fd 1 (the
        // substitution buffer here; pipeline elements likewise get the pipe
        // bound to fd 1 first, execute_cmd.c:2702-2723), so the alias-
        // generation record a parent `1>&N` left on fd 1 must not survive
        // the boundary: a `Some(None)` record would route the body's plain
        // stdout writes to the REAL process stdout (escaping the capture,
        // rubash#368) and a `Some(gen)` record to the parent's capture
        // generation instead of this child's buffer (the #335 residue the
        // function-call fast path already clears — see
        // embedded_mutations.rs). Descriptor bindings on every OTHER fd
        // survive verbatim: GNU's fork keeps fd >= 3 at the parent's
        // binding, which is exactly what `{ v=$(cmd 3>&1 1>&4); } 4>&1`
        // needs (rubash#368).
        fd_table.stdout_alias_generation.remove(&1);
        Executor {
            shell_state,
            fd_table,
            exit_code: self.exit_code,
            parse_error_occurred: false,
            // GNU exit.def bash_logout: subshells never source ~/.bash_logout
            // (subshell_environment check); inherit the parent's latch so a
            // subshell logout cannot double-source it either.
            bash_logout_sourced: true,
            shell_pid: self.shell_pid,
            owns_signal_mailbox: false,
            is_process_exit_executor: false,
            arithmetic_last_error_expression: std::cell::RefCell::new(String::new()),
            arithmetic_last_eval_input: std::cell::RefCell::new(String::new()),
            assignment_command_name: None,
            buffer_assignment_diagnostics: false,
            pending_assignment_diagnostics: Vec::new(),
            parameter_assignment_failure: Cell::new(false),
            tempenv_names: Vec::new(),
            tempenv_marks: Vec::new(),
            tempenv_promoted_names: Vec::new(),
            tempenv_previous: HashMap::new(),
            tempenv_propagated_names: Vec::new(),
            function_tempenv_names: Vec::new(),
            evalerror_pending: Cell::new(false),
            evalerror_line: Cell::new(None),
            evalerror_exec_depth: Cell::new(0),
            reader_command_line: Cell::new(None),
            line_lex_locales: std::cell::RefCell::new(HashMap::new()),
            unit_lex_locale: std::cell::RefCell::new(None),
            ambient_line: Cell::new(None),
            conditional_invert_pending: Cell::new(false),
            inside_compound_condition: Cell::new(false),
            inside_assignment_rhs: Cell::new(false),
            background_children: HashMap::new(),
            coproc_stderr_forwarders: HashMap::new(),
            assignment_output_process_substitutions: HashMap::new(),
            pending_scalar_assignment: false,
            suppress_errexit: self.suppress_errexit,
            command_builtin_depth: self.command_builtin_depth,
            debug_trap_running: false,
            line_env_os_value: std::cell::RefCell::new(None),
            return_trap_running: false,
            signal_trap_running: false,
            error_trap_running: false,
            sigchld_notifications_pending: std::cell::Cell::new(0),
            // jobs.c:4607-4612: a comsub child (startup_state 2 +
            // SUBSHELL_COMSUB) never prints job-status notices, so the
            // fork copy starts with an empty notice list.
            pending_signal_notices: std::cell::RefCell::new(Vec::new()),
            source_debug_suppressed: false,
            host_internal_depth: std::cell::Cell::new(self.host_internal_depth.get()),
            debug_trap_function_line: None,
            last_command_substitution_status: Cell::new(None),
            comsub_stdin_writeback: Cell::new(None),
            pipeline_stdin_consumed: Cell::new(None),
            pipeline_stage_fds_pre_wired: std::cell::Cell::new(false),
            last_heredoc_warning_source: RefCell::new(None),
            comsub_leading_newlines: Cell::new(0),
            current_shell_substitution_exit: Cell::new(self.current_shell_substitution_exit.get()),
            last_command_substitution_parse_error: Cell::new(false),
            parser_error_errexited: Cell::new(false),
            last_command_inverted: Cell::new(false),
            exit_jump_pending: Cell::new(false),
            special_builtin_failed: Cell::new(false),
            last_builtin_write_failed: Cell::new(false),
            alias_introduced_words: Cell::new(0),
            redirect_target_memo: RefCell::new(HashMap::new()),
            assignment_expansion_memo: RefCell::new(HashMap::new()),
            fd_var_external_undo: Vec::new(),
            read_deadline: None,
            read_timed_out: false,
            read_eof_no_delimiter: false,
            stdout_capture: None,
            stderr_capture: None,
            external_stdio_outcome: None,
            external_stdio_piped_fallback: false,
            host_external_command_handler: None,
            #[cfg(windows)]
            elevation_handler: None,
            external_file_builtins_enabled: self.external_file_builtins_enabled,
            // GNU subst.c:7143 command_substitute forks: the child's process
            // environment is the parent's CURRENT exported env at fork time,
            // and nothing the child does (or its exit) can alter the parent's
            // environment — a variable the parent unset stays unset
            // (variables.c unbind_variable is permanent for the session).
            // The Drop restore models "the child leaves the parent's process
            // env untouched", so its restore point must be the fork-time env,
            // NOT the parent's startup snapshot: propagating the startup
            // snapshot here made every comsub subshell's drop resurrect
            // startup values the parent had unset (rubash#182 —
            // `unset LANG; Y="$(dirname "D:/x")"` revived LANG, read back
            // through the bare-$NAME std::env::var fallback in
            // embedded_parameters.rs).
            process_env_snapshot: std::env::vars().collect(),
            history_provider: self.history_provider.clone(),
        }
    }

    /// subst.c:6917 function_substitute: the bash-5.3 nofork command
    /// substitution `${ cmd; }` (funsub) runs the body with fd 1 captured
    /// to an anonymous file whose contents — trailing newlines stripped by
    /// read_comsub — become the substitution text
    /// (subst.c:7100-7107/7133-7136). `${| cmd; }` (valsub) captures
    /// nothing: the body's fd 1 stays the shell's own stdout and the
    /// substitution value is the body's final $REPLY (subst.c:7041-7045
    /// make_local_variable("REPLY"), 7112-7115 — the local binding dies
    /// with the unwind frame). Status in both forms:
    /// last_command_subst_status = the body's parse_and_execute result,
    /// and last_command_exit_value follows it outside POSIX mode
    /// (subst.c:7120-7122).
    ///
    /// Persistence: GNU runs the body in the CURRENT shell, so variable
    /// assignments, unsets and function definitions inside the body
    /// outlive the substitution. rubash's subshell executor forwards those
    /// mutations back (verified against WSL GNU Bash 5.3.0, probes
    /// target/p196/fresh/f4a.sh: `x=1; r=${ x=7; }` -> x=7,
    /// `${ unset y; }` -> y unset, `${ f2() {...}; }` -> f2 defined —
    /// all byte-identical to GNU). The parser-level admission
    /// (parser.h:85 FUNSUB_CHAR, and the required `;`/newline terminator
    /// before `}`) is enforced exactly; an escaped quote inside the body
    /// when the whole word is double-quoted (`"${ echo \"}\"; }"`) still
    /// mis-scans (documented gap).
    pub(in crate::executor) fn function_substitute(&self, body: &str, valsub: bool) -> String {
        let body = body.trim();
        if body.is_empty() {
            // subst.c:6931-6935: a body of only blanks is no command to
            // run; the substitution is empty with success status.
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        let start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let tokens =
            crate::lexer::tokenize_comsub_body(body, self.posix_mode_enabled(), start_line, true);
        let mut ast = crate::parser::parse(&tokens);
        normalize_comsub_body_statement_lines(&mut ast, &tokens);
        if ast.commands.iter().any(command_has_parse_error) {
            if let Some(command) = ast.commands.iter().find_map(command_parse_error_node) {
                self.report_command_parse_error(command);
            }
            self.last_command_substitution_parse_error.set(true);
            self.last_command_substitution_status.set(Some(2));
            return String::new();
        }
        let saved_dir = env::current_dir().ok();
        let mut subshell = self.command_substitution_executor();
        // GNU subst.c:7101 runs the funsub body through parse_and_execute
        // ("nofork comsub") → evalstring.c:348 indirection_level++, so the
        // body's traces render one PS4 level deeper (`${ echo cur; }` under
        // `set -x` prints `++ echo cur`). Same body-reader bump as
        // command_list_substitution_output_typed (rubash#254).
        subshell
            .shell_state
            .xtrace_indirection_level
            .set(subshell.shell_state.xtrace_indirection_level.get() + 1);
        let posix_mode = self.posix_mode_enabled();
        let inherit_errexit =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "inherit_errexit");
        let run_body = |subshell: &mut Executor| -> Result<(), ExecuteError> {
            if posix_mode || inherit_errexit {
                subshell.execute_ast(&ast)
            } else {
                // subst.c:7023-7029: without inherit_errexit the funsub
                // clears the -e flag for the body (same adjustment as
                // command_substitute's fork).
                subshell.suppress_errexit = 0;
                subshell.shell_state.env_vars.remove("__RUBASH_ERREXIT");
                crate::builtins::set::set_shell_option(
                    &mut subshell.shell_state.env_vars,
                    "errexit",
                    false,
                );
                subshell.execute_ast(&ast)
            }
        };
        let (result, captured) = if valsub {
            (run_body(&mut subshell), None)
        } else {
            subshell.stdout_capture = Some(Vec::new());
            let (thread_captured, result) =
                crate::executor::shell_options::capture_stdout(|| run_body(&mut subshell));
            let mut output = subshell.stdout_capture.take().unwrap_or_default();
            output.extend_from_slice(&thread_captured);
            (result, Some(output))
        };
        if subshell.parse_error_occurred {
            self.last_command_substitution_parse_error.set(true);
        }
        let status = command_substitution_result_status(result, subshell.exit_code);
        if let Some(saved_dir) = saved_dir {
            let _ = env::set_current_dir(saved_dir);
        }
        self.last_command_substitution_status.set(Some(status));
        match captured {
            Some(bytes) => crate::executor::substitution_metadata::bytes_to_shell_text(&bytes)
                .trim_capture_terminator()
                .to_string(),
            // valsub: the value is the body's final $REPLY, quote-protected
            // the way comsub output is (subst.c:7113 comsub_quote_string).
            None => protect_command_substitution_output(&substitution_result_visible_text(
                subshell
                    .shell_state
                    .env_vars
                    .get("REPLY")
                    .map(String::as_str)
                    .unwrap_or(""),
            )),
        }
    }

    /// y.tab.c:7191 parse_string_to_command (SEVAL_ONECMD, whole-string
    /// consumption) plus builtins/evalstring.c:199 can_optimize_cat_file:
    /// the `$(< file)` shortcut admits only a body that parses as exactly
    /// one simple command with no words, no assignments, and exactly one
    /// fd-0 r_input_direction. Anything else (trailing words, extra or
    /// non-input redirections, `;` lists, parse errors) returns None so the
    /// body reaches the real parser/executor below. The admission parse
    /// uses the real lexer/parser, not a text heuristic, so nested
    /// substitutions in the operand (`$(< $(echo f))`) and comment quirks
    /// (`$(< f # c)` is a read-time EOF error, not a cat) behave exactly as
    /// the fallback parse would. Returns the dequoted redirect target word
    /// and the raw (quote-carrying) form used for quote-state decisions.
    fn comsub_cat_file_target(&self, source: &str) -> Option<(String, String)> {
        let start_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok())
            .filter(|line| *line > 0)
            .unwrap_or(1);
        let tokens =
            crate::lexer::tokenize_comsub_body(source, self.posix_mode_enabled(), start_line, true);
        let ast = crate::parser::parse(&tokens);
        let [command] = ast.commands.as_slice() else {
            return None;
        };
        if command_has_parse_error(command) {
            return None;
        }
        if !command.words.is_empty() || !command.assignments.is_empty() {
            return None;
        }
        let [redirect] = command.redirects.as_slice() else {
            return None;
        };
        if redirect.kind != crate::parser::RedirectKind::Input {
            return None;
        }
        if redirect.fd.is_some_and(|fd| fd != 0) {
            return None;
        }
        Some((
            redirect.target.clone(),
            redirect.target_metadata.raw.clone(),
        ))
    }

    /// Runs the admitted `$(< file)` body: builtins/evalstring.c:759
    /// open_redir_file on the redirect operand. Expansion follows
    /// redir.c:298 redirection_expand — expand_words_no_vars on the single
    /// word: an unquoted expansion field-splits its output, unquoted glob
    /// characters (or glob characters produced by an unquoted expansion)
    /// pathname-expand, and zero or several resulting words return NULL,
    /// which open_redir_file reports as redirection_error (r,
    /// AMBIGUOUS_REDIRECT, 0) with the dequoted word text (redir.c:168-174
    /// redirection_error falls back to redirectee.filename->word). A single
    /// word is opened with open(2); failure prints internal_error
    /// ("%s: %s", fn, strerror (errno)) and the substitution yields empty
    /// output with EXECUTION_FAILURE (subst.c:7169-7173).
    fn comsub_cat_file_substitute(&self, target: &str, raw_word: &str) -> String {
        let ambiguity = |word: &str| -> String {
            let diagnostic = format!("{}{}: ambiguous redirect\n", self.diagnostic_prefix(), word);
            self.write_diagnostic_fd2(diagnostic.as_bytes());
            self.last_command_substitution_status.set(Some(1));
            String::new()
        };
        // redirection_expand materializes a process-substitution operand
        // as /dev/fd/N (subst.c process_substitute); the cat then reads the
        // substitution's output. Mirror that by running the substitution
        // and reading its captured stdout.
        if let Some(source) = target
            .strip_prefix("<(")
            .or_else(|| target.strip_prefix(">("))
            .and_then(|target| target.strip_suffix(')'))
        {
            let mut subshell = self.command_substitution_executor();
            let content = subshell
                .process_substitution_output(source)
                .unwrap_or_default();
            self.last_command_substitution_status.set(Some(0));
            return content.trim_capture_terminator().to_string();
        }
        let posix_no_glob = self.posix_mode_enabled();
        let quote_state = RedirectionWordQuotes::scan(raw_word);
        let expanded = strip_matching_quotes(&self.expand_word(target)).to_string();
        // Field split only output that came from an unquoted expansion; a
        // quoted or literal word is one field even when it contains IFS
        // characters.
        let fields = if quote_state.unquoted_expansion {
            self.field_split_values(&expanded)
        } else {
            vec![expanded]
        };
        // Pathname expansion: unquoted glob characters in the word, or glob
        // characters produced by an unquoted expansion (pathexp.c
        // unquoted_glob_pattern_p walks the word's quoting); posixly_correct
        // disables globbing for the operand (evalstring.c:763-766
        // open_redir_file sets disallow_filename_globbing).
        let glob_allowed =
            !posix_no_glob && (quote_state.unquoted_glob_char || quote_state.unquoted_expansion);
        let mut words: Vec<String> = Vec::new();
        for field in fields {
            if glob_allowed && !field.is_empty() {
                words.extend(self.apply_command_substitution_pathname_expansion(&field));
            } else {
                words.push(field);
            }
        }
        // redirection_expand returned NULL: zero words (empty unquoted
        // expansion) or several words (field split / glob match).
        let [file_word] = words.as_slice() else {
            return ambiguity(strip_matching_quotes(target));
        };
        // Model the Linux open(dir, O_RDONLY)+read dance: open succeeds and
        // the first zread fails EISDIR, so read_comsub (subst.c:6700-6712)
        // returns NULL with EXECUTION_SUCCESS — a directory operand is
        // silent, empty, rc 0. Windows CreateFile on a directory would
        // surface "Permission denied" instead.
        let read_path = shell_path_to_windows(file_word, &self.shell_state.env_vars);
        // A materialized input process substitution resolves to /dev/fd/N
        // (subst.c process_substitute + move_to_high_fd): the bytes live in
        // the fd table's ProcessSubstitution endpoint, not on disk. Reading
        // the path with fs would fail with ENOENT where GNU reads the fd.
        if let Some(fd_text) = file_word.strip_prefix("/dev/fd/") {
            if let Ok(fd) = fd_text.parse::<u32>() {
                if let Some((data, offset)) = self.fd_table.input_snapshot_bytes(fd) {
                    if let Some(content) = data.get(offset..).map(<[u8]>::to_vec) {
                        self.last_command_substitution_status.set(Some(0));
                        return bytes_to_shell_text(&content)
                            .trim_capture_terminator()
                            .to_string();
                    }
                }
            }
        }
        if read_path.is_dir() {
            self.last_command_substitution_status.set(Some(0));
            return String::new();
        }
        match fs::read_to_string(&read_path) {
            Ok(value) => {
                self.last_command_substitution_status.set(Some(0));
                value.trim_capture_terminator().to_string()
            }
            Err(error) => {
                // path_error already formats "<word>: <strerror>" (and maps
                // the Windows wildcard-path EINVAL to the ENOENT text
                // Linux open(2) reports — posix_errors.rs), so it is the
                // whole internal_error payload.
                let payload = crate::posix_errors::message(&crate::posix_errors::path_error(
                    file_word, error,
                ));
                let diagnostic = format!("{}{}\n", self.diagnostic_prefix(), payload);
                self.write_diagnostic_fd2(diagnostic.as_bytes());
                self.last_command_substitution_status.set(Some(1));
                String::new()
            }
        }
    }
}

/// Quote-state scan of a raw (quote-carrying) redirection word: mirrors the
/// lexer-level quoting that GNU expand_words_no_vars sees — an unquoted `$`
/// or backtick starts an expansion whose output field-splits and
/// pathname-expands, and an unquoted `*`/`?`/`[` makes the word a glob
/// pattern (pathexp.c:66 unquoted_glob_pattern_p). Escaped and quoted
/// characters never count.
struct RedirectionWordQuotes {
    unquoted_expansion: bool,
    unquoted_glob_char: bool,
}

impl RedirectionWordQuotes {
    fn scan(raw_word: &str) -> Self {
        let mut state = Self {
            unquoted_expansion: false,
            unquoted_glob_char: false,
        };
        let mut single_quoted = false;
        let mut double_quoted = false;
        let mut escaped = false;
        for ch in raw_word.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && !single_quoted {
                escaped = true;
                continue;
            }
            match ch {
                '\'' if !double_quoted => single_quoted = !single_quoted,
                '"' if !single_quoted => double_quoted = !double_quoted,
                '$' | '`' if !single_quoted && !double_quoted => {
                    state.unquoted_expansion = true;
                }
                '*' | '?' | '[' if !single_quoted && !double_quoted => {
                    state.unquoted_glob_char = true;
                }
                _ => {}
            }
        }
        state
    }
}

/// parser.h:85 FUNSUB_CHAR: the characters that may follow `${` to
/// introduce a bash-5.3 nofork command substitution. `|` selects the valsub
/// form (`${| cmd; }`), the others the funsub form (`${ cmd; }`).
pub(in crate::executor) fn funsub_introducer(ch: char) -> Option<bool> {
    match ch {
        ' ' | '\t' | '\n' => Some(false),
        '|' => Some(true),
        _ => None,
    }
}

/// Extract a `${ body; }` nofork-substitution body from `chars`, which is
/// positioned right after the introducer character following `{`. Returns
/// the body text and whether the form was valsub, with the closing `}`
/// consumed. Scanning mirrors the walker's `$()` arm: quotes and backslash
/// escapes are opaque, `(`/`)` nest (a `}` inside a nested substitution
/// never closes the construct), and `{`/`}` nest so an inner `${...}` or
/// brace group does not terminate the body. `None` (closing `}` never
/// found) falls through to the existing parameter diagnostics.
///
/// GNU parse.y:4451 parse_comsub parses the body with the real parser
/// (yyparse, DOLBRACE), so the closing `}` must be a LONE WORD in command
/// position — parse.y:3465-3468 special_case_tokens returns the `}' token
/// only when reserved_word_acceptable(last_read_token) holds: after `;`,
/// `&`, `|`, a newline, or a closed `{ }`/`( )` command construct, and with
/// a word-breaking character (blanks, `()<>;&|`) or end of input after it.
/// A `}` inside a word (`\"}\" in `${ echo \"}\"; }`, `a}b`, `x=}`) is word
/// text, and a `{` opens a brace group only in command position
/// (`${ echo {; }` runs `echo {`).
/// Consume one `$'-introduced unit (`${...}', `$(...)', `$((...))',
/// `$'...'') from `chars`, appending its full text to `out`. Shared by the
/// two executor funsub scanners.
///
/// GNU parse.y:5494 read_token_word (shellexp branch): `$' followed by `{',
/// `(' or `'' is read as ONE word unit at ANY word position — there is no
/// command-position gate on it, unlike the bare `{' group opener. Inside a
/// funsub body a nested `${ ... }' therefore never opens a bare brace group
/// and its matching `}' never terminates the body; only a word BEGINNING
/// with `}' does (parse.y:5400-5416, the PST_FUNSUBST special case).
pub(in crate::executor) fn consume_nested_dollar_unit(
    chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
    out: &mut String,
) {
    fn push_quoted(
        chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
        out: &mut String,
        close: char,
    ) {
        while let Some(qc) = chars.next() {
            out.push(qc);
            if qc == '\\' {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            } else if qc == close {
                break;
            }
        }
    }
    match chars.peek().copied() {
        Some('{') => {
            chars.next();
            let funsub = chars.peek().is_some_and(|c| *c == '|' || c.is_whitespace());
            if funsub {
                // extract_funsub_body's head consumes the introducer (`{'
                // or `|') and the body starts AT the blank — calling it
                // here (with `{' already consumed) would eat that blank
                // (`${ echo x; }' became `${echo x; }', and the funsub
                // introducer was lost downstream). Run the tail scan
                // directly, with the `{' already consumed above.
                if let Some(inner) = extract_funsub_body_tail(chars) {
                    out.push('{');
                    out.push_str(&inner);
                    out.push('}');
                } else {
                    out.push('{');
                }
                return;
            }
            out.push('{');
            let mut depth = 1usize;
            while let Some(pc) = chars.next() {
                match pc {
                    '\\' => {
                        out.push(pc);
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    }
                    '\'' | '"' => {
                        out.push(pc);
                        push_quoted(chars, out, pc);
                    }
                    '{' => {
                        depth += 1;
                        out.push(pc);
                    }
                    '}' => {
                        depth -= 1;
                        out.push(pc);
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => out.push(pc),
                }
            }
        }
        Some('(') => {
            chars.next();
            out.push('(');
            let mut depth = 1usize;
            while let Some(pc) = chars.next() {
                match pc {
                    '\\' => {
                        out.push(pc);
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    }
                    '\'' | '"' => {
                        out.push(pc);
                        push_quoted(chars, out, pc);
                    }
                    '(' => {
                        depth += 1;
                        out.push(pc);
                    }
                    ')' => {
                        depth -= 1;
                        out.push(pc);
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => out.push(pc),
                }
            }
        }
        Some('\'') => {
            chars.next();
            out.push('\'');
            push_quoted(chars, out, '\'');
        }
        _ => {}
    }
}

pub(in crate::executor) fn extract_funsub_body(
    chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
) -> Option<(String, bool)> {
    let introducer = *chars.peek()?;
    let valsub = funsub_introducer(introducer)?;
    chars.next();
    extract_funsub_body_tail(chars).map(|body| (body, valsub))
}

/// The body scan of `extract_funsub_body` with the introducer already
/// consumed: the iterator sits at the first body character (the blank after
/// `${' or the first byte after `${|'). Consumes through the terminating
/// `}' (exclusive) and returns the verbatim body text, or None when the
/// input ended unterminated.
fn extract_funsub_body_tail(
    chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
) -> Option<String> {
    let mut body = String::new();
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    // Command position: true at body start and after a command terminator
    // (`;` `&` `|` newline) or a closed `{ }` group / `( )` subshell — the
    // same term model continuation.rs's funsub delimiter carries.
    let mut term = true;
    while let Some(ch) = chars.next() {
        if escaped {
            body.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && !single {
            body.push(ch);
            escaped = true;
            if !double {
                term = false;
            }
            continue;
        }
        if single {
            if ch == '\'' {
                single = false;
            }
            body.push(ch);
            continue;
        }
        if double {
            if ch == '"' {
                double = false;
            }
            body.push(ch);
            continue;
        }
        let mut word_text = false;
        match ch {
            '$' => {
                // parse.y:5494 read_token_word (shellexp branch): a `$'
                // unit is one word element at any position — see
                // consume_nested_dollar_unit. Without this arm
                // `${ echo X${ echo nested; }Y; }' ended the outer body at
                // the inner funsub's `}' (341bf41b follow-up).
                body.push(ch);
                consume_nested_dollar_unit(chars, &mut body);
                term = false;
                continue;
            }
            '\'' => {
                single = true;
                term = false;
            }
            '"' => {
                double = true;
                term = false;
            }
            '(' => {
                paren_depth += 1;
                term = false;
            }
            ')' if paren_depth > 0 => {
                paren_depth -= 1;
                // A closed outermost `( )` is a complete command.
                term = paren_depth == 0;
            }
            '{' if term && paren_depth == 0 => {
                brace_depth += 1;
                // A command follows the opening brace.
                term = true;
            }
            '}' if term && paren_depth == 0 => {
                // parse.y:5407-5416 read_token_word: a word BEGINNING with
                // `}` in command position (reserved_word_acceptable)
                // terminates the substitution even when more characters
                // follow (`${| REPLY=x; }-tail`); term==false covers the
                // mid-word case (`a}b`, `\"}`) — those `}` are word text.
                if brace_depth > 0 {
                    // A closed `{ }` group is a complete command: the
                    // funsub's own `}` may follow without another
                    // separator (`${ { echo x; } }`).
                    brace_depth -= 1;
                    term = true;
                } else {
                    return Some(body);
                }
            }
            ';' | '&' | '|' | '\n' if paren_depth == 0 => term = true,
            ' ' | '\t' | '\r' => {}
            _ => word_text = true,
        }
        if word_text {
            term = false;
        }
        body.push(ch);
    }
    None
}

/// parse.y:4475-4516 (parse_comsub / xparse_dolparen SX_FUNSUB,
/// parse.y:4713-4743): the funsub body must be a terminated command list —
/// a `;` or newline immediately before the closing `}` (after optional
/// blanks). `${ echo 6 }` is a read-time "unexpected EOF" error in GNU, not
/// a substitution; rubash's executor cannot raise that read-time error, so
/// unterminated bodies fall back to the existing parameter diagnostics
/// instead of silently substituting.
pub(in crate::executor) fn funsub_body_is_terminated(body: &str) -> bool {
    body.trim_end_matches([' ', '\t']).ends_with(';') || body.ends_with('\n')
}

fn command_has_parse_error(command: &CommandNode) -> bool {
    command_parse_error_node(command).is_some()
}

fn command_parse_error_node(command: &CommandNode) -> Option<&CommandNode> {
    if command.has_assignment("__RUBASH_PARSE_ERROR__") {
        return Some(command);
    }
    command
        .and_or_list
        .as_ref()
        .and_then(|list| list.commands.iter().find_map(command_parse_error_node))
        .or_else(|| {
            command
                .pipeline_command
                .as_ref()
                .and_then(|pipeline| pipeline.stages.iter().find_map(command_parse_error_node))
        })
}

/// rubash#117 whitelist admission for the word-level command-substitution
/// shortcuts. GNU subst.c:7143 command_substitute sends every body through
/// parse_and_execute; a shortcut is only equivalent when the body is
/// provably a single simple command of literal words — no quoting
/// (`'` `"` `\` `` ` ``), no expansion (`$` `~` glob `[` `*` `?`), no
/// operators or redirections (`;` `|` `&` `<` `>` `(` `)` `{` `}`), no
/// comment/negation introducers (`#` `!`), and no control or non-ASCII
/// bytes (which can carry in-band markers). Anything else must reach the
/// real parser/executor.
fn command_substitution_body_is_trivial(source: &str) -> bool {
    let source = source.trim();
    !source.is_empty()
        && source.bytes().all(|byte| {
            matches!(byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
                | b' ' | b'\t'
                | b'_' | b'-' | b'+' | b'=' | b'.' | b',' | b'/' | b':' | b'@'
                | b'%' | b'^')
        })
}

fn strip_command_substitution_comments(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut single = false;
    let mut double = false;
    let mut comment = false;
    let mut boundary = true;
    let mut escaped = false;

    for ch in source.chars() {
        if comment {
            if ch == '\n' {
                comment = false;
                boundary = true;
                output.push(ch);
            }
            continue;
        }
        if escaped {
            escaped = false;
            boundary = false;
            output.push(ch);
            continue;
        }
        if ch == '\\' && !single {
            escaped = true;
            output.push(ch);
            continue;
        }
        if !double && ch == '\'' {
            single = !single;
            boundary = false;
            output.push(ch);
            continue;
        }
        if !single && ch == '"' {
            double = !double;
            boundary = false;
            output.push(ch);
            continue;
        }
        if !single && !double && ch == '#' && boundary {
            comment = true;
            continue;
        }
        // GNU read_token: `#` starts a comment only at a token boundary —
        // after whitespace or a separator, not mid-word (`$#`, `a#b`).
        boundary = ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>');
        output.push(ch);
    }

    output
}

fn restore_old_style_backtick_markers(value: &str) -> String {
    value
        .replace(DATA_DOLLAR, "$")
        .replace(crate::executor::markers::DATA_BACKTICK, "`")
        .replace(crate::executor::markers::PROTECTED_BACKSLASH, "\\")
        .replace(crate::executor::markers::DATA_BACKSLASH, "\\")
}

fn readfile_path_is_quoted(path: &str) -> bool {
    path.chars().any(|ch| matches!(ch, '\'' | '"' | '\\'))
}

fn command_substitution_result_status(result: Result<(), ExecuteError>, exit_code: i32) -> i32 {
    match result {
        Ok(()) => exit_code,
        Err(ExecuteError::Return(status)) => status,
        Err(ExecuteError::ExitCode(status)) | Err(ExecuteError::ExpansionFailure(status)) => status,
        Err(_) => 1,
    }
}
