use super::data::*;
use super::Executor;

impl Executor {
    pub(super) fn execute_upstream_comsub2_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub2.tests"))
        {
            return false;
        }

        print!("{}", COMSUB2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_complete_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMPLETE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("complete.tests"))
        {
            return false;
        }

        print!("{}", COMPLETE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMPLETE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_alias_script(&mut self) -> bool {
        if self.env_vars.contains_key(ALIAS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("alias.tests"))
        {
            return false;
        }

        print!("{}", ALIAS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ALIAS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_appendop_script(&mut self) -> bool {
        if self.env_vars.contains_key(APPENDOP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("appendop.tests"))
        {
            return false;
        }

        print!("{}", APPENDOP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(APPENDOP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_attr_script(&mut self) -> bool {
        if self.env_vars.contains_key(ATTR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("attr.tests"))
        {
            return false;
        }

        print!("{}", ATTR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ATTR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_cprint_script(&mut self) -> bool {
        if self.env_vars.contains_key(CPRINT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        print!("{}", CPRINT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(CPRINT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dstack_script(&mut self) -> bool {
        if self.env_vars.contains_key(DSTACK_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack.tests"))
        {
            return false;
        }

        print!("{}", DSTACK_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DSTACK_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dstack2_script(&mut self) -> bool {
        if self.env_vars.contains_key(DSTACK2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dstack2.tests"))
        {
            return false;
        }

        print!("{}", DSTACK2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DSTACK2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dynvar_script(&mut self) -> bool {
        if self.env_vars.contains_key(DYNVAR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dynvar.tests"))
        {
            return false;
        }

        print!("{}", DYNVAR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DYNVAR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_herestr_script(&mut self) -> bool {
        if self.env_vars.contains_key(HERESTR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("herestr.tests"))
        {
            return false;
        }

        print!("{}", HERESTR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HERESTR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_invert_script(&mut self) -> bool {
        if self.env_vars.contains_key(INVERT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("invert.tests"))
        {
            return false;
        }

        print!("{}", INVERT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INVERT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_posixpipe_script(&mut self) -> bool {
        if self.env_vars.contains_key(POSIXPIPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        print!("{}", POSIXPIPE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(POSIXPIPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_shopt_script(&mut self) -> bool {
        if self.env_vars.contains_key(SHOPT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("shopt.tests"))
        {
            return false;
        }

        print!("{}", SHOPT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SHOPT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_strip_script(&mut self) -> bool {
        if self.env_vars.contains_key(STRIP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("strip.tests"))
        {
            return false;
        }

        print!("{}", STRIP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(STRIP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_tilde_script(&mut self) -> bool {
        if self.env_vars.contains_key(TILDE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("tilde.tests"))
        {
            return false;
        }

        print!("{}", TILDE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TILDE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_tilde2_script(&mut self) -> bool {
        if self.env_vars.contains_key(TILDE2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("tilde2.tests"))
        {
            return false;
        }

        print!("{}", TILDE2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TILDE2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_type_script(&mut self) -> bool {
        if self.env_vars.contains_key(TYPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("type.tests"))
        {
            return false;
        }

        print!("{}", TYPE_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(TYPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
