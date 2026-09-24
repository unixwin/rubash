#!/usr/bin/env bash
# check-utf8-boundary-hygiene.sh — static guard for the UTF-8 boundary bug
# class behind unixwin/niubash#92.
#
# Failure modes this guards:
#   1. Widening a payload byte into `char` (`bytes[i] as char`,
#      `*byte as char`, `let ch = bytes[i] as char`). On multibyte input that
#      Latin-1-encodes every byte into mojibake, and a widened continuation
#      byte (0x85/0xA0 inside e.g. U+60A0 or U+E0A0) can satisfy
#      `char::is_whitespace()`/`is_control()` and drive a mid-char slice.
#   2. Slicing a `&str`/`String` at a *character* counter instead of a byte
#      offset (`&raw[cursor + 1..]`), which panics on non-ASCII input.
#
# Usage:
#   scripts/check-utf8-boundary-hygiene.sh [SRC_DIR]     # default: src
#
# Exit status: 0 when clean, 1 when violations are found.
#
# Reviewed-safe `as char` sites (ASCII-only compares, Windows drive letters,
# octal/hex digit math, deliberate GNU-equivalent byte semantics) are listed
# in ALLOWLIST as `path::extended-regex` matched against the code part of
# each line (with `//` comments stripped). A new `as char` site fails this
# check until it is added here — that review step is the point of the guard.

set -u

SRC_DIR="${1:-src}"

if [[ ! -d "$SRC_DIR" ]]; then
    echo "check-utf8-boundary-hygiene: source dir '$SRC_DIR' not found" >&2
    exit 2
fi

