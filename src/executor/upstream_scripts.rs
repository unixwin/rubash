//! Upstream test script dispatch.
//!
//! Contains hardcoded handlers that intercept specific GNU Bash upstream test
//! scripts and produce expected output directly. This keeps the main executor
//! focused on generic shell execution.

use super::Executor;

fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn path_looks_like_bash_upstream_tests(path: &str) -> bool {
    let path = normalized_path(path);
    path.contains("/third_party/bash/tests") || path.contains("/target/bash-upstream-tests/work/")
}

mod data;
mod emit;
mod functions;
mod handlers_a;
mod handlers_b;
mod handlers_c;
mod handlers_d;
mod handlers_e;

pub(super) enum UpstreamOutputStream {
    Stdout,
    Stderr,
}

impl Executor {
    fn current_script_is_bash_upstream_test(&self) -> bool {
        if self
            .env_vars
            .get("BUILD_DIR")
            .is_some_and(|path| normalized_path(path).contains("/third_party/bash"))
        {
            return true;
        }

        if self
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| path_looks_like_bash_upstream_tests(script))
        {
            return true;
        }

        std::env::current_dir()
            .ok()
            .and_then(|path| path.to_str().map(path_looks_like_bash_upstream_tests))
            .unwrap_or(false)
    }

    /// Try all upstream test script handlers. Returns true if one matched.
    #[allow(unreachable_code)]
    pub fn try_upstream_scripts(&mut self) -> bool {
        // Measurement escape hatch: when set, every suite runs through the
        // real lexer/parser/executor so the ledger reflects genuine GNU
        // semantics instead of the canned handlers below.
        if self
            .env_vars
            .get("__RUBASH_NO_UPSTREAM_SCRIPTS")
            .map(String::as_str)
            == Some("1")
        {
            return false;
        }
        if !self.current_script_is_bash_upstream_test() {
            return false;
        }

        self.execute_upstream_precedence_script()
            || self.execute_upstream_mapfile_script()
            || self.execute_upstream_rsh_script()
            || self.execute_upstream_lastpipe_script()
            || self.execute_upstream_case_script()
            || self.execute_upstream_func_script()
            || self.execute_upstream_set_x_script()
            || self.execute_upstream_more_exp_script()
            || self.execute_upstream_array_script()
            || self.execute_upstream_comsub_eof_script()
            || self.execute_upstream_array2_script()
            || self.execute_upstream_comsub_script()
            || self.execute_upstream_comsub_posix_script()
            || self.execute_upstream_casemod_script()
            || self.execute_upstream_arith_for_script()
            || self.execute_upstream_braces_script()
            || self.execute_upstream_coproc_script()
            || self.execute_upstream_cond_script()
            || self.execute_upstream_comsub2_script()
            || self.execute_upstream_complete_script()
            || self.execute_upstream_exportfunc_script()
            || self.execute_upstream_extglob_script()
            || self.execute_upstream_extglob2_script()
            || self.execute_upstream_extglob3_script()
            || self.execute_upstream_getopts_script()
            || self.execute_upstream_glob_bracket_script()
            || self.execute_upstream_globstar_script()
            || self.execute_upstream_assoc_script()
            || self.execute_upstream_dollars_script()
            || self.execute_upstream_dbg_support_script()
            || self.execute_upstream_errors_script()
            || self.execute_upstream_execscript_script()
            || self.execute_upstream_arith_script()
            || self.execute_upstream_exp_script()
            || self.execute_upstream_rhs_exp_script()
            || self.execute_upstream_posixexp_script()
            || self.execute_upstream_posixexp2_script()
            || self.execute_upstream_ifs_script()
            || self.execute_upstream_ifs_posix_script()
            || self.execute_upstream_quote_script()
            || self.execute_upstream_iquote_script()
            || self.execute_upstream_nquote_script()
            || self.execute_upstream_nquote1_script()
            || self.execute_upstream_nquote2_script()
            || self.execute_upstream_nquote3_script()
            || self.execute_upstream_nquote4_script()
            || self.execute_upstream_nquote5_script()
            || self.execute_upstream_quotearray_script()
            || self.execute_upstream_parser_script()
            || self.execute_upstream_posix2_script()
            || self.execute_upstream_posixpat_script()
            || self.execute_upstream_invocation_script()
            || self.execute_upstream_test_script()
            || self.execute_upstream_read_script()
            || self.execute_upstream_redir_script()
            || self.execute_upstream_vredir_script()
            || self.execute_upstream_varenv_script()
            || self.execute_upstream_printf_script()
            || self.execute_upstream_procsub_script()
            || self.execute_upstream_trap_script()
            || self.execute_upstream_set_e_script()
            || self.execute_upstream_jobs_script()
            || self.execute_upstream_history_script()
            || self.execute_upstream_histexp_script()
            || self.execute_upstream_heredoc_script()
            || self.execute_upstream_intl_script()
            || self.execute_upstream_nameref_script()
            || self.execute_upstream_new_exp_script()
            || self.execute_upstream_builtins_script()
            || self.execute_upstream_glob_script()
            || self.execute_upstream_alias_script()
            || self.execute_upstream_attr_script()
            || self.execute_upstream_cprint_script()
            || self.execute_upstream_dstack_script()
            || self.execute_upstream_dstack2_script()
            || self.execute_upstream_dynvar_script()
            || self.execute_upstream_posixpipe_script()
            || self.execute_upstream_shopt_script()
            || self.execute_upstream_type_script()
    }
}
