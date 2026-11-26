mod common;

#[path = "issues_3600_3899/issue3605_alias_expansion_lines.rs"]
mod issue3605_alias_expansion_lines;
#[path = "issues_3600_3899/issue3605_alternate_fragment_separators.rs"]
mod issue3605_alternate_fragment_separators;
#[path = "issues_3600_3899/issue3605_alternate_procsub_materialization.rs"]
mod issue3605_alternate_procsub_materialization;
#[path = "issues_3600_3899/issue3605_assign_rhs_quoted_null.rs"]
mod issue3605_assign_rhs_quoted_null;
#[path = "issues_3600_3899/issue3605_child_latch_leak.rs"]
mod issue3605_child_latch_leak;
#[path = "issues_3600_3899/issue3605_compound_eof_last_line.rs"]
mod issue3605_compound_eof_last_line;
#[path = "issues_3600_3899/issue3605_comsub_word_eof_line.rs"]
mod issue3605_comsub_word_eof_line;
#[path = "issues_3600_3899/issue3605_dollar_at_star_ifs_separators.rs"]
mod issue3605_dollar_at_star_ifs_separators;
#[path = "issues_3600_3899/issue3605_enoexec_binary_gate.rs"]
mod issue3605_enoexec_binary_gate;
#[path = "issues_3600_3899/issue3605_eof_exit_status_word_pairs.rs"]
mod issue3605_eof_exit_status_word_pairs;
#[path = "issues_3600_3899/issue3605_errexit_suppression_transport.rs"]
mod issue3605_errexit_suppression_transport;
#[path = "issues_3600_3899/issue3605_eval_eof_close_char_line.rs"]
mod issue3605_eval_eof_close_char_line;
#[path = "issues_3600_3899/issue3605_globignore_strmatch_multibyte.rs"]
mod issue3605_globignore_strmatch_multibyte;
#[path = "issues_3600_3899/issue3605_grouped_def_end_line.rs"]
mod issue3605_grouped_def_end_line;
#[path = "issues_3600_3899/issue3605_grouped_trailing_continuation.rs"]
mod issue3605_grouped_trailing_continuation;
#[path = "issues_3600_3899/issue3605_heredoc_end_line_rebase.rs"]
mod issue3605_heredoc_end_line_rebase;
#[path = "issues_3600_3899/issue3605_indirect_special_source.rs"]
mod issue3605_indirect_special_source;
#[path = "issues_3600_3899/issue3605_nested_case_comsub_spans.rs"]
mod issue3605_nested_case_comsub_spans;
#[path = "issues_3600_3899/issue3605_operator_value_ctlnul.rs"]
mod issue3605_operator_value_ctlnul;
#[path = "issues_3600_3899/issue3605_pipeline_async_stdin.rs"]
mod issue3605_pipeline_async_stdin;
#[path = "issues_3600_3899/issue3605_quoted_null_anchor.rs"]
mod issue3605_quoted_null_anchor;
#[path = "issues_3600_3899/issue3605_star7_splitspace_ifs_regressions.rs"]
mod issue3605_star7_splitspace_ifs_regressions;
#[path = "issues_3600_3899/issue3605_subscript_innermost_verdict.rs"]
mod issue3605_subscript_innermost_verdict;
#[path = "issues_3600_3899/issue3605_this_sh_child_exit_trap.rs"]
mod issue3605_this_sh_child_exit_trap;
#[path = "issues_3600_3899/issue3605_ulimit_infinity_rows.rs"]
mod issue3605_ulimit_infinity_rows;
#[path = "issues_3600_3899/issue3605_ulimit_region_virtualization.rs"]
mod issue3605_ulimit_region_virtualization;
#[path = "issues_3600_3899/issue3605_unquoted_inner_quoted_null.rs"]
mod issue3605_unquoted_inner_quoted_null;
#[path = "issues_3600_3899/issue3609_local_readonly_shadow.rs"]
mod issue3609_local_readonly_shadow;
#[path = "issues_3600_3899/issue3613_compound_declare_triple_expand.rs"]
mod issue3613_compound_declare_triple_expand;
#[path = "issues_3600_3899/issue3619_ansic_surviving_locale_decode.rs"]
mod issue3619_ansic_surviving_locale_decode;
#[path = "issues_3600_3899/issue3625_truncated_macho.rs"]
mod issue3625_truncated_macho;
#[path = "issues_3600_3899/issue3628_case_in_comsub_token.rs"]
mod issue3628_case_in_comsub_token;
#[path = "issues_3600_3899/issue3633_coproc_eof_regressions.rs"]
mod issue3633_coproc_eof_regressions;
#[path = "issues_3600_3899/issue3635_cp_create_kind_diagnostics.rs"]
mod issue3635_cp_create_kind_diagnostics;
#[path = "issues_3600_3899/issue3637_async_death_notice_regressions.rs"]
mod issue3637_async_death_notice_regressions;
#[path = "issues_3600_3899/issue3637_kill_cont_stopped_job.rs"]
mod issue3637_kill_cont_stopped_job;
#[path = "issues_3600_3899/issue3638_wait_stale_current_job.rs"]
mod issue3638_wait_stale_current_job;
#[path = "issues_3600_3899/issue3639_brace_fold_after_bang_regressions.rs"]
mod issue3639_brace_fold_after_bang_regressions;
#[path = "issues_3600_3899/issue3640_startup_cwd_fd3.rs"]
mod issue3640_startup_cwd_fd3;
#[path = "issues_3600_3899/issue3641_trap_unclosed_quotes.rs"]
mod issue3641_trap_unclosed_quotes;
#[path = "issues_3600_3899/issue3645_fc_s_empty_pattern_regressions.rs"]
mod issue3645_fc_s_empty_pattern_regressions;
#[path = "issues_3600_3899/issue3647_bang_after_pipe_source_echo_regressions.rs"]
mod issue3647_bang_after_pipe_source_echo_regressions;
#[path = "issues_3600_3899/issue3648_bang_brace_regressions.rs"]
mod issue3648_bang_brace_regressions;
#[path = "issues_3600_3899/issue3649_substring_offset_writes.rs"]
mod issue3649_substring_offset_writes;
#[path = "issues_3600_3899/issue3650_read_bracket_control_bytes.rs"]
mod issue3650_read_bracket_control_bytes;
#[path = "issues_3600_3899/issue3655_heredoc_vs_file_redirect.rs"]
mod issue3655_heredoc_vs_file_redirect;
#[path = "issues_3600_3899/issue3659_mapfile_read_attr_conversion.rs"]
mod issue3659_mapfile_read_attr_conversion;
#[path = "issues_3600_3899/issue3660_mapfile_callback_stdin.rs"]
mod issue3660_mapfile_callback_stdin;
#[path = "issues_3600_3899/issue3661_posixly_correct_runtime_hook_regressions.rs"]
mod issue3661_posixly_correct_runtime_hook_regressions;
#[path = "issues_3600_3899/issue3662_bg_job_in_comsub_serializer_regressions.rs"]
mod issue3662_bg_job_in_comsub_serializer_regressions;
#[path = "issues_3600_3899/issue3663_unquoted_assignment_backslash.rs"]
mod issue3663_unquoted_assignment_backslash;
#[path = "issues_3600_3899/issue3664_backquote_dquote_eval_reparse.rs"]
mod issue3664_backquote_dquote_eval_reparse;
#[path = "issues_3600_3899/issue3706_group_redirect_fd_restore.rs"]
mod issue3706_group_redirect_fd_restore;
#[path = "issues_3600_3899/issue3725_loop_heredoc_fd_restore.rs"]
mod issue3725_loop_heredoc_fd_restore;
#[path = "issues_3600_3899/issue3726_net_bind_fd_restore.rs"]
mod issue3726_net_bind_fd_restore;
#[path = "issues_3600_3899/issue3727_function_call_stdin_fd_restore.rs"]
mod issue3727_function_call_stdin_fd_restore;
#[path = "issues_3600_3899/issue3728_read_tcp_fd_restore.rs"]
mod issue3728_read_tcp_fd_restore;
#[path = "issues_3600_3899/issue3729_stdio_marker_fd_restore.rs"]
mod issue3729_stdio_marker_fd_restore;
#[path = "issues_3600_3899/issue3730_cond_stdio_fd_restore.rs"]
mod issue3730_cond_stdio_fd_restore;
#[path = "issues_3600_3899/issue3731_procsub_body_fd0_restore.rs"]
mod issue3731_procsub_body_fd0_restore;
#[path = "issues_3600_3899/issue3732_procsub_spawn_plan_fd_restore.rs"]
mod issue3732_procsub_spawn_plan_fd_restore;
#[path = "issues_3600_3899/issue3733_redirect_mid_list_undo.rs"]
mod issue3733_redirect_mid_list_undo;
#[path = "issues_3600_3899/issue3756_comsub_parse_error_discard.rs"]
mod issue3756_comsub_parse_error_discard;
#[path = "issues_3600_3899/issue3770_read_writeonly_fd0.rs"]
mod issue3770_read_writeonly_fd0;
#[path = "issues_3600_3899/issue3782_declare_i_plus_eq_wrap_regressions.rs"]
mod issue3782_declare_i_plus_eq_wrap_regressions;
#[path = "issues_3600_3899/issue3783_test_v_positional_regressions.rs"]
mod issue3783_test_v_positional_regressions;
#[path = "issues_3600_3899/issue3784_indirect_expansion_zero.rs"]
mod issue3784_indirect_expansion_zero;
#[path = "issues_3600_3899/issue3785_read_n_bytes_redirect_pipeline.rs"]
mod issue3785_read_n_bytes_redirect_pipeline;
#[path = "issues_3600_3899/issue3786_printf_half_rounding.rs"]
mod issue3786_printf_half_rounding;
#[path = "issues_3600_3899/issue3787_echo_e_nonscalar_escapes.rs"]
mod issue3787_echo_e_nonscalar_escapes;
#[path = "issues_3600_3899/issue3788_cd_no_search_permission_regressions.rs"]
mod issue3788_cd_no_search_permission_regressions;
#[path = "issues_3600_3899/issue3790_declare_lu_simple_mapping.rs"]
mod issue3790_declare_lu_simple_mapping;
#[path = "issues_3600_3899/issue3791_regex_bracket_nonascii_clocale_regressions.rs"]
mod issue3791_regex_bracket_nonascii_clocale_regressions;
#[path = "issues_3600_3899/issue3792_dup_write_only_fd.rs"]
mod issue3792_dup_write_only_fd;
#[path = "issues_3600_3899/issue3802_delivered_signal_guard.rs"]
mod issue3802_delivered_signal_guard;
#[path = "issues_3600_3899/issue3802_jobs_table_killed_wording.rs"]
mod issue3802_jobs_table_killed_wording;
#[path = "issues_3600_3899/issue3803_windows_wait_death_notice_regressions.rs"]
mod issue3803_windows_wait_death_notice_regressions;
#[path = "issues_3600_3899/issue3806_procsub_input_rebind_body.rs"]
mod issue3806_procsub_input_rebind_body;
#[path = "issues_3600_3899/issue3808_procsub_group_body.rs"]
mod issue3808_procsub_group_body;
#[path = "issues_3600_3899/issue3813_alias_comsub_comment_quote_regressions.rs"]
mod issue3813_alias_comsub_comment_quote_regressions;
#[path = "issues_3600_3899/issue3814_subshell_close_fd_isolation.rs"]
mod issue3814_subshell_close_fd_isolation;
#[path = "issues_3600_3899/issue3815_nested_group_null_device_fd.rs"]
mod issue3815_nested_group_null_device_fd;
#[path = "issues_3600_3899/issue3816_literal_control_bytes.rs"]
mod issue3816_literal_control_bytes;
#[path = "issues_3600_3899/issue3818_input_redirect_fd_exhaustion.rs"]
mod issue3818_input_redirect_fd_exhaustion;
#[path = "issues_3600_3899/issue3819_pipe_fd_exhaustion.rs"]
mod issue3819_pipe_fd_exhaustion;
#[path = "issues_3600_3899/issue3820_getopts_colon_optarg.rs"]
mod issue3820_getopts_colon_optarg;
#[path = "issues_3600_3899/issue3823_funsub_lexer_residual.rs"]
mod issue3823_funsub_lexer_residual;
#[path = "issues_3600_3899/issue3824_hash_empty_p_path.rs"]
mod issue3824_hash_empty_p_path;
#[path = "issues_3600_3899/issue3825_hash_type_nonexec_path.rs"]
mod issue3825_hash_type_nonexec_path;
#[path = "issues_3600_3899/issue3826_path_tilde_expansion.rs"]
mod issue3826_path_tilde_expansion;
#[path = "issues_3600_3899/issue3827_exec_failure_diagnostic_relative_path.rs"]
mod issue3827_exec_failure_diagnostic_relative_path;
#[path = "issues_3600_3899/issue3828_umask_redirect_permission.rs"]
mod issue3828_umask_redirect_permission;
#[path = "issues_3600_3899/issue3833_bang_reserved_words.rs"]
mod issue3833_bang_reserved_words;
#[path = "issues_3600_3899/issue3834_nested_case_comsub_body.rs"]
mod issue3834_nested_case_comsub_body;
#[path = "issues_3600_3899/issue3838_comsub_alias_newlines.rs"]
mod issue3838_comsub_alias_newlines;
#[path = "issues_3600_3899/issue3852_redirect_failure_exit_status.rs"]
mod issue3852_redirect_failure_exit_status;
#[path = "issues_3600_3899/issue3859_comsub_ansic_escape.rs"]
mod issue3859_comsub_ansic_escape;
#[path = "issues_3600_3899/issue3864_parameter_errors_regression.rs"]
mod issue3864_parameter_errors_regression;
#[path = "issues_3600_3899/issue3875_probe_bind_residuals.rs"]
mod issue3875_probe_bind_residuals;
#[path = "issues_3600_3899/issue3892_exec_varassign_no_undo.rs"]
mod issue3892_exec_varassign_no_undo;
#[path = "issues_3600_3899/issue3893_exec_var_heredoc_fd_allocation.rs"]
mod issue3893_exec_var_heredoc_fd_allocation;
#[path = "issues_3600_3899/issue3894_varassign_heredoc_no_fd0.rs"]
mod issue3894_varassign_heredoc_no_fd0;
