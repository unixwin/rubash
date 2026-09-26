//! `subst.c expand_subscript_string` for associative-array subscripts.
//!
//! GNU reaches it from two places and both must produce the same key:
//!
//!   * the assignment path -- `assign_array_element_internal`
//!     (`arrayfunc.c:392-398`) after `general.c:480 assignment()` split the
//!     SYNTACTIC word into name / subscript / value;
//!   * the arithmetic path -- `array_variable_part` -> `array_value_internal`
//!     (`arrayfunc.c:1483`) reached from `expr_streval` (`expr.c:1150`).
//!
//! The subscript is word-expanded exactly once, with `W_NOSPLIT2 |
//! W_NOPROCSUB`: parameter, command, arithmetic and tilde expansion plus
//! quote removal, and no field splitting, no pathname expansion and no
//! process substitution. The result is the key verbatim and is never
//! re-expanded.

use super::*;
use crate::executor::markers::{DATA_DOLLAR, STORAGE_WORD_PREFIX};

/// GNU `array_expand_once` (arrayfunc.c:50) / `SET_VFLAGS`
/// (builtins/common.h:277-289) provenance for a subscript reaching an
/// array consumer.
///
/// GNU tags the word's subscript with `VA_NOEXPAND` / `ASS_NOEXPAND` (and
/// `VA_ONEWORD` for `W_ARRAYREF`) when the option is set, so the text that
/// arrives at `array_expand_index` / `expand_subscript_string` is already
/// the once-expanded data and must be consumed verbatim. When the option
/// is unset the consumer performs its own `expand_subscript_string` pass —
/// the "second expansion" GNU applies to builtin operands.
#[derive(Clone, Copy, Debug)]
pub(in crate::executor) enum SubscriptSource<'a> {
    /// Text that was never word-expanded — the body of a `${a[...]}`
    /// parameter expansion or raw subscript text inside arithmetic.
    /// It receives its single `expand_subscript_string` expansion here,
    /// matching GNU's assign_array_element_internal / array_value_internal
    /// callers (arrayfunc.c:392-425, 1483, 1560-1601).
    Raw(&'a str),
    /// Text that already went through word expansion once — a builtin argv
    /// operand (`declare`/`unset`/`printf -v`/`read`/`test -v`), a `[[ ]]`
    /// operand, or a stored compound-assignment element subscript. With
    /// `array_expand_once` it is the final data (`VA_NOEXPAND` /
    /// `ASS_NOEXPAND`); without it the consumer performs the second
    /// `expand_subscript_string` pass GNU performs.
    ExpandedOnce(&'a str),
    /// Verbatim in both modes — a subscript word GNU protected with
    /// `Q_ARITH` during word expansion, or an already-decoded opaque key.
    Protected(&'a str),
}

/// How a builtin-operand subscript resolves: mirrors which GNU flag
/// (ASS_NOEXPAND / AV_NOEXPAND, or none) the consumer's caller attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::executor) enum OperandSubscriptMode {
    /// Option-gated: verbatim with array_expand_once, a deferred
    /// expand_subscript_string pass without it (SET_VFLAGS /
    /// assoc_noexpand paths).
    ExpandedOnce,
    /// Verbatim in both modes (protected expansion products).
    Verbatim,
    /// Re-expanded unconditionally — GNU consumers that call
    /// assign_array_element / array_expand_index without NOEXPAND flags
    /// (e.g. a quoted `declare "a[$x]=v"` operand, which never carried
    /// W_ASSIGNMENT).
    AlwaysExpand,
}

/// Result of resolving and evaluating an indexed-array subscript
/// (arrayfunc.c:1353-1391 `array_expand_index` -> `evalexp`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::executor) enum IndexedSubscript {
    /// Valid index (possibly negative; the caller applies GNU's
    /// max-index-relative adjustment).
    Index(i128),
    /// The resolved text was empty. GNU's behavior is consumer-specific:
    /// `a[]=v` prints `a[]: bad array subscript` (rc 1) while
    /// `unset 'a[]'` is a silent no-op (rc 0), so the caller decides.
    Empty,
    /// Evaluation failed; the "operand expected" diagnostic
    /// (expr.c `evalerror` via `lasttp`) was already printed.
    Error,
}

/// Legacy single-byte data markers shared with the lexer and the
/// embedded-parameter walker (see `executor/parameter_errors.rs`).
const LITERAL_BACKSLASH: char = crate::executor::markers::DATA_BACKSLASH;
const LITERAL_SINGLE_QUOTE: char = crate::executor::markers::DATA_SQUOTE;
const LITERAL_DOUBLE_QUOTE: char = crate::executor::markers::DATA_DQUOTE;
const LITERAL_BACKTICK: char = crate::executor::markers::DATA_BACKTICK;
const LITERAL_DOLLAR: char = crate::executor::markers::DATA_DOLLAR;

impl Executor {
    /// One `expand_subscript_string` pass over the raw subscript text.
    pub(in crate::executor) fn expand_subscript_string(&self, raw: &str) -> String {
        // Nothing expands inside a single-quoted span, so a subscript covered
        // entirely by single quotes is literal data: `A['$v']` keys on `$v`
        // and `A['a\b']` keeps the backslash.
        if let Some(literal) = wholly_single_quoted_literal(raw) {
            return literal;
        }
        // GNU subst.c:11063 expand_subscript_string -> expand_string ->
        // expand_word_internal: the `$'...'` arm ANSI-C decodes the span
        // (ansicstr, lib/sh/strtrans.c:130). Raw subscript text that bypassed
        // the lexer still carries `$'...'` source form; decode it to the
        // carrier-marked data form the embedded walker already consumes.
        let raw_owned;
        let raw = if raw.contains("$'") {
            raw_owned = decode_ansi_c_spans(raw);
            raw_owned.as_str()
        } else {
            raw
        };
        // Quote removal belongs to the LEXER token -- `\X` loses its backslash
        // there -- while the parameter/command/arithmetic walker below only
        // reads data. Resolve the escapes first so the walker never mistakes a
        // quoted `$`, `` ` `` or quote for an expansion (GNU subst.c
        // `expand_word_internal` sees the same already-dequoted characters).
        let masked = mask_subscript_escapes(raw);
        let expanded = self.expand_embedded_parameters(&masked);
        // Expansion-produced whitespace rides the \x1c/E309 data tags so the
        // field/compound splitter leaves it alone; a resolved subscript is
        // cooked text only (GNU expand_subscript_string -> expand_string
        // yields plain bytes), so the tags come off here — otherwise a key
        // like `20 40 80` stores marker-laden bytes that never match the
        // canonical form reads and the kvlist path produce.
        let expanded = expanded
            .replace(crate::executor::COMPOUND_EXPANSION_WS_TAG, "")
            .replace(crate::executor::markers::IFS_GLUE, "");
        // A leading unquoted `~` tilde-expands; `x~` and `a:~` stay literal
        // and `"~"` never reaches here (its first character is the quote).
        if raw.starts_with('~') {
            return tilde_expand::expand_word_prefix(&expanded, &self.shell_state.env_vars)
                .unwrap_or(expanded);
        }
        expanded
    }

