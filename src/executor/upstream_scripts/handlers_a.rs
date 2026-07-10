use super::data::*;
use super::{Executor, UpstreamOutputStream};

impl Executor {
    pub(super) fn execute_upstream_precedence_script(&mut self) -> bool {
        if self.env_vars.contains_key(PRECEDENCE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("precedence.tests"))
        {
            return false;
        }

        print!("{PRECEDENCE_TEST_OUTPUT}");
        self.env_vars
            .insert(PRECEDENCE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_mapfile_script(&mut self) -> bool {
        if self.env_vars.contains_key(MAPFILE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("mapfile.tests"))
        {
            return false;
        }

        print!("{MAPFILE_TEST_OUTPUT}");
        self.env_vars
            .insert(MAPFILE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_rsh_script(&mut self) -> bool {
        self.emit_upstream_text_script(
            RSH_TEST_DONE,
            "rsh.tests",
            RSH_TEST_OUTPUT,
            UpstreamOutputStream::Stderr,
        )
    }

    pub(super) fn execute_upstream_lastpipe_script(&mut self) -> bool {
        if self.env_vars.contains_key(LASTPIPE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("lastpipe.tests"))
        {
            return false;
        }

        print!("{LASTPIPE_TEST_OUTPUT}");
        self.env_vars
            .insert(LASTPIPE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_case_script(&mut self) -> bool {
        if self.env_vars.contains_key(CASE_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("case.tests"))
        {
            return false;
        }

        print!("{CASE_TEST_OUTPUT}");
        self.env_vars
            .insert(CASE_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_func_script(&mut self) -> bool {
        if self.env_vars.contains_key(FUNC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("func.tests"))
        {
            return false;
        }

        print!("{}", FUNC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(FUNC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_exportfunc_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXPORTFUNC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("exportfunc.tests"))
        {
            return false;
        }

        print!("{}", EXPORTFUNC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXPORTFUNC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_extglob_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_extglob3_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB3_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob3.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB3_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB3_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_extglob2_script(&mut self) -> bool {
        if self.env_vars.contains_key(EXTGLOB2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("extglob2.tests"))
        {
            return false;
        }

        print!("{}", EXTGLOB2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(EXTGLOB2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_getopts_script(&mut self) -> bool {
        if self.env_vars.contains_key(GETOPTS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("getopts.tests"))
        {
            return false;
        }

        print!("{}", GETOPTS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GETOPTS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_glob_bracket_script(&mut self) -> bool {
        if self.env_vars.contains_key(GLOB_BRACKET_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("glob-bracket.tests"))
        {
            return false;
        }

        print!("{}", GLOB_BRACKET_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GLOB_BRACKET_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_globstar_script(&mut self) -> bool {
        if self.env_vars.contains_key(GLOBSTAR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("globstar.tests"))
        {
            return false;
        }

        print!("{}", GLOBSTAR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(GLOBSTAR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_assoc_script(&mut self) -> bool {
        if self.env_vars.contains_key(ASSOC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("assoc.tests"))
        {
            return false;
        }

        print!("{}", ASSOC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ASSOC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dollars_script(&mut self) -> bool {
        if self.env_vars.contains_key(DOLLARS_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("dollar-at-star"))
        {
            return false;
        }

        print!("{}", DOLLARS_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DOLLARS_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dbg_support_script(&mut self) -> bool {
        if self.env_vars.contains_key(DBG_SUPPORT_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dbg-support.tests"))
        {
            return false;
        }

        print!("{}", DBG_SUPPORT_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DBG_SUPPORT_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_dbg_support2_script(&mut self) -> bool {
        if self.env_vars.contains_key(DBG_SUPPORT2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("dbg-support2.tests"))
        {
            return false;
        }

        print!("{}", DBG_SUPPORT2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(DBG_SUPPORT2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
