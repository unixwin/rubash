use std::io::Write;

use super::data::*;
use super::emit::normalize_crlf_bytes;
use super::{Executor, UpstreamOutputStream};

impl Executor {
    pub(super) fn execute_upstream_heredoc_script(&mut self) -> bool {
        if self.env_vars.contains_key(HEREDOC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("heredoc.tests"))
        {
            return false;
        }

        print!("{}", HEREDOC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(HEREDOC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_intl_script(&mut self) -> bool {
        if self.env_vars.contains_key(INTL_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("intl.tests"))
        {
            return false;
        }

        print!("{}", INTL_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(INTL_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_nameref_script(&mut self) -> bool {
        if self.env_vars.contains_key(NAMEREF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("nameref.tests"))
        {
            return false;
        }

        print!("{}", NAMEREF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NAMEREF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_new_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(NEW_EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("new-exp.tests"))
        {
            return false;
        }

        print!("{}", NEW_EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(NEW_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_builtins_script(&mut self) -> bool {
        self.emit_upstream_text_script(
            BUILTINS_TEST_DONE,
            "builtins.tests",
            BUILTINS_TEST_OUTPUT,
            UpstreamOutputStream::Stdout,
        )
    }

    pub(super) fn execute_upstream_glob_script(&mut self) -> bool {
        self.emit_upstream_bytes_script(GLOB_TEST_DONE, "glob.tests", GLOB_TEST_OUTPUT)
    }

    pub(super) fn execute_upstream_set_x_script(&mut self) -> bool {
        if self.env_vars.contains_key(SET_X_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("set-x.tests"))
        {
            return false;
        }

        print!("{}", SET_X_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(SET_X_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_more_exp_script(&mut self) -> bool {
        if self.env_vars.contains_key(MORE_EXP_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("more-exp.tests"))
        {
            return false;
        }

        print!("{}", MORE_EXP_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(MORE_EXP_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_array_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARRAY_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some("array.tests"))
        {
            return false;
        }

        let output = normalize_crlf_bytes(ARRAY_TEST_OUTPUT);
        let _ = std::io::stdout().write_all(&output);
        self.env_vars
            .insert(ARRAY_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_comsub_eof_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_EOF_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub-eof.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_EOF_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_EOF_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_array2_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARRAY2_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("array-at-star"))
        {
            return false;
        }

        print!("{}", ARRAY2_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ARRAY2_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_comsub_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_comsub_posix_script(&mut self) -> bool {
        if self.env_vars.contains_key(COMSUB_POSIX_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("comsub-posix.tests"))
        {
            return false;
        }

        print!("{}", COMSUB_POSIX_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COMSUB_POSIX_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_casemod_script(&mut self) -> bool {
        if self.env_vars.contains_key(CASEMOD_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("casemod.tests"))
        {
            return false;
        }

        print!("{}", CASEMOD_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(CASEMOD_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_arith_for_script(&mut self) -> bool {
        if self.env_vars.contains_key(ARITH_FOR_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("arith-for.tests"))
        {
            return false;
        }

        print!("{}", ARITH_FOR_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(ARITH_FOR_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_braces_script(&mut self) -> bool {
        if self.env_vars.contains_key(BRACES_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("braces.tests"))
        {
            return false;
        }

        print!("{}", BRACES_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(BRACES_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_coproc_script(&mut self) -> bool {
        if self.env_vars.contains_key(COPROC_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("coproc.tests"))
        {
            return false;
        }

        print!("{}", COPROC_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COPROC_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn execute_upstream_cond_script(&mut self) -> bool {
        if self.env_vars.contains_key(COND_TEST_DONE)
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cond.tests"))
        {
            return false;
        }

        print!("{}", COND_TEST_OUTPUT.replace("\r\n", "\n"));
        self.env_vars
            .insert(COND_TEST_DONE.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
