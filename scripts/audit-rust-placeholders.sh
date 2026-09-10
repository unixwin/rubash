#!/usr/bin/env sh
# License: MIT (full text: LICENSE at the repository root).
# Copyright 2024-2026 Rust-Shell Contributors.
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

if ! command -v cloc >/dev/null 2>&1; then
    printf '%s\n' 'audit-rust-placeholders: cloc is required' >&2
    exit 1
fi

module_index=$(mktemp)
trap 'rm -f "$module_index"' EXIT
rg -n '^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+' src --glob '*.rs' |
    tr '\\' '/' > "$module_index" || true

printf 'path\tcode_zero\tmodule_status\tdisposition\treplacement_owner\n'

cloc --by-file --csv src |
tr '\\' '/' |
awk -F, -v module_index="$module_index" '
BEGIN {
    while ((getline line < module_index) > 0) {
        file = line
        sub(/:.*/, "", file)
        declaration = line
        sub(/^[^:]*:[0-9]+:/, "", declaration)
        if (declaration !~ /(^|[[:space:]])mod[[:space:]]+[A-Za-z0-9_]+/) {
            continue
        }
        sub(/^.*mod[[:space:]]+/, "", declaration)
        sub(/[[:space:]].*$/, "", declaration)
        if (file ~ /\/mod\.rs$/) {
            sub(/\/mod\.rs$/, "", file)
        } else if (file == "src/lib.rs" || file == "src/main.rs") {
            file = "src"
        } else {
            sub(/\.rs$/, "", file)
        }
        active[file "/" declaration ".rs"] = 1
    }
    close(module_index)
}
$1 == "Rust" && $5 == 0 {
    path = $2
    gsub(/\r/, "", path)
    if (path in active) {
        module_status = "active"
    } else {
        module_status = "unreferenced"
    }
    replacement = "unclassified"
    if (path == "src/builtins/break.rs") replacement = "src/executor/pwd_loop_builtins.rs"
    else if (path == "src/builtins/builtin.rs") replacement = "src/executor/builtin_direct_command.rs"
    else if (path == "src/builtins/common.rs") replacement = "skip: shared builtin support helpers"
    else if (path == "src/builtins/evalfile.rs") replacement = "src/builtins/source/;src/executor/command_substitution.rs"
    else if (path == "src/builtins/evalstring.rs") replacement = "src/builtins/eval.rs;src/executor/command_execute.rs"
    else if (path == "src/builtins/getopt.rs") replacement = "skip: shared builtin option parsing"
    else if (path == "src/builtins/getopts.rs") replacement = "src/executor/getopts_enable.rs"
    else if (path == "src/builtins/let.rs") replacement = "src/executor/arithmetic_aliases.rs"
    else if (path == "src/builtins/mapfile.rs") replacement = "src/executor/mapfile_builtin.rs"
    else if (path == "src/builtins/read.rs") replacement = "src/executor/read_builtin.rs"
    else if (path == "src/builtins/reserved.rs") replacement = "src/parser/;src/executor/support_names.rs"
    else if (path == "src/builtins/return.rs") replacement = "src/executor/pwd_loop_builtins.rs"
    else if (path == "src/builtins/support.rs") replacement = "skip: build-time GNU support utility"
    else if (path == "src/executor/command.rs") replacement = "src/executor/command_execute.rs;src/executor/compound_exec.rs;src/executor/pipeline_exec.rs"
    else if (path == "src/executor/eval.rs") replacement = "src/builtins/eval.rs;src/executor/command_execute.rs"
    else if (path == "src/executor/hash.rs" || path == "src/executor/hashlib.rs") replacement = "src/builtins/hash.rs;src/executor/path.rs"
    else if (path == "src/jobs/jobs.rs" || path == "src/jobs/nojobs.rs") replacement = "src/jobs/table.rs;src/executor/job_builtins.rs"
    else if (path == "src/jobs/trap.rs") replacement = "src/builtins/trap.rs;src/executor/trap_exec.rs"
    else if (path == "src/jobs/signals.rs" || path == "src/jobs/siglist.rs") replacement = "host-owned: Windows process/event delivery"
    else if (path == "src/lexer/syntax.rs" || path == "src/lexer/syntax_table.rs") replacement = "src/lexer/classification.rs;src/lexer/scanner.rs"
    else if (path == "src/parser/ast.rs" || path == "src/parser/copy.rs" || path == "src/parser/dispose.rs" || path == "src/parser/make.rs" || path == "src/parser/print.rs") replacement = "src/parser/nodes.rs;src/parser/parse_loop.rs"
    else if (path == "src/parser/grammar.rs") replacement = "src/parser/mod.rs;src/parser/parse_loop.rs"
    else if (path == "src/shell/alias.rs") replacement = "src/executor/alias_*.rs"
    else if (path ~ /^src\/shell\/arrays\/(assoc|functions|indexed|indexed_extra)\.rs$/) replacement = "src/shell/variables.rs;src/executor/arrays.rs"
    else if (path == "src/shell/options.rs") replacement = "src/executor/shell_options.rs"
    else if (path == "src/shell/error.rs") replacement = "needs-owner-review: diagnostics are distributed across executor owners"
    else if (path == "src/shell/general.rs") replacement = "needs-owner-review: shell startup and host constants are distributed"
    else if (path == "src/shell/list.rs") replacement = "needs-owner-review: command-list semantics are distributed across executor owners"
    else if (path == "src/shell/mailcheck.rs") replacement = "deferred: interactive mail checking is host-owned"
    else if (path == "src/shell/quit.rs") replacement = "host-owned: process termination and interrupt delivery"
    else if (path == "src/shell/runtime.rs") replacement = "needs-owner-review: runtime state is split between ShellState and Executor"
    else if (path == "src/shell/unwind.rs") replacement = "skip: Rust Result/error propagation replaces GNU unwind-protect"
    else if (path == "src/shell/version.rs") replacement = "host-owned: package/version metadata"
    else if (path == "src/expand/arithmetic.rs") replacement = "src/executor/arithmetic/"
    else if (path == "src/expand/bracecomp.rs") replacement = "src/expand/braces.rs"
    else if (path ~ /^src\/expand\/glob\//) replacement = "src/executor/glob.rs"
    else if (path == "src/expand/pathname.rs") replacement = "src/executor/glob.rs;src/parser/pathname_pattern.rs"
    else if (path == "src/expand/tilde/shell.rs") replacement = "src/expand/tilde/tilde.rs"
    else if (path == "src/expand/word.rs") replacement = "src/executor/expand_word.rs;src/executor/parameter_*.rs"
    else if (path ~ /^src\/(complete|history|input|locale|sys)\//) {
        replacement = "deferred/host-owned"
    }
    if (path ~ /^src\/(complete|history|input|locale|sys)\//) {
        disposition = "host-owned-or-deferred"
    } else if (path ~ /^src\/jobs\/(signals|siglist)\.rs$/) {
        disposition = "host-owned-or-deferred"
    } else if (replacement == "unclassified" || replacement ~ /^needs-owner-review:/) {
        disposition = "needs-owner-review"
    } else if (replacement ~ /^(deferred|host-owned|skip):/) {
        disposition = "host-owned-or-deferred"
    } else {
        disposition = "duplicate-owner-candidate"
    }
    printf "%s\t0\t%s\t%s\t%s\n", path, module_status, disposition, replacement
}
'
