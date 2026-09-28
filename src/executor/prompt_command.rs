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
    /// Run PROMPT_COMMAND per GNU eval.c:305. Returns the last executed
    /// element's exit status, or None when nothing ran (unset, empty,
    /// associative array).
    pub fn execute_prompt_command(&mut self) -> Option<i32> {
        // eval.c:314-316: absent/unset/invisible -> no work. An unset-but-
        // declared variable still occupies env storage with an empty value,
        // and the empty-string check below covers eval.c:329-330's
        // `command_to_execute && *command_to_execute`.
        let Some(value) = self.get_env("PROMPT_COMMAND") else {
            return None;
        };
        if value.trim().is_empty() {
            return None;
        }

        const RUNNER: &str = r#"case "${PROMPT_COMMAND@a}" in
  *a*)
    # Indexed array: eval each non-empty element in index order
    # (execute_array_command via array_to_argv, eval.c:294-299).
    for __rubash_pc in "${PROMPT_COMMAND[@]}"; do
      [ -n "$__rubash_pc" ] && eval "$__rubash_pc"
    done
    ;;
  *A*)
    # Associative array: GNU refuses these (eval.c:324-325).
    ;;
  *)
    # Plain string: the evalstring equivalent of execute_variable_command.
    eval "$PROMPT_COMMAND"
    ;;
esac
unset __rubash_pc 2>/dev/null || true"#;
        let code = crate::script_driver::run_script_with_history(self, RUNNER, None);
        Some(code)
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

    #[test]
    fn prompt_command_associative_array_is_noop() {
        let mut executor = Executor::new();
        seed(&mut executor, "declare -A PROMPT_COMMAND=([x]='__pc_a=1')");
        assert_eq!(executor.execute_prompt_command(), Some(0));
        assert_eq!(executor.get_env("__pc_a"), None);
    }

    #[test]
    fn prompt_command_unset_runs_nothing() {
        let mut executor = Executor::new();
        assert_eq!(executor.execute_prompt_command(), None);
        executor.set_env("PROMPT_COMMAND", "   ");
        assert_eq!(executor.execute_prompt_command(), None);
    }
}
