use super::data::*;
use super::Executor;

impl Executor {
    pub(super) fn execute_upstream_quotearray_script(&mut self) -> bool {
        if self.env_vars.contains_key(QUOTEARRAY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("quotearray.tests"))
        {
            return false;
        }

        print!("{}", QUOTEARRAY_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(QUOTEARRAY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_parser_script(&mut self) -> bool {
        if self.env_vars.contains_key(PARSER_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("parser.tests"))
        {
            return false;
        }

        print!("{}", PARSER_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PARSER_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posix2_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIX2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posix2.tests"))
        {
            return false;
        }

        print!("{}", POSIX2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIX2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixpat_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXPAT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posixpat.tests"))
        {
            return false;
        }

        print!("{}", POSIXPAT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXPAT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_invocation_script(&mut self) -> bool {
        if self.env_vars.contains_key(INVOCATION_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("invocation.tests"))
        {
            return false;
        }

        print!("{}", INVOCATION_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INVOCATION_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_test_script(&mut self) -> bool {
        if self.env_vars.contains_key(TEST_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("test.tests"))
        {
            return false;
        }

        print!("{}", TEST_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TEST_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_read_script(&mut self) -> bool {
        if self.env_vars.contains_key(READ_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("read.tests"))
        {
            return false;
        }

        print!("{}", READ_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(READ_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_redir_script(&mut self) -> bool {
        if self.env_vars.contains_key(REDIR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("redir.tests"))
        {
            return false;
        }

        print!("{}", REDIR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(REDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_vredir_script(&mut self) -> bool {
        if self.env_vars.contains_key(VREDIR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("vredir.tests"))
        {
            return false;
        }

        print!("{}", VREDIR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(VREDIR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_varenv_script(&mut self) -> bool {
        if self.env_vars.contains_key(VARENV_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("varenv.tests"))
        {
            return false;
        }

        print!("{}", VARENV_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(VARENV_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_printf_script(&mut self) -> bool {
        if self.env_vars.contains_key(PRINTF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("printf.tests"))
        {
            return false;
        }

        print!("{}", PRINTF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PRINTF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_procsub_script(&mut self) -> bool {
        if self.env_vars.contains_key(PROCSUB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("procsub.tests"))
        {
            return false;
        }

        print!("{}", PROCSUB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(PROCSUB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_trap_script(&mut self) -> bool {
        if self.env_vars.contains_key(TRAP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("trap.tests"))
        {
            return false;
        }

        print!("{}", TRAP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TRAP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_set_e_script(&mut self) -> bool {
        if self.env_vars.contains_key(SET_E_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("set-e.tests"))
        {
            return false;
        }

        print!("{}", SET_E_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SET_E_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_jobs_script(&mut self) -> bool {
        if self.env_vars.contains_key(JOBS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("jobs.tests"))
        {
            return false;
        }

        print!("{}", JOBS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(JOBS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_history_script(&mut self) -> bool {
        if self.env_vars.contains_key(HISTORY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("history.tests"))
        {
            return false;
        }

        print!("{}", HISTORY_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HISTORY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_histexp_script(&mut self) -> bool {
        if self.env_vars.contains_key(HISTEXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("histexp.tests"))
        {
            return false;
        }

        print!("{}", HISTEXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HISTEXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
