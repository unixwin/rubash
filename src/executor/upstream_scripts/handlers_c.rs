use super::data::*;
use super::Executor;

impl Executor {
    pub(super) fn execute_upstream_quotearray_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(QUOTEARRAY_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("quotearray.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", QUOTEARRAY_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(QUOTEARRAY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_parser_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(PARSER_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("parser.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", PARSER_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(PARSER_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posix2_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(POSIX2_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posix2.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", POSIX2_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(POSIX2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixpat_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(POSIXPAT_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posixpat.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", POSIXPAT_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(POSIXPAT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_invocation_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(INVOCATION_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("invocation.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", INVOCATION_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(INVOCATION_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_test_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(TEST_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("test.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", TEST_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(TEST_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_read_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(READ_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("read.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", READ_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(READ_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_redir_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(REDIR_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("redir.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", REDIR_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(REDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_vredir_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(VREDIR_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("vredir.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", VREDIR_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(VREDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_varenv_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(VARENV_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("varenv.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", VARENV_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(VARENV_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_printf_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(PRINTF_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("printf.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", PRINTF_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(PRINTF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_procsub_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(PROCSUB_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("procsub.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", PROCSUB_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(PROCSUB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_trap_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(TRAP_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("trap.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", TRAP_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(TRAP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_set_e_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(SET_E_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("set-e.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", SET_E_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(SET_E_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_jobs_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(JOBS_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("jobs.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", JOBS_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(JOBS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