    /// Retroactive `array_expand_once` application (arrayfunc.c:50,
    /// builtins/common.h:277-289 `SET_VFLAGS`): resolve the subscript text
    /// to the form that reaches `array_expand_index` (indexed arrays) or
    /// the associative key store. The result is OPAQUE once-expanded data
    /// — callers must never run it through `expand_subscript_string` /
    /// `expand_embedded_parameters` again, which is what kept executing
    /// `$(...)` text a second time (audit C11).
    pub(in crate::executor) fn resolve_array_subscript(
        &self,
        source: SubscriptSource<'_>,
    ) -> String {
        match source {
            SubscriptSource::Protected(text) => text.to_string(),
            SubscriptSource::Raw(raw) => {
                // expand_subscript_string runs expansion side effects
                // (`$((i++))`, `$(...)`): dedup repeated resolves of the
                // same `${}` fragment across the validate/pre-scan/real
                // passes (SUB_RES_XPASS docs).
                let Some(key) = crate::executor::expand_braced_indices::sub_site_key(raw) else {
                    return self.expand_subscript_string(raw);
                };
                if let Some(hit) = crate::executor::expand_braced_indices::sub_res_lookup(&key) {
                    return hit;
                }
                let resolved = self.expand_subscript_string(raw);
                crate::executor::expand_braced_indices::sub_res_store(key, resolved.clone());
                resolved
            }
            SubscriptSource::ExpandedOnce(text) => {
                if crate::builtins::shopt::option_enabled(
                    &self.shell_state.env_vars,
                    "array_expand_once",
                ) {
                    // VA_NOEXPAND / ASS_NOEXPAND: the first expansion was the
                    // word expansion; the consumer uses the text verbatim.
                    text.to_string()
                } else {
                    // expand_subscript_string re-parses the subscript text:
                    // operand-resident quotes are SYNTAX (`unset
                    // 'assoc["@"]'` unsets key `@`), while quote characters
                    // produced by expansions inside the pass stay data
                    // (subst.c:11063 -> expand_word_internal: produced text
                    // is not re-lexed). Marking the whole operand's quotes
                    // as data here is wrong — it breaks the deferred-quote
                    // semantics the builtin paths rely on.
                    let Some(key) = crate::executor::expand_braced_indices::sub_site_key(text)
                    else {
                        return self.expand_subscript_string(text);
                    };
                    if let Some(hit) = crate::executor::expand_braced_indices::sub_res_lookup(&key)
                    {
                        return hit;
                    }
                    let resolved = self.expand_subscript_string(text);
                    crate::executor::expand_braced_indices::sub_res_store(key, resolved.clone());
                    resolved
                }
            }
        }
    }

    /// GNU arrayfunc.c:1353-1391 `array_expand_index` -> `evalexp`: resolve
    /// the indexed-array subscript once (option-aware), then evaluate the
    /// resolved text under `evalexp`'s no-expansion rules — a surviving
    /// `$(...)` or `$name` fails "operand expected" (expr.c:1120) instead
    /// of executing. Arithmetic side effects (`a[i++]=v`) still apply.
    /// The "operand expected" diagnostic is printed here; `Empty` is left
    /// for the caller because GNU's empty-subscript behavior differs per
    /// consumer (`a[]=v` errors, `unset 'a[]'` is silent).
    pub(in crate::executor) fn eval_indexed_subscript(
        &mut self,
        source: SubscriptSource<'_>,
    ) -> IndexedSubscript {
        let resolved = self.resolve_array_subscript(source);
        if resolved.is_empty() {
            return IndexedSubscript::Empty;
        }
        // The same `${}` fragment is re-checked by layered passes; GNU's
        // single array_expand_index evaluates the resolved text once.
        let memo_key = crate::executor::expand_braced_indices::sub_site_key(&resolved);
        if let Some(hit) = memo_key
            .as_ref()
            .and_then(crate::executor::expand_braced_indices::sub_idx_lookup)
        {
            return hit;
        }
        let result = match self.eval_indexed_subscript_expression(&resolved) {
            Some(index) => IndexedSubscript::Index(index),
            None => {
                self.report_indexed_subscript_error(&resolved);
                IndexedSubscript::Error
            }
        };
        if let Some(key) = memo_key {
            crate::executor::expand_braced_indices::sub_idx_store(key, result);
        }
        result
    }

    /// `&self` variant of [`Executor::eval_indexed_subscript`] for the
    /// `&self` parameter-expansion walkers (`${a[sub]}`, `${#a[sub]}` —
    /// subst.c `array_variable_part` -> `array_expand_index` -> `evalexp`).
    /// Arithmetic side effects (`a[i++]`) are evaluated in a cloned env and
    /// queued through `PENDING_SUBSCRIPT_WRITES` for the mutable caller to
    /// apply; an evaluation failure prints the operand-expected diagnostic
    /// and raises the evalerror abort exactly like the mutable variant.
    pub(in crate::executor) fn eval_indexed_subscript_deferred(
        &self,
        source: SubscriptSource<'_>,
    ) -> IndexedSubscript {
        let resolved = self.resolve_array_subscript(source);
        if resolved.is_empty() {
            return IndexedSubscript::Empty;
        }
        // Same single-evaluation dedup as eval_indexed_subscript: the
        // layered `${}` passes re-resolve the identical site+text.
        let memo_key = crate::executor::expand_braced_indices::sub_site_key(&resolved);
        if let Some(hit) = memo_key
            .as_ref()
            .and_then(crate::executor::expand_braced_indices::sub_idx_lookup)
        {
            return hit;
        }
        let overlaid =
            crate::executor::expand_braced_indices::env_vars_with_pending_subscript_writes(
                &self.shell_state.env_vars,
            );
        let (result, writes) = eval_conditional_arith_value_with_writes(&resolved, &overlaid);
        if !writes.is_empty() {
            crate::executor::expand_braced_indices::PENDING_SUBSCRIPT_WRITES
                .with(|pending| pending.borrow_mut().extend(writes));
        }
        let result = match result {
            Some(index) => IndexedSubscript::Index(index),
            None => {
                self.report_indexed_subscript_error(&resolved);
                // This variant runs while expanding a PENDING command's
                // words (subst.c expand_word_internal -> param_expand), so
                // GNU's evalerror DISCARD abandons the command itself:
                // `echo "x=${a[$c]}"` never runs echo. The fatal expansion
                // flags give command_execute.rs its ExpansionFailure skip;
                // evalerror_pending then bounds the discard to the failing
                // command's source line. The &mut variant deliberately does
                // NOT set them — its callers run inside an already-running
                // command (assignments, builtins) where the flag would leak
                // into and wrongly skip the NEXT command.
                self.shell_state.arithmetic_expansion_error.set(true);
                self.shell_state.arithmetic_fatal_error.set(true);
                IndexedSubscript::Error
            }
        };
        if let Some(key) = memo_key {
            crate::executor::expand_braced_indices::sub_idx_store(key, result);
        }
        result
    }

