//! PROMPT_COMMAND execution, ported from GNU eval.c:305
//! `execute_prompt_command()`:
//!
//! - a set indexed array runs each non-empty element in order
//!   (eval.c:286 `execute_array_command` skips empty elements);
//! - an associative array is a no-op ("currently don't allow associative
//!   arrays here", eval.c:324-325);
//! - a plain string runs once via the evalstring equivalent;
//! - unset/invisible/empty returns before any work (eval.c:314-316).
//!
//! GNU calls this from `parse_command` (eval.c:336 "This is where
//! PROMPT_COMMAND is executed") under `interactive && bash_input.type !=
//! st_string && parser_expanding_alias() == 0` — i.e. before each primary
//! prompt read. Interactive hosts (rubash's own REPL loops, embedding
//! shells) drive it from their pre-prompt hook.
//!
//! Shape dispatch uses `${var@a}` attribute expansion, the shell-space
//! equivalent of GNU's `array_p`/`assoc_p` cell-type checks, so the
//! engine's own array semantics decide the branch.

impl crate::executor::Executor {
    /// evalstring.c parse_and_execute flag for PROMPT_COMMAND: the runner
    /// text is not a readline line and never enters the history list
    /// (eval.c:318-330 execute_variable_command passes the string straight
    /// to parse_and_execute; nothing in that path calls bash_add_history).
    /// The grouped driver consults this marker to skip both the history
    /// EXPANSION and the RECORD of the runner lines.
    pub(crate) const PROMPT_COMMAND_NOHIST: &str = "__RUBASH_PC_NOHIST";

    /// Run PROMPT_COMMAND per GNU eval.c:305. Returns the last executed
    /// element's exit status, or None when nothing ran (unset, empty,
    /// associative array).
    pub fn execute_prompt_command(&mut self) -> Option<i32> {
        // eval.c:314-316: absent/unset/invisible -> no work. An unset-but-
        // declared variable still occupies env storage with an empty value,
        // and the empty-string check below covers eval.c:329-330's
        // `command_to_execute && *command_to_execute`.
        let Some(value) = self.get_env("PROMPT_COMMAND").map(str::to_owned) else {
            return None;
        };
        if value.trim().is_empty() {
            return None;
        }

        // eval.c:322-327: the array_p/assoc_p dispatch on the variable's
        // cell type. eval.c:324-325 refuses associative arrays outright.
        if self.is_assoc_parameter_array("PROMPT_COMMAND") {
            return None;
        }

        let session = self.get_session_history();
        self.set_env(Self::PROMPT_COMMAND_NOHIST, "1");
        let array_storage = self.parameter_array_storage("PROMPT_COMMAND");
        let code = if array_storage
            .as_deref()
            .is_some_and(crate::executor::arrays::is_array_storage)
        {
            // eval.c:286-307 execute_array_command: a plain C loop over
            // array_to_argv; each non-empty element goes through its OWN
            // execute_variable_command (eval.c:318-330 ->
            // parse_and_execute), so nothing an ELEMENT evaluates can
            // unwind the loop — a failing status is discarded and a
            // `break` inside an element is a top-level "only meaningful in
            // a `for', `while', or `until' loop" error while the remaining
            // elements still run (verified GNU bash 5.3.0). The old
            // shell-level `for ... eval` runner shared ITS loop with the
            // evaluated text, so an element's `break` broke the runner
            // itself and silently dropped every later element
            // (rubash#430). Existing elements in index order (GNU
            // array_to_argv walks the element list — sparse holes never
            // stop the walk), empties skipped (eval.c:296-298).
            let storage = array_storage.unwrap();
            let mut last = 0;
            for (_, element) in crate::executor::arrays::indexed_array_entries(&storage) {
                let element = crate::executor::arrays::normalize_array_expanded_value(element);
                if !element.is_empty() {
                    last = self.run_prompt_command_text(&element, session.clone());
                    // eval.c:291-301 execute_array_command: the element's
                    // parse_and_execute re-raises an EXITPROG/ERREXIT jump
                    // (evalstring.c:618-619), and the longjmp UNWINDS this
                    // loop — elements after an `exit` element never run
                    // (verified WSL GNU 5.3.0: PC=([0]='echo A' [1]='exit'
                    // [2]='echo C') prints only A, rc 0). run_prompt_command_text
                    // re-arms the jump below; stop the walk to match.
                    if self.exit_jump_pending.get() {
                        break;
                    }
                }
            }
            last
        } else {
            // eval.c:327-330: a plain string runs once — the text itself is
            // the parse_and_execute payload (execute_variable_command),
            // with no extra eval layer.
            self.run_prompt_command_text(&value, session)
        };
        self.remove_env(Self::PROMPT_COMMAND_NOHIST);
        Some(code)
    }

