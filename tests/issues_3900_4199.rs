mod common;

#[path = "issues_3900_4199/issue3904_compassign_subscript_closer.rs"]
mod issue3904_compassign_subscript_closer;
#[path = "issues_3900_4199/issue3904_compassign_token_error.rs"]
mod issue3904_compassign_token_error;
#[path = "issues_3900_4199/issue3905_comsub_compound_assignment_eof.rs"]
mod issue3905_comsub_compound_assignment_eof;
#[path = "issues_3900_4199/issue3906_funnel_var_heredoc_list_position.rs"]
mod issue3906_funnel_var_heredoc_list_position;
#[path = "issues_3900_4199/issue3909_startup_extdebug_bash_argc.rs"]
mod issue3909_startup_extdebug_bash_argc;
#[path = "issues_3900_4199/issue3921_zero_length_pua_boundary_regressions.rs"]
mod issue3921_zero_length_pua_boundary_regressions;
#[path = "issues_3900_4199/issue3932_nofork_spawn_shlvl_regressions.rs"]
mod issue3932_nofork_spawn_shlvl_regressions;
#[path = "issues_3900_4199/issue3936_while_until_body_latched_error.rs"]
mod issue3936_while_until_body_latched_error;
#[path = "issues_3900_4199/issue3938_exec_redirect_integrity.rs"]
mod issue3938_exec_redirect_integrity;
#[path = "issues_3900_4199/issue3939_dev_zero_rw_shared.rs"]
mod issue3939_dev_zero_rw_shared;
#[path = "issues_3900_4199/issue3942_printf_q_nonprintable_multibyte.rs"]
mod issue3942_printf_q_nonprintable_multibyte;
#[path = "issues_3900_4199/issue3943_read_d_raw_byte_overmatch.rs"]
mod issue3943_read_d_raw_byte_overmatch;
#[path = "issues_3900_4199/issue3944_read_d_word_first_byte.rs"]
mod issue3944_read_d_word_first_byte;
#[path = "issues_3900_4199/issue3945_read_t0_readiness.rs"]
mod issue3945_read_t0_readiness;
#[path = "issues_3900_4199/issue3946_ansic_quote_line_paragraph_separator.rs"]
mod issue3946_ansic_quote_line_paragraph_separator;
#[path = "issues_3900_4199/issue3947_ansic_unicode_extended_utf8.rs"]
mod issue3947_ansic_unicode_extended_utf8;
#[path = "issues_3900_4199/issue3948_coproc_eof_reap_regressions.rs"]
mod issue3948_coproc_eof_reap_regressions;
#[path = "issues_3900_4199/issue3949_coproc_trailing_ampersand_regressions.rs"]
mod issue3949_coproc_trailing_ampersand_regressions;
#[path = "issues_3900_4199/issue3950_compgen_unassigned_vars.rs"]
mod issue3950_compgen_unassigned_vars;
#[path = "issues_3900_4199/issue3951_env_not_a_builtin.rs"]
mod issue3951_env_not_a_builtin;
#[path = "issues_3900_4199/issue3962_heredoc_quoted_ltlt_regressions.rs"]
mod issue3962_heredoc_quoted_ltlt_regressions;
#[path = "issues_3900_4199/issue3964_cat_dev_stdin_operand_contexts_regressions.rs"]
mod issue3964_cat_dev_stdin_operand_contexts_regressions;
#[path = "issues_3900_4199/issue3964_comsub_quickread_dev_stdin_regressions.rs"]
mod issue3964_comsub_quickread_dev_stdin_regressions;
#[path = "issues_3900_4199/issue3965_spawn_env_raw_bytes.rs"]
mod issue3965_spawn_env_raw_bytes;
#[path = "issues_3900_4199/issue3967_herestring_brace_group_regressions.rs"]
mod issue3967_herestring_brace_group_regressions;
#[path = "issues_3900_4199/issue3969_brace_backquote_rescan_regressions.rs"]
mod issue3969_brace_backquote_rescan_regressions;
#[path = "issues_3900_4199/issue3970_declare_compound_brace.rs"]
mod issue3970_declare_compound_brace;
#[path = "issues_3900_4199/issue3972_empty_comsub_whitespace.rs"]
mod issue3972_empty_comsub_whitespace;
#[path = "issues_3900_4199/issue3972_funsub_blank_body_regressions.rs"]
mod issue3972_funsub_blank_body_regressions;
#[path = "issues_3900_4199/issue3980_procsub_ro_fd_write_diag.rs"]
mod issue3980_procsub_ro_fd_write_diag;
#[path = "issues_3900_4199/issue3985_dev_full_exec_binding_regressions.rs"]
mod issue3985_dev_full_exec_binding_regressions;
#[path = "issues_3900_4199/issue3991_undo_save_materialization_regressions.rs"]
mod issue3991_undo_save_materialization_regressions;
#[path = "issues_3900_4199/issue3994_null_command_procsub_redirect_regressions.rs"]
mod issue3994_null_command_procsub_redirect_regressions;
#[path = "issues_3900_4199/issue3995_grouped_dash_stdin_operand_regressions.rs"]
mod issue3995_grouped_dash_stdin_operand_regressions;
#[path = "issues_3900_4199/issue3997_full_device_ambient_delivery_regressions.rs"]
mod issue3997_full_device_ambient_delivery_regressions;
#[path = "issues_3900_4199/issue3999_read_undo_save_regressions.rs"]
mod issue3999_read_undo_save_regressions;
#[path = "issues_3900_4199/issue4000_dup_move_shared_filefd.rs"]
mod issue4000_dup_move_shared_filefd;
#[path = "issues_3900_4199/issue4001_redirect_word_param_errors.rs"]
mod issue4001_redirect_word_param_errors;
#[path = "issues_3900_4199/issue4003_dolbrace_quoted_ifs_redirect.rs"]
mod issue4003_dolbrace_quoted_ifs_redirect;
#[path = "issues_3900_4199/issue4004_dolbrace_quoted_operator_word.rs"]
mod issue4004_dolbrace_quoted_operator_word;
#[path = "issues_3900_4199/issue4007_case_procsub.rs"]
mod issue4007_case_procsub;
#[path = "issues_3900_4199/issue4008_coproc_redir_eof_regressions.rs"]
mod issue4008_coproc_redir_eof_regressions;
#[path = "issues_3900_4199/issue4017_funsub_compound_tail.rs"]
mod issue4017_funsub_compound_tail;
#[path = "issues_3900_4199/issue4024_exec_walk_save_stdio_graft.rs"]
mod issue4024_exec_walk_save_stdio_graft;
#[path = "issues_3900_4199/issue4024_exec_walk_save_stdio_source.rs"]
mod issue4024_exec_walk_save_stdio_source;
#[path = "issues_3900_4199/issue4027_move_close_saved_source_regressions.rs"]
mod issue4027_move_close_saved_source_regressions;
#[path = "issues_3900_4199/issue4028_move_fd10_bad_fd_diag.rs"]
mod issue4028_move_fd10_bad_fd_diag;
#[path = "issues_3900_4199/issue4029_bad_substitution_raw_word_regressions.rs"]
mod issue4029_bad_substitution_raw_word_regressions;
#[path = "issues_3900_4199/issue4031_funsub_case_placement.rs"]
mod issue4031_funsub_case_placement;
#[path = "issues_3900_4199/issue4035_extended_utf8_length.rs"]
mod issue4035_extended_utf8_length;
#[path = "issues_3900_4199/issue4044_nested_exec_stdio_close_isolation.rs"]
mod issue4044_nested_exec_stdio_close_isolation;
#[path = "issues_3900_4199/issue4047_async_redir_swallowed.rs"]
mod issue4047_async_redir_swallowed;
#[path = "issues_3900_4199/issue4059_printf_t2_seconds.rs"]
mod issue4059_printf_t2_seconds;
#[path = "issues_3900_4199/issue4060_imported_noassign.rs"]
mod issue4060_imported_noassign;
#[path = "issues_3900_4199/issue4062_imported_scalar_default.rs"]
mod issue4062_imported_scalar_default;
#[path = "issues_3900_4199/issue4068_bad_subst_comsub_order.rs"]
mod issue4068_bad_subst_comsub_order;
#[path = "issues_3900_4199/issue4080_unclosed_dolbrace_diag.rs"]
mod issue4080_unclosed_dolbrace_diag;
#[path = "issues_3900_4199/issue4098_array_elem_assign_redirect.rs"]
mod issue4098_array_elem_assign_redirect;
#[path = "issues_3900_4199/issue4099_redirect_word_spawn_walks.rs"]
mod issue4099_redirect_word_spawn_walks;
#[path = "issues_3900_4199/issue4102_severed_dynamic_array_faces.rs"]
mod issue4102_severed_dynamic_array_faces;
#[path = "issues_3900_4199/issue4103_procsub_mtime.rs"]
mod issue4103_procsub_mtime;
#[path = "issues_3900_4199/issue4125_rematch_carrier_bytes_regressions.rs"]
mod issue4125_rematch_carrier_bytes_regressions;
#[path = "issues_3900_4199/issue4127_procsub_coproc_env_case.rs"]
mod issue4127_procsub_coproc_env_case;
#[path = "issues_3900_4199/issue4128_comsub_quoted_heredoc_glue_regressions.rs"]
mod issue4128_comsub_quoted_heredoc_glue_regressions;
#[path = "issues_3900_4199/issue4129_procsub_ambient_fd_regressions.rs"]
mod issue4129_procsub_ambient_fd_regressions;
#[path = "issues_3900_4199/issue4130_arith_assignment_subst_regressions.rs"]
mod issue4130_arith_assignment_subst_regressions;
#[path = "issues_3900_4199/issue4131_builtin_redirect_handle_retire.rs"]
mod issue4131_builtin_redirect_handle_retire;
#[path = "issues_3900_4199/issue4132_sourced_backquote_ltlt_exit_regressions.rs"]
mod issue4132_sourced_backquote_ltlt_exit_regressions;
#[path = "issues_3900_4199/issue4133_regex_subject_raw_bytes.rs"]
mod issue4133_regex_subject_raw_bytes;
#[path = "issues_3900_4199/issue4135_input_procsub_ambient_fd_regressions.rs"]
mod issue4135_input_procsub_ambient_fd_regressions;
#[path = "issues_3900_4199/issue4138_noclobber_heredoc_regressions.rs"]
mod issue4138_noclobber_heredoc_regressions;
#[path = "issues_3900_4199/issue4140_heredoc_eof_warning_per_command_regressions.rs"]
mod issue4140_heredoc_eof_warning_per_command_regressions;
#[path = "issues_3900_4199/issue4141_quoted_comsub_piped_heredoc_delimiter_regressions.rs"]
mod issue4141_quoted_comsub_piped_heredoc_delimiter_regressions;
#[path = "issues_3900_4199/issue4149_console_line_discipline.rs"]
mod issue4149_console_line_discipline;
#[path = "issues_3900_4199/issue4150_mkdir_p_drive_prefix_regressions.rs"]
mod issue4150_mkdir_p_drive_prefix_regressions;
#[path = "issues_3900_4199/issue4151_dev_console_device_files.rs"]
mod issue4151_dev_console_device_files;
#[path = "issues_3900_4199/issue4158_eval_input_redirects.rs"]
mod issue4158_eval_input_redirects;
#[path = "issues_3900_4199/issue4159_redirect_discard_next_command.rs"]
mod issue4159_redirect_discard_next_command;
#[path = "issues_3900_4199/issue4161_this_command_name_lifecycle.rs"]
mod issue4161_this_command_name_lifecycle;
#[path = "issues_3900_4199/issue4166_heredoc_eof_warning_comsub_piped.rs"]
mod issue4166_heredoc_eof_warning_comsub_piped;
#[path = "issues_3900_4199/issue4169_two_heredoc_comsubs_one_unit_regressions.rs"]
mod issue4169_two_heredoc_comsubs_one_unit_regressions;
#[path = "issues_3900_4199/issue4170_coproc_declined_launch.rs"]
mod issue4170_coproc_declined_launch;
#[path = "issues_3900_4199/issue4172_drain_pipe_invariance.rs"]
mod issue4172_drain_pipe_invariance;
#[path = "issues_3900_4199/issue4175_exec_persisted_stdout_cat.rs"]
mod issue4175_exec_persisted_stdout_cat;
#[path = "issues_3900_4199/issue4180_cat_own_redirect_target.rs"]
mod issue4180_cat_own_redirect_target;
#[path = "issues_3900_4199/issue4181_coproc_subshell_sigpipe.rs"]
mod issue4181_coproc_subshell_sigpipe;
#[path = "issues_3900_4199/issue4184_comsub_completion_coproc_retire.rs"]
mod issue4184_comsub_completion_coproc_retire;
#[path = "issues_3900_4199/issue4185_async_subshell_coproc_closeall.rs"]
mod issue4185_async_subshell_coproc_closeall;
#[path = "issues_3900_4199/issue4188_colon_lookup_funnel_regressions.rs"]
mod issue4188_colon_lookup_funnel_regressions;
#[path = "issues_3900_4199/issue4189_procsub_redirect_first_regressions.rs"]
mod issue4189_procsub_redirect_first_regressions;
#[path = "issues_3900_4199/issue4190_comsub_warning_occurrence_identity_regressions.rs"]
mod issue4190_comsub_warning_occurrence_identity_regressions;
#[path = "issues_3900_4199/issue4193_herestring_comsub_heredoc_warning_regressions.rs"]
mod issue4193_herestring_comsub_heredoc_warning_regressions;
