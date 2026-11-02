use super::*;

pub(in crate::executor) fn unset_args_need_builtin_diagnostics(args: &[String]) -> bool {
    let mut functions = false;
    let mut variables = false;

    for arg in args {
        if arg == "--" || arg == "-" {
            break;
        }
        if !arg.starts_with('-') {
            break;
        }

        for option in arg[1..].chars() {
            match option {
                'f' => functions = true,
                'v' => variables = true,
                'n' => {}
                _ => return true,
            }
        }
    }

    functions && variables
}

pub(in crate::executor) fn command_has_no_effect(cmd: &CommandNode) -> bool {
    // A command carrying words is executable — GNU execute_cmd.c dispatches
    // every word-bearing command through execute_simple_command (and a brace
    // group's body through execute_command), so `time { echo x; }` must run
    // the group body. This predicate only recognizes nodes with literally
    // nothing to execute.
    cmd.words.is_empty()
        && cmd.assignments.is_empty()
        && cmd.redirect_in.is_none()
        && cmd.redirect_out.is_none()
        && cmd.append.is_none()
        && cmd.redirect_err.is_none()
        && cmd.redirect_err_append.is_none()
        && cmd.redirects.is_empty()
        && cmd.heredoc.is_none()
        && cmd.heredoc_delimiter.is_none()
        && cmd.here_string.is_none()
        && cmd.pipe.is_none()
        && cmd.and_or.is_none()
        && !cmd.background
        && !cmd.inverted
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && !cmd.subshell
        && !cmd.subshell_end
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.time_command.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.for_command.is_none()
        && cmd.arithmetic_command.is_none()
        && cmd.if_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.conditional_command.is_none()
        && cmd.subshell_command.is_none()
        && cmd.case_command.is_none()
        && cmd.function_command.is_none()
        && cmd
            .brace_group
            .as_ref()
            .map_or(true, |bg| bg.body.iter().all(command_has_no_effect))
}

pub(in crate::executor) fn normalize_leading_assignment_words(cmd: &mut CommandNode) {
    let mut count = 0;
    while let Some(word) = cmd.words.get(count) {
        let Some((name, value)) = split_assignment_word(word) else {
            break;
        };
        cmd.insert_assignment(name.to_string(), value.to_string());
        count += 1;
    }
    if count > 0 {
        cmd.words.drain(0..count);
    }
}

pub(in crate::executor) fn command_has_redirect(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || !cmd.redirects.is_empty()
}

pub(in crate::executor) fn function_definition_command_uses_source_text(
    command: &CommandNode,
) -> bool {
    command.words.is_empty()
        || command.pipeline_command.is_some()
        || command.and_or_list.is_some()
        || command.time_command.is_some()
        || command.background_command.is_some()
        || command.inverted_command.is_some()
        || command.arithmetic_command.is_some()
        || command.for_command.is_some()
        || command.if_command.is_some()
        || command.loop_command.is_some()
        || command.conditional_command.is_some()
        || command.subshell_command.is_some()
        || command.case_command.is_some()
        || command.select_command.is_some()
        || command.brace_group.is_some()
        || command.coproc_command.is_some()
        || command.function_command.is_some()
}

pub(in crate::executor) fn append_function_redirect(
    line: &mut String,
    redirect: Option<&crate::parser::Redirect>,
    op: &str,
) {
    let Some(redirect) = redirect else {
        return;
    };

    // A dynamic varredir already contributes its fd word (for example
    // {fd}<&0); Bash keeps the operator adjacent to that word.
    if redirect.fd_var.is_none() {
        line.push(' ');
    }
    let target = redirect.target.as_str();
    if target.starts_with('&') {
        // Bash keeps fd duplication operators adjacent to their raw token.
        if redirect.fd.is_none()
            && redirect.fd_var.is_none()
            && matches!(op, "<" | ">")
            && target[1..].chars().all(|ch| ch.is_ascii_digit())
        {
            line.push_str(if op == "<" { "0" } else { "1" });
        }
        line.push_str(op);
        line.push_str(target);
    } else {
        line.push_str(op);
        line.push(' ');
        line.push_str(target);
    }
}

