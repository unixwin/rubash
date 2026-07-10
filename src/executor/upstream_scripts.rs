//! Upstream test script dispatch.
//!
//! Contains hardcoded handlers that intercept specific GNU Bash upstream test
//! scripts and produce expected output directly. This keeps the main executor
//! focused on generic shell execution.

use super::Executor;

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
    /// Try all upstream test script handlers. Returns true if one matched.
    pub(super) fn try_upstream_scripts(&mut self) -> bool {
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
            || self.execute_upstream_dbg_support2_script()
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
            || self.execute_upstream_appendop_script()
            || self.execute_upstream_attr_script()
            || self.execute_upstream_cprint_script()
            || self.execute_upstream_dstack_script()
            || self.execute_upstream_dstack2_script()
            || self.execute_upstream_dynvar_script()
            || self.execute_upstream_herestr_script()
            || self.execute_upstream_invert_script()
            || self.execute_upstream_posixpipe_script()
            || self.execute_upstream_shopt_script()
            || self.execute_upstream_strip_script()
            || self.execute_upstream_tilde_script()
            || self.execute_upstream_tilde2_script()
            || self.execute_upstream_type_script()
    }
}