# --- Allowlist: path::<extended regex over the line's code portion> --------
# Keep entries tight enough that a *different* `as char` in the same file
# still trips the check.
ALLOWLIST=$(cat <<'EOF'
src/builtins/cd/paths.rs::(as_bytes\(\)|bytes)\[[0-9]+\] as char
src/builtins/printf/float.rs::digits\[value as usize\] as char
src/builtins/pwd.rs::(as_bytes\(\)|bytes)\[[0-9]+\] as char
src/executor/alias_helpers.rs::for ch in &text\[cursor\.\.start\]
src/executor/alias_helpers.rs::for ch in &text\[cursor\.min\(text\.len\(\)\)\.\.\]
src/executor/arithmetic/expression.rs::is_shell_name_char\(\*ch as char\)
src/executor/arithmetic/factor.rs::(is_shell_name_(char|start)\(ch as char\)|arithmetic_digit_value\(ch as char|peek\(\)\? as char)
src/executor/arithmetic/lvalue.rs::(is_shell_name_char\(ch as char\)|peek\(\)\? as char)
src/executor/arithmetic/mod.rs::(pair\[[0-9]+\] as char|as_bytes\(\)\[\*index\] as char|\(c as char\))
src/executor/arithmetic/value.rs::is_shell_name_char\(bytes\[end\] as char\)
src/executor/arrays/storage.rs::(out\.push\(byte as char\)|b'0' \+ .*as char)
src/executor/command_substitution_pipelines.rs::as_bytes\(\)\[[0-9]+\] as char
src/executor/conditional.rs::is_shell_name_char\(bytes\[name_start - 1\] as char\)
src/executor/conditional.rs::is_shell_name_start\(bytes\[name_start\] as char\)
src/executor/declare_local.rs::is_shell_name_(start|char)\(c as char\)
src/executor/execution_misc.rs::(as_bytes\(\)\[0\] as char|out\.push\(byte as char\)|b'0' \+ .*as char)
src/executor/init.rs::bytes\[0\] as char
src/executor/parameter_core.rs::bytes\[index\] as char
src/executor/path.rs::(as_bytes\(\)|bytes)\[[0-9]+\] as char
src/executor/pipeline_exec.rs::\|byte\| byte as char
src/executor/shift_echo_builtins.rs::\+ 0x40\) as char
src/expand/braces.rs::\(byte as char\)\.to_string\(\)
src/lexer/word.rs::as_bytes\(\)\[name_start - 1\] as char
src/parser/ast_print.rs::bytes\[index\] as char
src/parser/token_actions.rs::Some\(c as char\)
src/executor/parameter_replace.rs::&value\[cursor
EOF
)

allowlisted() {
    # $1 = file path, $2 = code portion of the line
    local entry entry_file entry_re
    while IFS= read -r entry; do
        [[ -z "$entry" ]] && continue
        entry_file=${entry%%::*}
        entry_re=${entry#*::}
        if [[ "$1" == "$entry_file" ]] && printf '%s' "$2" | grep -qE "$entry_re"; then
            return 0
        fi
    done <<< "$ALLOWLIST"
    return 1
}

findings=""

# Every `as char` occurrence outside `//` comments. (Stripping at the first
# `//` is a heuristic; `as char` inside a string literal containing `//`
# would be missed — acceptable for a source guard.)
hits=$(grep -rnE 'as char' "$SRC_DIR" --include='*.rs' \
    | sed 's://.*::' \
    | grep -E 'as char' || true)

# --- Check 1: every `as char` site must be allowlisted ---------------------
while IFS= read -r hit; do
    [[ -z "$hit" ]] && continue
    file=${hit%%:*}
    rest=${hit#*:}
    lineno=${rest%%:*}
    code=$(printf '%s' "${rest#*:}" | sed 's/^ *//')
    if ! allowlisted "$file" "$code"; then
        findings="$findings
$file:$lineno: unreviewed byte->char widening: $code"
    fi
done <<< "$hits"

# --- Check 2: widened byte reaching a non-ASCII-capable char predicate -----
# `char::is_whitespace`/`is_control`/`is_alphabetic`/... return true for
# code points reachable by Latin-1-widening a UTF-8 continuation byte — the
# exact mechanism of the lexer/mod.rs heredoc panic. Unconditional: no
# allowlist entry exempts it.
predicate_re='is_(whitespace|control|alphabetic|numeric|alphanumeric|uppercase|lowercase)'

# 2a. Same line: `(byte_expr as char).is_whitespace()` etc.
while IFS= read -r hit; do
    [[ -z "$hit" ]] && continue
    findings="$findings
$hit (widened byte passed to non-ASCII-capable predicate)"
done < <(printf '%s\n' "$hits" | grep -E "as char\)?[[:space:]]*\.${predicate_re}\(" || true)

# 2b. Cross-line: `let V = ... as char` followed later by `V.is_whitespace()`
#     — the lexer panic had this shape (`let ch = bytes[i] as char`).
#     Rebinding (`let V = ...` without `as char`, a `|V|` closure parameter,
#     or `for V in`) clears the taint so shadowed real-char bindings do not
#     false-positive.
check2b=$(while IFS= read -r file; do
    [[ -z "$file" ]] && continue
    sed 's://.*::' "$file" | awk -v file="$file" -v pre="$predicate_re" '
        function bind_name(seg) {
            sub(/^let[[:space:]]+/, "", seg)
            sub(/[[:space:]]*=.*/, "", seg)
            return seg
        }
        {
            line = $0
            # A `let V =` rebinds V; clear any earlier byte-widened taint.
            if (match(line, /let[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=/)) {
                delete bound[bind_name(substr(line, RSTART, RLENGTH))]
            }
            # `let V = ... as char` taints V as a widened byte.
            if (match(line, /let[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=[^;]*as[[:space:]]+char/)) {
                bound[bind_name(substr(line, RSTART, RLENGTH))] = 1
            }
            # Closure param `|V|` and `for V in` rebind V to a real char.
            if (match(line, /\|[A-Za-z_][A-Za-z0-9_]*\|/)) {
                seg = substr(line, RSTART + 1, RLENGTH - 2)
                delete bound[seg]
            }
            if (match(line, /for[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]+in/)) {
                seg = substr(line, RSTART, RLENGTH)
                sub(/^for[[:space:]]+/, "", seg)
                sub(/[[:space:]]+in.*/, "", seg)
                delete bound[seg]
            }
            re = "\\.(" pre ")\\("
            if (line ~ re) {
                for (v in bound) {
                    if (index(line, v ".") > 0) {
                        printf "%s:%d: widened byte var %s reaches predicate: %s\n", file, NR, v, line
                    }
                }
            }
        }
    '
done < <(printf '%s\n' "$hits" | cut -d: -f1 | sort -u))
[[ -n "$check2b" ]] && findings="$findings
$check2b"

# --- Check 3: &str sliced at a char counter --------------------------------
# `&s[cursor + 1..]` where `cursor` counts characters was the original
# niubash#92 panic. Flag `&ident[name..]`/`&ident[name + K..]` for index
# names that conventionally denote char positions; byte-offset uses must be
# allowlisted.
while IFS= read -r hit; do
    [[ -z "$hit" ]] && continue
    file=${hit%%:*}
    rest=${hit#*:}
    lineno=${rest%%:*}
    code=$(printf '%s' "${rest#*:}" | sed 's/^ *//')
    if ! allowlisted "$file" "$code"; then
        findings="$findings
$file:$lineno: char-counter string slice: $code"
    fi
done < <(grep -rnE '&[A-Za-z_][A-Za-z0-9_]*\[\s*(cursor|char_index|char_pos|chars_i|char_idx)([^A-Za-z0-9_]|$)' "$SRC_DIR" --include='*.rs' \
    | sed 's://.*::' \
    | grep -E '&[A-Za-z_][A-Za-z0-9_]*\[' || true)

# --- Tally -----------------------------------------------------------------
count=$(printf '%s\n' "$findings" | grep -c . || true)
echo "check-utf8-boundary-hygiene: scanned $SRC_DIR"
if [[ "$count" -gt 0 ]]; then
    printf '%s\n' "$findings" | sed '/^$/d'
    echo
    echo "check-utf8-boundary-hygiene: $count violation(s) — review the sites above;"
    echo "fix the indexing/widening, or add an allowlist entry only after confirming the"
    echo "byte involved can never be a UTF-8 continuation byte (>= 0x80)."
    exit 1
fi
echo "check-utf8-boundary-hygiene: clean"
exit 0