pub(in crate::executor) fn fd_stdin_key(fd: u32) -> String {
    format!("{FD_STDIN_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_stdin_offset_key(fd: u32) -> String {
    format!("{FD_STDIN_OFFSET_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_dynamic_input_key(fd: u32) -> String {
    format!("{FD_DYNAMIC_INPUT_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_output_key(fd: u32) -> String {
    format!("{FD_OUTPUT_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_output_process_substitution_key(fd: u32) -> String {
    format!("{FD_OUTPUT_PROCESS_SUBSTITUTION_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_closed_key(fd: u32) -> String {
    format!("{FD_CLOSED_PREFIX}{fd}")
}

pub(in crate::executor) fn fd_terminal_key(fd: u32) -> String {
    format!("{FD_TERMINAL_PREFIX}{fd}")
}

pub(in crate::executor) fn command_has_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || cmd
            .redirects
            .iter()
            .any(|redirect| redirect.is_output_side())
}

pub(in crate::executor) fn command_has_input_or_output_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.heredoc.is_some()
        || cmd.here_string.is_some()
        || cmd
            .redirects
            .iter()
            .any(|redirect| redirect.is_input_side())
        || command_has_output_redirects(cmd)
}

/// GNU `line_number` ownership (execute_cmd.c): these command kinds stamp
/// `line_number` from their own `->line` field while they execute —
/// cm_simple (:936), cm_subshell (:696), cm_for (:3001), cm_select
/// (:3513), cm_case (:3653), cm_arith (:3904), cm_cond (:4138),
/// cm_arith_for (:3236). Everything else — while/until, if, `{ }`,
/// coproc, function definitions, and the pipeline/&&/||/&/!/time wrapper
/// nodes — runs under the ambient `line_number` left by the reader or
/// the enclosing command.
pub(in crate::executor) fn command_sets_own_line(cmd: &CommandNode) -> bool {
    cmd.loop_command.is_none()
        && cmd.if_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.coproc_command.is_none()
        && cmd.function_command.is_none()
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.time_command.is_none()
        && !command_is_time_prefixed_compound(cmd)
}

/// GNU `Simple->line` for a plain simple command is `simplecmd_lineno`
/// (parse.y:316), which read_token_word captures at the END of the first
/// word in command position (parse.y:5849-5851 `simplecmd_lineno =
/// line_number`). A first word that spans physical lines — a multi-line
/// assignment RHS or a quoted multi-line word — therefore stamps the line
/// where that word FINISHED reading, not the line where it started
/// (rubash#201: `x="$(for ...; done)"` spanning lines 1-3 executes with
/// line_number 3, so the comsub string is numbered from 3; verified
/// against WSL GNU Bash 5.3.0, probes q4/q7/q9 under
/// target/deep201/probes). Only plain simple commands use this; compound
/// keywords keep their own token line (For->line etc.).
pub(in crate::executor) fn simple_command_first_word_end_line(cmd: &CommandNode) -> Option<usize> {
    let start = cmd.line?;
    let simple = cmd.for_command.is_none()
        && cmd.select_command.is_none()
        && cmd.loop_command.is_none()
        && cmd.if_command.is_none()
        && cmd.case_command.is_none()
        && cmd.function_command.is_none()
        && cmd.brace_group.is_none()
        && cmd.subshell_command.is_none()
        && cmd.coproc_command.is_none()
        && cmd.conditional_command.is_none()
        && cmd.arithmetic_command.is_none()
        && cmd.pipeline_command.is_none()
        && cmd.and_or_list.is_none()
        && cmd.background_command.is_none()
        && cmd.inverted_command.is_none()
        && cmd.time_command.is_none();
    if !simple {
        return Some(start);
    }
    let first_word_newlines = if !cmd.assignments.is_empty() {
        let raw = cmd
            .assignment_raws
            .first()
            .filter(|raw| !raw.is_empty())
            .unwrap_or(&cmd.assignments[0].1);
        raw.matches('\n').count()
    } else {
        cmd.words.first().map(|word| word.matches('\n').count())?
    };
    Some(start + first_word_newlines)
}

pub(in crate::executor) fn command_needs_process_line_env(cmd: &CommandNode) -> bool {
    cmd.words
        .iter()
        .any(|word| command_word_needs_process_line_env(word))
}

fn command_word_needs_process_line_env(word: &str) -> bool {
    matches!(
        word,
        "alias"
            | "unalias"
            | "declare"
            | "typeset"
            | "export"
            | "readonly"
            | "hash"
            | "help"
            | "kill"
            | "let"
            | "local"
            | "shopt"
            | "shift"
            | "ulimit"
            | "unset"
    )
}

/// GNU print_cmd.c prints the command node's OWN redirection list. A
/// redirect cloned into this node by an enclosing compound's splice
/// (support_names::injected_group_redirect — the mark's own contract is
/// "NOT the command's own redirect") is fd-routing state, not text:
/// rendering it made trap-visible text inside `{ ...; } >>f 2>>f` read
/// `body_fn >> f 2>> f` where GNU reports `body_fn` (rubash#1872).
/// Every redirect-to-text funnel filters through here so no render of
/// the node can re-attach an ancestor's redirections.
fn own_redirect(redirect: Option<&Redirect>) -> Option<&Redirect> {
    redirect
        .filter(|redirect| !crate::executor::support_names::is_injected_group_redirect(redirect))
}

// rubash#2388: heredoc rendering for the respawn text renderer. GNU's
// printer emits a heredoc's HEADER inline in the redirect list and
// defers its BODY past the whole command line — past `|', `&&'/`||'
// connectors and the `&' terminator — printing the collected bodies
// after the line (print_cmd.c print_redirection_list /
// print_heredoc_bodies / print_deferred_heredocs). This renderer
// dropped heredocs entirely (the headers live only in cmd.redirects
// as RedirectKind::HereDoc and no mirror field carries them), so a
// respawned async child re-parsed a body-less header-less command and
// the heredoc content vanished. The thread-locals below mirror
// print_cmd.c's deferred_heredocs queue: connector arms hold the
// queue while they render their elements, and the outermost render
// (or the list separator logic) flushes the bodies after the line.
thread_local! {
    static DEFERRED_HEREDOC_BODIES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static HEREDOC_RENDER_HOLD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RENDER_ENDED_WITH_HEREDOC: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// RAII hold on the deferred-heredoc queue: while any hold is active,
/// rendered heredoc bodies queue instead of landing in the text, so a
/// connector arm can put its ` | ' / ` && ' / ` &' after the header
/// line and let the enclosing render flush the bodies past it.
struct HeredocRenderHold;

impl HeredocRenderHold {
    fn new() -> Self {
        HEREDOC_RENDER_HOLD.with(|hold| hold.set(hold.get() + 1));
        Self
    }
}

impl Drop for HeredocRenderHold {
    fn drop(&mut self) {
        HEREDOC_RENDER_HOLD.with(|hold| hold.set(hold.get() - 1));
    }
}

pub(in crate::executor) fn render_ended_with_heredoc() -> bool {
    RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.get())
}

/// The (headers, bodies) of `cmd`'s own heredocs, correlated exactly
/// like the parser-side printer (heredoc_redirect_info): the header
/// spells `[fd]<<[-]['DELIM']`; each body is the stored body text plus
/// its delimiter line, ready to be flushed after the command line.
fn heredoc_headers_and_bodies(cmd: &CommandNode) -> (Vec<String>, Vec<String>) {
    let mut headers = Vec::new();
    let mut bodies = Vec::new();
    let mut heredoc_index = 0usize;
    for redirect in &cmd.redirects {
        if redirect.kind != crate::parser::RedirectKind::HereDoc {
            continue;
        }
        let info = crate::parser::ast_print::heredoc_redirect_info(cmd, redirect, heredoc_index);
        heredoc_index += 1;
        headers.push(crate::parser::ast_print::heredoc_header(&info));
        bodies.push(crate::parser::ast_print::heredoc_body_text(cmd, &info));
    }
    (headers, bodies)
}

/// Attach rendered heredoc bodies to `text`: queued while a connector
/// arm holds the render, otherwise appended after the command line
/// (and the render is marked as ending with a heredoc body, which the
/// separator/closer logic consults — a delimiter line admits only a
/// newline after it).
fn attach_heredoc_bodies(text: &mut String, bodies: Vec<String>) {
    if bodies.is_empty() {
        return;
    }
    if HEREDOC_RENDER_HOLD.with(|hold| hold.get()) > 0 {
        DEFERRED_HEREDOC_BODIES.with(|queue| queue.borrow_mut().extend(bodies));
        return;
    }
    text.push('\n');
    text.push_str(&bodies.join("\n"));
    RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.set(true));
}

/// Flush the deferred queue into `text` (after the enclosing line).
/// Returns true when bodies were flushed.
fn flush_deferred_heredoc_bodies(text: &mut String) -> bool {
    let bodies = DEFERRED_HEREDOC_BODIES.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    if bodies.is_empty() {
        return false;
    }
    text.push('\n');
    text.push_str(&bodies.join("\n"));
    true
}

pub(in crate::executor) fn bash_command_text(cmd: &CommandNode) -> String {
    let mut parts = Vec::new();
    let raws_aligned = cmd.assignment_raws.len() == cmd.assignments.len();
    for (index, (name, value)) in cmd.assignments.iter().enumerate() {
        // GNU print_cmd.c print_simple_command prints each assignment
        // word's TEXT as lexed (rubash#202, ast_print.rs): the raw RHS
        // carries the source quotes, so `x="$(a) ($(b | cat))"` re-parses
        // as one assignment. The cooked value drops those quotes, which
        // corrupted respawned background children (rubash#1664: the
        // payload became `x=$(a) ($(b cat))`, a syntax error; rubash#1704:
        // a multi-line quoted value re-parsed as separate commands and a
        // quoted comsub lost its quotes in BASH_COMMAND and every other
        // text exposure). Prefer the raw RHS when the parser recorded
        // it; fall back to the cooked value (with the lexer's private
        // quoted-RHS marker stripped, so BASH_COMMAND never exposes
        // that sentinel). The raws are only trusted when they align with
        // the assignments; synthesized assignments carry no raw.
        let raw = raws_aligned
            .then(|| cmd.assignment_raws.get(index))
            .flatten()
            .filter(|raw| !raw.is_empty());
        let text = match raw {
            Some(raw) => format!("{name}={raw}"),
            None => {
                let value = value
                    .strip_prefix(crate::executor::markers::IFS_GLUE)
                    .unwrap_or(value);
                format!("{name}={value}")
            }
        };
        // GNU parse.y:4451/4632 (parse_comsub -> print_comsub) replaces every
        // `$()` body with the canonical print_cmd.c serialization at parse
        // time, so the enclosing command's BASH_COMMAND carries that form:
        // `v=$(echo    a   b)` exposes `v=$(echo a b)` and multi-clause
        // bodies expose the re-indented canonical text (rubash#274).
        parts.push(super::print_comsub::canonicalize_comsub_spans(&text));
    }
    let words = command_words_source_text_for_command(cmd);
    if !words.is_empty() {
        parts.push(words);
    }

    if let Some(redirect) = own_redirect(cmd.redirect_in.as_deref()) {
        parts.push(format_redirect("<", redirect));
    }
    if let Some(redirect) = own_redirect(cmd.redirect_out.as_deref()) {
        parts.push(format_redirect(
            if redirect.clobber { ">|" } else { ">" },
            redirect,
        ));
    }
    if let Some(redirect) = own_redirect(cmd.append.as_deref()) {
        parts.push(format_redirect(">>", redirect));
    }
    if let Some(redirect) = own_redirect(cmd.redirect_err.as_deref()) {
        parts.push(format_redirect("2>", redirect));
    }
    if let Some(redirect) = own_redirect(cmd.redirect_err_append.as_deref()) {
        parts.push(err_append_source_text(cmd, redirect));
    }
    // rubash#2388: heredoc headers ride the redirect parts (the bodies
    // attach after the joined line, print_cmd.c-style).
    let (heredoc_headers, heredoc_bodies) = heredoc_headers_and_bodies(cmd);
    parts.extend(heredoc_headers);
    if let Some(here_string) = &cmd.here_string {
        parts.push(format!("<<< {here_string}"));
    }

    let mut text = parts.join(" ");
    attach_heredoc_bodies(&mut text, heredoc_bodies);
    text
}

/// `2>&1` after `>file` is mirrored into `redirect_err_append` as a
/// synthesized `2>>file` (parser/redirect_assign.rs assign_redirect_err_target)
/// so legacy consumers can route fd 2 to the resolved file — but the ordered
/// `redirects` list keeps the real `2>&1` dup. Reprints must use the real
/// entry (GNU print_cmd.c prints `r_instruction` literally): a serialized
/// `2>>file` re-opened by a `-c` child loses the shared open file
/// description (redir7.sub's `eval \`...\`` spawned `( sleep > /dev/null
/// 2>> /dev/null & )` instead of `2>&1`).
fn err_append_source_text(cmd: &CommandNode, redirect: &Redirect) -> String {
    if !cmd.redirects.contains(redirect) {
        if let Some(dup) = cmd.redirects.iter().find(|entry| {
            matches!(entry.kind, crate::parser::RedirectKind::DuplicateOutput)
                && entry.fd.unwrap_or(1) == 2
        }) {
            return format_redirect(&dup.operator, dup);
        }
    }
    format_redirect("2>>", redirect)
}

pub(in crate::executor) fn command_words_source_text_for_command(cmd: &CommandNode) -> String {
    let mut rendered = cmd
        .words
        .iter()
        .enumerate()
        .map(|(index, word)| command_word_source_text(index, word, &cmd.word_metadata))
        .collect::<Vec<_>>();

    for assignment in &cmd.compound_assignments {
        if let Some(index) = assignment.word_index {
            if let Some(word) = rendered.get_mut(index) {
                *word = compound_assignment_source_text(assignment);
            }
        }
    }

    rendered.join(" ")
}

pub(in crate::executor) fn command_words_source_text(
    words: &[String],
    metadata: &[WordMetadata],
) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| command_word_source_text(index, word, metadata))
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_word_source_text(index: usize, word: &str, metadata: &[WordMetadata]) -> String {
    let text = metadata
        .get(index)
        .filter(|metadata| metadata.value == *word && !metadata.raw.is_empty())
        .map(|metadata| metadata.raw.clone())
        .unwrap_or_else(|| shell_single_quote_assignment_value(word));
    // Every `$()` span in the word text is the canonical print_comsub
    // serialization in GNU (parse.y:4632), not the raw source text
    // (rubash#274).
    super::print_comsub::canonicalize_comsub_spans(&text)
}

fn compound_assignment_source_text(assignment: &crate::parser::CompoundAssignment) -> String {
    format!(
        "{}{}{}",
        assignment.name, assignment.operator, assignment.value
    )
}

pub(in crate::executor) fn bash_command_sequence_text(commands: &[CommandNode]) -> String {
    // print_cmd.c:1520-1529 semicolon(): GNU suppresses the `;' separator
    // when the printed command ends with `" &"` — after `&` the list
    // grammar admits only a newline_list (parse.y:1275/1290
    // `list1 '&' newline_list [list1]`; a `;' at list start is the
    // parse.y:1326 error arm). Joining a serialized `... &` command with
    // `"; "` produced `true &; while ...`, a syntax error that killed
    // every respawned background child whose enclosing script carried a
    // function definition (rubash#294: the async `kill $$` never ran, so
    // a trap-terminated `until` loop hung forever).
    let mut text = String::new();
    // rubash#2388: a part that ends with a heredoc delimiter line
    // takes the newline separator too — the delimiter must stand alone
    // on its line, so neither `; ' nor anything else may follow it
    // (print_cmd.c print_deferred_heredocs swallows the `;' for the
    // newline the same way).
    let mut prev_ended_with_heredoc = false;
    for part in commands.iter().map(bash_command_source_text) {
        let ended_with_heredoc = render_ended_with_heredoc();
        if part.trim().is_empty() {
            continue;
        }
        if !text.is_empty() {
            if text.ends_with('&') || prev_ended_with_heredoc {
                text.push('\n');
            } else {
                text.push_str("; ");
            }
        }
        text.push_str(&part);
        prev_ended_with_heredoc = ended_with_heredoc;
    }
    // The flag must describe THIS text for the separator/closer logic
    // of the enclosing arm (a trailing filtered-out part may have
    // reset it during its own render).
    RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.set(prev_ended_with_heredoc));
    text
}