    /// eval.c:318-330 execute_variable_command: parse_and_execute the text
    /// in the CURRENT shell environment, never recording it as a readline
    /// line (eval.c:305 execute_prompt_command -> execute_variable_command
    /// -> parse_and_execute(..., SEVAL_NONINT|SEVAL_NOHIST); nothing in
    /// that path calls bash_add_history). SEVAL_NONINT zeroes the
    /// `interactive` global for the duration (evalstring.c:284-285, restored
    /// at :612 from interactive_shell): the exit.def:59-62 interactive
    /// "exit" echo is suppressed while the text runs (verified WSL GNU 5.3.0
    /// piped-`-i`: PROMPT_COMMAND='exit' dies rc=0 with NO "exit" echo), and
    /// the `__RUBASH_INTERACTIVE_FLAG_OFF` env is this engine's stand-in for
    /// that zeroed global. A top-level unwind inside the text — a real
    /// `exit`, or the errexit break — is RE-ARMED as the exit jump the
    /// caller must honor: evalstring.c:396-403 catches EXITPROG/ERREXIT at
    /// parse_and_execute's own setjmp and :618-619 re-raises
    /// jump_to_top_level, so the jump continues through
    /// execute_variable_command into the reader's top level and ends the
    /// shell (rubash#433; verified: PC='exit' dies rc=0, PC='exit 5' dies
    /// rc=5, PC='set -e; false' dies rc=1 — while a parse error
    /// (PC='echo )') and a plain failing status (PC='false') leave the shell
    /// alive). The session history list stays live across the prompt —
    /// bash-it themes drive `history -a/-c/-r` from PROMPT_COMMAND against
    /// the user's own list, and the grouped driver's fresh-SessionHistory
    /// swap + self-record used to append the whole runner into $HISTFILE at
    /// every prompt and drop the commands typed since the last prompt
    /// (wt90/themehang). PROMPT_COMMAND_NOHIST gates both the history
    /// EXPANSION and the RECORD inside that driver.
    fn run_prompt_command_text(
        &mut self,
        text: &str,
        session: Option<std::rc::Rc<std::cell::RefCell<crate::history::SessionHistory>>>,
    ) -> i32 {
        // evalstring.c:284-285 / :612 — SEVAL_NONINT's zeroed `interactive`
        // global, keyed the way main.rs's `bash -i script` path keys it.
        self.set_env("__RUBASH_INTERACTIVE_FLAG_OFF", "1");
        // parse.y:3013/3021 execute_variable_command brackets the PC's
        // parse_and_execute with save_parser_state/restore_parser_state;
        // the snapshot carries last_command_exit_value (parse.y:7221) and
        // PIPESTATUS (:7223) and is written back at :7313/:7315 — a PC that
        // returns normally leaves $? and PIPESTATUS untouched (verified WSL
        // GNU 5.3.0 piped-`-i`: after PROMPT_COMMAND=false the next typed
        // command sees $? = the pre-PC value, 0). The restore line sits
        // AFTER parse_and_execute, so the re-raised exit jump longjmps past
        // it — an `exit'/errexit unwind keeps the jump's status (verified:
        // PC='exit 5' -> rc 5, PC='set -e; false' -> rc 1).
        let saved_exit_code = self.exit_code;
        let saved_pipestatus = self.shell_state.pipestatus.clone();
        let (status, exit_shell) = match session {
            Some(session) => {
                crate::script_driver::run_script_with_history_in_tracked(self, text, session, None)
            }
            None => {
                let fresh = std::rc::Rc::new(std::cell::RefCell::new(
                    crate::history::SessionHistory::new(),
                ));
                crate::script_driver::run_script_with_history_in_tracked(self, text, fresh, None)
            }
        };
        self.remove_env("__RUBASH_INTERACTIVE_FLAG_OFF");
        if exit_shell {
            // evalstring.c:618-619: the re-raise. The reader's pre-prompt
            // hook takes the jump and unwinds its read loop; the exit status
            // is already in the executor (exit N set it before raising).
            // parse.y:3021's restore is skipped, like the longjmp past it.
            self.exit_jump_pending.set(true);
        } else {
            // parse.y:7313/:7315 restore_parser_state: $? and PIPESTATUS
            // roll back to the pre-PC values on every normal return.
            self.exit_code = saved_exit_code;
            self.set_pipestatus(saved_pipestatus);
        }
        status
    }
}