    /// GNU `test -v name[sub]` / `[ -v name[sub] ]` (test.c:650-668):
    /// `aflags = array_expand_once ? AV_NOEXPAND : 0` is passed to
    /// `valid_array_reference`, which consumes VA_* bits — and AV_NOEXPAND
    /// (0x020) sets none of them (arrayfunc.h:62 vs :69-70), so the operand
    /// is ALWAYS validated flag-0, the quote-aware matched-pair scan:
    /// `a[80's]` is an unterminated quote and `a[]]` an empty subscript in
    /// every option state. The ELEMENT lookup does consume AV flags
    /// (get_array_value -> array_variable_name, arrayfunc.c:1414-1420):
    /// array_expand_once takes the subscript verbatim, otherwise the
    /// deferred `expand_subscript_string` pass runs once.
    /// The returned operand carries the FINAL form for the env-only builtin
    /// lookup: `name[<index>]` for indexed subscripts and `name[\x1e<hex>]`
    /// for associative keys (the carrier encoding keeps `]`/`=`/quoting
    /// inside a key from corrupting the re-parse). `Err(())` means
    /// evaluation already failed — the operand-expected diagnostic was
    /// printed and the evalerror abort raised, so the caller only supplies
    /// status 1.
    pub(in crate::executor) fn rewrite_operand_array_subscript(
        &mut self,
        operand: &str,
    ) -> Result<String, ()> {
        let expand_once =
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "array_expand_once");
        self.rewrite_operand_subscript_typed(
            operand,
            if expand_once {
                OperandSubscriptMode::Verbatim
            } else {
                OperandSubscriptMode::ExpandedOnce
            },
            None,
            false,
            false,
        )
    }

    /// `rewrite_operand_array_subscript` with the VA_ONEWORD half of
    /// `SET_VFLAGS` (builtins/common.h:279): `printf -v`/`wait -p` set it
    /// when `array_expand_once` is on and the operand word carried
    /// W_ARRAYREF; test -v and the declare family never do.
    pub(in crate::executor) fn rewrite_operand_array_subscript_flags(
        &mut self,
        operand: &str,
        oneword: bool,
    ) -> Result<String, ()> {
        self.rewrite_operand_subscript_typed(
            operand,
            OperandSubscriptMode::ExpandedOnce,
            None,
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "array_expand_once"),
            oneword,
        )
    }

    /// `[[ -v name[sub] ]]` (execute_cmd.c:4008-4031): the operand expands
    /// under `cond_expand_word(op, 3)` (Q_ARITH, subst.c:4077) and GNU
    /// then calls `set_expand_once(0, 0)`, so test.c's -v machinery always
    /// validates and tokenizes the expanded operand flag-0 — the quote-aware
    /// matched-pair scan in EVERY option state — and resolves the subscript
    /// with one more `expand_array_subscript` pass.
    ///
    /// `arrayref` is the TEST_ARRAYEXP bit: `valid_array_reference(raw,
    /// VA_NOEXPAND)` passed on the RAW operand (execute_cmd.c:4015). GNU
    /// then keeps every `]`/`'`/`"` PRODUCED by expansion inside the
    /// subscript backslash-protected (`set -x` prints `[[ -v F[\]] ]]` for
    /// k=`]`), so flag-0 reads them as escaped data and the subscript
    /// extends to the LAST `]` — modeled below as the last-`]` boundary on
    /// the cooked text with cooked quotes hoisted to data carriers before
    /// the expand pass. A raw word that fails flag-1 (`F[]]`) has no
    /// protected `]` and fails flag-0 outright.
    pub(in crate::executor) fn rewrite_conditional_v_operand(
        &mut self,
        operand: &str,
        arrayref: bool,
    ) -> Result<String, ()> {
        let Some((name, subscript)) = parse_array_subscript(operand) else {
            return Ok(operand.to_string());
        };
        if !is_shell_name(name) {
            return Ok(operand.to_string());
        }
        let assoc = is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name);
        // GNU test.def/test_variable: for an ASSOCIATIVE array `@`/`*` are
        // ordinary literal keys (`[[ -v assoc[@] ]]` tests key `@`); only an
        // indexed array's `@`/`*` mean the whole array.
        if !assoc && matches!(subscript, "@" | "*") {
            return Ok(operand.to_string());
        }
        // TEST_ARRAYEXP implies the raw token ended with `]` (flag-1 needs
        // it as terminator), so the cooked text ends with `]` too and a
        // non-empty subscript is the whole validity question. Without it,
        // flag-0 decides on the cooked text directly.
        // GNU's cond_expand_word \-protects every `'`/`"`/`]` the expansion
        // PRODUCED (set -x shows `[[ -v H[80\'s] ]]`), so the flag-0 scan
        // reads cooked quote bytes as data. The cooked text here carries no
        // such protection, so hoist cooked `'`/`"` to the \x17/\x18 data
        // carriers before running the flag-0 validity scan — an expansion-
        // produced `'` is data, never an opening quote.
        let hoisted_operand = operand
            .replace('\'', crate::executor::markers::DATA_SQUOTE_STR)
            .replace('"', crate::executor::markers::DATA_DQUOTE_STR);
        let valid = if arrayref && assoc {
            !subscript.is_empty()
        } else {
            valid_array_reference_env(&hoisted_operand, false, false, &self.shell_state.env_vars)
        };
        if !valid {
            return Ok(operand.to_string());
        }
        // TEST_ARRAYEXP: the raw token already was a valid array reference,
        // so get_array_value's aflags carry AV_NOEXPAND and the cooked
        // subscript is the key VERBATIM — array_expand_index skips
        // expand_subscript_string entirely (aa[$key] with key='$(date >&2)'
        // must NOT execute the command substitution). Non-TEST_ARRAYEXP
        // operands take the flag-0 tokenize + one deferred
        // expand_subscript_string pass on the quote-hoisted subscript —
        // produced `'`/`"` stay data while `$x`/`$(...)` still expand.
        let hoisted;
        let source = if arrayref {
            SubscriptSource::Protected(subscript)
        } else {
            hoisted = subscript
                .replace('\'', crate::executor::markers::DATA_SQUOTE_STR)
                .replace('"', crate::executor::markers::DATA_DQUOTE_STR);
            SubscriptSource::Raw(&hoisted)
        };
        if assoc {
            let key = self.resolve_array_subscript(source);
            if key.is_empty() && !arrayref {
                // GNU expand_subscript_string -> array_expand_index: a
                // flag-0-valid reference whose subscript expands to nothing
                // ('aa[$undef]', 'aa[$(true)]') reports
                // `aa: bad array subscript` (sh_badsubscript) instead of
                // silently testing a literal name.
                eprintln!("{}{name}: bad array subscript", self.diagnostic_prefix());
            }
            return Ok(format!(
                "{name}[{}]",
                crate::executor::arithmetic::encode_arithmetic_assoc_key(&key)
            ));
        }
        match self.eval_indexed_subscript(source) {
            IndexedSubscript::Index(index) => Ok(format!("{name}[{index}]")),
            IndexedSubscript::Empty => {
                if !arrayref {
                    eprintln!("{}{name}: bad array subscript", self.diagnostic_prefix());
                }
                Ok(operand.to_string())
            }
            IndexedSubscript::Error => Err(()),
        }
    }

    /// GNU declare.def:429 (`assoc_noexpand = array_expand_once &&
    /// wflags & W_ASSIGNMENT`): only an operand whose raw token carried
    /// W_ASSIGNMENT — an unquoted assignment-shaped word — gets ASS_NOEXPAND
    /// verbatim treatment. A quoted or otherwise non-assignment operand's
    /// subscript is re-expanded unconditionally
    /// (expand_arith_string / expand_subscript_string run in both modes).
    pub(in crate::executor) fn rewrite_assignment_builtin_operand(
        &mut self,
        operand: &str,
        w_assignment: bool,
    ) -> Result<String, ()> {
        self.rewrite_operand_subscript(
            operand,
            if w_assignment {
                OperandSubscriptMode::ExpandedOnce
            } else {
                OperandSubscriptMode::AlwaysExpand
            },
        )
    }

    fn rewrite_operand_subscript(
        &mut self,
        operand: &str,
        mode: OperandSubscriptMode,
    ) -> Result<String, ()> {
        self.rewrite_operand_subscript_typed(
            operand,
            mode,
            None,
            crate::builtins::shopt::option_enabled(&self.shell_state.env_vars, "array_expand_once"),
            false,
        )
    }

    /// `assoc` overrides the variable-type probe: GNU decide indexed-vs-
    /// associative from the variable the builtin will actually bind, which
    /// for a function-scope `declare name[sub]=v` is the fresh LOCAL
    /// indexed array `making_array_special` creates (declare.def:641-642,
    /// 959-962) — a global assoc of the same name is shadowed and must not
    /// route the operand down the assoc path.
    /// `noexpand`/`oneword` are the VA_NOEXPAND/VA_ONEWORD flag set the
    /// caller derived (SET_VFLAGS builtins/common.h:279 for
    /// read/printf -v/wait -p, `array_expand_once ? AV_NOEXPAND : 0` for
    /// test -v, forced 0 for `[[ -v ]]` which calls set_expand_once(0,0)
    /// at execute_cmd.c:4027). With VA_NOEXPAND|VA_ONEWORD an assoc
    /// operand's subscript closes at the LAST `]` (`printf -v A[$k]` with
    /// k=`]` stores key `]`).
    pub(in crate::executor) fn rewrite_operand_subscript_typed(
        &mut self,
        operand: &str,
        mode: OperandSubscriptMode,
        assoc: Option<bool>,
        noexpand: bool,
        oneword: bool,
    ) -> Result<String, ()> {
        // W_ARRAYREF arrives in-band as an ARRAYREF_FLAG prefix on the
        // operand text (execute_cmd.c:4366 fix_arrayref_words); callers
        // already derived the VA_ONEWORD half via word_is_arrayref, so strip
        // the carrier byte before name/subscript parsing.
        let operand = crate::builtins::arrayref::take_arrayref_flag(operand).1;
        let Some((name, subscript)) = parse_array_subscript(operand) else {
            return Ok(operand.to_string());
        };
        let is_assoc =
            assoc.unwrap_or_else(|| is_marked_var(&self.shell_state.env_vars, ASSOC_VARS, name));
        // GNU test.c/expr.c: for an ASSOCIATIVE base, `@` and `*` are
        // ordinary subscript keys (`test -v 'assoc[@]'` tests the `@`
        // element); only indexed arrays treat them as whole-array
        // subscripts that bypass element resolution.
        if !is_shell_name(name) || (!is_assoc && matches!(subscript, "@" | "*")) {
            return Ok(operand.to_string());
        }
        if mode != OperandSubscriptMode::AlwaysExpand {
            // GNU printf.def:305 / read.def:405 order: the builtin validates
            // the operand with valid_array_reference(name, arrayflags)
            // BEFORE binding. The naive parse above accepts `a[80's]`, but
            // the flag-0 matched-pair scan (subst.c:2186 skipsubscript)
            // treats the `'` as an unterminated quote — invalid. Pass the
            // operand through so the consumer emits `not a valid
            // identifier` (sh_invalidid) instead of binding `80s`. The
            // AlwaysExpand mode models GNU paths that never run this
            // operand check (declare-family assignment words).
            if !valid_array_reference_env(operand, noexpand, oneword, &self.shell_state.env_vars) {
                return Ok(operand.to_string());
            }
        }
        let source = match mode {
            OperandSubscriptMode::ExpandedOnce => SubscriptSource::ExpandedOnce(subscript),
            OperandSubscriptMode::Verbatim => SubscriptSource::Protected(subscript),
            // The subscript text is re-expanded unconditionally, matching
            // GNU's array_expand_index without AV_NOEXPAND /
            // assign_array_element_internal without ASS_NOEXPAND.
            OperandSubscriptMode::AlwaysExpand => SubscriptSource::Raw(subscript),
        };
        if is_assoc {
            let key = self.resolve_array_subscript(source);
            return Ok(format!(
                "{name}[{}]",
                crate::executor::arithmetic::encode_arithmetic_assoc_key(&key)
            ));
        }
        match self.eval_indexed_subscript(source) {
            IndexedSubscript::Index(index) => Ok(format!("{name}[{index}]")),
            // GNU leaves an empty subscript to the consumer (`test -v 'a[]'`
            // is a quiet false), so the operand passes through unchanged.
            IndexedSubscript::Empty => Ok(operand.to_string()),
            IndexedSubscript::Error => Err(()),
        }
    }

    /// GNU `invalid_subscript` diagnostic (arrayfunc.c `err_badarraysub`):
    /// `<lhs>: bad array subscript`, emitted by the assignment paths for an
    /// empty resolved subscript.
    pub(in crate::executor) fn report_bad_array_subscript(&self, lhs: &str) {
        eprintln!("{}{}: bad array subscript", self.diagnostic_prefix(), lhs);
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    /// GNU arrayfunc.c:557-618 `expand_compound_array_assignment` +
    /// :700-836 `assign_compound_array_list`: resolve each `[sub]=` /
    /// `[sub]+=` element subscript inside a stored `( ... )` compound
    /// value and rewrite it to its final form, so the storage helpers see
    /// plain `[index]=value` / `[key]=value` tokens.
    ///
    /// Provenance differs by caller:
    ///
    ///   * `preexpanded == true` — `name=(...)` assignment words whose
    ///     value text already went through word expansion once. Indexed
    ///     subscripts are ExpandedOnce data (option gates the second
    ///     `expand_arith_string` pass inside `array_expand_index`);
    ///     associative subscripts are the already-expanded literal key
    ///     (`expand_subscript_string` at arrayfunc.c:817 runs on the
    ///     *unexpanded* GNU list, which corresponds to our stored text —
    ///     expansion products inside it stay literal, so a surviving
    ///     `$(...)` is data, not a command to run).
    ///   * `preexpanded == false` — `declare`/`local` `name=(...)` argument
    ///     text, which GNU expands inside `expand_compound_array_assignment`
    ///     (`expand_words_no_vars` for indexed, `expand_subscript_string`
    ///     for assoc). Indexed subscripts then pass through
    ///     `array_expand_index` which expands AGAIN (GNU 5.3.0 executes a
    ///     `$(...)` produced by the first pass here — verified empirically),
    ///     so two `expand_subscript_string` passes run unconditionally.
    ///
    /// Returns `None` after printing the GNU diagnostic when an indexed
    /// subscript fails evaluation ("operand expected") — the caller must
    /// abandon the whole assignment with status 1.
    /// Returns Err(partial) on an element failure: GNU
    /// assign_compound_array_list (arrayfunc.c:765-830) reports the error
    /// and BREAKS — elements already processed still bind and the array is
    /// materialized (`b=([k]=v [""]=x [k2]=v2)` leaves `b=([k]="v")`, rc=1).
    /// Callers must store the partial text instead of dropping the
    /// assignment wholesale.
    pub(in crate::executor) fn rewrite_compound_element_subscripts(
        &mut self,
        name: &str,
        value: &str,
        assoc: bool,
        preexpanded: bool,
    ) -> Result<String, String> {
        let Some(inner) = value
            .strip_prefix('(')
            .and_then(|value| value.strip_suffix(')'))
        else {
            return Ok(value.to_string());
        };
        if !preexpanded {
            return self.expand_declare_compound_elements(name, inner, assoc);
        }
        let mut out = String::with_capacity(inner.len());
        let mut index = 0usize;
        let mut token_start = true;
        while index < inner.len() {
            // GNU expand_compound_array_assignment (arrayfunc.c:557) hands the
            // stored list to parse_string_to_word_list (arrayfunc.c:580),
            // which copies element text verbatim as bytes — multibyte
            // characters and carrier bytes pass through untouched. Decode
            // whole chars here: `bytes[index] as char` latin-1-promoted every
            // UTF-8 byte, corrupting both non-ASCII elements (`x=(é)` stored
            // `Ã©`) and the PUA data-quote carriers (U+E010/U+E011 stand in
            // for the CTLESC protection GNU gives decoded $'...' quotes via
            // sh_single_quote at parse.y:5566-5575 — issue #109: `x=($'a"b')`
            // leaked the marker bytes as î\x80\x91).
            let ch = inner[index..]
                .chars()
                .next()
                .expect("index < inner.len() yields a char");
            // A `[` at a token start may begin a `[sub]=value` element.
            if ch == '[' && token_start {
                if let Some((sub_end, after)) = scan_compound_subscript(inner, index) {
                    let sub = &inner[index + 1..sub_end];
                    if after == CompoundSubscriptTail::Assignment {
                        if assoc {
                            let key = if preexpanded {
                                // The stored list already went through word
                                // expansion once: every `$`/`$(...)` still in
                                // the subscript text is a protected expansion
                                // product, so the deferred
                                // expand_subscript_string at arrayfunc.c:817
                                // yields the literal text in BOTH option
                                // modes (verified against GNU 5.3.0 —
                                // `A=([$k]=v)` never re-executes the
                                // substitution). Only quote removal is left
                                // to model.
                                dequote_compound_subscript(sub)
                            } else {
                                // Declare-path unexpanded list:
                                // expand_subscript_string at arrayfunc.c:817
                                // expands the stored subscript text, quotes
                                // included.
                                self.expand_subscript_string(sub)
                            };
                            if key.is_empty() {
                                // GNU err_badarraysub prints the element
                                // word as rebuilt by
                                // expand_compound_array_assignment.
                                self.report_bad_array_subscript(&compound_element_diagnostic_word(
                                    inner, index, sub_end, &key, true,
                                ));
                                return Err(format!("({out})"));
                            }
                            out.push('[');
                            out.push_str(&encode_compound_assoc_key(&key));
                            out.push(']');
                            index = sub_end + 1;
                            token_start = false;
                            continue;
                        }
                        let resolved = if preexpanded {
                            // expand_words_no_vars already ran once; only
                            // quote removal on the subscript text is left.
                            let once = dequote_compound_subscript(sub);
                            self.resolve_array_subscript(SubscriptSource::ExpandedOnce(&once))
                        } else {
                            // GNU expand_words_no_vars expands the element
                            // word once, then array_expand_index expands
                            // the subscript text again (5.3.0: the second
                            // pass is not gated by array_expand_once on
                            // this path).
                            let once = self.expand_subscript_string(sub);
                            self.expand_subscript_string(&once)
                        };
                        if resolved.is_empty() {
                            self.report_bad_array_subscript(&compound_element_diagnostic_word(
                                inner, index, sub_end, &resolved, false,
                            ));
                            return Err(format!("({out})"));
                        }
                        let Some(index_value) = self.eval_indexed_subscript_expression(&resolved)
                        else {
                            self.report_indexed_subscript_error(&resolved);
                            return Err(format!("({out})"));
                        };
                        out.push('[');
                        out.push_str(&index_value.to_string());
                        out.push(']');
                        index = sub_end + 1;
                        token_start = false;
                        continue;
                    }
                }
            }
            out.push(ch);
            index += ch.len_utf8();
            token_start = ch.is_ascii_whitespace();
        }
        Ok(format!("({out})"))
    }

    /// GNU arrayfunc.c:557-617 `expand_compound_array_assignment` +
    /// :623-665 `assign_assoc_from_kvlist` + :700-836
    /// `assign_compound_array_list` on the `declare`/`local`/`typeset`
    /// operand path (`preexpanded == false`): the compound body is still
    /// literal text here — word expansion deliberately leaves it alone — so
    /// each element word is parsed and expanded the way the builtin does
    /// it:
    ///
    ///   * indexed bare elements go through `expand_words_no_vars`
    ///     (subst.c:12590): full word expansion plus field splitting;
    ///   * indexed `[sub]=value` words expand once (their RHS keeps
    ///     W_ASSIGNMENT no-split semantics, subst.c:4357
    ///     expand_string_assignment), then `array_expand_index` re-evaluates
    ///     the subscript text — modeled by the existing two-pass
    ///     `expand_subscript_string` + `eval_indexed_subscript_expression`;
    ///   * assoc `[key]=value` words expand the key through
    ///     `expand_subscript_string` (arrayfunc.c:817/865) and the value
    ///     through `expand_assignment_string_to_string` (subst.c:3881) —
    ///     no field split;
    ///   * assoc kv-pair words alternate the same two expanders
    ///     (assign_assoc_from_kvlist arrayfunc.c:644-665).
    ///
    /// A `\x03` lead-in (DEFERRED_COMPOUND_BODY, token_actions.rs) marks a
    /// whole-single-quoted operand body whose carriers stand for the
    /// original syntax characters: `\x1f` decodes to a real `$`, `\x18` to
    /// a real `"`, etc. Without it the body already went through word
    /// expansion and a surviving `\x1f` is a protected literal `$` (data).
    fn expand_declare_compound_elements(
        &mut self,
        name: &str,
        inner: &str,
        assoc: bool,
    ) -> Result<String, String> {
        let body = match inner.strip_prefix(crate::executor::types::DEFERRED_COMPOUND_BODY) {
            Some(rest) => std::borrow::Cow::Owned(decode_deferred_compound_body(rest)),
            None => std::borrow::Cow::Borrowed(inner),
        };
        let tokens: Vec<String> =
            crate::executor::assignment_helpers::split_storage_words(&body).collect();
        let mut elements: Vec<String> = Vec::new();

        if assoc {
            // GNU kvpair_assignment_p (arrayfunc.c:665): the FIRST word
            // decides — a `[`-led word selects the strict [key]=value loop,
            // anything else is alternating literal key/value pairs.
            let strict = tokens.first().is_some_and(|token| token.starts_with('['));
            if !strict {
                for pair in tokens.chunks(2) {
                    let key = self.expand_subscript_string(&pair[0]);
                    let value = pair
                        .get(1)
                        .map(|value| self.expand_compound_assignment_rhs(name, value))
                        .unwrap_or_default();
                    elements.push(quote_compound_field_value(&key));
                    elements.push(quote_compound_field_value(&value));
                }
                return Ok(format!("({})", elements.join(" ")));
            }
            for token in &tokens {
                let subscripted = token
                    .starts_with('[')
                    .then(|| scan_compound_subscript(token, 0))
                    .flatten()
                    .filter(|(_, tail)| *tail == CompoundSubscriptTail::Assignment);
                let Some((sub_end, _)) = subscripted else {
                    // A bare word in strict form draws "must use subscript"
                    // from assoc_bare_elements upstream; keep its text.
                    elements.push(token.clone());
                    continue;
                };
                let sub = &token[1..sub_end];
                // GNU arrayfunc.c:817/865: the assoc key takes one
                // expand_subscript_string pass on the stored text.
                let key = self.expand_subscript_string(sub);
                if key.is_empty() {
                    self.report_bad_array_subscript(&compound_token_diagnostic_word(
                        token, sub_end, &key, true,
                    ));
                    return Err(format!("({})", elements.join(" ")));
                }
                let tail = &token[sub_end + 1..];
                let (op, raw_value) = tail
                    .strip_prefix("+=")
                    .map(|value| ("+=", value))
                    .unwrap_or_else(|| ("=", tail.strip_prefix('=').unwrap_or(tail)));
                let expanded_value = self.expand_compound_assignment_rhs(name, raw_value);
                elements.push(format!(
                    "[{}]{}{}",
                    encode_compound_assoc_key(&key),
                    op,
                    quote_compound_field_value(&expanded_value)
                ));
            }
            return Ok(format!("({})", elements.join(" ")));
        }

        for token in &tokens {
            // Field-split products pre-marked by word-stage expansion
            // (\x10) and rendered-array words (\x1d) are already final.
            if token.starts_with(ARRAY_FIELD_SPLIT_MARKER) || token.starts_with(STORAGE_WORD_PREFIX)
            {
                elements.push(token.clone());
                continue;
            }
            let subscripted = token
                .starts_with('[')
                .then(|| scan_compound_subscript(token, 0))
                .flatten()
                .filter(|(_, tail)| *tail == CompoundSubscriptTail::Assignment);
            if let Some((sub_end, _)) = subscripted {
                let sub = &token[1..sub_end];
                // GNU expand_words_no_vars expands the element word once,
                // then array_expand_index expands the subscript text again
                // (5.3.0: the second pass is not gated by array_expand_once
                // on this path).
                let once = self.expand_subscript_string(sub);
                let resolved = self.expand_subscript_string(&once);
                if resolved.is_empty() {
                    self.report_bad_array_subscript(&compound_token_diagnostic_word(
                        token, sub_end, &resolved, false,
                    ));
                    return Err(format!("({})", elements.join(" ")));
                }
                let Some(index_value) = self.eval_indexed_subscript_expression(&resolved) else {
                    self.report_indexed_subscript_error(&resolved);
                    return Err(format!("({})", elements.join(" ")));
                };
                let tail = &token[sub_end + 1..];
                let (op, raw_value) = tail
                    .strip_prefix("+=")
                    .map(|value| ("+=", value))
                    .unwrap_or_else(|| ("=", tail.strip_prefix('=').unwrap_or(tail)));
                // W_ASSIGNMENT element: the RHS expands with assignment
                // semantics — no field splitting (array.tests:
                // declare -a b1='([1]=$v)' stores [1]="a b").
                let expanded_value = self.expand_compound_assignment_rhs(name, raw_value);
                elements.push(format!(
                    "[{index_value}]{op}{}",
                    quote_compound_field_value(&expanded_value)
                ));
                continue;
            }
            // Bare element: GNU expand_words_no_vars (arrayfunc.c:610) runs
            // the full word expansion on the element text — parameter and
            // command substitution, "${a[@]}" fan-out, field splitting and
            // pathname expansion included. Re-lexing the raw token through
            // the command-word expander reproduces all of it, including the
            // multi-word result of a quoted "${d[@]}" element.
            for field in self.expand_alternate_word_fragment(token) {
                let fields =
                    match super::glob::pathname_expand_word(&field, &self.shell_state.env_vars) {
                        super::glob::PathnameExpansion::Matches(matches) => matches,
                        super::glob::PathnameExpansion::NoMatch => vec![field],
                        super::glob::PathnameExpansion::Fail(pattern) => {
                            self.report_failglob(&pattern);
                            return Err(format!("({})", elements.join(" ")));
                        }
                    };
                for field in fields {
                    // \x10 marks the field as a word-expansion product so the
                    // storage layer stores it bare even when it looks like a
                    // [subscript]= assignment (GNU: the flag is parse-time only).
                    elements.push(format!(
                        "{ARRAY_FIELD_SPLIT_MARKER}{}",
                        quote_compound_field_value(&field)
                    ));
                }
            }
        }
        Ok(format!("({})", elements.join(" ")))
    }

    /// GNU arrayfunc.c:753/865: a `[sub]=value` (or assoc kv-pair value)
    /// element word carries W_ASSIGNMENT, so the value side expands through
    /// expand_assignment_string_to_string (subst.c:3881) — full expansion
    /// plus quote removal, no field splitting. The raw element text still
    /// holds real quote syntax here (`[1]=""`, `[2]="$v"`); re-lexing it as
    /// an assignment RHS turns those quotes into the carrier encoding
    /// expand_assignment_value expects, so syntax quotes are consumed and
    /// expansion-produced `"`s stay data.
    fn expand_compound_assignment_rhs(&mut self, name: &str, raw_value: &str) -> String {
        // The /E309 whitespace tags are CTLESC-style protection for the
        // following character (embedded_mutations expansion_ws_marked):
        // re-lexing raw text would treat the space after the tag as a word
        // delimiter and drop the rest of the value (assoc12.sub: a
        // `[k]=$bar` RHS `3 4 5` shrank to `3`). Rewriting tag+char as
        // backslash+char keeps it one literal character in the RHS.
        let mut escaped = String::with_capacity(raw_value.len());
        let mut chars = raw_value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == crate::executor::markers::IFS_GLUE
                || ch == crate::executor::COMPOUND_EXPANSION_WS_TAG
            {
                if let Some(next) = chars.next() {
                    escaped.push('\\');
                    escaped.push(next);
                }
            } else {
                escaped.push(ch);
            }
        }
        let raw_value = &escaped;
        let probe = format!("__v={raw_value}");
        let rhs = crate::lexer::tokenize(&probe)
            .into_iter()
            .next()
            .and_then(|token| token.value.split_once('=').map(|(_, rhs)| rhs.to_string()));
        match rhs {
            Some(rhs) => self.expand_assignment_value(name, &rhs),
            None => self.expand_assignment_value(name, raw_value),
        }
    }
}