/// Whitelist raw-join render for BASH_COMMAND text (rubash#117
/// discipline): the twin of `try_render_literal_command` for words that
/// carry unquoted parameter expansions (`export BK_V$i=x`). When every
/// word's cooked text is byte-identical to its raw source, no raw holds a
/// `$(` span, no assignments/redirects/heredocs are present, and no
/// compound command shape is set, the full `bash_command_source_text`
/// pipeline provably returns `words.join(" ")`: the simple-command arm
/// renders assignments (absent here) plus per-word `metadata.raw` (equal
/// to the word by admission, and `canonicalize_comsub_spans` is the
/// identity without `$(`), joined with single spaces, with no redirects
/// to append and no compound-assignment overlay to apply. Any doubt falls
/// through to the full renderer. Owned `String` in and out.
pub(in crate::executor) fn try_render_raw_join_command(cmd: &CommandNode) -> Option<String> {
    if cmd.for_command.is_some()
        || cmd.pipeline_command.is_some()
        || cmd.and_or_list.is_some()
        || cmd.time_command.is_some()
        || cmd.background_command.is_some()
        || cmd.inverted_command.is_some()
        || cmd.arithmetic_command.is_some()
        || cmd.if_command.is_some()
        || cmd.loop_command.is_some()
        || cmd.conditional_command.is_some()
        || cmd.subshell_command.is_some()
        || cmd.select_command.is_some()
        || cmd.case_command.is_some()
        || cmd.coproc_command.is_some()
        || cmd.function_command.is_some()
        || cmd.brace_group.is_some()
    {
        return None;
    }
    if !cmd.assignments.is_empty()
        || !cmd.compound_assignments.is_empty()
        || cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || !cmd.redirects.is_empty()
        || cmd.heredoc.is_some()
        || !cmd.heredoc_redirects.is_empty()
        || cmd.here_string.is_some()
    {
        return None;
    }
    if cmd.words.is_empty() {
        return None;
    }
    for (index, word) in cmd.words.iter().enumerate() {
        // Mirror `command_word_source_text`'s own selection: the slow path
        // renders `metadata.raw` only when the recorded cooked value still
        // equals the word and the raw is non-empty; anything else takes the
        // `shell_single_quote_assignment_value` fallback, which this join
        // cannot reproduce.
        let raw_matches = cmd.word_metadata.get(index).is_some_and(|metadata| {
            !metadata.raw.is_empty() && metadata.raw == *word && metadata.value == *word
        });
        if !raw_matches {
            return None;
        }
        // `canonicalize_comsub_spans` short-circuits without `$(` and
        // leaves every other byte untouched, so the per-word slow-path
        // text is exactly `raw` here. Backticks take the same no-op
        // branch today, but admit strictly: a backtick span would still
        // render byte-identically, yet keeping it out costs nothing and
        // keeps the admission obviously sound.
        if word.contains("$(") || word.contains('`') {
            return None;
        }
    }
    Some(cmd.words.join(" "))
}

