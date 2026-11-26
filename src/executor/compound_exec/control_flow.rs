use crate::executor::*;

/// rubash#1611: true when a timed command's inner is itself a time
/// frame through a "pure" chain — directly (`time time ...`) or
/// through sole-command subshells (`time (time ...)`, recursively).
/// GNU
/// prints exactly ONE report for a pure chain — the innermost
/// frame's — for completions and unwinds alike (verified against GNU
/// bash 5.3.20: `time time false`, `time time exit 3`,
/// `time (time false)`, `time ((time false))` all print one report;
/// a brace group, a function call, or a subshell with further
/// commands breaks the chain and every frame prints). The outer
/// frames therefore delegate: they still adjudicate status, traps,
/// and errexit, but emit no report of their own.
fn time_inner_is_pure_time_chain(inner: &CommandNode) -> bool {
    if inner.time_command.is_some() {
        return true;
    }
    // A pipeline inner never delegates: the pipeline frame owns the
    // single report (a stage's own time report is hoisted into it,
    // rubash#1263) — GNU `time time false | cat` prints one report,
    // from the outer frame, and `time (time false) | cat` prints two.
    if let Some(subshell) = &inner.subshell_command {
        return subshell.body.len() == 1 && time_inner_is_pure_time_chain(&subshell.body[0]);
    }
    inner.words.first().map(String::as_str) == Some("time")
}

pub(in crate::executor) fn command_is_time_prefixed_compound(cmd: &CommandNode) -> bool {
    // rubash#1218: the posix dash-word rule applies to compounds too —
    // `time -p ( ... )` parsed under posix mode is not the time keyword.
    // rubash#1520: the decision is the parse-time-frozen one recorded on
    // the node, never re-derived from the live mode.
    !time_dash_word_not_keyword(cmd)
        && cmd.words.first().map(String::as_str) == Some("time")
        && (cmd.for_command.is_some()
            || cmd.if_command.is_some()
            || cmd.loop_command.is_some()
            || cmd.select_command.is_some()
            || cmd.case_command.is_some()
            || cmd.coproc_command.is_some()
            || cmd.subshell_command.is_some()
            || cmd.brace_group.is_some())
}

/// Whitelist for the rubash#661 condition-state fast path: exactly one
/// bare `:` or `true` with every assignment/redirect/substitution channel empty
/// (shadowing is gated at the call site: a `true()` function or alias
/// makes the word execute user code that can fail).
/// (same trigger-free class as #607's literal admission — keep in sync).
/// GNU's execute_if_command (execute_cmd.c:3862) runs such a condition with
/// no observable state traffic: the literal word cannot fail expansion, it
/// exits 0 without touching variables, fds, or traps, and it observes
/// neither the compound-condition marking (consulted only on
/// ExpansionFailure paths) nor the errexit suppression (consulted only on
/// nonzero status). Anything else falls through to the marked path, so a
/// false negative only costs speed.
fn if_condition_is_side_effect_free(condition: &[CommandNode]) -> bool {
    let [command] = condition else {
        return false;
    };
    if !matches!(command.words.as_slice(), [word] if word == ":" || word == "true") {
        return false;
    }
    command.assignments.is_empty()
        && command.assignment_raws.is_empty()
        && command.compound_assignments.is_empty()
        && command.array_element_assignments.is_empty()
        && command.process_substitutions.is_empty()
        && command.extglob_patterns.is_empty()
        && command.pathname_patterns.is_empty()
        && command.redirects.is_empty()
        && command.redirect_in.is_none()
        && command.redirect_out.is_none()
        && command.append.is_none()
        && command.redirect_err.is_none()
        && command.redirect_err_append.is_none()
        && command.heredoc.is_none()
        && command.heredoc_body.is_none()
        && command.heredoc_delimiter.is_none()
        && command.heredoc_redirects.is_empty()
        && command.here_string.is_none()
        && command.here_string_carrier.is_none()
        && command.pipe.is_none()
        && !command.background
        && command.and_or.is_none()
        && !command.inverted
        && !command.subshell
        && !command.subshell_end
        && command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none()
}

/// Whitelist for the arith-for counter fast driver: exactly one bare `:`
/// command with every assignment/redirect/substitution/channel field empty —
/// the same structural admission as #661's `if_condition_is_side_effect_free`
/// (kept in sync field-for-field), narrowed to `:` alone. `Some` admits and
/// hands back the node for the fast loop's end-state replication;
/// everything else is doubt and runs the slow loop. Shadowing (a `:`
/// function, a disabled `:`) is gated at the call site.
fn for_body_is_bare_colon(body: &[CommandNode]) -> Option<&CommandNode> {
    let [command] = body else {
        return None;
    };
    if !matches!(command.words.as_slice(), [word] if word == ":") {
        return None;
    }
    if !(command.assignments.is_empty()
        && command.assignment_raws.is_empty()
        && command.compound_assignments.is_empty()
        && command.array_element_assignments.is_empty()
        && command.process_substitutions.is_empty()
        && command.extglob_patterns.is_empty()
        && command.pathname_patterns.is_empty()
        && command.redirects.is_empty()
        && command.redirect_in.is_none()
        && command.redirect_out.is_none()
        && command.append.is_none()
        && command.redirect_err.is_none()
        && command.redirect_err_append.is_none()
        && command.heredoc.is_none()
        && command.heredoc_body.is_none()
        && command.heredoc_delimiter.is_none()
        && command.heredoc_redirects.is_empty()
        && command.here_string.is_none()
        && command.here_string_carrier.is_none()
        && command.pipe.is_none()
        && !command.background
        && command.and_or.is_none()
        && !command.inverted
        && !command.subshell
        && !command.subshell_end
        && command.pipeline_command.is_none()
        && command.and_or_list.is_none()
        && command.time_command.is_none()
        && command.background_command.is_none()
        && command.inverted_command.is_none()
        && command.for_command.is_none()
        && command.arithmetic_command.is_none()
        && command.if_command.is_none()
        && command.loop_command.is_none()
        && command.conditional_command.is_none()
        && command.subshell_command.is_none()
        && command.case_command.is_none()
        && command.select_command.is_none()
        && command.function_command.is_none()
        && command.brace_group.is_none()
        && command.coproc_command.is_none())
    {
        return None;
    }
    Some(command)
}

/// True when the compound statement's own CommandNode carries any redirect,/// heredoc, here-string, or structured process substitution — i.e. when the
/// clone-and-apply machinery in the `*_with_redirects` wrappers has work to
/// do. Mirrors every field consulted by materialize_compound_output_
/// process_substitutions, apply_*_redirects, and with_command_input_redirects
/// (rubash#619 fast path).
pub(in crate::executor) fn compound_command_has_redirects(cmd: &CommandNode) -> bool {
    cmd.redirect_in.is_some()
        || cmd.redirect_out.is_some()
        || cmd.append.is_some()
        || cmd.redirect_err.is_some()
        || cmd.redirect_err_append.is_some()
        || !cmd.redirects.is_empty()
        || cmd.heredoc.is_some()
        || cmd.heredoc_body.is_some()
        || !cmd.heredoc_redirects.is_empty()
        || cmd.here_string.is_some()
        || cmd.here_string_carrier.is_some()
        || !cmd.process_substitutions.is_empty()
}

fn flatten_if_command_for_alias_scan(
    cmd: &CommandNode,
    if_command: &IfCommand,
) -> Vec<CommandNode> {
    let mut commands = Vec::new();
    push_if_condition(&mut commands, "if", &if_command.condition);
    commands.push(command_with_words(["then"]));
    commands.extend(if_command.then_body.clone());
    for branch in &if_command.elif_branches {
        push_if_condition(&mut commands, "elif", &branch.condition);
        commands.push(command_with_words(["then"]));
        commands.extend(branch.body.clone());
    }
    if let Some(body) = &if_command.else_body {
        commands.push(command_with_words(["else"]));
        commands.extend(body.clone());
    }
    let mut fi = command_with_words(["fi"]);
    fi.redirect_in = cmd.redirect_in.clone();
    fi.redirect_out = cmd.redirect_out.clone();
    fi.append = cmd.append.clone();
    fi.redirect_err = cmd.redirect_err.clone();
    fi.redirect_err_append = cmd.redirect_err_append.clone();
    fi.heredoc = cmd.heredoc.clone();
    fi.heredoc_body = cmd.heredoc_body.clone();
    fi.heredoc_delimiter = cmd.heredoc_delimiter.clone();
    fi.heredoc_redirects = cmd.heredoc_redirects.clone();
    fi.here_string = cmd.here_string.clone();
    fi.here_string_carrier = cmd.here_string_carrier.clone();
    commands.push(fi);
    commands
}

fn push_if_condition(commands: &mut Vec<CommandNode>, keyword: &str, condition: &[CommandNode]) {
    let Some((first, rest)) = condition.split_first() else {
        commands.push(command_with_words([keyword]));
        return;
    };

    let mut first = first.clone();
    first.words.insert(0, keyword.to_string());
    commands.push(first);
    commands.extend(rest.iter().cloned());
}

fn command_with_words<const N: usize>(words: [&str; N]) -> CommandNode {
    let mut command = CommandNode::new();
    command.words = words.iter().map(|word| (*word).to_string()).collect();
    command
}

struct TimePrefixParts {
    command_index: usize,
    inverted: bool,
    posix_format: bool,
}

/// GNU parse.y, posix mode (rubash#1218): `time` is a reserved word
/// only when the word after it does not begin with `-`. With a dash
/// word (`time -p cmd`, `time -- cmd`, `time -x cmd`) the command is an
/// ordinary external command named `time`, resolved through PATH —
/// oracle-verified against GNU 5.3 (a fake `time` in PATH runs with
/// those arguments; without one the report is `time: command not
/// found`, status 127).
pub(in crate::executor) fn posix_time_dash_word(words: &[String], posix_mode: bool) -> bool {
    posix_mode
        && words
            .get(1)
            .is_some_and(|word| word.len() > 1 && word.starts_with('-'))
}

/// rubash#1520: the parse-time-frozen form of the #1218 veto, for nodes
/// the parser produced. GNU decides whether `time` is the reserved word
/// when the token is READ (parse.y:3286 `time_command_acceptable`, via
/// `CHECK_FOR_RESERVED_WORD` at parse.y:3179), under the mode in effect
/// for that reader unit; the lexer freezes that outcome into the token
/// kind (see `freeze_posix_time_dash_tokens`) and the parser stamps it
/// onto the node (`time_head_plain_word`) as the head word is recorded.
/// A head token recorded as a plain Word — reclassified by the lexer
/// under a POSIX pass because a dash word followed, or quoted in the
/// source — means the command is an ordinary external command named
/// `time`, whatever follows it and no matter how the live mode changed
/// after the unit was parsed. Synthesized nodes (never stamped) keep
/// the keyword reading, matching the pre-#1520 behaviour.
pub(in crate::executor) fn time_dash_word_not_keyword(cmd: &CommandNode) -> bool {
    cmd.words.first().map(String::as_str) == Some("time") && cmd.time_head_plain_word
}