/// Whether the text after a `[`+subscript+`]` span continues an element
/// assignment (`=`/`+=`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CompoundSubscriptTail {
    Assignment,
    Other,
}

/// Scan `text` for the `]` closing a `[sub]` span starting at `start`
/// (which must index a `[`), honoring backslash escapes, single and double
/// quotes, `$(`/`${` nesting and nested `[`/`]` pairs — mirroring GNU's
/// `skipsubscript` + `parse_string_to_word_list` handling of the stored
/// compound text. Returns (index of `]`, tail classification).
pub(super) fn scan_compound_subscript(
    text: &str,
    start: usize,
) -> Option<(usize, CompoundSubscriptTail)> {
    let bytes = text.as_bytes();
    let mut pos = start + 1;
    let mut bracket_depth = 1usize;
    let mut paren_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' if !in_single => pos += 1,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'$' if !in_single && !in_double => match bytes.get(pos + 1) {
                Some(b'(') => {
                    paren_depth += 1;
                    pos += 1;
                }
                Some(b'{') => {
                    brace_depth += 1;
                    pos += 1;
                }
                _ => {}
            },
            b'(' if !in_single && !in_double && paren_depth > 0 => paren_depth += 1,
            b')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
            b'{' if !in_single && !in_double && brace_depth > 0 => brace_depth += 1,
            b'}' if !in_single && !in_double && brace_depth > 0 => brace_depth -= 1,
            b'[' if !in_single && !in_double && paren_depth == 0 && brace_depth == 0 => {
                bracket_depth += 1;
            }
            b']' if !in_single && !in_double && paren_depth == 0 && brace_depth == 0 => {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    let after = &text[pos + 1..];
                    let tail = if after.starts_with('=') || after.starts_with("+=") {
                        CompoundSubscriptTail::Assignment
                    } else {
                        CompoundSubscriptTail::Other
                    };
                    return Some((pos, tail));
                }
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