#[cfg(test)]
mod tests {
    // Pre-existing dead import removed for the CI -D warnings gate; the
    // tests below reach the impl via `crate::` paths only.
    use crate::executor::Executor;
    use crate::lexer::tokenize;
    use crate::parser::parse;

    fn seed(executor: &mut Executor, script: &str) {
        let ast = parse(&tokenize(script));
        executor.execute_ast(&ast).ok();
    }

    // GNU anchors: eval.c:305 execute_prompt_command — string runs once,
    // indexed array runs each non-empty element in order (eval.c:294-299),
    // associative arrays are refused (eval.c:324-325), unset runs nothing
    // (eval.c:314-316).
    #[test]
    fn prompt_command_string_runs_once() {
        let mut executor = Executor::new();
        executor.set_env("PROMPT_COMMAND", "__pc_ran=1");
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_ran"), Some("1"));
    }

    #[test]
    fn prompt_command_indexed_array_runs_each_nonempty_element() {
        let mut executor = Executor::new();
        seed(
            &mut executor,
            "PROMPT_COMMAND=([0]='__pc_a=1' [1]='' [2]='__pc_b=2')",
        );
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_a"), Some("1"));
        assert_eq!(executor.get_env("__pc_b"), Some("2"));
    }

    // rubash#430: eval.c:286-307 execute_array_command runs each non-empty
    // element through its OWN execute_variable_command — nothing an element
    // evaluates (a failing status, or a `break` whose loop jump GNU reports
    // as "only meaningful in a `for', `while', or `until' loop") stops the
    // remaining elements. The old shell-level `for ... eval` runner let the
    // element's `break` break the runner loop itself, silently dropping the
    // rest of the array.

    #[test]
    fn prompt_command_failing_element_does_not_abort_remaining() {
        let mut executor = Executor::new();
        seed(
            &mut executor,
            "PROMPT_COMMAND=([0]='false' [1]='__pc_b=1' [2]='false')",
        );
        // The last element's status is the reported one, and the middle
        // element still ran.
        assert_eq!(executor.execute_prompt_command(), Some(1));
        assert_eq!(executor.get_env("__pc_b"), Some("1"));
    }

    #[test]
    fn prompt_command_break_element_does_not_abort_remaining() {
        let mut executor = Executor::new();
        seed(&mut executor, "PROMPT_COMMAND=([0]='break' [1]='__pc_b=1')");
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_b"), Some("1"));
    }

    #[test]
    fn prompt_command_sparse_array_runs_in_index_order() {
        let mut executor = Executor::new();
        seed(
            &mut executor,
            "PROMPT_COMMAND=([0]='__pc_log=A' [7]='__pc_log=\"$__pc_log B\"')",
        );
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_log"), Some("A B"));
    }

    #[test]
    fn prompt_command_associative_array_is_noop() {
        let mut executor = Executor::new();
        seed(&mut executor, "declare -A PROMPT_COMMAND=([x]='__pc_a=1')");
        // eval.c:324-325 refuses associative arrays: nothing runs, and the
        // documented contract returns None ("nothing ran") — the old shell
        // runner returned its own 0 because the case arm still executed.
        assert_eq!(executor.execute_prompt_command(), None);
        assert_eq!(executor.get_env("__pc_a"), None);
    }