fn time_prefix_parts(cmd: &CommandNode, posix_mode: bool) -> Option<TimePrefixParts> {
    let words = &cmd.words;
    if words.first().map(String::as_str) != Some("time") {
        return None;
    }
    if posix_time_dash_word(words, posix_mode) {
        return None;
    }

    // rubash#1548: operand window per parse.y special_case_tokens —
    // a quoted '-p' is the timed command's name, never time's option
    // (see crate::parser::scan_time_prefix_words).
    let aligned_metadata = if cmd.word_metadata.len() == words.len() {
        Some(cmd.word_metadata.as_slice())
    } else {
        None
    };
    let scan = crate::parser::scan_time_prefix_words(words, aligned_metadata);
    Some(TimePrefixParts {
        command_index: scan.next,
        inverted: scan.inverted,
        posix_format: scan.posix_format,
    })
}

/// rubash#809 CR F2: does a raw word carry an assignment-typed braced
/// parameter expansion — `${x=v}` / `${x:=v}` (also `${x=[w]}`)? These assign
/// in the expanding shell, so they must expand in the wrapper child, never in
/// the parent. `${x:-v}` / `${x-v}` / `${x:?v}` are reads and stay on the
/// fast path.
pub(super) fn parameter_expansion_assigns(raw: &str) -> bool {
    // All indexing below is on the BYTE slice: every offset is a raw byte
    // position (no char counting). UTF-8 continuation bytes are >= 0x80, so
    // the two-byte "${" window can only match at a character boundary, and
    // the identifier walk only advances over ASCII bytes — both keep
    // `cursor` on a boundary by construction without slicing `raw`.
    let bytes = raw.as_bytes();
    let mut scan_from = 0usize;
    while scan_from + 1 < bytes.len() {
        let Some(rel) = bytes[scan_from..]
            .windows(2)
            .position(|window| window == b"${")
        else {
            break;
        };
        let mut cursor = scan_from + rel + 2;
        let name_start = cursor;
        while cursor < bytes.len()
            && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
        {
            cursor += 1;
        }
        if cursor == name_start && cursor < bytes.len() && bytes[cursor] >= 0x80 {
            // A multibyte character sits where the parameter name would be
            // (`${中文=…}`): unicode names cannot be classified by the ASCII
            // walk, so stay conservative and keep the wrapper.
            return true;
        }
        if cursor > name_start
            && (bytes[cursor..].starts_with(b"=") || bytes[cursor..].starts_with(b":="))
        {
            return true;
        }
        scan_from = name_start.max(scan_from + 1);
    }
    false
}

#[cfg(test)]
mod arith_counter_fast_loop_tests {
    //! Contract for the canonical-counter fast driver
    //! (`try_enter_arith_counter_fast_loop` / `run_arith_counter_fast_loop`):
    //! whitelisted shapes must admit and evaluate exactly like the slow
    //! loop, and every doubt shape must fall through to it. The differential
    //! test below runs each script twice — once fast, once forced slow via
    //! an undelivered WINCH trap that installs an action (so the trap gate
    //! disqualifies) without changing any other semantics — and compares
    //! the full observable end-state.
    use super::for_body_is_bare_colon;
    use crate::executor::arithmetic::{arith_counter_loop_shapes, ArithCounterLoop};
    use crate::executor::Executor;

    fn for_body(script: &str) -> Vec<crate::parser::CommandNode> {
        let tokens = crate::lexer::tokenize(script);
        let ast = crate::parser::parse(&tokens);
        ast.commands
            .first()
            .and_then(|cmd| cmd.for_command.as_ref())
            .map(|for_command| for_command.body.clone())
            .unwrap_or_else(|| panic!("no for body in {script:?}"))
    }

    #[test]
    fn counter_shapes_admit_canonical() {
        assert_eq!(
            arith_counter_loop_shapes("i=0", "i<200000", "i++"),
            Some(ArithCounterLoop {
                var: "i".to_string(),
                init: 0,
                bound: 200000,
            })
        );
        // Whitespace around the tokens still parses to the same counter.
        assert_eq!(
            arith_counter_loop_shapes("i = 0", "i < 200000", "i ++"),
            Some(ArithCounterLoop {
                var: "i".to_string(),
                init: 0,
                bound: 200000,
            })
        );
    }

    #[test]
    fn counter_shapes_reject_doubt() {
        // Empty sections, other operators, other updates, name mismatches,
        // signed/octal/huge bounds, dynamic names: all slow.
        for (init, test, update) in [
            ("", "i<3", "i++"),
            ("i=0", "", "i++"),
            ("i=0", "i<3", ""),
            ("i=0", "i<=3", "i++"),
            ("i=0", "i==3", "i++"),
            ("i=0", "i!=3", "i++"),
            ("i=0", "i>3", "i++"),
            ("i=0", "i<3", "i--"),
            ("i=0", "i<3", "++i"),
            ("i=0", "i<3", "i+=1"),
            ("i=0", "i<3", "i=i+1"),
            ("i=0", "i<3", "i=3"),
            ("j=0", "i<3", "i++"),
            ("i=0", "i<3", "j++"),
            ("i=0", "i<N", "i++"),
            ("i=+0", "i<3", "i++"),
            ("i=-1", "i<3", "i++"),
            ("i=00", "i<3", "i++"),
            ("i=0", "i<007", "i++"),
            ("i=0", "i<99999999999999999999999", "i++"),
            ("i=99999999999999999999999", "i<3", "i++"),
            ("RANDOM=0", "RANDOM<3", "RANDOM++"),
            ("i=0", "i<3", "RANDOM++"),
            ("i=0", "i++", "i++"),
            ("i<3", "i<3", "i++"),
        ] {
            assert!(
                arith_counter_loop_shapes(init, test, update).is_none(),
                "admitted doubt ({init:?}, {test:?}, {update:?})"
            );
        }
    }

    #[test]
    fn bare_colon_body_admission() {
        assert!(for_body_is_bare_colon(&for_body("for ((i=0;i<3;i++)); do :; done")).is_some());
        for script in [
            "for ((i=0;i<3;i++)); do : x; done",
            "for ((i=0;i<3;i++)); do :>f; done",
            "for ((i=0;i<3;i++)); do true; done",
            "for ((i=0;i<3;i++)); do :; :; done",
            "for ((i=0;i<3;i++)); do ((x++)); done",
            "for ((i=0;i<3;i++)); do ! :; done",
            "for ((i=0;i<3;i++)); do : & done",
        ] {
            assert!(
                for_body_is_bare_colon(&for_body(script)).is_none(),
                "admitted non-trivial body {script:?}"
            );
        }
    }

    /// Observable end-state compared field-for-field between the fast loop
    /// and the same loop forced slow.
    fn run_and_snapshot(script: &str) -> (i32, Vec<(String, String)>) {
        let tokens = crate::lexer::tokenize(script);
        let ast = crate::parser::parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        let code = executor.last_exit_code();
        let mut watched = [
            "i",
            "_",
            "__RUBASH_CURRENT_LINE",
            "__RUBASH_CMD_START_LINE",
            "__RUBASH_LAST_COMMAND",
            "__RUBASH_CURRENT_COMMAND",
        ];
        watched.sort();
        let mut snapshot = Vec::new();
        for key in watched {
            snapshot.push((
                key.to_string(),
                executor.get_env(key).unwrap_or("<unset>").to_string(),
            ));
        }
        // The arithmetic write log must drain on both paths.
        assert!(
            crate::executor::arithmetic::arith_write_log_is_empty(),
            "write log pending after {script:?}"
        );
        (code, snapshot)
    }

    #[test]
    fn fast_and_slow_loops_agree() {
        // (script body, expected final `i`)
        for (body, expected_i) in [
            ("for ((i=0;i<0;i++)); do :; done", "0"),
            ("for ((i=0;i<1;i++)); do :; done", "1"),
            ("for ((i=0;i<50;i++)); do :; done", "50"),
            ("for ((i = 0; i < 50; i ++)); do :; done", "50"),
            ("i=999; for ((i=0;i<50;i++)); do :; done", "50"),
            ("for ((i=7;i<60;i++)); do :; done", "60"),
        ] {
            // A `: PIVOT` prefix pins `_` identically on both arms: the
            // slow-forcing `trap` command below would otherwise bind its own
            // last word (`WINCH`) and skew the comparison.
            let fast_script = format!(": PIVOT; {body}");
            let (fast_code, fast_state) = run_and_snapshot(&fast_script);
            // An installed-but-undelivered WINCH action disqualifies the
            // fast loop via the trap gate without changing any semantics.
            let slow_script = format!("trap 'true' WINCH; : PIVOT; {body}");
            let (slow_code, slow_state) = run_and_snapshot(&slow_script);
            assert_eq!(fast_code, slow_code, "exit of {body:?}");
            assert_eq!(fast_state, slow_state, "state of {body:?}");
            assert_eq!(fast_code, 0, "loop must end 0: {body:?}");
            let final_i = fast_state
                .iter()
                .find(|(key, _)| key == "i")
                .map(|(_, value)| value.as_str());
            assert_eq!(final_i, Some(expected_i), "final i of {body:?}");
        }
    }

    #[test]
    fn fast_loop_binds_canonical_counter() {
        let tokens = crate::lexer::tokenize("for ((i=0;i<5;i++)); do :; done");
        let ast = crate::parser::parse(&tokens);
        let mut executor = Executor::new();
        executor.execute_ast(&ast).ok();
        assert_eq!(executor.get_env("i"), Some("5"));
        assert_eq!(executor.get_env("_"), Some(":"));
        assert_eq!(executor.last_exit_code(), 0);
    }

    #[test]
    fn gates_fall_back_without_changing_semantics() {
        // Each gate-breaker forces the slow loop; results must still match
        // GNU (verified byte-for-byte against system bash during review).
        for script in [
            // DEBUG action installed: trap fires on the slow path.
            "trap 'echo -n' DEBUG; for ((i=0;i<3;i++)); do :; done; echo \"i=$i\"",
            // `:` shadowed by a function.
            ":() { echo -n FN; }; for ((i=0;i<3;i++)); do :; done; echo \"i=$i\"",
            // Non-canonical body.
            "for ((i=0;i<3;i++)); do echo -n \"B$i \"; done",
            // Non-canonical update.
            "for ((i=0;i<3;i++)); do :; done; for ((i=0;i<3;i+=1)); do :; done; echo \"i=$i\"",
        ] {
            let tokens = crate::lexer::tokenize(script);
            let ast = crate::parser::parse(&tokens);
            let mut executor = Executor::new();
            executor.execute_ast(&ast).ok();
            assert_eq!(executor.get_env("i"), Some("3"), "{script:?}");
        }
    }
}