/// Decode a deferred (`\x03`-marked) single-quoted compound body: every
/// lexer carrier stands for the original syntax character, which GNU's
/// parse_string_to_word_list re-parse sees as real quoting/expansion
/// syntax (arrayfunc.c:580). `\x1f` -> `$`, `\x18` -> `"`, `\x1a` ->
/// backtick, `\x14` -> `\`, `\x17` -> `'`.
/// Decode each `$'...'` span in raw subscript text (GNU subst.c:11063
/// expand_subscript_string -> expand_word_internal's ANSI-C arm). The decoded
/// bytes pass through escape_decoded_ansi_c_quotes so quotes and `$` in the
/// result stay DATA for the embedded-parameter walker.
fn decode_ansi_c_spans(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0usize;
    let mut in_single = false;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\\' && !in_single && index + 1 < chars.len() {
            // A backslash pair outside quotes is literal syntax here; keep it
            // for mask_subscript_escapes.
            output.push(ch);
            output.push(chars[index + 1]);
            index += 2;
            continue;
        }
        if ch == '\'' {
            in_single = !in_single;
            output.push(ch);
            index += 1;
            continue;
        }
        if !in_single && ch == '$' && chars.get(index + 1) == Some(&'\'') {
            // `$'...'`: the close quote is the first unescaped `'`.
            let mut inner_end = index + 2;
            let mut inner = String::new();
            let mut closed = false;
            while inner_end < chars.len() {
                let c = chars[inner_end];
                if c == '\\' && inner_end + 1 < chars.len() {
                    inner.push(c);
                    inner.push(chars[inner_end + 1]);
                    inner_end += 2;
                    continue;
                }
                if c == '\'' {
                    closed = true;
                    break;
                }
                inner.push(c);
                inner_end += 1;
            }
            if closed {
                let decoded = crate::lexer::decode_ansi_c_quoted(&inner);
                output.push_str(&crate::lexer::escape_decoded_ansi_c_quotes(&decoded));
                index = inner_end + 1;
                continue;
            }
        }
        output.push(ch);
        index += 1;
    }
    output
}