    #[test]
    fn prompt_command_unset_runs_nothing() {
        let mut executor = Executor::new();
        assert_eq!(executor.execute_prompt_command(), None);
        executor.set_env("PROMPT_COMMAND", "   ");
        assert_eq!(executor.execute_prompt_command(), None);
    }

    // rubash#433: evalstring.c:396-403 catches an EXITPROG/ERREXIT jump at
    // parse_and_execute's own setjmp and :618-619 RE-RAISES jump_to_top_level
    // — the jump continues out of execute_variable_command (parse.y:3007)
    // into the reader's top level, ending the shell. eval.c:291-301
    // execute_array_command's loop is unwound by the same longjmp, so
    // elements after an `exit` element never run (verified WSL GNU 5.3.0
    // piped-`-i`: PC=([0]='echo A' [1]='exit' [2]='echo C') prints only A,
    // rc 0). parse.y:3013/3021 execute_variable_command brackets the run
    // with save_parser_state/restore_parser_state — last_command_exit_value
    // and PIPESTATUS snapshot at parse.y:7221/:7223, restored at
    // :7313/:7315 on every NORMAL return (verified: after PC='false' the
    // next typed command sees the pre-PC $?), and the restore is skipped
    // when the jump unwinds (verified: PC='exit 5' -> rc 5).

    #[test]
    fn prompt_command_exit_rearms_jump_and_sets_status() {
        let mut executor = Executor::new();
        executor.set_env("PROMPT_COMMAND", "exit 3");
        assert_eq!(executor.execute_prompt_command(), Some(3));
        assert_eq!(executor.exit_jump_pending.get(), true);
        assert_eq!(executor.last_exit_code(), 3);
        // The nohist marker must not leak past an exiting run either.
        assert_eq!(executor.get_env(Executor::PROMPT_COMMAND_NOHIST), None);
    }

    #[test]
    fn prompt_command_exit_element_stops_remaining_elements() {
        let mut executor = Executor::new();
        seed(
            &mut executor,
            "PROMPT_COMMAND=([0]='__pc_a=1' [1]='exit 7' [2]='__pc_c=1')",
        );
        assert_eq!(executor.execute_prompt_command(), Some(7));
        assert_eq!(executor.get_env("__pc_a"), Some("1"));
        // The longjmp unwound execute_array_command: the later element never
        // ran and the jump is armed for the reader.
        assert_eq!(executor.get_env("__pc_c"), None);
        assert_eq!(executor.exit_jump_pending.get(), true);
        assert_eq!(executor.last_exit_code(), 7);
    }

    #[test]
    fn prompt_command_failing_status_rolls_back_exit_code_and_pipestatus() {
        let mut executor = Executor::new();
        seed(&mut executor, "true");
        assert_eq!(executor.last_exit_code(), 0);
        executor.set_env("PROMPT_COMMAND", "false");
        // The run's own last status is still reported (rubash#430 contract).
        assert_eq!(executor.execute_prompt_command(), Some(1));
        // parse.y:7313/:7315 restore: $? and PIPESTATUS keep the pre-PC
        // values, and NO exit jump is armed — the shell must prompt again.
        assert_eq!(executor.last_exit_code(), 0);
        assert_eq!(executor.shell_state.pipestatus, vec![0]);
        assert_eq!(executor.exit_jump_pending.get(), false);
    }

    #[test]
    fn prompt_command_errexit_break_propagates_jump() {
        // evalstring.c:387-393: ERREXIT joins EXITPROG in the re-raise.
        // Verified WSL GNU 5.3.0 piped-`-i`: PROMPT_COMMAND='set -e; false'
        // ends the shell rc 1 before the next line is read.
        let mut executor = Executor::new();
        seed(&mut executor, "PROMPT_COMMAND='set -e; false'");
        assert_eq!(executor.execute_prompt_command(), Some(1));
        assert_eq!(executor.exit_jump_pending.get(), true);
        assert_eq!(executor.last_exit_code(), 1);
    }