/// rubash#3325: whether the command a `time` runs is a statement compound — the kinds GNU's execute_command_internal
/// never adjudicates for ERR at their own level beyond their own
/// do_redirections failure (execute_cmd.c:854-883): brace group, if,
/// for (both forms), while/until, case, select.
fn timed_root_is_statement_compound(root: &CommandNode) -> bool {
    root.brace_group.is_some()
        || root.if_command.is_some()
        || root.for_command.is_some()
        || root.loop_command.is_some()
        || root.case_command.is_some()
        || root.select_command.is_some()
}
impl Executor {
    pub(in crate::executor) fn execute_inverted_ast_command(
        &mut self,
        inverted_command: &InvertedCommand,
    ) -> Result<(), ExecuteError> {
        let commands = Ast {
            commands: vec![(*inverted_command.command).clone()],
        };
        self.with_errexit_suppressed(|executor| executor.execute_ast(&commands))?;
        // GNU execute_cmd.c:854-883 vs :1204: a compound's OWN redirect
        // failure returns before the invert step, so the flip is suppressed
        // while compound_redirect_failed is set (rubash#846). The mark is
        // left set on purpose: an outer `! ! C` wrapper suppresses in turn
        // (GNU: the inner command never runs — the outer redirect fails
        // first — so both levels observe status 1), and the owning
        // boundary consumes it (subshell-region single-inverted-body flip
        // at :1831, pipeline wait-side flip at :763, comsub/function/eval
        // completion discard, flat-loop backstop clear).
        if self.compound_redirect_failed.get() {
            return Ok(());
        }
        self.exit_code = invert_exit_status(self.exit_code);
        Ok(())
    }

    /// Record a compound's own do_redirections failure (GNU
    /// execute_cmd.c:854-883). Call only with the apply-stage IoError —
    /// never ExpansionFailure (failglob aborts before do_redirections and
    /// flips normally) and never a body write error.
    pub(in crate::executor) fn note_compound_redirect_failure(&self) {
        self.compound_redirect_failed.set(true);
    }

    /// Consume the compound-redirect-failure mark (fork/call boundaries and
    /// the pipeline wait-side flip own the adjudicated status from here).
    pub(in crate::executor) fn take_compound_redirect_failed(&self) -> bool {
        self.compound_redirect_failed.replace(false)
    }

    pub(in crate::executor) fn execute_time_ast_command(
        &mut self,
        time_command: &TimeCommand,
    ) -> Result<(), ExecuteError> {
        // rubash#1611: the errexit-conversion mark is scoped to THIS
        // timed command's inner dispatch — clear any stale value before
        // the inner runs (the mark is taken right after it below).
        self.errexit_status_converted.set(false);
        // Q6 arm B: the errexit-unwind mark travels with the
        // conversion mark — same scoping discipline.
        self.errexit_unwind_pending = false;
        // rubash#2117: the POSIX special-builtin conversion mark is
        // scoped the same way; the ast_exec time branch takes it.
        self.posix_special_status_converted.set(false);
        // rubash#3325: the ERR-ownership verdict is recomputed by this
        // frame after its inner dispatch; never inherit a stale one.
        self.timed_err_owned_by_body.set(None);
        // rubash#1218: in posix mode a dash word right after `time`
        // means `time` was never a keyword (GNU parse.y), so the
        // reconstructed command (`time`, the prefix words, then the
        // inner command's words) dispatches as an ordinary command
        // named `time` (PATH-resolved; 127 when absent). Only the simple
        // inner shape is rebuilt; pipeline/compound inners keep the
        // timed path.
        // rubash#1520: the verdict is the parse-time-frozen one recorded
        // on the node (keyword_was_plain_word), never the live mode —
        // the parser folds only keyword heads, so a same-unit `set -o
        // posix` that executed after the fold cannot reach this branch.
        if time_command.keyword_was_plain_word
            && time_command.command.pipeline_command.is_none()
            && !time_command.command.words.is_empty()
        {
            let inner = &time_command.command;
            let mut plain = CommandNode::new();
            plain.line = inner.line;
            plain.logical_line = inner.logical_line;
            let mut words = vec![time_command.keyword.clone()];
            words.extend(time_command.prefix_words.iter().cloned());
            words.extend(inner.words.iter().cloned());
            plain.words = words;
            plain.redirects = inner.redirects.clone();
            plain.redirect_in = inner.redirect_in.clone();
            plain.redirect_out = inner.redirect_out.clone();
            plain.append = inner.append.clone();
            plain.redirect_err = inner.redirect_err.clone();
            plain.redirect_err_append = inner.redirect_err_append.clone();
            return self.execute_command(&plain);
        }
        // rubash#1528: a timed coproc returns at spawn, but GNU times
        // the job's whole lifetime and reports at completion notice
        // (measured on the 5.3.20 oracle: `time coproc C { sleep 2; }`
        // + wait reports real 2.017s at the wait, an instant body
        // reports 0.000s — both at notice time, never at spawn).
        // Snapshot the child set so the spawn below can be identified
        // and its report deferred to the completion funnel.
        // rubash#1611: a pure time chain delegates its report to the
        // innermost frame (see time_inner_is_pure_time_chain) — this
        // frame adjudicates status/traps/errexit but prints nothing.
        let delegated_report = time_inner_is_pure_time_chain(&time_command.command);
        let timed_coproc_shape = time_command.command.coproc_command.is_some()
            || time_command.command.words.first().map(String::as_str) == Some("coproc");
        let children_before: std::collections::HashSet<u32> = if timed_coproc_shape {
            self.background_children.keys().copied().collect()
        } else {
            std::collections::HashSet::new()
        };
        let started = time_command_started();
        // rubash#1611: for the span of the inner dispatch below, the
        // dispatch-level errexit conversion sites convert WITHOUT
        // firing the ERR trap — the timed command completes here and
        // the ast_exec time branch performs the single trap
        // adjudication for it (the rubash#1277 design). The depth is
        // raised/lowered around the single dispatch statement; the
        // statement contains no `?` (the DEBUG-trap arm captures its
        // result instead of propagating), so the lowering cannot be
        // bypassed.
        self.timed_dispatch_depth
            .set(self.timed_dispatch_depth.get() + 1);
        // rubash#3325: scoped to this frame's root dispatch (a nested
        // time frame clears it again for its own root and leaves the
        // verdict set for this frame to read).
        self.timed_root_redirect_failure.set(false);
        // rubash#1611: clear the conversion mark immediately before the
        // inner dispatch (the DEBUG-trap call in the simple-command arm
        // below runs its own commands first; their conversions must not
        // be mistaken for the inner command's).
        self.errexit_status_converted.set(false);
        self.posix_special_status_converted.set(false);
        // Q6 arm B: same pre-dispatch clear for the unwind mark.
        self.errexit_unwind_pending = false;
        // Snapshot the parse-error latch: a parse-diagnostic ExitCode(2)
        // out of the inner dispatch means the timed command never
        // executed (GNU reports no timing for it), unlike a genuine
        // runtime unwind.
        let parse_error_before_inner = self.parse_error_occurred;
        let inner_result = if let Some(coproc_cmd) = &time_command.command.coproc_command {
            self.execute_coproc_command(&time_command.command, coproc_cmd)
        } else if time_command.command.words.is_empty()
            && command_has_no_effect(&time_command.command)
        {
            self.exit_code = 0;
            Ok(())
        } else if let Some(pipeline_command) = &time_command.command.pipeline_command {
            self.execute_pipeline_command(pipeline_command)
        } else if time_command.command.brace_group.is_some() {
            self.execute_brace_group_pipeline(&time_command.command)
                .map(|_| ())
        } else if time_command.command.words.first().map(String::as_str) == Some("coproc") {
            self.execute_time_reparsed_coproc(&time_command.command)
        } else {
            // GNU execute_time_command dispatches the inner command through
            // execute_command, so a simple/`(( ))`/`[[ ]]` inner fires its own
            // run_debug_trap (execute_cmd.c:4506/3920/4153) — `time true`
            // reports `DBG:true`, never `DBG:time true`. Compound inners
            // (for/if/while/select/case/subshell/brace/pipeline/coproc/and-or)
            // fire inside their own handlers, so they are skipped here.
            let inner = &time_command.command;
            let inner_defers_debug = inner.for_command.is_some()
                || inner.if_command.is_some()
                || inner.loop_command.is_some()
                || inner.select_command.is_some()
                || inner.case_command.is_some()
                || inner.coproc_command.is_some()
                || inner.subshell_command.is_some()
                || inner.subshell
                || inner.brace_group.is_some()
                || inner.inverted_command.is_some()
                || inner.background_command.is_some()
                || inner.pipeline_command.is_some()
                || inner.pipe.is_some()
                || inner.and_or_list.is_some()
                || inner.function_command.is_some()
                || inner.time_command.is_some();
            // rubash#1611: the trap result is captured, not `?`-
            // propagated, so the dispatch statement below the timed-
            // dispatch depth raise contains no early exit and the
            // depth lowering after it always runs. An erroring trap
            // still abandons the timed command with that error, as
            // the `?` did.
            let trap_result = if !inner_defers_debug && self.debug_trap_in_scope() {
                let inner_text = bash_command_source_text(inner);
                self.run_debug_trap(crate::debugger::SeamKind::TimeInner, &inner_text)
                    .map(|_| ())
            } else {
                Ok(())
            };
            match trap_result {
                Ok(()) => {
                    // rubash#1611: the DEBUG trap above is the time
                    // wrapper's own pre-step, not part of the inner
                    // command — a conversion inside the trap action
                    // must not be read as the inner command's
                    // conversion mark.
                    self.errexit_status_converted.set(false);
                    self.posix_special_status_converted.set(false);
                    self.execute_command(&time_command.command)
                }
                Err(error) => Err(error),
            }
        };
        self.timed_dispatch_depth
            .set(self.timed_dispatch_depth.get() - 1);
        // rubash#3325: GNU time_command (execute_cmd.c:1430) only hands
        // the command to execute_command_internal; whether the ERR trap
        // runs is decided inside that command's own execution. A
        // statement compound reaches run_error_trap at its own level
        // only for its do_redirections failure (execute_cmd.c:854-883);
        // its body's failing commands fire the trap themselves (nested
        // lists run with the timed deferral masked). The time branch's
        // final-status adjudication therefore stands only for the
        // non-compound roots (simple, pipeline, subshell, (( )), [[ ]],
        // function call) and a compound root's own redirect failure.
        let body_owns_err = timed_root_is_statement_compound(&time_command.command)
            && !self.timed_root_redirect_failure.get();
        // rubash#1611: take the errexit-conversion mark left by the
        // inner dispatch (cleared at entry above).
        let errexit_converted = self.errexit_status_converted.replace(false);
        // A `time time ...` chain: the nested frame (dispatched through
        // the execute_command funnel, rubash#1805) already computed the
        // verdict for the innermost command, with the errexit mark it
        // consumed — keep it instead of recomputing from this frame's
        // (already taken) mark.
        if time_command.command.time_command.is_none() {
            self.timed_err_owned_by_body
                .set(body_owns_err.then_some(errexit_converted));
        }
        // Q6 arm B: a converted ExitCode was adjudicated here, so
        // its unwind mark is consumed with it; the ast_exec time
        // branch's own gate re-marks if it raises the break itself.
        // A genuine unwind (mark clear / converted false) keeps its
        // mark for run_source_impl.
        if errexit_converted {
            let _ = std::mem::take(&mut self.errexit_unwind_pending);
        }
        let parse_error_during_inner = self.parse_error_occurred && !parse_error_before_inner;
        match inner_result {
            Ok(()) => {}
            // GNU execute_cmd.c execute_time_command reports the timed
            // command's status either way: a DISCARD-class expansion
            // failure inside it (expr.c evalerror / failglob) is the
            // command's failure status, not a shell unwind — time prints
            // its report and returns that status, while the armed
            // reader-level abandon still discards the rest of the line
            // (rubash#753 case 5: the discard penetrates `time`;
            // rubash#1240).
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
            }
            Err(ExecuteError::ExitCode(code)) if errexit_converted => {
                // rubash#1611: the ExitCode is the dispatch-level
                // errexit conversion of a COMPLETED inner command
                // (execute_materialized_command's tail and its siblings
                // in execute_command), not a jump out of the timed
                // command. GNU's execute_time_command completes and
                // prints its report; the errexit jump belongs to ITS
                // caller (the ast_exec time branch re-adjudicates after
                // flushing the queued report). A genuine unwind (exit
                // builtin, FORCE_EOF) leaves the mark clear and takes
                // the print-and-propagate arm below.
                self.exit_code = code;
            }
            Err(ExecuteError::ExitCode(code)) if !parse_error_during_inner => {
                // rubash#1611: a genuine runtime unwind whose target is
                // NOT inside this frame — the exit builtin (and exit
                // reached through eval, a brace group, or a function).
                // GNU's execute_time_command still prints its report
                // before the jump takes effect (verified against GNU
                // bash 5.3.20: `time exit 3`, `time { exit 3; }`,
                // `time eval "exit 3"` all print, rc preserved). The
                // report cannot ride the defer queue — the caller's
                // unwind arm discards it — so print it now, then let
                // the unwind proceed. A parse-diagnostic ExitCode is
                // excluded above: the timed command never executed, and
                // GNU prints no report for it.
                self.exit_code = code;
                if !delegated_report {
                    self.print_time_shell(time_command.posix_format, started);
                }
                return Err(ExecuteError::ExitCode(code));
            }
            Err(error @ (ExecuteError::Break(_) | ExecuteError::Continue(_))) => {
                // rubash#1611: break/continue unwind to the enclosing
                // loop, outside this frame — GNU's execute_time_command
                // prints its report first (`time break` in a loop
                // prints, verified against GNU bash 5.3.20). Print now
                // (the caller's unwind arms discard the defer queue),
                // then propagate.
                if !delegated_report {
                    self.print_time_shell(time_command.posix_format, started);
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        }
        // rubash#1528: when the timed inner was a coproc that is still
        // running, defer the report to the job-completion funnel
        // (flush_timed_coproc_report) instead of measuring the spawn.
        // A coproc already reaped by the time control reaches here
        // (no longer in background_children) keeps the immediate print:
        // its lifetime is the elapsed interval either way.
        let running_timed_coproc = if timed_coproc_shape {
            self.background_children.keys().copied().find(|pid| {
                !children_before.contains(pid) && self.shell_state.coproc_names.contains_key(pid)
            })
        } else {
            None
        };
        if delegated_report {
            // rubash#1611: pure time chain — the innermost frame owns
            // the report; this frame emits none (no queue entry either,
            // or the caller's flush would print it anyway).
        } else if let Some(pid) = running_timed_coproc {
            self.pending_timed_coproc_reports
                .push((pid, started, time_command.posix_format));
        } else if self.defer_list_time_report {
            // rubash#1277: under the list loop's defer flag the report is
            // queued, not printed — the loop flushes it after the ERR trap
            // adjudication (GNU fires the trap inside the inner
            // execute_command, before execute_time_command's print).
            self.pending_list_time_reports
                .push((started, time_command.posix_format));
        } else {
            self.print_time_shell(time_command.posix_format, started);
        }
        if time_command.inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
        }
        Ok(())
    }