fn decode_deferred_compound_body(inner: &str) -> String {
    inner
        .chars()
        .map(|ch| match ch {
            LITERAL_DOLLAR => '$',
            LITERAL_BACKTICK => '`',
            LITERAL_BACKSLASH => '\\',
            LITERAL_DOUBLE_QUOTE => '"',
            LITERAL_SINGLE_QUOTE => '\'',
            other => other,
        })
        .collect()
}

/// The element word as GNU's `err_badarraysub` sees it, token-based variant
/// of compound_element_diagnostic_word for expand_declare_compound_elements:
/// assoc elements report as `['key']='value'` (quote_string'd rebuild),
/// indexed as the expanded-unquoted `[sub]=value`.
fn compound_token_diagnostic_word(
    token: &str,
    sub_end: usize,
    subscript: &str,
    assoc: bool,
) -> String {
    let tail = &token[sub_end + 1..];
    let (op, value) = tail
        .strip_prefix("+=")
        .map(|value| ("+=", value))
        .unwrap_or_else(|| ("=", tail.strip_prefix('=').unwrap_or(tail)));
    let dequoted = dequote_compound_subscript(value);
    if assoc {
        format!(
            "[{}]{}{}",
            diagnostic_single_quote(subscript),
            op,
            diagnostic_single_quote(&dequoted)
        )
    } else {
        format!("[{subscript}]{op}{dequoted}")
    }
}

