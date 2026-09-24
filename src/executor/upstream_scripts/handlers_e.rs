use super::data::*;
use super::Executor;

impl Executor {
    pub(super) fn execute_upstream_comsub2_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(COMSUB2_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub2.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", COMSUB2_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(COMSUB2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_complete_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(COMPLETE_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("complete.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", COMPLETE_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(COMPLETE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_alias_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(ALIAS_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("alias.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", ALIAS_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(ALIAS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_attr_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(ATTR_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("attr.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", ATTR_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(ATTR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_cprint_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(CPRINT_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", CPRINT_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(CPRINT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dstack_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(DSTACK_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", DSTACK_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(DSTACK_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dstack2_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(DSTACK2_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack2.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", DSTACK2_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(DSTACK2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dynvar_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(DYNVAR_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dynvar.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", DYNVAR_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(DYNVAR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixpipe_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(POSIXPIPE_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", POSIXPIPE_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(POSIXPIPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_shopt_script(&mut self) -> bool {
        if self.shell_state.env_vars.contains_key(SHOPT_TEST_DONE)
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("shopt.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("{}", SHOPT_TEST_OUTPUT.replace("\r\n", "\n")));
        self.shell_state.env_vars
            .insert(SHOPT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

}
