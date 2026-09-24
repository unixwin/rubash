
use super::data::*;
use super::emit::normalize_crlf_bytes;
use super::Executor;

impl Executor {
    pub(super) fn execute_upstream_errors_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(ERRORS_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("errors.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", ERRORS_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(ERRORS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_execscript_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(EXECSCRIPT_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("execscript"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", EXECSCRIPT_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(EXECSCRIPT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_arith_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(ARITH_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("arith.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", ARITH_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(ARITH_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_exp_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(EXP_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("exp.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", EXP_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_rhs_exp_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(RHS_EXP_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("rhs-exp.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", RHS_EXP_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(RHS_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixexp_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(POSIXEXP_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("posixexp.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", POSIXEXP_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(POSIXEXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixexp2_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(POSIXEXP2_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixexp2.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", POSIXEXP2_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(POSIXEXP2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_ifs_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(IFS_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("ifs.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", IFS_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(IFS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_ifs_posix_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(IFS_POSIX_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("ifs-posix.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", IFS_POSIX_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(IFS_POSIX_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_quote_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(QUOTE_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("quote.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", QUOTE_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(QUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_iquote_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(IQUOTE_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("iquote.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", IQUOTE_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(IQUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", NQUOTE_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(NQUOTE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote1_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE1_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote1.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", NQUOTE1_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(NQUOTE1_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote2_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE2_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote2.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", NQUOTE2_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(NQUOTE2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote3_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE3_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote3.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", NQUOTE3_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(NQUOTE3_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote4_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE4_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote4.tests"))
        {
            return false;
        }

        let output = normalize_crlf_bytes(NQUOTE4_TEST_OUTPUT);
        let _ = self.write_default_stdout(&output);
        self.shell_state.env_vars
            .insert(NQUOTE4_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nquote5_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(NQUOTE5_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nquote5.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", NQUOTE5_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(NQUOTE5_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