/// The `[sub]=value` element word starting at `start` — the raw text span
/// up to the next unquoted whitespace.
fn compound_element_word(text: &str, start: usize) -> &str {
    let bytes = text.as_bytes();
    let mut pos = start;
    let mut in_single = false;
    let mut in_double = false;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\\' if !in_single => pos += 1,
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b' ' | b'\t' | b'\n' if !in_single && !in_double => break,
            _ => {}
        }
        pos += 1;
    }
    &text[start..pos]
}

/// The element word as GNU's `err_badarraysub` sees it: the assoc compound
/// word list was rebuilt by expand_compound_array_assignment with each
/// expanded part quote_string'd, so `[""]="v"` reports as `['']='v'`;
/// indexed elements keep the expanded-but-unquoted text (`[]=v`).
/// `sub_end` indexes the `]` closing the subscript in `text`; `subscript`
/// is the expanded/dequoted subscript text.
fn compound_element_diagnostic_word(
    text: &str,
    start: usize,
    sub_end: usize,
    subscript: &str,
    assoc: bool,
) -> String {
    let element = compound_element_word(text, start);
    let tail = &element[sub_end - start + 1..];
    let (op, value) = tail
        .strip_prefix("+=")
        .map(|value| ("+=", value))
        .unwrap_or_else(|| ("=", tail.strip_prefix('=').unwrap_or(tail)));
    let dequoted = dequote_compound_subscript(value);
    if assoc {
        format!(
            "[{}]{}{}",
            diagnostic_single_quote(subscript),
            op,
            diagnostic_single_quote(&dequoted)
        )
    } else {
        format!("[{subscript}]{op}{dequoted}")
    }
}

/// lib/sh/shquote.c sh_single_quote: wrap in single quotes, rendering each
/// embedded quote as `'\''`.
fn diagnostic_single_quote(s: &str) -> String {
    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// Dequote a compound-element subscript the way GNU's re-parse of the
/// stored `(...)` list does (parse_string_to_word_list quote removal +
/// `expand_subscript_string`'s dequoting): single and double quotes
/// vanish, backslash escapes collapse to the quoted character. No
/// expansion runs — expansion products already in the stored text stay
/// data.
fn dequote_compound_subscript(sub: &str) -> String {
    if let Some(literal) = wholly_single_quoted_literal(sub) {
        return literal;
    }
    let mut out = String::with_capacity(sub.len());
    let mut chars = sub.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if !in_single => match chars.next() {
                Some(next) if !in_double || matches!(next, '$' | '`' | '"' | '\\' | '\n') => {
                    if next != '\n' {
                        out.push(next);
                    }
                }
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            },
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => out.push(ch),
        }
    }
    // The stored subscript text already went through word expansion once, so
    // it can carry the \x1c/E309 expansion-whitespace data tags; the resolved
    // key is cooked text only — strip the tags the same way
    // expand_subscript_string does for its freshly-expanded result.
    out.replace(crate::executor::COMPOUND_EXPANSION_WS_TAG, "")
        .replace(crate::executor::markers::IFS_GLUE, "")
}

/// Encode a resolved associative compound-element key for the `[key]=value`
/// token the storage helpers will parse. Keys containing `]`, `=`,
/// whitespace or quoting characters would corrupt the `[`/`]`/`=` token
/// syntax, so they travel hex-encoded behind the `\x1e` carrier
/// (ARITH_ASSOC_KEY_MARKER), which `assoc_assignment_token` decodes.
fn encode_compound_assoc_key(key: &str) -> String {
    let safe = !key.is_empty()
        && !key.chars().any(|ch| {
            matches!(
                ch,
                '[' | ']'
                    | '='
                    | '+'
                    | '\''
                    | '"'
                    | '\\'
                    | crate::executor::markers::SUBSCRIPT_CARRIER
                    | DATA_DOLLAR
                    | '`'
                    | '$'
            ) || ch.is_ascii_whitespace()
        });
    if safe {
        key.to_string()
    } else {
        crate::executor::arithmetic::encode_arithmetic_assoc_key(key)
    }
}

