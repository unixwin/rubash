//! unixwin/niubash#481 (same normalizer-parity class as niubash#483): a
//! bare Windows drive designator — a lone `D:` word in command position —
//! is the drive switch the product REPL's fast-path normalizer rewrites
//! to `cd D:/` (niubash-runtime shell.rs
//! `normalize_bare_windows_drive_commands`). That pass is host-side and
//! only runs on the one submission route that reaches it: the grouped
//! reader (`D:; pwd` trips the batch gate), the engine's piped `-i`
//! stdin driver, and every nested context the AST pass does not descend
//! into (`eval`, function bodies) all hand the bare word to the executor
//! untouched, where it surfaced as `D:: command not found` on a route
//! the user cannot distinguish from the one that works.
//!
//! The rewrite therefore lives at the engine's simple-command dispatch —
//! the single funnel every route shares — and reproduces the host pass's
//! shape contract exactly (a lone unquoted `X:` word; no assignments,
//! redirects, pipes, background/`!`/`time` markers, or compound
//! structure), so an engine-driven run behaves identically whether or
//! not the host normalized first. GNU has no drive designators — there
//! is no upstream anchor to match; the contract is the product persona
//! the host normalizer already defines.

use super::*;

/// The drive-root operand (`D:/`) when `cmd` is a bare drive-designator
/// command, else None. The predicate is a verbatim port of the host
/// pass's `is_bare_windows_drive_command_shape` +
/// `command_has_redirects` + `bare_windows_drive_command_root`
/// (niubash-runtime shell.rs) — same inclusions, same exclusions — so
/// the normalized fast path and the un-normalized engine routes agree
/// on exactly which nodes switch drives.
pub(in crate::executor) fn bare_windows_drive_command_root(cmd: &CommandNode) -> Option<String> {
    if !cfg!(windows)
        || cmd.pipe.is_some()
        || cmd.background
        || cmd.inverted
        || cmd.pipeline_command.is_some()
        || cmd.and_or_list.is_some()
        || cmd.time_command.is_some()
        || cmd.background_command.is_some()
        || cmd.inverted_command.is_some()
        || cmd.subshell
        || cmd.subshell_end
        || cmd.for_command.is_some()
        || cmd.arithmetic_command.is_some()
        || cmd.if_command.is_some()
        || cmd.loop_command.is_some()
        || cmd.conditional_command.is_some()
        || cmd.subshell_command.is_some()
        || cmd.case_command.is_some()
        || cmd.select_command.is_some()
        || cmd.function_command.is_some()
        || cmd.brace_group.is_some()
        || cmd.coproc_command.is_some()
        || command_has_redirect(cmd)
        || cmd.heredoc.is_some()
        || cmd.heredoc_delimiter.is_some()
        || !cmd.heredoc_redirects.is_empty()
        || cmd.here_string.is_some()
        || !cmd.assignments.is_empty()
        || !cmd.compound_assignments.is_empty()
        || !cmd.array_element_assignments.is_empty()
    {
        return None;
    }

    let [word] = cmd.words.as_slice() else {
        return None;
    };
    // A quoted or substituted spelling (`"D:"`, `${x}`) is data, not the
    // drive-switch shorthand — the host pass requires raw == word with no
    // recorded quoting, the same admission the port keeps here.
    if cmd
        .word_metadata
        .first()
        .is_some_and(|metadata| !metadata.word_quotes.is_empty() || metadata.raw.as_str() != word)
    {
        return None;
    }

    let bytes = word.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        Some(format!("{}:/", (bytes[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

impl Executor {
    /// Rewrite a bare `D:` command to the same `cd D:/` node the host
    /// fast-path normalizer produces (words, kinds, literal metadata —
    /// the identical three-field replacement) and run it through the
    /// ordinary command dispatch so `$_`, xtrace, errexit, and
    /// command-not-found classification all follow the `cd` semantics
    /// both host routes already observe.
    pub(in crate::executor) fn execute_bare_windows_drive_command(
        &mut self,
        cmd: &CommandNode,
        drive_root: String,
    ) -> Result<(), ExecuteError> {
        let mut rewritten = cmd.clone();
        rewritten.words = vec!["cd".to_string(), drive_root];
        rewritten.word_kinds = vec![crate::lexer::TokenKind::Word; 2];
        rewritten.word_metadata = rewritten
            .words
            .iter()
            .enumerate()
            .map(|(index, word)| WordMetadata::literal(index, word.clone(), word.clone()))
            .collect();
        self.execute_command(&rewritten)
    }
}