/// Whitelist fast render for BASH_COMMAND text (rubash#117 discipline):
/// when every word is fast-literal with `raw == word`, no compound command
/// shape is set, and no expandable structure exists, the full
/// `bash_command_source_text` pipeline provably returns `words.join(" ")`:
/// the simple-command arm renders assignments (absent here) plus
/// per-word `metadata.raw` (equal to the word by admission, and
/// `canonicalize_comsub_spans` is identity without `$(`), joined with
/// single spaces, and there are no redirects to append. Any doubt falls
/// through to the full renderer. Owned `String` in and out — no pointers,
/// no lifetime tracking, no way to render a stale or freed node.
pub(in crate::executor) fn try_render_literal_command(cmd: &CommandNode) -> Option<String> {
    if cmd.for_command.is_some()
        || cmd.pipeline_command.is_some()
        || cmd.and_or_list.is_some()
        || cmd.time_command.is_some()
        || cmd.background_command.is_some()
        || cmd.inverted_command.is_some()
        || cmd.arithmetic_command.is_some()
        || cmd.if_command.is_some()
        || cmd.loop_command.is_some()
        || cmd.conditional_command.is_some()
        || cmd.subshell_command.is_some()
        || cmd.select_command.is_some()
        || cmd.case_command.is_some()
        || cmd.coproc_command.is_some()
        || cmd.function_command.is_some()
        || cmd.brace_group.is_some()
    {
        return None;
    }
    if super::Executor::command_has_expandable_structure(cmd) {
        return None;
    }
    if cmd.words.is_empty() {
        return None;
    }
    for (index, word) in cmd.words.iter().enumerate() {
        if !super::assignment_helpers::is_expand_fast_literal_word(word) {
            return None;
        }
        let raw_matches = cmd
            .word_metadata
            .get(index)
            .is_some_and(|metadata| metadata.raw == *word);
        if !raw_matches {
            return None;
        }
    }
    Some(cmd.words.join(" "))
}

pub(in crate::executor) fn bash_command_source_text(cmd: &CommandNode) -> String {
    // rubash#2388: the outermost render of a command flushes the
    // heredoc bodies the connector arms deferred (print_cmd.c prints
    // them after the complete command line) and records whether the
    // text ends with a delimiter line, which the list separator and
    // closer logic consult.
    RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.set(false));
    let mut text = bash_command_source_text_render(cmd);
    if HEREDOC_RENDER_HOLD.with(|hold| hold.get()) == 0 && flush_deferred_heredoc_bodies(&mut text)
    {
        RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.set(true));
    }
    text
}