    fn execute_time_reparsed_coproc(&mut self, command: &CommandNode) -> Result<(), ExecuteError> {
        let source = bash_command_source_text(command);
        let tokens = crate::lexer::tokenize(&source);
        let reparsed = crate::parser::parse(&tokens);
        if let Some(coproc_command) = reparsed
            .commands
            .first()
            .and_then(|command| command.coproc_command.as_ref())
        {
            return self.execute_coproc_command(command, coproc_command);
        }

        self.execute_command(command)
    }

    pub(in crate::executor) fn execute_time_prefixed_compound_command(
        &mut self,
        cmd: &CommandNode,
    ) -> Result<(), ExecuteError> {
        // Classification already applied the parse-time-frozen #1218
        // decision (command_is_time_prefixed_compound, rubash#1520), so
        // prefix extraction here must not re-veto against the live mode.
        let inverted = time_prefix_parts(cmd, false)
            .map(|parts| parts.inverted)
            .unwrap_or(false);
        let started = time_command_started();
        let result = if let Some(for_command) = &cmd.for_command {
            self.execute_for_command_with_redirects(for_command, cmd)
        } else if let Some(if_command) = &cmd.if_command {
            self.execute_if_command_with_redirects(cmd, if_command)
        } else if let Some(loop_command) = &cmd.loop_command {
            self.execute_loop_command_with_redirects(cmd, loop_command)
        } else if let Some(select_command) = &cmd.select_command {
            self.execute_select_command(cmd, select_command)
        } else if let Some(case_command) = &cmd.case_command {
            self.execute_case_command_with_redirects(cmd, case_command)
        } else if let Some(coproc_cmd) = &cmd.coproc_command {
            self.execute_coproc_command(cmd, coproc_cmd)
        } else if let Some(subshell_command) = &cmd.subshell_command {
            self.execute_subshell_command_with_redirects(cmd, subshell_command)
        } else if cmd.brace_group.is_some() {
            self.execute_brace_group_pipeline(cmd).map(|_| ())
        } else {
            Ok(())
        };
        self.print_time_shell(
            time_prefix_parts(cmd, false).is_some_and(|parts| parts.posix_format),
            started,
        );
        result?;
        if inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
        }
        Ok(())
    }

    pub(in crate::executor) fn execute_time_prefixed_command_sequence(
        &mut self,
        commands: &[CommandNode],
        index: usize,
    ) -> Result<Option<usize>, ExecuteError> {
        let Some(command) = commands.get(index) else {
            return Ok(None);
        };
        // rubash#1355: an assignment prefix makes `time` a plain command
        // name (GNU parse.y), never the timing keyword.
        if !command.assignments.is_empty()
            || !command.compound_assignments.is_empty()
            || !command.array_element_assignments.is_empty()
        {
            return Ok(None);
        }
        // rubash#1520: classification uses the parse-time-frozen #1218
        // decision recorded on the node, not the live mode.
        if time_dash_word_not_keyword(command) {
            return Ok(None);
        }
        let Some(prefix) = time_prefix_parts(command, false) else {
            return Ok(None);
        };
        if !matches!(
            command.words.get(prefix.command_index).map(String::as_str),
            Some("if" | "while" | "until")
        ) {
            return Ok(None);
        }

        let mut timed_ast = Ast {
            commands: commands[index..].to_vec(),
        };
        if let Some(first) = timed_ast.commands.first_mut() {
            first.words = command.words[prefix.command_index..].to_vec();
            if command.word_kinds.len() == command.words.len() {
                first.word_kinds = command.word_kinds[prefix.command_index..].to_vec();
            }
            // rubash#2237: same keyed shift as the other time-prefix
            // strips — word_metadata entries address words by
            // WordMetadata::word_index and the vector is sparse (a
            // process-substitution word carries no entry), so drop the
            // consumed prefix's entries and shift the survivors' keys
            // instead of slicing only the dense case.
            first.word_metadata = command
                .word_metadata
                .iter()
                .filter_map(|metadata| {
                    metadata
                        .word_index
                        .checked_sub(prefix.command_index)
                        .map(|shifted| {
                            let mut metadata = metadata.clone();
                            metadata.word_index = shifted;
                            metadata
                        })
                })
                .collect();
        }

        let started = time_command_started();
        let next_index = match timed_ast.commands[0].words.first().map(String::as_str) {
            Some("if") => crate::builtins::source::execute_simple_if(self, &timed_ast.commands, 0)?,
            Some("while" | "until") => self.execute_simple_loop(&timed_ast.commands, 0)?,
            _ => None,
        };
        let Some(next_index) = next_index else {
            return Ok(None);
        };

        self.print_time_shell(prefix.posix_format, started);
        if prefix.inverted {
            // rubash#846 (GNU execute_cmd.c:854-883 vs :1204): a compound
            // inner's OWN redirect failure suppresses the `!` flip (mark
            // left set for an outer `! !` wrapper, as elsewhere).
            if !self.compound_redirect_failed.get() {
                self.exit_code = invert_exit_status(self.exit_code);
            }
        }
        Ok(Some(index + next_index))
    }

    pub(in crate::executor) fn execute_arithmetic_for_command(
        &mut self,
        arithmetic: &ArithmeticForCommand,
        body: &[CommandNode],
    ) -> Result<(), ExecuteError> {
        let mut arithmetic_failed = false;
        // GNU 5.3 execute_arith_for_command increments loop_level before
        // evaluating init and always decrements after, even on init failure.
        // Git Bash 5.2 (and the temporary baseline in this task) omits the
        // decrement on failed init, leaving loop_level==1 so the following
        // `break` is considered in-loop, suppresses its diagnostic and aborts
        // the remainder of the script (arith-for.tests: `for ((j=;;))` with
        // `j=` empty RHS). Match the Git Bash baseline for now.
        self.shell_state.loop_depth += 1;
        // GNU execute_arith_for_command:3236 sets line_number = arith_lineno
        // = arith_for_command->line, and eval_arith_for_expr:3187 runs the
        // DEBUG trap before each expression evaluation (init once; test and
        // step once per iteration) with that line restored, so $LINENO inside
        // the fire is the for command's line (dbg-support.tests: the double
        // "debug lineno: 108 main" per iteration).
        // wt34/perf2: the capture is the parsed line number (GNU's C int),
        // not the env entry's String — the restore is then one equality-
        // gated write through the single CURRENT_LINE author instead of a
        // key+value allocation pair per expression evaluation, and the
        // body's ambient line below reuses the parsed value instead of
        // re-parsing the string once per iteration.
        let for_line = self
            .shell_state
            .env_vars
            .get("__RUBASH_CURRENT_LINE")
            .and_then(|line| line.parse::<usize>().ok());
        let restore_for_line = |executor: &mut Executor| {
            if let Some(line) = for_line {
                executor.set_current_line_value(line);
            }
        };
        // execute_cmd.c:3191-3199 (eval_arith_for_expr): a non-zero DEBUG
        // status under debugging_mode makes the expression evaluate as 0
        // with *okp = 1 — the evaluation is skipped, not failed.
        let init_skipped = if self.debug_trap_in_scope() && !arithmetic.init.trim().is_empty() {
            restore_for_line(self);
            // An Err here unwinds past the loop: balance the ENTRY
            // increment before returning (single-exit invariant,
            // execute_cmd.c:3296; see the body match below, #1911).
            match self.run_debug_trap(crate::debugger::SeamKind::ArithForInit, &arithmetic.init) {
                Ok(outcome) => outcome.is_some(),
                Err(error) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(error);
                }
            }
        } else {
            false
        };
        if !init_skipped && !arithmetic.init.trim().is_empty() && {
            // GNU execute_cmd.c:3201 (eval_arith_for_expr): if `set -x`
            // is on, print `(( expr ))` before evaluating each for-loop
            // expression (init, test, update).  Use the raw expression to
            // preserve original whitespace (e.g. `i++ ` trailing space).
            // execute_cmd.c:3236 keeps line_number at the arith-for's own
            // line across iterations (execute_command_internal restores it
            // after each body command), so the same restore that precedes
            // the DEBUG fire must precede this trace — PS4's $LINENO renders
            // the for line, not the line the last body command left behind
            // (rubash#275: iteration-2 `(( i++ ))` printed the body line).
            restore_for_line(self);
            self.xtrace_print_arith_cmd(&arithmetic.init_metadata.expression);
            self.eval_arithmetic_command_value(&arithmetic.init)
                .is_none()
        } {
            self.report_arithmetic_error_raw_display(&arithmetic.init_metadata.expression);
            self.exit_code = 1;
            arithmetic_failed = true;
        }

        let mut ran_body = false;
        // Canonical-counter fast loop (see run_arith_counter_fast_loop): the
        // slow init above already ran with full semantics (DEBUG fire,
        // xtrace, failure handling), so a failed init never reaches here.
        if !arithmetic_failed {
            if let Some(counter) = self.try_enter_arith_counter_fast_loop(arithmetic, body) {
                let result = self.run_arith_counter_fast_loop(&counter, body, for_line);
                // Mirror the function tail below: the entry `loop_depth += 1`
                // is balanced here; the fast loop always ends with status 0.
                self.shell_state.loop_depth -= 1;
                return result;
            }
        }
        // Header strings are borrowed from the parsed node, so the body
        // cannot mutate them: trim once per loop entry. GNU eval_arith_for_
        // expr (execute_cmd.c:3163) consults the trap table per evaluation;
        // the probe is its cheap gate (no DEBUG action means run_debug_trap
        // returns None), re-probed after the body, which can change it.
        let test_empty = arithmetic.test.trim().is_empty();
        let update_empty = arithmetic.update.trim().is_empty();
        while !arithmetic_failed {
            // execute_cmd.c:3191-3199: the skipped test evaluates as 0 —
            // the loop exits like any failing test (probe k2/m-series).
            let mut debug_may_fire = crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
            let mut test_skipped = false;
            if !test_empty
                && ((debug_may_fire && self.debug_trap_in_scope()) || self.debugger_session_here())
            {
                restore_for_line(self);
                // Balance the ENTRY increment on an unwinding Err
                // (single-exit invariant, execute_cmd.c:3296; #1911).
                match self.run_debug_trap(crate::debugger::SeamKind::ArithForTest, &arithmetic.test)
                {
                    Ok(outcome) => test_skipped = outcome.is_some(),
                    Err(error) => {
                        self.shell_state.loop_depth -= 1;
                        return Err(error);
                    }
                }
            }
            // rubash#2838: GNU execute_cmd.c:3252-3256
            // execute_arith_for_command — a stop in the test set
            // `breaking`; consume one level and abandon the loop,
            // before the test-outcome break (a false test must not
            // let the count leak to an enclosing loop).
            // rubash#2969: a pending DEBUG-trap flag banked by the init
            // or step expression's firing is consumed at this same
            // checkpoint (GNU execute_cmd.c:3252-3262 reads the flags
            // once per iteration): a break ends the loop; a continue
            // drained to zero falls through to the test and body.
            match self.consume_pending_loop_control() {
                LoopControlFlow::BreakLoop => break,
                LoopControlFlow::NextIteration | LoopControlFlow::None => {}
            }
            if self.consume_stopped_loop_breaking() {
                break;
            }
            if test_skipped {
                break;
            }
            if !test_empty {
                let _t = crate::executor::exec_profile::PhaseTimer::new(
                    &crate::executor::exec_profile::P_FOR_TEST,
                );
                // GNU execute_cmd.c:3201: xtrace before test expression.
                // Use raw expression to preserve original whitespace. The
                // line restore before it mirrors the DEBUG fire above
                // (execute_cmd.c:3236, rubash#275).
                restore_for_line(self);
                self.xtrace_print_arith_cmd(&arithmetic.test_metadata.expression);
                match self.eval_arithmetic_command_value(&arithmetic.test) {
                    Some(0) => break,
                    Some(_) => {}
                    None => {
                        self.report_arithmetic_error_raw_display(
                            &arithmetic.test_metadata.expression,
                        );
                        self.exit_code = 1;
                        arithmetic_failed = true;
                        break;
                    }
                }
            }

            // GNU execute_cmd.c:3267 REAP() runs once per arith-for
            // iteration, after the test succeeds and before the body.
            self.reap_dead_jobs_after_loop_body();
            ran_body = true;
            // rubash#1390: NO second loop_depth increment for the body —
            // the entry increment above already spans the whole arith-for
            // (GNU execute_arith_for_command bumps loop_level once,
            // before init). Counting the body again made the body see
            // depth 2 for a single loop, so `break 2`/`continue 2` were
            // not clamped to the real depth and escaped as a spurious
            // Break(1)/Continue(1) that aborted the script.
            let _t = crate::executor::exec_profile::PhaseTimer::new(
                &crate::executor::exec_profile::P_FOR_BODY,
            );
            // execute_cmd.c:3236: line_number = arith_for_command->line is
            // the ambient for the body's non-line-setting commands. GNU
            // execute_arith_for_command (execute_cmd.c:3219) re-walks the
            // stored body by POINTER per iteration; execute_ast_inner walks
            // the borrowed slice the same way (while/until and for-in bodies
            // already skip the reader-level wrapper through it).
            let result =
                self.with_ambient_line(for_line, |executor| executor.execute_ast_inner(body));
            drop(_t);
            match result {
                Ok(()) => {}
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    break;
                }
                // Propagating past this loop: balance the ENTRY
                // increment before returning (the function tail's
                // decrement is skipped by the early return), or the
                // leaked depth makes later `break`s look in-loop.
                Err(ExecuteError::Break(level)) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(ExecuteError::Break(level - 1));
                }
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                }
                Err(ExecuteError::Continue(level)) => {
                    self.shell_state.loop_depth -= 1;
                    return Err(ExecuteError::Continue(level - 1));
                }
                // Any other unwind crossing this loop (an expansion
                // failure from the body, an exit/return jump, ...):
                // balance the ENTRY increment before returning, like
                // the Break/Continue arms above. GNU
                // execute_arith_for_command (execute_cmd.c:3219) has a
                // single exit whose loop_level-- (execute_cmd.c:3296)
                // runs unconditionally — the body result there is a
                // status, never an escape past the frame. Leaving the
                // depth raised here defeats the enclosing list walk's
                // DISCARD absorption, whose guard reads
                // loop_depth == 0: an evalerror raised in the body
                // (expr.c evalerror -> jump_to_top_level (DISCARD))
                // then escapes as a script-fatal error instead of
                // discarding just this for command (rubash#1911).
                Err(error) => {
                    // rubash#2969: frame abandoned; drop banked flags.
                    self.clear_pending_loop_control();
                    self.shell_state.loop_depth -= 1;
                    return Err(error);
                }
            }
            // rubash#2969: GNU execute_cmd.c:3277-3290 — consume one
            // pending DEBUG-trap flag level after the body, before the
            // step: a break ends the loop; a continue drained to zero
            // falls through to the step expression.
            match self.consume_pending_loop_control() {
                LoopControlFlow::BreakLoop => break,
                LoopControlFlow::NextIteration | LoopControlFlow::None => {}
            }
            // rubash#2838: GNU execute_cmd.c:3277-3280
            // execute_arith_for_command — a SIGTSTP stop in the body set
            // `breaking`; consume one level and abandon the loop, before
            // the step expression (GNU checks breaking before stepping).
            if self.consume_stopped_loop_breaking() {
                break;
            }

            // execute_cmd.c:3191-3199: the skipped update evaluates as 0 —
            // the index variable keeps its value, so the next test sees the
            // unchanged index (probe k2: the body re-ran with the same `i`).
            // Re-probe: the body above may have installed/removed the trap.
            debug_may_fire = crate::builtins::trap::has_active_trap_action(
                &self.shell_state.env_vars,
                "DEBUG",
                "__RUBASH_TRAP_DEBUG",
            );
            let update_skipped = if !update_empty
                && ((debug_may_fire && self.debug_trap_in_scope()) || self.debugger_session_here())
            {
                restore_for_line(self);
                // Balance the ENTRY increment on an unwinding Err
                // (single-exit invariant, execute_cmd.c:3296; #1911).
                match self.run_debug_trap(
                    crate::debugger::SeamKind::ArithForUpdate,
                    &arithmetic.update,
                ) {
                    Ok(outcome) => outcome.is_some(),
                    Err(error) => {
                        self.shell_state.loop_depth -= 1;
                        return Err(error);
                    }
                }
            } else {
                false
            };
            if !update_skipped && !update_empty {
                let _t = crate::executor::exec_profile::PhaseTimer::new(
                    &crate::executor::exec_profile::P_FOR_UPDATE,
                );
                // GNU execute_cmd.c:3201: xtrace before update expression.
                // Use raw expression to preserve original whitespace. The
                // line restore before it mirrors the DEBUG fire above
                // (execute_cmd.c:3236, rubash#275).
                restore_for_line(self);
                self.xtrace_print_arith_cmd(&arithmetic.update_metadata.expression);
                if self
                    .eval_arithmetic_command_value(&arithmetic.update)
                    .is_none()
                {
                    self.report_arithmetic_error_raw_display(
                        &arithmetic.update_metadata.expression,
                    );
                    self.exit_code = 1;
                    arithmetic_failed = true;
                    break;
                }
            }
        }

        if arithmetic_failed {
            self.exit_code = 1;
        } else if !ran_body {
            self.exit_code = 0;
        }
        // GNU 5.3 execute_arith_for_command always decrements loop_level
        // after the loop, even when init failed. Git Bash 5.2 omitted the
        // decrement on failed init, leaving loop_level==1 so a following
        // `break` was treated as in-loop and aborted the rest of the script
        // (arith-for.tests: `for ((j=;;))` with `j=` empty RHS). Match GNU.
        self.shell_state.loop_depth -= 1;
        // rubash#3287: a fatal arithmetic expansion error raised while
        // evaluating a `for (( ))` header (a nested `$(( 1/0 ))` in
        // init/test/update) must be consumed HERE, by the same compound
        // consumer the case/loop-word paths already use — a plain
        // evaluation failure (e.g. `for (( i=1/0; ...))`) sets no flag
        // so the call is a no-op and the status stays 1, while a nested
        // fatal yields the DISCARD (rest of line abandoned, script
        // continues on the next line) and clears both flags. Without
        // this the stale fatal flag poisoned the NEXT command.
        self.abandon_on_arithmetic_expansion_error()?;
        Ok(())
    }

    /// Admission probe for the canonical-counter fast loop: `Some` admits,
    /// `None` is doubt and the caller runs the slow loop unchanged.
    /// Whitelist only (rubash#117): the counter shapes
    /// (`arith_counter_loop_shapes`), a single bare-`:` body, the canonical
    /// init binding left by the slow init, clean variable marks, an empty
    /// arithmetic write log, no installed trap actions, no tracing, an
    /// enabled builtin-`:` target and no background jobs. Every condition
    /// the skipped per-iteration work could observe is verified here; the
    /// `:` body cannot change any of them (it runs no code at all on the
    /// fast path), and no trap handler can run either, so entry validation
    /// holds for the whole loop by induction.
    fn try_enter_arith_counter_fast_loop(
        &self,
        arithmetic: &ArithmeticForCommand,
        body: &[CommandNode],
    ) -> Option<crate::executor::arithmetic::ArithCounterLoop> {
        for_body_is_bare_colon(body)?;
        let counter = crate::executor::arithmetic::arith_counter_loop_shapes(
            &arithmetic.init,
            &arithmetic.test,
            &arithmetic.update,
        )?;
        // The slow init ran first: the binding must be exactly its canonical
        // decimal rendering, proving the variable reads plainly.
        let rendered = counter.init.to_string();
        if self
            .shell_state
            .env_vars
            .get(&counter.var)
            .map(String::as_str)
            != Some(rendered.as_str())
        {
            return None;
        }
        if crate::executor::arithmetic::arith_counter_var_doubtful(
            &self.shell_state.env_vars,
            &counter.var,
        ) {
            return None;
        }
        if !crate::executor::arithmetic::arith_write_log_is_empty() {
            return None;
        }
        if !crate::executor::arithmetic::arith_locale_bind_log_is_empty() {
            return None;
        }
        if self.arith_counter_trap_installed() {
            return None;
        }
        if crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "xtrace")
            || crate::builtins::set::shell_option_enabled(&self.shell_state.env_vars, "verbose")
        {
            return None;
        }
        // `enable -n :` is mishandled by the slow path today (rubash still
        // runs the builtin where GNU reports 127); a function named `:`
        // overrides the builtin in both shells. Either doubt runs slow so
        // the fast loop can never diverge from the slow one.
        if crate::builtins::enable::is_disabled(&self.shell_state.env_vars, ":")
            || self.shell_state.functions.contains_key(":")
        {
            return None;
        }
        if !self.shell_state.job_table.jobs.is_empty() || !self.background_children.is_empty() {
            return None;
        }
        Some(counter)
    }

    /// Whether any trap action is installed. The fast loop executes no
    /// commands, so a trapped signal delivered mid-loop would sit in the
    /// mailbox until the loop ends while the slow path dispatches it at the
    /// next `:` boundary. Any non-empty `__RUBASH_TRAP_*` action —
    /// DEBUG/ERR/EXIT/RETURN/signal alike, including reset-suppressed ones
    /// (doubt runs slow) — disqualifies. Empty actions (`trap '' SIG` =
    /// SIG_IGN) install no shell code and stay admissible: dispositions
    /// cannot change without a `trap` call, which nothing on the fast path
    /// can issue. Metadata keys (`TRAP_RESET`, `TRAP_ORIG_IGNORES`) are not
    /// actions and are skipped.
    fn arith_counter_trap_installed(&self) -> bool {
        use crate::builtins::trap::{TRAP_ORIG_IGNORES, TRAP_PREFIX, TRAP_RESET};
        self.shell_state.env_vars.iter().any(|(key, action)| {
            key.starts_with(TRAP_PREFIX)
                && key.as_str() != TRAP_RESET
                && key.as_str() != TRAP_ORIG_IGNORES
                && !action.is_empty()
        })
    }

    /// Fast driver for canonical `for ((v=C0; v<C1; v++)); do :; done`
    /// loops admitted by `try_enter_arith_counter_fast_loop`. GNU
    /// execute_arith_for_command (execute_cmd.c:3219) re-evaluates the test
    /// and update through expr.c descent and dispatches the `:` body through
    /// execute_command_internal on every iteration; the fast loop performs
    /// the identical counter directly (i64 wrapping, mirroring bash_arith),
    /// the identical update write pipeline (env insert, write-log record,
    /// process-env mirror, typed-cell sync — the same calls in the same
    /// order as the slow `PostInc` path) and the identical net end-state
    /// (line/command stamps, `_`, status 0). Per-iteration work the slow
    /// path repeats — the DEBUG/line/xtrace probes, the arithmetic ticket
    /// (pre-expansion scans, nounset machinery, dynamic-context snapshot,
    /// parse and diagnostics) and the full `:` dispatch — is provably
    /// unobservable here: no DEBUG action or trace can fire (gated at
    /// entry, nothing mid-loop can arm one), the counter variable stays
    /// canonical (only fast writes touch it), and no jobs, traps or
    /// failures can appear. A fatal signal soldiers through the periodic
    /// mailbox drain exactly as the slow body's per-command boundary does
    /// (delivery timing is inherently racy either way).
    fn run_arith_counter_fast_loop(
        &mut self,
        counter: &crate::executor::arithmetic::ArithCounterLoop,
        body: &[CommandNode],
        for_line: Option<usize>,
    ) -> Result<(), ExecuteError> {
        // Pure-function admission over the same slice: entry verified a bare
        // `:` body, so this re-derivation yields the same node (precedent:
        // the `changed implies present` expect).
        let colon = for_body_is_bare_colon(body).expect("fast-loop entry admits only bare-`:`");
        let var = counter.var.clone();
        let mut current = counter.init;
        let mut iters: u64 = 0;
        // GNU pins the for line as the ambient line_number while the body
        // runs (execute_cmd.c:3236); the envelope keeps that net effect
        // while nothing inside observes it.
        let result: Result<(), ExecuteError> = self.with_ambient_line(for_line, |executor| {
            loop {
                if current >= counter.bound {
                    break Ok(());
                }
                // Signal-death parity with the slow body's per-command
                // boundary (see above); with no actions installed only the
                // fatal-delivery Err paths can trigger, which propagate
                // exactly as a slow body error does. The 1024-iteration tick
                // amortizes the mailbox poll to ~zero per iteration.
                if iters & 1023 == 0 {
                    // rubash#2810: the fast loop bypasses the statement
                    // boundary poll — the same tick carries the token
                    // check (one relaxed load per 1024 iterations).
                    if executor.host_cancel_requested() {
                        break Err(ExecuteError::HostCancelled);
                    }
                    executor.run_pending_signal_traps()?;
                }
                // Wrapping increment mirrors bash_arith (`value as i64`).
                let updated = current.wrapping_add(1);
                let new_value = updated.to_string();
                // By induction `old` is exactly the stored rendering: entry
                // verified the canonical init and only fast writes (same
                // rendering) touch the binding mid-loop.
                let old_value = Some(current.to_string());
                executor
                    .shell_state
                    .env_vars
                    .insert(var.clone(), new_value.clone());
                crate::executor::arithmetic::record_arith_write(&var, old_value);
                crate::executor::arithmetic::record_arith_locale_bind(
                    &var,
                    &new_value,
                    executor
                        .shell_state
                        .env_vars
                        .get("LC_ALL")
                        .map(String::as_str)
                        .unwrap_or(""),
                    executor.shell_state.env_vars.attr_of("LC_ALL").array,
                    executor.shell_state.env_vars.attr_of("LC_ALL").assoc,
                );
                crate::executor::sync_shell_assignment_process_env(
                    &executor.shell_state.env_vars,
                    &var,
                    &new_value,
                );
                crate::executor::arithmetic::sync_arith_writes(executor);
                current = updated;
                iters += 1;
            }
        });
        result?;
        if iters > 0 {
            // The skipped `:` iterations each stamped these byte-identical
            // values; one call each leaves the identical net state (all
            // three helpers are equality-gated no-ops on repeat).
            self.set_current_line(colon);
            self.set_current_command(colon);
            self.bind_underscore(":");
        }
        // Net CURRENT_LINE of the slow loop: the update-restore stamps the
        // for line after the last body (and the failing test-restore stamps
        // it on a zero-trip loop).
        if let Some(line) = for_line {
            self.set_current_line_value(line);
        }
        // Every fast evaluation succeeds, as the slow ones provably do for
        // admitted shapes; no diagnostic slot is ever written.
        self.shell_state.arithmetic_last_error_category.set(None);
        // Verified against GNU 5.3: the loop always ends 0 here — a ran body
        // of `:` leaves 0, and a zero-trip loop returns 0 (`false;
        // for ((i=5;i<3;i++)); do :; done; echo $?` prints 0).
        self.exit_code = 0;
        Ok(())
    }

    pub(in crate::executor) fn execute_if_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        if_command: &IfCommand,
    ) -> Result<(), ExecuteError> {
        // RESTRICTED_SHELL (#804): the compound's redirects apply before
        // the body runs, so the refusal must land before any clone, splice
        // or target creation — GNU leaves no file behind and never runs
        // the body.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        if self.if_command_needs_alias_scan(if_command) {
            let flat = flatten_if_command_for_alias_scan(cmd, if_command);
            crate::builtins::source::execute_simple_if(self, &flat, 0)?;
            return Ok(());
        }

        // rubash#619: GNU execute_cmd.c execute_if_command walks the stored
        // condition/branch tree by POINTER and applies redirections only when
        // the statement carries them. With no redirect/procsub fields on `cmd`
        // the clone of the ~40-field CommandNode plus the deep clone of
        // if_command (condition/then/elif/else CommandNode trees) and the
        // guaranteed no-op redirect application are pure per-execution
        // overhead (measured: `if :; then :; fi` +10.9us/op vs GNU 1.3us/op).
        if !compound_command_has_redirects(cmd) {
            return self.execute_if_command(if_command);
        }

        // rubash#3454: `if ... fi > $nope` under `set -u` — GNU's
        // do_redirections (execute_cmd.c:854) fails the target expansion
        // with `name: unbound variable` and aborts the script; the
        // prologue arms expand with no nounset gate, so report before
        // any procsub materialization or create side effect and unwind
        // as ExitCode(1), the same fatal the brace gate returns.
        if self.compound_redirect_nounset_fatal(cmd)? {
            return Err(ExecuteError::ExitCode(1));
        }

        // rubash#2246: GNU walks the stored tree by pointer and binds
        // the redirects once at entry (execute_cmd.c:854-883). The
        // deep clones this wrapper used to take — cmd.clone() plus
        // if_command.clone() — existed only to feed mutations: the
        // procsub materialization (gated on a `>(` mirror target, the
        // compound_redirect_needs_procsub_rewrite whitelist) and the
        // per-leaf splice, which prepare_compound_output_redirects now
        // replaces with the entry side effects alone (the fd-table
        // binding in with_command_input_redirects does the rest).
        // Without a `>(` target both clones are skipped and the
        // original tree executes as-is.
        let mut redirect_owned;
        let group_outputs;
        let redirect_cmd: &CommandNode =
            if crate::executor::pipeline_exec::compound_redirect_needs_procsub_rewrite(cmd) {
                redirect_owned = cmd.clone();
                group_outputs =
                    self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
                &redirect_owned
            } else {
                group_outputs = Vec::new();
                cmd
            };
        // rubash#846 (GNU execute_cmd.c:854-883): a prepare-stage
        // IoError is the if's OWN redirect failure — mark it for the
        // `!` flip sites. Body-stage errors flow through unmarked and
        // flip normally.
        let result = match self.prepare_compound_output_redirects(redirect_cmd) {
            Err(error) => {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                Err(error)
            }
            // rubash#1867: numbered heredoc/herestring staging wraps
            // every compound executor (GNU do_redirections binds them
            // for the compound's lifetime) — the loop wrap is the
            // template; if/case/for/select carried no staging at all.
            Ok(()) => self.with_loop_fd_heredocs(cmd, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    executor.execute_if_command(if_command)
                })
            }),
        };
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    pub(in crate::executor) fn execute_loop_command_with_redirects(
        &mut self,
        cmd: &CommandNode,
        loop_command: &LoopCommand,
    ) -> Result<(), ExecuteError> {
        // RESTRICTED_SHELL (#804): refuse before any splice or creation.
        if compound_command_has_redirects(cmd) {
            if let Some(message) = self.restricted_compound_redirect_error(cmd)? {
                self.write_default_stderr(&message)?;
                self.exit_code = 1;
                return Ok(());
            }
        }
        // rubash#3454: same `set -u` unbound-target gate as `if` —
        // `while/until ... done > $nope` aborts in GNU's compound
        // do_redirections before any expansion side effect.
        if self.compound_redirect_nounset_fatal(cmd)? {
            return Err(ExecuteError::ExitCode(1));
        }
        // rubash#2246: same borrow conversion as the if wrapper —
        // cmd.clone() + loop_command.clone() existed only for the
        // procsub materialization (gated on a `>(` mirror target) and
        // the per-leaf splice (replaced by
        // prepare_compound_output_redirects + the entry fd binding).
        let mut redirect_owned;
        let group_outputs;
        let redirect_cmd: &CommandNode =
            if crate::executor::pipeline_exec::compound_redirect_needs_procsub_rewrite(cmd) {
                redirect_owned = cmd.clone();
                group_outputs =
                    self.materialize_compound_output_process_substitutions(&mut redirect_owned)?;
                &redirect_owned
            } else {
                group_outputs = Vec::new();
                cmd
            };
        // rubash#846 (GNU execute_cmd.c:854-883): a prepare-stage
        // IoError is the loop's OWN redirect failure — mark it for the
        // `!` flip sites. Body-stage errors flow through unmarked and
        // flip normally.
        let result = match self.prepare_compound_output_redirects(redirect_cmd) {
            Err(error) => {
                if matches!(error, ExecuteError::IoError(_)) {
                    self.note_compound_redirect_failure();
                }
                Err(error)
            }
            Ok(()) => self.with_loop_fd_heredocs(cmd, |executor| {
                executor.with_command_input_redirects(cmd, |executor| {
                    executor.execute_loop_command(loop_command)
                })
            }),
        };
        let status = self.exit_code;
        let finish_result = self.finish_compound_output_process_substitutions(group_outputs);
        self.exit_code = status;
        result?;
        finish_result?;
        self.exit_code = status;
        Ok(())
    }

    fn execute_loop_command(&mut self, loop_command: &LoopCommand) -> Result<(), ExecuteError> {
        let mut ran_body = false;
        let mut last_body_status = 0;
        // rubash#395: a break consumed by THIS loop ends it with the break
        // builtin's own status (GNU break.def: EXECUTION_FAILURE for an
        // out-of-range count); the tail normalization below must not
        // clobber it with last_body_status.
        let mut ended_by_break = false;
        // GNU execute_while_or_until (execute_cmd.c:3801+) walks BOTH the
        // condition and the body by POINTER — the COMMAND lists make_cmd.c
        // allocated once at parse are re-walked per iteration, never copied.
        // The clones this port paid once per loop (condition Vec clone plus
        // the body's normalize rewrite) reduce to: no clone for the
        // condition, and a body copy only when normalize_inline_compound_
        // commands would actually rewrite something. Its ONLY rewrite arm
        // (inline_arithmetic_for_command, builtins/source/flow.rs) requires a
        // command whose first word is literally `for`; a body with no such
        // word normalizes to itself, so the stored slice executes as-is.
        let body_needs_normalize = loop_command
            .body
            .iter()
            .any(|command| command.words.first().map(String::as_str) == Some("for"));
        let body_owned = body_needs_normalize.then(|| {
            crate::builtins::source::normalize_inline_compound_commands(loop_command.body.clone())
        });
        let body: &[CommandNode] = body_owned.as_deref().unwrap_or(&loop_command.body);

        loop {
            // GNU execute_cmd.c:3801 (execute_while_or_until) increments
            // loop_level before the first test evaluation, so `break` /
            // `continue` anywhere in the loop CONDITION are in-loop
            // (modernish bin/modernish:866 `while case $# in (0) break;;`).
            self.shell_state.loop_depth += 1;
            let condition_result = self.with_errexit_suppressed(|executor| {
                executor.execute_ast_inner(&loop_command.condition)
            });
            self.shell_state.loop_depth -= 1;
            match condition_result {
                Ok(()) => {}
                // GNU break-in-test: execute_cmd.c:635 makes the body a
                // no-op (returns last status, which `break` set to 0), then
                // the post-body `if (breaking) { breaking--; break; }` at
                // execute_cmd.c:3840 exits this loop. A level > 1 break
                // propagates one level outward.
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    ended_by_break = true;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                // GNU continue-in-test: post-body check at
                // execute_cmd.c:3846 decrements `continuing`; with 1 left
                // over it does not break, so the loop skips the body for
                // this iteration and re-evaluates the test.
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => {
                    // rubash#2969: frame abandoned; drop banked flags.
                    self.clear_pending_loop_control();
                    return Err(error);
                }
            }
            self.run_pending_signal_traps()?;
            let condition_matched = self.exit_code == 0;
            if condition_matched == loop_command.until {
                // rubash#2969: GNU execute_cmd.c:3819-3831 — the
                // test-side exit also decrements each pending DEBUG-trap
                // flag once, so a level banked against this loop (the
                // test's own trap action set it) does not leak into the
                // enclosing list.
                self.drain_pending_loop_control();
                // rubash#2838: GNU execute_cmd.c:3819-3831
                // execute_while_or_until — a stop in the test set
                // `breaking`; the test-side exit consumes one level so
                // the count does not leak to an enclosing loop.
                self.consume_stopped_loop_breaking();
                break;
            }

            ran_body = true;
            self.shell_state.loop_depth += 1;
            let result = self.execute_ast_inner(body);
            self.shell_state.loop_depth -= 1;
            // GNU execute_cmd.c:3838 REAP() (execute_while_or_until):
            // dead jobs — and, via cleanup_dead_jobs -> coproc_reap,
            // dead coprocs (rubash#2053) — are silently reaped after
            // every while/until body in a non-interactive or
            // non-job-control shell. The alias-reparse path had this
            // call (alias_loops.rs); the AST path missed it, so a
            // coproc that died during a while loop kept its variables
            // for the whole loop.
            self.reap_dead_jobs_after_loop_body();
            self.run_pending_signal_traps()?;
            match result {
                Ok(()) => {
                    last_body_status = self.exit_code;
                }
                Err(ExecuteError::Break(level)) if level <= 1 => {
                    self.exit_code = self.take_break_failure_status().unwrap_or(0);
                    ended_by_break = true;
                    break;
                }
                Err(ExecuteError::Break(level)) => return Err(ExecuteError::Break(level - 1)),
                Err(ExecuteError::Continue(level)) if level <= 1 => {
                    self.exit_code = 0;
                    continue;
                }
                Err(ExecuteError::Continue(level)) => {
                    return Err(ExecuteError::Continue(level - 1));
                }
                Err(error) => {
                    // rubash#2969: frame abandoned; drop banked flags.
                    self.clear_pending_loop_control();
                    return Err(error);
                }
            }
            // rubash#2969: GNU execute_cmd.c:3840-3850 — consume one
            // pending DEBUG-trap flag level after the body: a break
            // ends the loop (ended_by_break keeps the status the
            // consumer settled, like the Break arm above); a continue
            // re-tests, or — with levels remaining — ends this loop
            // with the flag standing for the outer driver.
            match self.consume_pending_loop_control() {
                LoopControlFlow::BreakLoop => {
                    ended_by_break = true;
                    break;
                }
                LoopControlFlow::NextIteration => continue,
                LoopControlFlow::None => {}
            }
            // rubash#2838: GNU execute_cmd.c:3840-3843
            // execute_while_or_until — a SIGTSTP stop in the body set
            // `breaking`; consume one level and abandon the loop. The
            // tail below keeps last_body_status (148 for the stopped
            // job): GNU returns the body status.
            if self.consume_stopped_loop_breaking() {
                break;
            }
        }

        if !ran_body {
            self.exit_code = 0;
        } else if !ended_by_break {
            // GNU execute_cmd.c execute_while_or_until returns body_status
            // unconditionally — the loop's status is the last body command's,
            // not the final condition test's (rubash#1358: `until` exiting
            // on a true condition must still report the body's status).
            self.exit_code = last_body_status;
        }
        Ok(())
    }

    pub(in crate::executor) fn with_loop_fd_heredocs<T, F>(
        &mut self,
        cmd: &CommandNode,
        f: F,
    ) -> Result<T, ExecuteError>
    where
        F: FnOnce(&mut Executor) -> Result<T, ExecuteError>,
    {
        let saved_fd_inputs = self.bind_numbered_heredoc_redirects(cmd, true)?;
        let result = f(self);
        self.restore_numbered_heredoc_redirects(saved_fd_inputs);
        result
    }

    /// The binding half of with_loop_fd_heredocs, split out so a caller
    /// whose body is not a single closure — the in-process script child
    /// (execute_direct_shell_script), whose run spans its whole
    /// save/swap/restore bracket — can bind the same set and restore it
    /// at its own boundary. Returns the shadowed fd-table entries for
    /// restore_numbered_heredoc_redirects. `include_stdin` false skips
    /// an explicit fd-0 heredoc: that face's fd 0 is served by its
    /// command-input scope, exactly like open_compound_numbered_input_
    /// redirects leaves fd-0 file inputs to the fd-0 binder.
    pub(in crate::executor) fn bind_numbered_heredoc_redirects(
        &mut self,
        cmd: &CommandNode,
        include_stdin: bool,
    ) -> Result<Vec<(u32, Option<crate::executor::fd_table::FdEntry>)>, ExecuteError> {
        let mut saved_fd_inputs = Vec::new();
        for redirect in &cmd.heredoc_redirects {
            let Some(fd) = redirect.fd else {
                continue;
            };
            if fd == 0 && !include_stdin {
                continue;
            }
            if redirect.body.is_none() && redirect.body_carrier.is_none() {
                continue;
            }
            // One canonical expansion for every staged numbered
            // heredoc/herestring (expand_heredoc_redirect_text): the
            // typed carrier wins (a Preexpanded body was already
            // expanded at the do_redirections point and must not expand
            // again), a here-string's storage word decodes/expands and
            // gains its trailing newline. Feeding the raw stored body
            // to the heredoc expansion leaked the storage prefix into
            // the stream and lost the last line (rubash#1867).
            let body = self.expand_heredoc_redirect_text(redirect);
            // rubash#3706: the saved entry duplicates the displaced
            // descriptor (FdTable::saved_entry) — a plain clone shares
            // the descriptor the heredoc bind's open_input retires, so
            // the restore would replay a dead entry. A dup failure
            // unwinds the heredocs this call already bound (GNU
            // undo_partial_redirects, execute_cmd.c:855) before the
            // error propagates — the caller never sees a half-bound
            // list.
            let saved = match self.fd_table.saved_entry(fd) {
                Ok(saved) => saved,
                Err(error) => {
                    self.restore_numbered_heredoc_redirects(std::mem::take(&mut saved_fd_inputs));
                    return Err(ExecuteError::IoError(error));
                }
            };
            saved_fd_inputs.push((fd, saved));
            self.fd_table
                .open_input(fd, FdReadEndpoint::text(&body), true);
        }
        Ok(saved_fd_inputs)
    }

    /// The restore half of bind_numbered_heredoc_redirects: each bound
    /// slot returns to the entry it shadowed (or to unbound).
    pub(in crate::executor) fn restore_numbered_heredoc_redirects(
        &mut self,
        saved_fd_inputs: Vec<(u32, Option<crate::executor::fd_table::FdEntry>)>,
    ) {
        for (fd, old_entry) in saved_fd_inputs {
            match old_entry {
                Some(entry) => {
                    self.fd_table.entries.insert(fd, entry);
                }
                None => {
                    self.fd_table.entries.remove(&fd);
                }
            }
        }
    }

    fn if_command_needs_alias_scan(&self, if_command: &IfCommand) -> bool {
        if !self.alias_expansion_enabled() {
            return false;
        }

        if self.commands_contain_alias_if_control(&if_command.then_body) {
            return true;
        }
        if if_command.elif_branches.iter().any(|branch| {
            self.commands_contain_alias_if_control(&branch.condition)
                || self.commands_contain_alias_if_control(&branch.body)
        }) {
            return true;
        }
        if let Some(body) = &if_command.else_body {
            return self.commands_contain_alias_if_control(body);
        }
        false
    }

    fn commands_contain_alias_if_control(&self, commands: &[CommandNode]) -> bool {
        // rubash#661: GNU parse.y alias_expand_token passes words through
        // untouched when the alias table is empty (and words from the
        // grouped alias-stream driver are already expanded, so executor
        // expansion is the identity there too) — the raw first word IS the
        // expanded first word. Skip the per-command words.to_vec() plus the
        // per-word table probes of expand_aliases in that case.
        if self.shell_state.aliases.is_empty() || self.alias_streamed() {
            return commands.iter().any(|command| {
                matches!(
                    command.words.first().map(String::as_str),
                    Some("if" | "then" | "elif" | "else" | "fi")
                )
            });
        }
        commands.iter().any(|command| {
            let words = self.expand_aliases(&command.words);
            matches!(
                words.first().map(String::as_str),
                Some("if" | "then" | "elif" | "else" | "fi")
            )
        })
    }

    fn execute_if_command(&mut self, if_command: &IfCommand) -> Result<(), ExecuteError> {
        // GNU 5.3.20 (rubash#1240/#1253): a word-expansion failure in
        // an if/elif condition abandons the whole enclosing list
        // (neither branch runs, and the commands after the `fi` on the
        // same line are discarded) — if_condition_matches propagates
        // the failure instead of selecting the else branch.
        let matched = match self.if_condition_matches(&if_command.condition)? {
            Some(matched) => matched,
            None => return Ok(()),
        };
        if matched {
            // GNU execute_if_command (execute_cmd.c:5200 family) walks the
            // branch body by POINTER; the normalize pass only rewrites
            // lexer-collapsed `for ((...))` words (first word `for`), so a
            // body without one executes the stored slice without the clone.
            return self.execute_if_branch_body(&if_command.then_body);
        }

        for branch in &if_command.elif_branches {
            let matched = match self.if_condition_matches(&branch.condition)? {
                Some(matched) => matched,
                None => return Ok(()),
            };
            if matched {
                return self.execute_if_branch_body(&branch.body);
            }
        }

        if let Some(body) = &if_command.else_body {
            return self.execute_if_branch_body(body);
        }

        self.exit_code = 0;
        Ok(())
    }

    /// GNU runs if/elif/else bodies by POINTER (execute_cmd.c:5200+; no
    /// compound executor copies its body). The stored slice executes
    /// directly unless the inline-arithmetic-for normalize pass would
    /// rewrite it (its only arm needs a `for` first word — see
    /// normalize_inline_compound_commands, builtins/source/flow.rs).
    fn execute_if_branch_body(&mut self, body: &[CommandNode]) -> Result<(), ExecuteError> {
        if body
            .iter()
            .any(|command| command.words.first().map(String::as_str) == Some("for"))
        {
            let normalized =
                crate::builtins::source::normalize_inline_compound_commands(body.to_vec());
            return self.execute_ast_inner(&normalized);
        }
        self.execute_ast_inner(body)
    }

    /// Returns `Some(true)` when the condition held, `Some(false)` when
    /// it did not. A fatal word-expansion failure in the condition is
    /// NOT absorbed into a branch decision: it propagates as
    /// ExecuteError::ExpansionFailure so the enclosing list is discarded
    /// (GNU jump_to_top_level (DISCARD); rubash#1240/#1253).
    fn if_condition_matches(
        &mut self,
        condition: &[CommandNode],
    ) -> Result<Option<bool>, ExecuteError> {
        // GNU evaluates the condition list by pointer (execute_cmd.c:5200
        // execute_if_command -> execute_command on the stored COMMAND); the
        // slice entry re-walks the stored nodes without the Vec clone.
        // rubash#661: a side-effect-free condition exits 0 with no
        // expansion work, so it can neither fail (the only reader of the
        // compound-condition marking) nor trip errexit (the only reader of
        // the suppression). Run it unmarked; GNU's execute_if_command
        // (execute_cmd.c:3862) pays no equivalent state traffic for a
        // trivial condition either.
        // A shadowing function or alias makes `:`/`true` execute user code
        // that can fail (a `true()` returning nonzero under `set -e` must
        // keep the slow path's errexit suppression). Gate it here.
        let condition_word_unshadowed = matches!(condition, [command]
            if command.words.len() == 1
                && !self.shell_state.functions.contains_key(command.words[0].as_str())
                && !self.shell_state.aliases.contains_key(command.words[0].as_str()));
        let result = if condition_word_unshadowed && if_condition_is_side_effect_free(condition) {
            self.execute_ast_inner(condition)
        } else {
            let saved_condition = self.inside_compound_condition.replace(true);
            let result =
                self.with_errexit_suppressed(|executor| executor.execute_ast_inner(condition));
            self.inside_compound_condition.set(saved_condition);
            result
        };
        match result {
            Err(ExecuteError::ExpansionFailure(code)) => {
                self.exit_code = code;
                // GNU expr.c evalerror -> shell.c jump_to_top_level
                // (DISCARD): a fatal expansion failure in a condition
                // discards the whole enclosing list, not just the if
                // command (rubash#1240/#1253, GNU 5.3.20: `if [ $((1/0))
                // -eq 1 ]; then ...; fi; echo after` prints neither
                // branch and never reaches `after`). Keep the error
                // unwinding; the reader-level loop applies the discard.
                Err(ExecuteError::ExpansionFailure(code))
            }
            other => {
                other?;
                Ok(Some(self.exit_code == 0))
            }
        }
    }
}
