> **ARCHIVED** — This document is historical. The typed-expansion migration is
> complete. Last current: 2026-08-27.

# Typed Expansion Migration Checkpoint

Updated: 2026-08-24
Branch: agentteams/typed-provenance

## Purpose

This is the durable recursive checkpoint for GNU Bash substitution and expansion work. The primary GNU owners are third_party/bash/subst.c (read_comsub, command_substitute, param_expand, expand_word_internal, shell_expand_word_list) and third_party/bash/redir.c (here-string, heredoc, input redirection, dynamic fd ownership).

The migration chain is:

lexer/parser -> raw word metadata -> parameter scanner -> typed substitution carrier -> quote/splitting context -> assignment/path/redirect boundary -> command execution -> status/trap/state restoration -> materialization

A fix is incomplete when it only changes the final assertion and leaves the upstream carrier or semantic owner ambiguous.

## Typed Carriers

Current carriers are SubstitutionOutput { bytes, status, context }, ExpandedFragment { bytes, quoted, splittable }, and ExpandedWord { fragments, status }. SubstitutionOutput readback removes NUL and trailing LF according to command-substitution rules, while preserving other bytes until an explicit text boundary.

## Completed Pushed Commits

- ec94603d typed mutable word expansion boundary
- 278652ed here-string mutable expansion
- 082c96d2 independent mutable heredoc boundary
- 8f432190 loop-fd heredoc owner
- 1eea003e read heredoc owner
- 2c5f9a8e mapfile heredoc owner
- ad13029d external fd heredoc owner
- dff6879d external stdin mutable wrapper
- d267f422 pipeline and trap stdin owners
- bf33127f function stdin owner
- d91b972c read timeout stdin owner
- 0a357ffd external file and read stdin owners
- e339e716 external sed stdin owner
- cd00e4a7 typed heredoc substitution boundary
- bb4f4639 direct mutable heredoc shortcut input
- 6d2f3602 Windows extended path normalization for cd ..; pwd
- b91a477b structured current-shell substitution span scanner
- d1c6fb07 current-shell precedence before ordinary braced parameters
- dfae1f01 current-shell forms bypass parameter-error preflight

## Current Owner Graph

Migrated mutable owners: command_input_scope, compound_exec loop heredocs, read_io, mapfile_helpers, external_setup, external_inner, external_file_builtins, function_calls, pipeline_exec, trap_exec, and embedded_mutations typed command substitution.

Intentional legacy boundaries: shell_options stdin_string_for_command(&self) for immutable utility and command-substitution callers; command_substitution_heredoc_output(&self) for the immutable path; decode_command_substitution_payload in assignment, heredoc legacy, backtick, and command-preparation boundaries. These cannot be removed by replacing them with lossy UTF-8 conversion. command_prepare currently materializes variable_expanded.words through this adapter, while expand_simple_substitution_fragments already uses typed fragments and is the next migration owner.

## Focused Evidence

Repeated passing gates:

- cargo check
- cargo test --test executor_tests command_chaining::part_080: 156 passed
- cargo test --test heredoc_regressions: 2 passed
- cargo test --test cli_tests read: 30 passed
- cargo test --test cli_tests mapfile: 3 passed
- command_chaining part_005 heredoc tests: passing
- command_chaining::part_080: 156 passed / 0 failed after command-word materialization boundary
- arithmetic fatality contracts: 2 passed / 0 failed
- printf focused slice: 9 passed / 0 failed
- typed substitution metadata: 14 passed / 0 failed
- heredoc_regressions: 2 passed / 0 failed, including FIFO ownership and raw C0 payload
- command_substitution heredoc pipeline/sequential focused tests: passed; mutable callers cover production paths, shell_options immutable fallback remains the sole legacy owner
- payload decoder contract tests: 3 passed / 0 failed, covering raw C0, escaped prefix, and malformed literals
- assignment mixed substitution audit: ExpandedWord owns prefix/substitution/suffix ordering and materializes only at the assignment String boundary; remaining decoder is non-mixed fallback
- assignment typed differential coverage: unquoted backtick, mixed $()+backtick, and quoted mixed context all pass; compat_issue_regressions is 84 passed / 0 failed after these additions
- command preparation audit: expansion and field splitting precede pathname expansion; final command-word materialization restores markers and decodes payloads only after glob matching; assignment C0/status/quoted-space focused probes all pass

Known unrelated command-substitution slice failure: bashdb_info_files_reports_source_files_without_command_substitution_error, a fixed bashdb path assertion.

## Resolved Current-Shell Chain

The two previously persistent failures are now resolved. The root causes were separate: parameter_core routed whole-word current-shell forms after ordinary braced parameters, and parameter_errors rejected whitespace-led current-shell forms before execution. The structured span scanner, precedence change, and preflight exemption now cover the stdout/side-effect form and nested reply expansion.

Focused evidence: command_chaining::part_005 is 41 passed / 0 failed after dfae1f01.

Additional byte and printf evidence (2026-08-24): substitution_metadata tests are 14 passed / 0 failed, including NUL, trailing LF, invalid UTF-8, C0-like bytes, quote provenance, status, and IFS splitting. command_chaining::part_003::test_printf is 9 passed / 0 failed, covering printf -v, %n, array targets, arithmetic subscripts, negative subscripts, and POSIX time formats.

Arithmetic evidence and fix (2026-08-24): command_chaining::part_071 initially had 13 passed / 2 failed because InvalidLiteral was classified nonfatal and empty-word command execution returned before arithmetic status handling. arithmetic_expansion_is_fatal now includes InvalidLiteral, and command_execute preserves pending arithmetic errors before empty-word early return. The slice is now 15 passed / 0 failed.

The failed assignment-priority experiment and command-substitution dispatch reorder remain documented as reverted experiments; they are not needed after fixing the preflight owner.

## Recursive Next Steps

1. Add scanner tests for current-shell braced forms, nested braces, quotes, escaped braces, and ${| command } reply mode.
2. Replace the boolean word_contains_current_shell_command_substitution detector with a structured span/source result.
3. Route that span through expand_command_substitution_mut_typed_with_context, preserving side effects, REPLY, status, positional parameters, and environment restoration.
4. Then migrate assignment and command-preparation marker decoding to typed fragments.
5. Add differential tests for raw C0, NUL, repeated LF, invalid UTF-8, arithmetic status 1 versus 127, printf bytes, arrays, eval, heredocs, traps, and redirects.
6. Update docs/semantic-ownership.tsv only after focused evidence and validate with scripts/validate-semantic-map.sh.

Immutable backtick audit (2026-08-24): expand_backtick_substitution_typed is currently &mut self and returns SubstitutionOutput, while expand_word's fallback is &self and returns String. Keep the dual API until an interior-state typed capture/readback carrier exists; direct substitution would lose status, quote context, and raw payload provenance. The new expand_command_substitution_readback_with_context(&self, ...) carrier now reuses the immutable execution path, captures last-command-substitution status, and preserves quote context before the legacy String boundary. part_005 remains 41 passed / 0 failed after routing immutable backticks through it.

## Recursion Rules

Read this checkpoint before changing the next owner. Do not modify third_party/bash. Do not run unbounded full suites. Keep artifacts under target/issue-suites/results. Run git diff --check, cargo check, and the smallest relevant focused tests before each commit. Check for residual rubash.exe, bash.exe, cargo.exe, and suite runners before closing a testing turn. Never claim completion while a persistent failure or intentional legacy boundary remains.