fn bash_command_source_text_render(cmd: &CommandNode) -> String {
    let mut text = if let Some(for_command) = &cmd.for_command {
        for_command_source_text(for_command)
    } else if let Some(pipeline_command) = &cmd.pipeline_command {
        pipeline_command_source_text(pipeline_command)
    } else if let Some(and_or_list) = &cmd.and_or_list {
        and_or_list_source_text(and_or_list)
    } else if let Some(time_command) = &cmd.time_command {
        time_command_source_text(time_command)
    } else if let Some(background_command) = &cmd.background_command {
        background_command_source_text(background_command)
    } else if let Some(inverted_command) = &cmd.inverted_command {
        inverted_command_source_text(inverted_command)
    } else if let Some(arithmetic_command) = &cmd.arithmetic_command {
        arithmetic_command_source_text(arithmetic_command)
    } else if let Some(if_command) = &cmd.if_command {
        if_command_source_text(if_command)
    } else if let Some(loop_command) = &cmd.loop_command {
        loop_command_source_text(loop_command)
    } else if let Some(conditional_command) = &cmd.conditional_command {
        conditional_command_source_text(conditional_command)
    } else if let Some(subshell_command) = &cmd.subshell_command {
        subshell_command_source_text(subshell_command)
    } else if let Some(select_command) = &cmd.select_command {
        select_command_source_text(select_command)
    } else if let Some(case_command) = &cmd.case_command {
        case_command_source_text(case_command)
    } else if let Some(coproc_command) = &cmd.coproc_command {
        coproc_command_source_text(coproc_command)
    } else if let Some(function_command) = &cmd.function_command {
        function_command_source_text(function_command)
    } else if let Some(brace_group) = &cmd.brace_group {
        let body = bash_command_sequence_text(&brace_group.body);
        // print_cmd.c:693-728 print_group_command: the closer is
        // `semicolon(); " "' before `}' — and semicolon()
        // (print_cmd.c:1520-1530) prints NOTHING when the accumulated
        // output ends with `" &"' or `\n' (only a newline_list may
        // follow `&': parse.y:1275/1290), so GNU spells a trailing
        // background command `{ ...cmd & }'. The unconditional `; }'
        // here rendered `{ ...cmd &; }' — unparseable — corrupting
        // every serialized function body that contains a
        // redirect-carrying brace group ending in `&' (the modernish
        // die trap's `{ { ...; } &; } 8<&0', rubash#2493) inside
        // respawned background children. push_command_separator is the
        // repo's #294/#2388 spelling of the same suppression: newline
        // after `&' or a heredoc delimiter line, `; ' otherwise.
        let mut text = format!("{{ {body}");
        push_command_separator(&mut text);
        text.push('}');
        text
    } else {
        // bash_command_text already appends the command's redirections;
        // routing it through append_source_redirects below would render
        // each one twice (`2> /dev/null 2> /dev/null`).
        return bash_command_text(cmd);
    };
    append_source_redirects(&mut text, cmd);
    text
}

fn for_command_source_text(for_command: &ForCommand) -> String {
    let body = command_body_source_text(
        for_command.body_kind,
        for_command.body_open_delimiter.as_deref(),
        for_command.body_close_delimiter.as_deref(),
        &for_command.body,
    );
    if let Some(arithmetic) = &for_command.arithmetic {
        return format!(
            "for (( {}; {}; {} )); {}",
            arithmetic.init, arithmetic.test, arithmetic.update, body
        );
    }

    if for_command.default_positional {
        // GNU print_cmd.c:602 print_for_command_head reprints the implicit
        // `for i; do` form's map_list as the literal `"$@"` word.
        format!("for {} in \"$@\"; {}", for_command.variable, body)
    } else {
        format!(
            "for {} in {}; {}",
            for_command.variable,
            for_command.words.join(" "),
            body
        )
    }
}

