mod common;

#[path = "issues_2000_2299/issue2006_2007_history_faces.rs"]
mod issue2006_2007_history_faces;
#[path = "issues_2000_2299/issue2020_pipeline_signal_mailbox.rs"]
mod issue2020_pipeline_signal_mailbox;
#[path = "issues_2000_2299/issue2023_interactive_source_busyspin_regressions.rs"]
mod issue2023_interactive_source_busyspin_regressions;
#[path = "issues_2000_2299/issue2024_quoted_procsub_bind_regressions.rs"]
mod issue2024_quoted_procsub_bind_regressions;
#[path = "issues_2000_2299/issue2025_heredoc_header_close_eof_gather_regressions.rs"]
mod issue2025_heredoc_header_close_eof_gather_regressions;
#[path = "issues_2000_2299/issue2026_quoted_comsub_heredoc_header_close_regressions.rs"]
mod issue2026_quoted_comsub_heredoc_header_close_regressions;
#[path = "issues_2000_2299/issue2027_cat_diagnostic_operand_quoted_regressions.rs"]
mod issue2027_cat_diagnostic_operand_quoted_regressions;
#[path = "issues_2000_2299/issue2029_fd_offset_fold_regressions.rs"]
mod issue2029_fd_offset_fold_regressions;
#[path = "issues_2000_2299/issue2030_compound_dup_bad_fd_regressions.rs"]
mod issue2030_compound_dup_bad_fd_regressions;
#[path = "issues_2000_2299/issue2031_comsub_function_redirect_regressions.rs"]
mod issue2031_comsub_function_redirect_regressions;
#[path = "issues_2000_2299/issue2032_procsub_positional_params_regressions.rs"]
mod issue2032_procsub_positional_params_regressions;
#[path = "issues_2000_2299/issue2033_alias_comsub_comment_park_regressions.rs"]
mod issue2033_alias_comsub_comment_park_regressions;
#[path = "issues_2000_2299/issue2036_precedence_introspection_regressions.rs"]
mod issue2036_precedence_introspection_regressions;
#[path = "issues_2000_2299/issue2036_s2_introspection_windows.rs"]
mod issue2036_s2_introspection_windows;
#[path = "issues_2000_2299/issue2036_s3_posix_source_special.rs"]
mod issue2036_s3_posix_source_special;
#[path = "issues_2000_2299/issue2036_s4_projection_regressions.rs"]
mod issue2036_s4_projection_regressions;
#[path = "issues_2000_2299/issue2039_close_final_state_regressions.rs"]
mod issue2039_close_final_state_regressions;
#[path = "issues_2000_2299/issue2040_optionsview_flagonly_regressions.rs"]
mod issue2040_optionsview_flagonly_regressions;
#[path = "issues_2000_2299/issue2042_filefd_test_r_regressions.rs"]
mod issue2042_filefd_test_r_regressions;
#[path = "issues_2000_2299/issue2046_s6_four_cells.rs"]
mod issue2046_s6_four_cells;
#[path = "issues_2000_2299/issue2053_coproc_wait_race_regressions.rs"]
mod issue2053_coproc_wait_race_regressions;
#[path = "issues_2000_2299/issue2057_coproc_exec_close_race_regressions.rs"]
mod issue2057_coproc_exec_close_race_regressions;
#[path = "issues_2000_2299/issue2058_respawn_transport_regressions.rs"]
mod issue2058_respawn_transport_regressions;
#[path = "issues_2000_2299/issue2062_perf_scaling_regressions.rs"]
mod issue2062_perf_scaling_regressions;
#[path = "issues_2000_2299/issue2064_undecodable_env_regressions.rs"]
mod issue2064_undecodable_env_regressions;
#[path = "issues_2000_2299/issue2065_sed_escape_decline_regressions.rs"]
mod issue2065_sed_escape_decline_regressions;
#[path = "issues_2000_2299/issue2066_pipeline_host_flag_regressions.rs"]
mod issue2066_pipeline_host_flag_regressions;
#[path = "issues_2000_2299/issue2071_test_empty_path_regressions.rs"]
mod issue2071_test_empty_path_regressions;
#[path = "issues_2000_2299/issue2072_respawn_history_transport_regressions.rs"]
mod issue2072_respawn_history_transport_regressions;
#[path = "issues_2000_2299/issue2073_test_v_env_fallback_regressions.rs"]
mod issue2073_test_v_env_fallback_regressions;
#[path = "issues_2000_2299/issue2078_session_parse_cache_regressions.rs"]
mod issue2078_session_parse_cache_regressions;
#[path = "issues_2000_2299/issue2081_posix_pipeline_special_regressions.rs"]
mod issue2081_posix_pipeline_special_regressions;
#[path = "issues_2000_2299/issue2082_posix_special_redirect_failure_regressions.rs"]
mod issue2082_posix_special_redirect_failure_regressions;
#[path = "issues_2000_2299/issue2086_comsub_child_stderr_regressions.rs"]
mod issue2086_comsub_child_stderr_regressions;
#[path = "issues_2000_2299/issue2087_loop_head_missing_do_regressions.rs"]
mod issue2087_loop_head_missing_do_regressions;
#[path = "issues_2000_2299/issue2090_exec_quoted_procsub_regressions.rs"]
mod issue2090_exec_quoted_procsub_regressions;
#[path = "issues_2000_2299/issue2091_devtcp_residual_regressions.rs"]
mod issue2091_devtcp_residual_regressions;
#[path = "issues_2000_2299/issue2093_literal_file_redirect_regressions.rs"]
mod issue2093_literal_file_redirect_regressions;
#[path = "issues_2000_2299/issue2094_comsub_pipeline_drain_regressions.rs"]
mod issue2094_comsub_pipeline_drain_regressions;
#[path = "issues_2000_2299/issue2096_dollar_dparen_subshell_or_list_regressions.rs"]
mod issue2096_dollar_dparen_subshell_or_list_regressions;
#[path = "issues_2000_2299/issue2098_devtcp_loser_open_regressions.rs"]
mod issue2098_devtcp_loser_open_regressions;
#[path = "issues_2000_2299/issue2101_directory_stdin_regressions.rs"]
mod issue2101_directory_stdin_regressions;
#[path = "issues_2000_2299/issue2102_devtcp_closed_peer_write_regressions.rs"]
mod issue2102_devtcp_closed_peer_write_regressions;
#[path = "issues_2000_2299/issue2103_pipeline_stage_stdin_binding_regressions.rs"]
mod issue2103_pipeline_stage_stdin_binding_regressions;
#[path = "issues_2000_2299/issue2104_identity_script_yield_regressions.rs"]
mod issue2104_identity_script_yield_regressions;
#[path = "issues_2000_2299/issue2105_touch_directory_operand_regressions.rs"]
mod issue2105_touch_directory_operand_regressions;
#[path = "issues_2000_2299/issue2106_compound_heredoc_output_redirect_regressions.rs"]
mod issue2106_compound_heredoc_output_redirect_regressions;
#[path = "issues_2000_2299/issue2108_loop_word_list_operator_regressions.rs"]
mod issue2108_loop_word_list_operator_regressions;
#[path = "issues_2000_2299/issue2111_empty_payload_stage_stdin_regressions.rs"]
mod issue2111_empty_payload_stage_stdin_regressions;
#[path = "issues_2000_2299/issue2117_posix_special_usage_error_regressions.rs"]
mod issue2117_posix_special_usage_error_regressions;
#[path = "issues_2000_2299/issue2121_devtcp_output_loser_and_merged_regressions.rs"]
mod issue2121_devtcp_output_loser_and_merged_regressions;
#[path = "issues_2000_2299/issue2124_devtcp_winner_bind_regressions.rs"]
mod issue2124_devtcp_winner_bind_regressions;
#[path = "issues_2000_2299/issue2125_posix_fdvar_redirect_fatality_regressions.rs"]
mod issue2125_posix_fdvar_redirect_fatality_regressions;
#[path = "issues_2000_2299/issue2126_posix_evalstring_syntax_fatality_regressions.rs"]
mod issue2126_posix_evalstring_syntax_fatality_regressions;
#[path = "issues_2000_2299/issue2127_flow_builtin_error_regressions.rs"]
mod issue2127_flow_builtin_error_regressions;
#[path = "issues_2000_2299/issue2128_flow_builtin_help_regressions.rs"]
mod issue2128_flow_builtin_help_regressions;
#[path = "issues_2000_2299/issue2129_startup_carrier_recursion.rs"]
mod issue2129_startup_carrier_recursion;
#[path = "issues_2000_2299/issue2131_fd_alias_own_redirect_regressions.rs"]
mod issue2131_fd_alias_own_redirect_regressions;
#[path = "issues_2000_2299/issue2132_inherited_stdio_classification_regressions.rs"]
mod issue2132_inherited_stdio_classification_regressions;
#[path = "issues_2000_2299/issue2134_async_child_forgery.rs"]
mod issue2134_async_child_forgery;
#[path = "issues_2000_2299/issue2135_function_stdin_import.rs"]
mod issue2135_function_stdin_import;
#[path = "issues_2000_2299/issue2137_execution_scope_markers.rs"]
mod issue2137_execution_scope_markers;
#[path = "issues_2000_2299/issue2140_login_exit_bash_logout.rs"]
mod issue2140_login_exit_bash_logout;
#[path = "issues_2000_2299/issue2142_caller_top_frame_source_regressions.rs"]
mod issue2142_caller_top_frame_source_regressions;
#[path = "issues_2000_2299/issue2143_logical_pwd_validation_regressions.rs"]
mod issue2143_logical_pwd_validation_regressions;
#[path = "issues_2000_2299/issue2170_mirror_governance_over_file_regressions.rs"]
mod issue2170_mirror_governance_over_file_regressions;
#[path = "issues_2000_2299/issue2171_proc_fd_operand_survival_regressions.rs"]
mod issue2171_proc_fd_operand_survival_regressions;
#[path = "issues_2000_2299/issue2174_printf_flush_ordering_regressions.rs"]
mod issue2174_printf_flush_ordering_regressions;
#[path = "issues_2000_2299/issue2179_procsub_exit_feed_regressions.rs"]
mod issue2179_procsub_exit_feed_regressions;
#[path = "issues_2000_2299/issue2184_pipeline_stage_procsub_regressions.rs"]
mod issue2184_pipeline_stage_procsub_regressions;
#[path = "issues_2000_2299/issue2195_procsub_spawn_fd_state_regressions.rs"]
mod issue2195_procsub_spawn_fd_state_regressions;
#[path = "issues_2000_2299/issue2196_numbered_fd_procsub_regressions.rs"]
mod issue2196_numbered_fd_procsub_regressions;
#[path = "issues_2000_2299/issue2200_coproc_numbered_fd_regressions.rs"]
mod issue2200_coproc_numbered_fd_regressions;
#[path = "issues_2000_2299/issue2201_srandom_unseedable_regressions.rs"]
mod issue2201_srandom_unseedable_regressions;
#[path = "issues_2000_2299/issue2202_lineno_span_transport_regressions.rs"]
mod issue2202_lineno_span_transport_regressions;
#[path = "issues_2000_2299/issue2203_suspend_single_cont_regressions.rs"]
mod issue2203_suspend_single_cont_regressions;
#[path = "issues_2000_2299/issue2210_arith_dquote_rescan_regressions.rs"]
mod issue2210_arith_dquote_rescan_regressions;
#[path = "issues_2000_2299/issue2211_comsub_cd_stale_cwd_regressions.rs"]
mod issue2211_comsub_cd_stale_cwd_regressions;
#[path = "issues_2000_2299/issue2212_bats_remainder_regressions.rs"]
mod issue2212_bats_remainder_regressions;
#[path = "issues_2000_2299/issue2212_bats_setup_teardown_chain.rs"]
mod issue2212_bats_setup_teardown_chain;
#[path = "issues_2000_2299/issue2214_job_start_notice_regressions.rs"]
mod issue2214_job_start_notice_regressions;
#[path = "issues_2000_2299/issue2217_dollar_bracket_arith_regressions.rs"]
mod issue2217_dollar_bracket_arith_regressions;
#[path = "issues_2000_2299/issue2219_comsub_cd_empty_operand_regressions.rs"]
mod issue2219_comsub_cd_empty_operand_regressions;
#[path = "issues_2000_2299/issue2220_comsub_pwd_logical_rendering_regressions.rs"]
mod issue2220_comsub_pwd_logical_rendering_regressions;
#[path = "issues_2000_2299/issue2221_cdable_vars_diag_regressions.rs"]
mod issue2221_cdable_vars_diag_regressions;
#[path = "issues_2000_2299/issue2227_bracket_quote_regressions.rs"]
mod issue2227_bracket_quote_regressions;
#[path = "issues_2000_2299/issue2227_dollar_bracket_extraction_regressions.rs"]
mod issue2227_dollar_bracket_extraction_regressions;
#[path = "issues_2000_2299/issue2227_verdict_names_dquote_segment_regressions.rs"]
mod issue2227_verdict_names_dquote_segment_regressions;
#[path = "issues_2000_2299/issue2234_chained_output_procsub_regressions.rs"]
mod issue2234_chained_output_procsub_regressions;
#[path = "issues_2000_2299/issue2235_list_input_procsub_regressions.rs"]
mod issue2235_list_input_procsub_regressions;
#[path = "issues_2000_2299/issue2236_nested_signal_death_regressions.rs"]
mod issue2236_nested_signal_death_regressions;
#[path = "issues_2000_2299/issue2238_external_stdin_drain_regressions.rs"]
mod issue2238_external_stdin_drain_regressions;
#[path = "issues_2000_2299/issue2239_function_heredoc_redirected_frame_regressions.rs"]
mod issue2239_function_heredoc_redirected_frame_regressions;
#[path = "issues_2000_2299/issue2240_function_input_redirect_cwd_regressions.rs"]
mod issue2240_function_input_redirect_cwd_regressions;
#[path = "issues_2000_2299/issue2241_job_group_signal_regressions.rs"]
mod issue2241_job_group_signal_regressions;
#[path = "issues_2000_2299/issue2244_compound_pipeline_deadlock_regressions.rs"]
mod issue2244_compound_pipeline_deadlock_regressions;
#[path = "issues_2000_2299/issue2249_function_readwrite_fd0_regressions.rs"]
mod issue2249_function_readwrite_fd0_regressions;
#[path = "issues_2000_2299/issue2255_pipeline_stage_redirects.rs"]
mod issue2255_pipeline_stage_redirects;
#[path = "issues_2000_2299/issue2256_devfd_fresh_open_regressions.rs"]
mod issue2256_devfd_fresh_open_regressions;
#[path = "issues_2000_2299/issue2261_child_script_readwrite_fd0_regressions.rs"]
mod issue2261_child_script_readwrite_fd0_regressions;
#[path = "issues_2000_2299/issue2265_numbered_fd_dash_heredoc_regressions.rs"]
mod issue2265_numbered_fd_dash_heredoc_regressions;
#[path = "issues_2000_2299/issue2270_external_numbered_fd_heredoc_regressions.rs"]
mod issue2270_external_numbered_fd_heredoc_regressions;
#[path = "issues_2000_2299/issue2274_multiline_array_completeness_regressions.rs"]
mod issue2274_multiline_array_completeness_regressions;
#[path = "issues_2000_2299/issue2276_socket_spelling_enxio.rs"]
mod issue2276_socket_spelling_enxio;
#[path = "issues_2000_2299/issue2278_numbered_stdio_dup_regressions.rs"]
mod issue2278_numbered_stdio_dup_regressions;
#[path = "issues_2000_2299/issue2279_enoexec_fd_scoping_regressions.rs"]
mod issue2279_enoexec_fd_scoping_regressions;
#[path = "issues_2000_2299/issue2280_inprocess_pending_closes.rs"]
mod issue2280_inprocess_pending_closes;
#[path = "issues_2000_2299/issue2285_external_alias_output_regressions.rs"]
mod issue2285_external_alias_output_regressions;
#[path = "issues_2000_2299/issue2287_pipeline_pgrp_regressions.rs"]
mod issue2287_pipeline_pgrp_regressions;
#[path = "issues_2000_2299/issue2292_ambient_text_spawn_delivery_regressions.rs"]
mod issue2292_ambient_text_spawn_delivery_regressions;
#[path = "issues_2000_2299/issue2293_stage_stdio_dup_regressions.rs"]
mod issue2293_stage_stdio_dup_regressions;
#[path = "issues_2000_2299/issue2294_stage_stderr_redirect_regressions.rs"]
mod issue2294_stage_stderr_redirect_regressions;
#[path = "issues_2000_2299/issue2295_stage_alias_output_regressions.rs"]
mod issue2295_stage_alias_output_regressions;
#[path = "issues_2000_2299/issue2298_numbered_readwrite_spawn_regressions.rs"]
mod issue2298_numbered_readwrite_spawn_regressions;