/// Mark bare `'` bytes in already-expanded subscript text with the \x17
/// data carrier (CTLESC port). GNU's lexer resolves single-quoted spans
/// before expansion — expand_word_internal never treats `'` as syntax —
/// so every `'` surviving in a once-expanded operand is data produced by
/// that expansion (`dict["$k"]` with k=`'`). Escaped quotes are left for
/// mask_subscript_escapes, which resolves `\'` to the same carrier.
pub(crate) fn mark_expanded_once_data_squotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        out.push(if ch == '\'' { LITERAL_SINGLE_QUOTE } else { ch });
    }
    out
}

/// Same once-expanded-data invariant for `"` bytes: a `"` surviving in a
/// word-expanded operand is expansion output (GNU's CTLESC'd data), never a
/// quote delimiter — `unset -v dict["$k"]` with k=`"` removes key `"` while
/// the literal 'dict["]' operand fails valid_array_reference
/// (assoc9.sub del loop). Mark with the \x18 data carrier so
/// expand_subscript_string's walker emits it as a literal `"`.
pub(crate) fn mark_expanded_once_data_dquotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(ch);
            if let Some(next) = chars.next() {
                out.push(next);
            }
            continue;
        }
        out.push(if ch == '"' { LITERAL_DOUBLE_QUOTE } else { ch });
    }
    out
}

/// Resolve the subscript token's quoting the way the lexer does, leaving the
/// walker a string whose remaining `$`, `` ` `` and quote characters are all
/// live syntax.
///
/// A backslash quotes the following character. In unquoted context the pair
/// collapses to the character alone; inside double quotes a backslash is
/// special before `$`, `` ` ``, `"` and `\` only (subst.c
/// `string_extract_double_quoted`) and stays literal before anything else.
fn mask_subscript_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(ch) = chars.next() {
        if in_single {
            out.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            continue;
        }
        if ch == '"' {
            in_double = !in_double;
            out.push(ch);
            continue;
        }
        if ch != '\\' {
            if ch == '\'' && !in_double {
                in_single = true;
            }
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            // A trailing backslash is literal.
            out.push('\\');
            break;
        };
        if next == '\n' {
            // Line continuation: the backslash and the newline both vanish.
            continue;
        }
        match next {
            '\\' => out.push(LITERAL_BACKSLASH),
            '$' => out.push(LITERAL_DOLLAR),
            '`' => out.push(LITERAL_BACKTICK),
            '\'' if !in_double => out.push(LITERAL_SINGLE_QUOTE),
            '"' => out.push(LITERAL_DOUBLE_QUOTE),
            _ if in_double => {
                out.push('\\');
                out.push(next);
            }
            _ => out.push(next),
        }
    }
    out
}

/// The concatenated contents of `text` when it is covered entirely by
/// single-quoted spans (`'a b'`, `'a''b'`); `None` when any character sits
/// outside a single-quoted span, in which case the subscript still has to be
/// expanded.
pub(in crate::executor) fn wholly_single_quoted_literal(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    let mut saw_span = false;
    while !rest.is_empty() {
        // '\u{E307}' is the compound-assignment hoisted single-quote
        // sentinel (SQ_DATA, assignment_expansion.rs) and crate::executor::markers::DATA_SQUOTE is the
        // embedded-parameter walker's literal-single-quote data marker —
        // arithmetic input arrives with `'` already converted to `\x17`
        // (expand_arithmetic_special_parameters), so `$(( ${A['$(..)']}
        // ))` must treat `\x17$(..)\x17` as the same literal span. Both
        // carry the same "no expansion inside" guarantee as a literal `'`.
        let (inner, close) = if let Some(inner) = rest.strip_prefix('\'') {
            (inner, '\'')
        } else if let Some(inner) = rest.strip_prefix('\u{E307}') {
            (inner, '\u{E307}')
        } else if let Some(inner) = rest.strip_prefix(crate::executor::markers::DATA_SQUOTE) {
            (inner, crate::executor::markers::DATA_SQUOTE)
        } else {
            return None;
        };
        let end = inner.find(close)?;
        out.push_str(&inner[..end]);
        rest = &inner[end + 1..];
        saw_span = true;
    }
    saw_span.then_some(out)
}

/// GNU `valid_array_reference` (`arrayfunc.c:1350` ->
/// `tokenize_array_reference`, arrayfunc.c:1288): `name[sub]` is valid when
/// the base is a valid identifier and the `]` matching the first `[` is the
/// last character with a non-empty subscript.
///
/// `noexpand`/`oneword` model the `VA_NOEXPAND`/`VA_ONEWORD` flag set
/// (`arrayfunc.h:69-70`) a builtin derived for the operand:
///
/// * `VA_NOEXPAND|VA_ONEWORD` AND `base` names an existing assoc:
///   `tokenize_array_reference` closes the subscript at `strlen(t)-1` —
///   the LAST `]` — so `A[]]` keys on `]` and `A[foo]bar]` keys on
///   `foo]bar` (arrayfunc.c:1311-1314). Only len==1 (`A[]`) fails.
/// * `VA_NOEXPAND` alone AND `base` names an existing assoc:
///   `skipsubscript(t, 0, 1)` takes the FIRST `]` verbatim — `a[80's]`
///   is a valid `80's` key, but `A[]]` ends the subscript at len 1 and
///   is rejected.
/// * otherwise: `skipsubscript` runs the flag-0 quote-aware matched-pair
///   scan (subst.c:2186 -> skip_matched_pair subst.c:2086), so `'`/`"`
///   open quoting and `a[80's]` has no depth-0 `]` — invalid.
pub(crate) fn valid_array_reference_env(
    name: &str,
    noexpand: bool,
    oneword: bool,
    env_vars: &HashMap<String, String>,
) -> bool {
    let Some(open) = name.find('[') else {
        return false;
    };
    let base = &name[..open];
    if !is_shell_name(base) {
        return false;
    }
    // tokenize_array_reference only consults the variable under VA_NOEXPAND;
    // a flag-0 caller always runs the quote-aware else-branch, even for an
    // assoc, and a flag-1 caller falls back to flag-0 when the variable is
    // unset or not associative.
    if noexpand && is_marked_var(env_vars, ASSOC_VARS, base) {
        let sub = &name[open + 1..];
        if oneword {
            // strlen(t)-1 — the last `]` closes the subscript.
            return sub.len() > 1 && sub.ends_with(']');
        }
        // skipsubscript flag-1: the FIRST `]` must be the last byte and
        // must leave a non-empty subscript (arrayfunc.c:1323-1324).
        return sub
            .find(']')
            .is_some_and(|close| close >= 1 && close == sub.len() - 1);
    }
    match scan_compound_subscript(name, open) {
        Some((close, _)) => close == name.len() - 1 && close > open + 1,
        None => false,
    }
}

impl Executor {
    /// GNU `execute_cmd.c:4366-4401 fix_arrayref_words`: whether the word
    /// at `index` in `cmd.words` carried `W_ARRAYREF` — i.e. its raw token
    /// passed flag-0 `valid_array_reference` (`[` outside quotes, matched
    /// `]` at the end). `builtin_arrayref_flags` (builtins/common.c:1040)
    /// and `SET_VFLAGS` (builtins/common.h:279) both consume this bit; the
    /// expanded text cannot recover it (`dict["'"]` and `dict[']'` cook to
    /// the same string).
    pub(in crate::executor) fn word_is_arrayref(&self, cmd: &CommandNode, index: usize) -> bool {
        cmd.word_metadata
            .iter()
            .find(|metadata| metadata.word_index == index)
            .is_some_and(|metadata| {
                valid_array_reference_env(&metadata.raw, false, false, &self.shell_state.env_vars)
            })
    }
}