    // wt90/themehang: eval.c:305 -> execute_variable_command ->
    // parse_and_execute runs the runner in the CURRENT shell environment
    // and never records it (it is not a readline line). The grouped
    // driver's fresh-SessionHistory swap + self-record appended the whole
    // runner into $HISTFILE on every prompt (via a theme's `history -a`)
    // and dropped the commands typed since the last prompt.

    #[test]
    fn prompt_command_keeps_live_session_and_records_nothing() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut executor = Executor::new();
        let session = Rc::new(RefCell::new(crate::history::SessionHistory::new()));
        session.borrow_mut().record("echo USER1", "", "", None);
        executor.set_session_history(Some(session.clone()));
        executor.set_env("PROMPT_COMMAND", "__pc_ran=1");
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_ran"), Some("1"));
        let shell = session.borrow();
        assert_eq!(shell.entries, vec!["echo USER1".to_string()]);
        assert_eq!(shell.lines_this_session, 1);
    }

    #[test]
    fn prompt_command_history_a_flushes_user_lines_not_runner() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = std::env::temp_dir().join(format!("wt90-pc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let histfile = dir.join("hist.txt");
        std::fs::write(&histfile, "echo seed\n").unwrap();
        let histpath = histfile.to_string_lossy().to_string();

        let mut executor = Executor::new();
        let session = Rc::new(RefCell::new(crate::history::SessionHistory::new()));
        session.borrow_mut().record("echo USER2", "", "", None);
        executor.set_session_history(Some(session.clone()));
        executor.set_env("PROMPT_COMMAND", "history -a");
        executor.set_env("HISTFILE", &histpath);
        executor.set_env("HISTCONTROL", "auto");
        assert_eq!(executor.execute_prompt_command(), Some(0));
        let file = std::fs::read_to_string(&histfile).unwrap();
        assert!(
            file.contains("echo USER2"),
            "the user's line must be flushed: {file:?}"
        );
        assert!(
            !file.contains("PROMPT_COMMAND"),
            "the runner text must never reach the file: {file:?}"
        );
        assert!(!file.contains("__rubash_pc"), "runner leak: {file:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prompt_command_save_reload_cycle_matches_gnu() {
        // The bash-it theme shape per prompt: history -a && history -c &&
        // history -r on the LIVE session (HISTCONTROL=auto branch of
        // _bash-it-history-auto-load). The user line is written exactly
        // once, the reload restores the file, and nothing compounds.
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = std::env::temp_dir().join(format!("wt90-srl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let histfile = dir.join("hist.txt");
        std::fs::write(&histfile, "echo seed\n").unwrap();
        let histpath = histfile.to_string_lossy().to_string();

        let mut executor = Executor::new();
        let session = Rc::new(RefCell::new(crate::history::SessionHistory::new()));
        executor.set_session_history(Some(session.clone()));
        executor.set_env("PROMPT_COMMAND", "history -a && history -c && history -r");
        executor.set_env("HISTFILE", &histpath);
        executor.set_env("HISTCONTROL", "auto");

        // Two prompts with one command between them (the loop shape).
        assert_eq!(executor.execute_prompt_command(), Some(0));
        session.borrow_mut().record("echo USER3", "", "", None);
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.execute_prompt_command(), Some(0));

        let file = std::fs::read_to_string(&histfile).unwrap();
        assert_eq!(
            file.matches("echo USER3").count(),
            1,
            "the user line must be written exactly once: {file:?}"
        );
        assert!(
            !file.contains("PROMPT_COMMAND") && !file.contains("__rubash_pc"),
            "runner leak: {file:?}"
        );
        let shell = session.borrow();
        assert!(
            shell.entries.iter().any(|e| e == "echo USER3"),
            "the reload must restore the user line into the live list: {:?}",
            shell.entries
        );
        assert!(!shell.entries.iter().any(|e| e.contains("__rubash_pc")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