fn pipeline_command_source_text(pipeline_command: &PipelineCommand) -> String {
    // rubash#2388: stage heredoc bodies defer past the whole pipeline
    // line (print_cmd.c flushes them after the connection).
    let _hold = HeredocRenderHold::new();
    pipeline_command
        .stages
        .iter()
        .map(bash_command_source_text)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The per-stage printed commands GNU records on a job's `j->pipe`
/// PROCESS list (jobs.c:643 REVERSE_LIST — first stage first), which is
/// what builtins/common.c:632 get_job_by_name matches `%name`/`%?name`
/// against. A `a | b` job contributes `["a", "b"]`; `!`/`time` are
/// pipeline flags, not stage text, so they unwrap before the stage
/// split. Anything else contributes its whole printed command.
pub(in crate::executor) fn bash_job_stage_commands(cmd: &CommandNode) -> Vec<String> {
    let mut inner = cmd;
    loop {
        if let Some(inverted_command) = &inner.inverted_command {
            inner = &inverted_command.command;
        } else if let Some(time_command) = &inner.time_command {
            inner = &time_command.command;
        } else {
            break;
        }
    }
    if let Some(pipeline_command) = &inner.pipeline_command {
        return pipeline_command
            .stages
            .iter()
            .map(bash_command_source_text)
            .collect();
    }
    vec![bash_command_source_text(cmd)]
}

fn and_or_list_source_text(and_or_list: &AndOrListCommand) -> String {
    // rubash#2388: element heredoc bodies defer past the connectors,
    // to the end of the list line (print_cmd.c print_deferred_heredocs).
    let _hold = HeredocRenderHold::new();
    let mut text = String::new();
    for (index, command) in and_or_list.commands.iter().enumerate() {
        if index > 0 {
            let connector = and_or_list
                .connectors
                .get(index - 1)
                .copied()
                .unwrap_or(true);
            text.push_str(if connector { " && " } else { " || " });
        }
        text.push_str(&bash_command_source_text(command));
    }
    text
}

fn time_command_source_text(time_command: &TimeCommand) -> String {
    let mut parts = vec!["time".to_string()];
    if time_command.posix_format {
        parts.push("-p".to_string());
    }
    if time_command.inverted {
        parts.push("!".to_string());
    }
    parts.push(bash_command_source_text(&time_command.command));
    parts.join(" ")
}

fn background_command_source_text(background_command: &BackgroundCommand) -> String {
    // rubash#2388: the ` &' terminator belongs on the header line; the
    // inner command's heredoc bodies flush after it.
    let _hold = HeredocRenderHold::new();
    format!(
        "{} &",
        bash_command_source_text(&background_command.command)
    )
}

fn inverted_command_source_text(inverted_command: &InvertedCommand) -> String {
    format!("! {}", bash_command_source_text(&inverted_command.command))
}

fn arithmetic_command_source_text(arithmetic_command: &ArithmeticCommand) -> String {
    format!("(( {} ))", arithmetic_command.expression)
}

fn if_command_source_text(if_command: &IfCommand) -> String {
    // print_cmd.c print_if_command: each body ends with
    // `semicolon(); newline("elif"/"else"/"fi")' — the print_cmd.c:1519-1529
    // semicolon() suppression applies after a body ending in `... &'
    // (rubash#294 family).
    let mut text = format!(
        "if {}; then {}",
        bash_command_sequence_text(&if_command.condition),
        bash_command_sequence_text(&if_command.then_body)
    );
    for branch in &if_command.elif_branches {
        push_command_separator(&mut text);
        text.push_str(&format!(
            "elif {}; then {}",
            bash_command_sequence_text(&branch.condition),
            bash_command_sequence_text(&branch.body)
        ));
    }
    if let Some(body) = &if_command.else_body {
        push_command_separator(&mut text);
        text.push_str("else ");
        text.push_str(&bash_command_sequence_text(body));
    }
    push_command_separator(&mut text);
    text.push_str("fi");
    text
}

fn loop_command_source_text(loop_command: &LoopCommand) -> String {
    // print_cmd.c:800-832 print_until_or_while: the body is printed, then
    // `semicolon(); newline("done")' — semicolon() (print_cmd.c:1519-1529)
    // suppresses the `;' after a body whose last command printed as
    // `... &' (only newline_list may follow `&': parse.y:1275/1290), so a
    // body ending in `&' joins its `done' with a newline, never `&; done'
    // (rubash#294 family: an alias-spliced `while ...; do break &; done'
    // inside a respawned background function corrupted every respawn).
    let mut text = format!(
        "{} {}; {} ",
        if loop_command.until { "until" } else { "while" },
        bash_command_sequence_text(&loop_command.condition),
        loop_command.body_open_delimiter
    );
    text.push_str(&bash_command_sequence_text(&loop_command.body));
    push_command_separator(&mut text);
    text.push_str(&loop_command.body_close_delimiter);
    text
}

fn conditional_command_source_text(conditional_command: &ConditionalCommand) -> String {
    format!("[[ {}", conditional_command.args.join(" "))
}

fn subshell_command_source_text(subshell_command: &SubshellCommand) -> String {
    let body = bash_command_sequence_text(&subshell_command.body);
    if render_ended_with_heredoc() {
        // rubash#2388: the closing paren goes on its own line after a
        // trailing heredoc body — a delimiter line admits nothing else.
        format!("( {body}\n)")
    } else {
        format!("( {body} )")
    }
}

/// rubash#720 form 2: the print_cmd text of a FLAT `f() ( list )` region —
/// the same shape subshell_command_source_text renders for a `( list )`
/// node, because GNU's make_command_string of the subshell is what
/// the_printed_command_except_trap carries at the :769 ERR wait
/// (execute_cmd.c:703-710) and the flat region has no node of its own.
pub(in crate::executor) fn flat_subshell_region_source_text(body: &[CommandNode]) -> String {
    format!("( {} )", bash_command_sequence_text(body))
}

fn select_command_source_text(select_command: &SelectCommand) -> String {
    let body = command_body_source_text(
        select_command.body_kind,
        select_command.body_open_delimiter.as_deref(),
        select_command.body_close_delimiter.as_deref(),
        &select_command.body,
    );
    if select_command.default_positional {
        // GNU print_cmd.c:656 print_select_command_head reprints the
        // implicit `select x; do` form's map_list as the literal `"$@"`.
        format!("select {} in \"$@\"; {}", select_command.variable, body)
    } else {
        format!(
            "select {} in {}; {}",
            select_command.variable,
            select_command.words.join(" "),
            body
        )
    }
}

/// print_cmd.c:1519-1529 semicolon(): GNU suppresses the `;' separator
/// when the printed command ends with `" &"` or a newline — after `&` the
/// list grammar admits only newline_list (parse.y:1275/1290; a `;' at list
/// start is the parse.y:1326 error arm). Serializers joining a trailing
/// `... &' command with the next text (or a compound closer — print_cmd.c
/// 626-631 emits `semicolon(); newline("done")') must join with a newline
/// instead (rubash#294 `&;' corruption family).
pub(in crate::executor) fn push_command_separator(source: &mut String) {
    // rubash#2388: a body ending with a heredoc delimiter line takes
    // the newline form too — the delimiter must stand alone on its
    // line before `fi' / `done' / `}' may follow.
    if source.ends_with('&') || render_ended_with_heredoc() {
        source.push('\n');
    } else if !source.ends_with('\n') {
        source.push_str("; ");
    }
}

fn command_body_source_text(
    body_kind: CommandBodyKind,
    open_delimiter: Option<&str>,
    close_delimiter: Option<&str>,
    body: &[CommandNode],
) -> String {
    let mut body = bash_command_sequence_text(body);
    // print_cmd.c:626-631/646-651: GNU prints the body then
    // `semicolon(); newline("done")' — the semicolon() suppression rule
    // (print_cmd.c:1519-1529) applies, so a body ending in `... &' joins
    // its closer with a newline, never `&; done' (rubash#294 family).
    match body_kind {
        CommandBodyKind::DoDone => {
            body = format!("{} ", open_delimiter.unwrap_or("do")) + &body;
            push_command_separator(&mut body);
            body.push_str(close_delimiter.unwrap_or("done"));
            body
        }
        CommandBodyKind::BraceGroup => {
            body = format!("{} ", open_delimiter.unwrap_or("{")) + &body;
            push_command_separator(&mut body);
            body.push_str(close_delimiter.unwrap_or("}"));
            body
        }
    }
}

/// GNU print_cmd.c:731-739 `print_case_command_head`: the `case` command's
/// printed form — `the_printed_command` / `the_printed_command_except_trap`,
/// which is what `$BASH_COMMAND` exposes — is the HEAD ONLY:
/// `case WORD in ` with WORD still carrying its source quote characters
/// (print_cmd.c:735-738 prints `case_command->word->word`; execute_cmd.c:3655
/// resets `command_string_index` and prints the head before expanding the
/// word, so a `${BASH_COMMAND}` inside the case word sees exactly this text —
/// probe: `case "${BASH_COMMAND}" in 'case "${BASH_COMMAND}" in '*)` matches
/// under GNU bash 5.3). The clause list belongs to the JOB's `command` field
/// (jobs.c make_job -> print_command), which rubash obtains separately via
/// [`case_command_source_text`] at background registration — not to the
/// per-execution printed command.
pub(in crate::executor) fn case_command_head_source_text(case_command: &CaseCommand) -> String {
    let raw_word = if case_command.word_metadata.raw.is_empty() {
        case_command.word.as_str()
    } else {
        case_command.word_metadata.raw.as_str()
    };
    format!("case {raw_word} in ")
}

fn case_command_source_text(case_command: &CaseCommand) -> String {
    // GNU print_cmd.c:750 (print_case_command) prints the subject word
    // and every clause pattern from their raw token text — quotes
    // intact — with patterns joined by command_print_word_list's
    // " | " (print_cmd.c:782). Rendering the cooked forms instead
    // dropped the quoting: an empty-string pattern `''` rendered as
    // nothing (an unparseable ` )` clause head) and a quoted subject
    // lost its quotes, corrupting every respawned background child
    // whose prelude carried such a case (rubash#1704:
    // nvm_download_artifact). Same raw contract ComsubPrinter's case
    // arm already keeps.
    let word = if case_command.word_metadata.value == case_command.word
        && !case_command.word_metadata.raw.is_empty()
    {
        case_command.word_metadata.raw.clone()
    } else {
        case_command.word.clone()
    };
    let mut text = format!("case {} in", word);
    for clause in &case_command.clauses {
        text.push(' ');
        let patterns = clause
            .patterns
            .iter()
            .enumerate()
            .map(|(index, pattern)| {
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
        if clause.patterns.first().is_some_and(|p| p == "esac") {
            text.push('(');
        }
        text.push_str(&patterns);
        text.push_str(") ");
        text.push_str(&bash_command_sequence_text(&clause.body));
        // GNU print_cmd.c:787-793 (print_case_clauses): the deferred
        // heredoc bodies flush after the clause action
        // (PRINT_DEFERRED_HEREDOCS, print_cmd.c:787) and the clause
        // terminator is then printed via newline(";;") /
        // newline(";&") / newline(";;&") — it starts on its OWN line.
        // When the clause body ends with a heredoc body, the delimiter
        // line admits nothing after it, so the terminator takes a
        // newline instead of the usual space: gluing it onto the
        // delimiter line (`HD ;;`) leaves the delimiter unterminated
        // and the whole case unparseable in the respawned child
        // (rubash#2429).
        if render_ended_with_heredoc() {
            text.push('\n');
        } else {
            text.push(' ');
        }
        text.push_str(match clause.terminator {
            CaseTerminator::Break => ";;",
            CaseTerminator::FallThrough => ";&",
            CaseTerminator::TestNext => ";;&",
        });
    }
    text.push_str(" esac");
    // The case text ends with `esac`, never with a heredoc body:
    // leave the flag describing THIS text for the separator/closer
    // logic of the enclosing render (the same contract
    // bash_command_sequence_text keeps for its own text).
    RENDER_ENDED_WITH_HEREDOC.with(|flag| flag.set(false));
    text
}

fn coproc_command_source_text(coproc_command: &crate::parser::CoprocCommand) -> String {
    let mut text = String::from("coproc");
    // GNU print_cmd.c `cm_coproc` (5.3.20) prints the name ONLY when
    // the body is not a simple command: `coproc sleep 5` for a
    // simple body (named or not — there is no `COPROC WORD
    // simple_command` production, so a simple body never carries a
    // real name), but `coproc COPROC { sleep 5; }` / `coproc C1
    // { sleep 5; }` for compound bodies. The old comment here
    // claimed GNU always prints the name; that matches bash 5.2's
    // print code, not 5.3.20's (rubash#1645).
    if coproc_command.body_kind != crate::parser::CoprocBodyKind::SimpleCommand {
        text.push(' ');
        text.push_str(coproc_command.name.as_deref().unwrap_or("COPROC"));
    }
    text.push(' ');
    if let Some(body) = &coproc_command.body {
        text.push_str("{ ");
        text.push_str(&bash_command_sequence_text(body));
        // print_cmd.c:357-363 (cm_coproc) delegates the body to
        // make_command_string_internal — a group body closes through
        // print_group_command's `semicolon()' (print_cmd.c:1520-1530
        // suppression after `... &' / a trailing newline), so the
        // closer never glues `; }' onto a trailing background command
        // (`coproc C { cmd &; }' is unparseable — rubash#2493 family).
        push_command_separator(&mut text);
        text.push('}');
    } else {
        text.push_str(&command_words_source_text(
            &coproc_command.words,
            &coproc_command.word_metadata,
        ));
    }
    text
}

fn function_command_source_text(function_command: &crate::parser::FunctionCommand) -> String {
    let mut text = if function_command.keyword {
        format!("function {}", function_command.name)
    } else {
        function_command.name.clone()
    };
    if function_command.has_parentheses {
        text.push_str("()");
    }

    let body = bash_command_sequence_text(&function_command.body);
    match function_command.body_kind {
        FunctionBodyKind::BraceGroup => {
            // rubash#2388: after a trailing heredoc body the closer
            // takes a newline, never `; }' (delimiter line rule).
            // print_cmd.c:1323+ print_function_def closes with
            // `newline("}")' — and the semicolon() suppression
            // (print_cmd.c:1520-1530) applies before it in every
            // printer, so a body ending `... &' takes the newline
            // form too: `f() { cmd &; }' is unparseable and corrupted
            // respawned children whose prelude carried such a nested
            // definition (rubash#2493 family).
            let mut body_text = format!(
                "{} {} {}",
                text,
                function_command
                    .body_open_delimiter
                    .as_deref()
                    .unwrap_or("{"),
                body
            );
            push_command_separator(&mut body_text);
            body_text.push_str(
                function_command
                    .body_close_delimiter
                    .as_deref()
                    .unwrap_or("}"),
            );
            body_text
        }
        FunctionBodyKind::Subshell => {
            let separator = if render_ended_with_heredoc() {
                "\n"
            } else {
                " "
            };
            format!(
                "{} {} {}{}{}",
                text,
                function_command
                    .body_open_delimiter
                    .as_deref()
                    .unwrap_or("("),
                body,
                separator,
                function_command
                    .body_close_delimiter
                    .as_deref()
                    .unwrap_or(")")
            )
        }
        FunctionBodyKind::CommandSequence | FunctionBodyKind::CompoundCommand => {
            format!("{text} {body}")
        }
    }
}

pub(in crate::executor) fn append_source_redirects(text: &mut String, cmd: &CommandNode) {
    append_function_redirect(text, own_redirect(cmd.redirect_in.as_deref()), "<");
    let combined = own_redirect(cmd.redirect_out.as_deref())
        .filter(|redirect| {
            matches!(
                redirect.kind,
                crate::parser::RedirectKind::CombinedOutput
                    | crate::parser::RedirectKind::CombinedAppend
            )
        })
        .or_else(|| {
            own_redirect(cmd.redirect_err_append.as_deref()).filter(|redirect| {
                matches!(
                    redirect.kind,
                    crate::parser::RedirectKind::CombinedOutput
                        | crate::parser::RedirectKind::CombinedAppend
                )
            })
        });
    if let Some(redirect) = combined {
        let op = if redirect.kind == crate::parser::RedirectKind::CombinedAppend {
            "&>>"
        } else {
            "&>"
        };
        append_function_redirect(text, Some(redirect), op);
    } else {
        append_function_redirect(
            text,
            own_redirect(cmd.redirect_out.as_deref()),
            own_redirect(cmd.redirect_out.as_deref())
                .filter(|redirect| redirect.clobber)
                .map(|_| ">|")
                .unwrap_or(">"),
        );
        append_function_redirect(text, own_redirect(cmd.append.as_deref()), ">>");
        append_function_redirect(text, own_redirect(cmd.redirect_err.as_deref()), "2>");
        if let Some(redirect) = own_redirect(cmd.redirect_err_append.as_deref()) {
            text.push(' ');
            text.push_str(&err_append_source_text(cmd, redirect));
        }
    }
    // Numbered redirects (fd>=3) and `{var}` dynamic fds live only in the
    // ordered `redirects` list — the mirror fields above cannot represent
    // them. GNU print_cmd prints every redirection, so reprint them too or
    // a `-c` child loses `3>f` entirely.
    for redirect in &cmd.redirects {
        // HereDoc entries render through the heredoc header/body path
        // below (a bare format_redirect would print the delimiter as a
        // filename target and lose the body) — rubash#2388.
        if redirect.is_list_only_redirect() && redirect.kind != crate::parser::RedirectKind::HereDoc
        {
            text.push(' ');
            text.push_str(&format_redirect(&redirect.operator, redirect));
        }
    }
    // rubash#2388: a compound node's own heredocs (`( ... ) <<EOF',
    // `while ...; done <<EOF') — headers here, bodies after the whole
    // compound text (attach_heredoc_bodies defers under a hold).
    let (heredoc_headers, heredoc_bodies) = heredoc_headers_and_bodies(cmd);
    for header in heredoc_headers {
        text.push(' ');
        text.push_str(&header);
    }
    if let Some(here_string) = &cmd.here_string {
        text.push_str(" <<< ");
        text.push_str(here_string);
    }
    attach_heredoc_bodies(text, heredoc_bodies);
}

#[cfg(test)]
mod literal_render_tests {
    use super::*;
    use crate::executor::assignment_helpers::is_expand_fast_literal_word;

    fn parse_one(src: &str) -> crate::parser::CommandNode {
        let tokens = crate::lexer::tokenize(src);
        let ast = crate::parser::parse(&tokens);
        assert_eq!(ast.commands.len(), 1, "{src:?} should parse to one command");
        ast.commands.into_iter().next().unwrap()
    }

    /// rubash#2493: GNU closes a brace group through
    /// print_cmd.c:693-728 print_group_command's `semicolon()'
    /// (print_cmd.c:1520-1530), which prints NOTHING after a body
    /// ending `... &' — only a newline_list may follow `&'
    /// (parse.y:1275/1290). The brace, coproc-body and nested
    /// function-definition renderers emitted `... &; }' —
    /// unparseable — so every respawned async child whose prelude
    /// carried a function with the modernish die trap's
    /// `{ { ...; } & } 8<&0' died at parse. Pin the newline spellings
    /// (the #294 convention) for all three closers.
    #[test]
    fn closers_after_trailing_background_take_newline_not_semicolon() {
        // Brace group whose last body command is backgrounded.
        let cmd = parse_one("{ { echo hi; } & } 8<&0");
        let text = bash_command_source_text(&cmd);
        assert!(
            text.contains("&\n}"),
            "brace closer must be a newline after `&': {text:?}"
        );
        assert!(!text.contains("&;"), "brace closer corrupted: {text:?}");

        // coproc body ending in a background command.
        let cmd = parse_one("coproc C { echo hi & }");
        let text = bash_command_source_text(&cmd);
        assert!(
            text.contains("&\n}"),
            "coproc closer must be a newline after `&': {text:?}"
        );
        assert!(!text.contains("&;"), "coproc closer corrupted: {text:?}");

        // A nested function definition whose body ends in `&'.
        let cmd = parse_one("g() { echo hi & }");
        let text = bash_command_source_text(&cmd);
        assert!(
            text.contains("&\n}"),
            "function-def closer must be a newline after `&': {text:?}"
        );
        assert!(
            !text.contains("&;"),
            "function-def closer corrupted: {text:?}"
        );

        // The non-`&' spelling is unchanged: `; }' after a plain body.
        let cmd = parse_one("{ echo hi; }");
        assert_eq!(bash_command_source_text(&cmd), "{ echo hi; }");
    }

    /// The fast render must agree byte-for-byte with the full renderer on
    /// every admitted shape (including dispatch verbs whose expansion is
    /// unaffected: `command`, `builtin`, `eval`, `declare`, `export`,
    /// `unset`, `test`), and must decline every shape the full renderer
    /// could treat specially (quotes, substitutions, assignments,
    /// redirects, compound forms, case — the head special-case stays on
    /// the slow path).
    #[test]
    fn literal_render_matches_full_renderer() {
        let admitted = [
            "[ 1 -eq 2 ]",
            "test 1 -eq 2",
            "true",
            ":",
            "true 1 2 3",
            "echo a b c",
            "command true",
            "builtin echo hi",
            "declare -a foo",
            "export foo",
            "unset foo",
            "eval true",
            "wait",
            "echo -n x",
            "printf %s x",
        ];
        for src in admitted {
            let cmd = parse_one(src);
            let fast =
                try_render_literal_command(&cmd).unwrap_or_else(|| panic!("{src:?} should admit"));
            assert_eq!(fast, bash_command_source_text(&cmd), "{src:?} fast != slow");
        }
        let declined = [
            "[ $x -eq 2 ]",
            "[ \"a\" = b ]",
            "echo a$b",
            "echo a b > out",
            "A=1 true",
            "echo $(date)",
            "echo `date`",
            "echo 'a b'",
            "echo \"a b\"",
            "for i in 1 2; do echo $i; done",
            "echo a{b,c}",
            "echo ~",
            "echo *",
        ];
        for src in declined {
            let cmd = parse_one(src);
            assert!(
                try_render_literal_command(&cmd).is_none(),
                "{src:?} should fall through"
            );
        }
        // Admission requires raw == word for every word.
        for word in ["[", "1", "-eq", "2", "]", "true", "="] {
            assert!(is_expand_fast_literal_word(word), "{word:?} should admit");
        }
    }

    /// The raw-join twin must agree byte-for-byte with the full renderer on
    /// every admitted shape (unquoted `$name` words print as their raw
    /// source), and must decline everything the full renderer could treat
    /// specially (quotes, `$(`/backtick spans, assignments, redirects,
    /// compound forms, raw/word mismatches).
    #[test]
    fn raw_join_render_matches_full_renderer() {
        let admitted = [
            "export BK_V$i=x",
            "unset BK_V$i",
            "export A$B C$D",
            "echo a$b c",
            "export X=1 Y=2",
            "unset foo bar",
        ];
        for src in admitted {
            let cmd = parse_one(src);
            let fast =
                try_render_raw_join_command(&cmd).unwrap_or_else(|| panic!("{src:?} should admit"));
            assert_eq!(fast, bash_command_source_text(&cmd), "{src:?} fast != slow");
        }
        let declined = [
            "echo a$b > out",
            "A=1 true",
            "echo $(date)",
            "echo `date`",
            "echo 'a b'",
            "echo \"a b\"",
            "for i in 1 2; do echo $i; done",
            "echo a | cat",
        ];
        for src in declined {
            let cmd = parse_one(src);
            assert!(
                try_render_raw_join_command(&cmd).is_none(),
                "{src:?} should fall through"
            );
        }
        // Subscript operands: whichever bucket they land in, the render
        // must agree with the full renderer.
        for src in ["export X[$i]=v", "unset 'a b'", "export A=\"x y\""] {
            let cmd = parse_one(src);
            if let Some(fast) = try_render_raw_join_command(&cmd) {
                assert_eq!(fast, bash_command_source_text(&cmd), "{src:?} fast != slow");
            }
        }
    }
}
