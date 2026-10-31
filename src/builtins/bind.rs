//! bind module — pure parsing and listing formats.
//!
//! GNU Bash source ownership:
//! - builtins/bind.def (option grammar, query_bindings/unbind_command/
//!   unbind_keyseq)
//! - bashline.c:4814 isolate_sequence / :4850 bind_keyseq_to_unix_command
//!   (the `-x` argument grammar)
//! - bashline.c:4720 print_unix_command + print_unix_command_map (`-X`
//!   listing format)
//! - The stateful half lives in the executor (`Executor::execute_bind`,
//!   the registry in [`crate::shell::bind_registry`]); hosts mirror the
//!   registry into their own line-editor keymaps.
//!
//! Probe evidence (WSL GNU bash 5.3.0, target/issue-suites/results/
//! wt100-i185/): `bind -X` prints `"\C-g" "echo bound-c-g"`; `bind -s`
//! prints `"\C-t": "ins-macro"`; `bind -S` prints `\C-t outputs
//! ins-macro`; `-q` answers `NAME can be invoked via "seq", ...` (rc 0)
//! or `NAME is not bound to any keys.` (rc 1) or
//! ``bind: `NAME': unknown function name`` (rc 1); `-p`/`-P`/`-l`/`-v`
//! print to stdout; `-x noquotes` errors with `first non-whitespace
//! character is not '"'`.

use std::io::{self, Write};

const EXECUTION_SUCCESS: i32 = 0;
const EX_USAGE: i32 = 2;

/// The readline function names, in GNU's funmap order — the order
/// `bind -l` lists them (rl_list_funmap_names) and the dumpers iterate
/// them. The table is readline's funmap.c `funmap[]` (153 entries) plus
/// the defuns bash itself registers in initialize_readline
/// (bashline.c rl_add_defun calls: shell-*, glob-*, spell-correct-word,
/// the complete-* family, display-shell-version, ...), 176 names total,
/// captured from the GNU 5.3.20 oracle's `bind -l` (rubash#2338: the
/// earlier 57-name reedline subset made `bind -l` omit `complete`,
/// `history-search-backward` and 117 more names GNU lists, and made
/// `-q`/`-u` reject them as unknown functions).
pub const READLINE_FUNCTIONS: &[&str] = &[
    "abort",
    "accept-line",
    "alias-expand-line",
    "arrow-key-prefix",
    "backward-byte",
    "backward-char",
    "backward-delete-char",
    "backward-kill-line",
    "backward-kill-word",
    "backward-word",
    "bash-vi-complete",
    "beginning-of-history",
    "beginning-of-line",
    "bracketed-paste-begin",
    "call-last-kbd-macro",
    "capitalize-word",
    "character-search",
    "character-search-backward",
    "clear-display",
    "clear-screen",
    "complete",
    "complete-command",
    "complete-filename",
    "complete-hostname",
    "complete-into-braces",
    "complete-username",
    "complete-variable",
    "copy-backward-word",
    "copy-forward-word",
    "copy-region-as-kill",
    "dabbrev-expand",
    "delete-char",
    "delete-char-or-list",
    "delete-horizontal-space",
    "digit-argument",
    "display-shell-version",
    "do-lowercase-version",
    "downcase-word",
    "dump-functions",
    "dump-macros",
    "dump-variables",
    "dynamic-complete-history",
    "edit-and-execute-command",
    "emacs-editing-mode",
    "end-kbd-macro",
    "end-of-history",
    "end-of-line",
    "exchange-point-and-mark",
    "execute-named-command",
    "export-completions",
    "fetch-history",
    "forward-backward-delete-char",
    "forward-byte",
    "forward-char",
    "forward-search-history",
    "forward-word",
    "glob-complete-word",
    "glob-expand-word",
    "glob-list-expansions",
    "history-and-alias-expand-line",
    "history-expand-line",
    "history-search-backward",
    "history-search-forward",
    "history-substring-search-backward",
    "history-substring-search-forward",
    "insert-comment",
    "insert-completions",
    "insert-last-argument",
    "kill-line",
    "kill-region",
    "kill-whole-line",
    "kill-word",
    "magic-space",
    "menu-complete",
    "menu-complete-backward",
    "next-history",
    "next-screen-line",
    "non-incremental-forward-search-history",
    "non-incremental-forward-search-history-again",
    "non-incremental-reverse-search-history",
    "non-incremental-reverse-search-history-again",
    "old-menu-complete",
    "operate-and-get-next",
    "overwrite-mode",
    "possible-command-completions",
    "possible-completions",
    "possible-filename-completions",
    "possible-hostname-completions",
    "possible-username-completions",
    "possible-variable-completions",
    "previous-history",
    "previous-screen-line",
    "print-last-kbd-macro",
    "quoted-insert",
    "re-read-init-file",
    "redraw-current-line",
    "reverse-search-history",
    "revert-line",
    "self-insert",
    "set-mark",
    "shell-backward-kill-word",
    "shell-backward-word",
    "shell-expand-line",
    "shell-forward-word",
    "shell-kill-word",
    "shell-transpose-words",
    "skip-csi-sequence",
    "spell-correct-word",
    "start-kbd-macro",
    "tab-insert",
    "tilde-expand",
    "transpose-chars",
    "transpose-words",
    "tty-status",
    "undo",
    "universal-argument",
    "unix-filename-rubout",
    "unix-line-discard",
    "unix-word-rubout",
    "upcase-word",
    "vi-append-eol",
    "vi-append-mode",
    "vi-arg-digit",
    "vi-bWord",
    "vi-back-to-indent",
    "vi-backward-bigword",
    "vi-backward-word",
    "vi-bword",
    "vi-change-case",
    "vi-change-char",
    "vi-change-to",
    "vi-char-search",
    "vi-column",
    "vi-complete",
    "vi-delete",
    "vi-delete-to",
    "vi-eWord",
    "vi-edit-and-execute-command",
    "vi-editing-mode",
    "vi-end-bigword",
    "vi-end-word",
    "vi-eof-maybe",
    "vi-eword",
    "vi-fWord",
    "vi-fetch-history",
    "vi-first-print",
    "vi-forward-bigword",
    "vi-forward-word",
    "vi-fword",
    "vi-goto-mark",
    "vi-insert-beg",
    "vi-insertion-mode",
    "vi-match",
    "vi-movement-mode",
    "vi-next-word",
    "vi-overstrike",
    "vi-overstrike-delete",
    "vi-prev-word",
    "vi-put",
    "vi-redo",
    "vi-replace",
    "vi-rubout",
    "vi-search",
    "vi-search-again",
    "vi-set-mark",
    "vi-subst",
    "vi-tilde-expand",
    "vi-undo",
    "vi-unix-word-rubout",
    "vi-yank-arg",
    "vi-yank-pop",
    "vi-yank-to",
    "yank",
    "yank-last-arg",
    "yank-nth-arg",
    "yank-pop",
];

/// The default emacs keymap's function bindings, in readline's keymap
/// scan order (rl_invoking_keyseqs_in_map, lib/readline/bind.c: keys in
/// byte order, submaps recursed in place — the order `bind -q` lists
/// invoking sequences). Content: the emacs_standard_keymap array
/// (lib/readline/emacs_keymap.c) with the emacs_meta_keymap and
/// emacs_ctlx_keymap submaps flattened in under their `\e` / `\C-x`
/// prefixes, plus the bindings readline's initialization adds —
/// bind_arrow_keys_internal (lib/readline/readline.c:1400-1418: the
/// `\e[A`..`\eOF` family and the ctrl/alt-arrow sequences) and the
/// bracketed-paste prefix (`\e[200~`). Captured row-for-row from the
/// GNU 5.3.20 oracle's `bind -p` dump (rubash#2338: the earlier 26-row
/// reedline-flavored table lacked e.g. `\eOD` for backward-char and
/// every meta/ctlx binding, and its order was not the scan order).
/// niubash's bind surface mirrors this table (see docs/api-ledger.md).
pub const DEFAULT_FUNCTION_BINDINGS: &[(&str, &str)] = &[
    ("\\C-@", "set-mark"),
    ("\\C-a", "beginning-of-line"),
    ("\\C-b", "backward-char"),
    ("\\C-d", "delete-char"),
    ("\\C-e", "end-of-line"),
    ("\\C-f", "forward-char"),
    ("\\C-g", "abort"),
    ("\\C-h", "backward-delete-char"),
    ("\\C-i", "complete"),
    ("\\C-j", "accept-line"),
    ("\\C-k", "kill-line"),
    ("\\C-l", "clear-screen"),
    ("\\C-m", "accept-line"),
    ("\\C-n", "next-history"),
    ("\\C-o", "operate-and-get-next"),
    ("\\C-p", "previous-history"),
    ("\\C-q", "quoted-insert"),
    ("\\C-r", "reverse-search-history"),
    ("\\C-s", "forward-search-history"),
    ("\\C-t", "transpose-chars"),
    ("\\C-u", "unix-line-discard"),
    ("\\C-v", "quoted-insert"),
    ("\\C-w", "unix-word-rubout"),
    ("\\C-x\\C-e", "edit-and-execute-command"),
    ("\\C-x\\C-g", "abort"),
    ("\\C-x\\C-r", "re-read-init-file"),
    ("\\C-x\\C-u", "undo"),
    ("\\C-x\\C-v", "display-shell-version"),
    ("\\C-x\\C-x", "exchange-point-and-mark"),
    ("\\C-x!", "possible-command-completions"),
    ("\\C-x$", "possible-variable-completions"),
    ("\\C-x(", "start-kbd-macro"),
    ("\\C-x)", "end-kbd-macro"),
    ("\\C-x*", "glob-expand-word"),
    ("\\C-x/", "possible-filename-completions"),
    ("\\C-x@", "possible-hostname-completions"),
    ("\\C-xA", "do-lowercase-version"),
    ("\\C-xB", "do-lowercase-version"),
    ("\\C-xC", "do-lowercase-version"),
    ("\\C-xD", "do-lowercase-version"),
    ("\\C-xE", "do-lowercase-version"),
    ("\\C-xF", "do-lowercase-version"),
    ("\\C-xG", "do-lowercase-version"),
    ("\\C-xH", "do-lowercase-version"),
    ("\\C-xI", "do-lowercase-version"),
    ("\\C-xJ", "do-lowercase-version"),
    ("\\C-xK", "do-lowercase-version"),
    ("\\C-xL", "do-lowercase-version"),
    ("\\C-xM", "do-lowercase-version"),
    ("\\C-xN", "do-lowercase-version"),
    ("\\C-xO", "do-lowercase-version"),
    ("\\C-xP", "do-lowercase-version"),
    ("\\C-xQ", "do-lowercase-version"),
    ("\\C-xR", "do-lowercase-version"),
    ("\\C-xS", "do-lowercase-version"),
    ("\\C-xT", "do-lowercase-version"),
    ("\\C-xU", "do-lowercase-version"),
    ("\\C-xV", "do-lowercase-version"),
    ("\\C-xW", "do-lowercase-version"),
    ("\\C-xX", "do-lowercase-version"),
    ("\\C-xY", "do-lowercase-version"),
    ("\\C-xZ", "do-lowercase-version"),
    ("\\C-xe", "call-last-kbd-macro"),
    ("\\C-xg", "glob-list-expansions"),
    ("\\C-xs", "spell-correct-word"),
    ("\\C-x~", "possible-username-completions"),
    ("\\C-x\\C-?", "backward-kill-line"),
    ("\\C-y", "yank"),
    ("\\e\\C-b", "shell-backward-word"),
    ("\\e\\C-d", "shell-kill-word"),
    ("\\e\\C-e", "shell-expand-line"),
    ("\\e\\C-f", "shell-forward-word"),
    ("\\e\\C-g", "abort"),
    ("\\e\\C-h", "backward-kill-word"),
    ("\\e\\C-i", "dynamic-complete-history"),
    ("\\e\\C-l", "clear-display"),
    ("\\e\\C-r", "revert-line"),
    ("\\e\\C-t", "shell-transpose-words"),
    ("\\e\\C-y", "yank-nth-arg"),
    ("\\e\\e", "complete"),
    ("\\e\\e[C", "forward-word"),
    ("\\e\\e[D", "backward-word"),
    ("\\e\\C-]", "character-search-backward"),
    ("\\e ", "set-mark"),
    ("\\e!", "complete-command"),
    ("\\e#", "insert-comment"),
    ("\\e$", "complete-variable"),
    ("\\e&", "tilde-expand"),
    ("\\e*", "insert-completions"),
    ("\\e-", "digit-argument"),
    ("\\e.", "insert-last-argument"),
    ("\\e.", "yank-last-arg"),
    ("\\e/", "complete-filename"),
    ("\\e0", "digit-argument"),
    ("\\e1", "digit-argument"),
    ("\\e2", "digit-argument"),
    ("\\e3", "digit-argument"),
    ("\\e4", "digit-argument"),
    ("\\e5", "digit-argument"),
    ("\\e6", "digit-argument"),
    ("\\e7", "digit-argument"),
    ("\\e8", "digit-argument"),
    ("\\e9", "digit-argument"),
    ("\\e<", "beginning-of-history"),
    ("\\e=", "possible-completions"),
    ("\\e>", "end-of-history"),
    ("\\e?", "possible-completions"),
    ("\\e@", "complete-hostname"),
    ("\\eA", "do-lowercase-version"),
    ("\\eB", "do-lowercase-version"),
    ("\\eC", "do-lowercase-version"),
    ("\\eD", "do-lowercase-version"),
    ("\\eE", "do-lowercase-version"),
    ("\\eF", "do-lowercase-version"),
    ("\\eG", "do-lowercase-version"),
    ("\\eH", "do-lowercase-version"),
    ("\\eI", "do-lowercase-version"),
    ("\\eJ", "do-lowercase-version"),
    ("\\eK", "do-lowercase-version"),
    ("\\eL", "do-lowercase-version"),
    ("\\eM", "do-lowercase-version"),
    ("\\eN", "do-lowercase-version"),
    ("\\eOA", "previous-history"),
    ("\\eOB", "next-history"),
    ("\\eOC", "forward-char"),
    ("\\eOD", "backward-char"),
    ("\\eOF", "end-of-line"),
    ("\\eOH", "beginning-of-line"),
    ("\\eP", "do-lowercase-version"),
    ("\\eQ", "do-lowercase-version"),
    ("\\eR", "do-lowercase-version"),
    ("\\eS", "do-lowercase-version"),
    ("\\eT", "do-lowercase-version"),
    ("\\eU", "do-lowercase-version"),
    ("\\eV", "do-lowercase-version"),
    ("\\eW", "do-lowercase-version"),
    ("\\eX", "do-lowercase-version"),
    ("\\eY", "do-lowercase-version"),
    ("\\eZ", "do-lowercase-version"),
    ("\\e[1;3C", "forward-word"),
    ("\\e[1;3D", "backward-word"),
    ("\\e[1;5C", "forward-word"),
    ("\\e[1;5D", "backward-word"),
    ("\\e[1~", "beginning-of-line"),
    ("\\e[200~", "bracketed-paste-begin"),
    ("\\e[2~", "quoted-insert"),
    ("\\e[3;5~", "kill-word"),
    ("\\e[3~", "delete-char"),
    ("\\e[4~", "end-of-line"),
    ("\\e[5C", "forward-word"),
    ("\\e[5D", "backward-word"),
    ("\\e[A", "previous-history"),
    ("\\e[B", "next-history"),
    ("\\e[C", "forward-char"),
    ("\\e[D", "backward-char"),
    ("\\e[F", "end-of-line"),
    ("\\e[H", "beginning-of-line"),
    ("\\e\\\\", "delete-horizontal-space"),
    ("\\e^", "history-expand-line"),
    ("\\e_", "insert-last-argument"),
    ("\\e_", "yank-last-arg"),
    ("\\eb", "backward-word"),
    ("\\ec", "capitalize-word"),
    ("\\ed", "kill-word"),
    ("\\ef", "forward-word"),
    ("\\eg", "glob-complete-word"),
    ("\\el", "downcase-word"),
    ("\\en", "non-incremental-forward-search-history"),
    ("\\ep", "non-incremental-reverse-search-history"),
    ("\\er", "revert-line"),
    ("\\et", "transpose-words"),
    ("\\eu", "upcase-word"),
    ("\\ex", "execute-named-command"),
    ("\\ey", "yank-pop"),
    ("\\e{", "complete-into-braces"),
    ("\\e~", "complete-username"),
    ("\\e\\C-?", "backward-kill-word"),
    ("\\C-]", "character-search"),
    ("\\C-_", "undo"),
    (" ", "self-insert"),
    ("!", "self-insert"),
    ("\\\"", "self-insert"),
    ("#", "self-insert"),
    ("$", "self-insert"),
    ("%", "self-insert"),
    ("&", "self-insert"),
    ("'", "self-insert"),
    ("(", "self-insert"),
    (")", "self-insert"),
    ("*", "self-insert"),
    ("+", "self-insert"),
    (",", "self-insert"),
    ("-", "self-insert"),
    (".", "self-insert"),
    ("/", "self-insert"),
    ("0", "self-insert"),
    ("1", "self-insert"),
    ("2", "self-insert"),
    ("3", "self-insert"),
    ("4", "self-insert"),
    ("5", "self-insert"),
    ("6", "self-insert"),
    ("7", "self-insert"),
    ("8", "self-insert"),
    ("9", "self-insert"),
    (":", "self-insert"),
    (";", "self-insert"),
    ("<", "self-insert"),
    ("=", "self-insert"),
    (">", "self-insert"),
    ("?", "self-insert"),
    ("@", "self-insert"),
    ("A", "self-insert"),
    ("B", "self-insert"),
    ("C", "self-insert"),
    ("D", "self-insert"),
    ("E", "self-insert"),
    ("F", "self-insert"),
    ("G", "self-insert"),
    ("H", "self-insert"),
    ("I", "self-insert"),
    ("J", "self-insert"),
    ("K", "self-insert"),
    ("L", "self-insert"),
    ("M", "self-insert"),
    ("N", "self-insert"),
    ("O", "self-insert"),
    ("P", "self-insert"),
    ("Q", "self-insert"),
    ("R", "self-insert"),
    ("S", "self-insert"),
    ("T", "self-insert"),
    ("U", "self-insert"),
    ("V", "self-insert"),
    ("W", "self-insert"),
    ("X", "self-insert"),
    ("Y", "self-insert"),
    ("Z", "self-insert"),
    ("[", "self-insert"),
    ("\\\\", "self-insert"),
    ("]", "self-insert"),
    ("^", "self-insert"),
    ("_", "self-insert"),
    ("`", "self-insert"),
    ("a", "self-insert"),
    ("b", "self-insert"),
    ("c", "self-insert"),
    ("d", "self-insert"),
    ("e", "self-insert"),
    ("f", "self-insert"),
    ("g", "self-insert"),
    ("h", "self-insert"),
    ("i", "self-insert"),
    ("j", "self-insert"),
    ("k", "self-insert"),
    ("l", "self-insert"),
    ("m", "self-insert"),
    ("n", "self-insert"),
    ("o", "self-insert"),
    ("p", "self-insert"),
    ("q", "self-insert"),
    ("r", "self-insert"),
    ("s", "self-insert"),
    ("t", "self-insert"),
    ("u", "self-insert"),
    ("v", "self-insert"),
    ("w", "self-insert"),
    ("x", "self-insert"),
    ("y", "self-insert"),
    ("z", "self-insert"),
    ("{", "self-insert"),
    ("|", "self-insert"),
    ("}", "self-insert"),
    ("~", "self-insert"),
    ("\\C-?", "backward-delete-char"),
    ("\\200", "self-insert"),
    ("\\201", "self-insert"),
    ("\\202", "self-insert"),
    ("\\203", "self-insert"),
    ("\\204", "self-insert"),
    ("\\205", "self-insert"),
    ("\\206", "self-insert"),
    ("\\207", "self-insert"),
    ("\\210", "self-insert"),
    ("\\211", "self-insert"),
    ("\\212", "self-insert"),
    ("\\213", "self-insert"),
    ("\\214", "self-insert"),
    ("\\215", "self-insert"),
    ("\\216", "self-insert"),
    ("\\217", "self-insert"),
    ("\\220", "self-insert"),
    ("\\221", "self-insert"),
    ("\\222", "self-insert"),
    ("\\223", "self-insert"),
    ("\\224", "self-insert"),
    ("\\225", "self-insert"),
    ("\\226", "self-insert"),
    ("\\227", "self-insert"),
    ("\\230", "self-insert"),
    ("\\231", "self-insert"),
    ("\\232", "self-insert"),
    ("\\233", "self-insert"),
    ("\\234", "self-insert"),
    ("\\235", "self-insert"),
    ("\\236", "self-insert"),
    ("\\237", "self-insert"),
    ("\\240", "self-insert"),
    ("\\241", "self-insert"),
    ("\\242", "self-insert"),
    ("\\243", "self-insert"),
    ("\\244", "self-insert"),
    ("\\245", "self-insert"),
    ("\\246", "self-insert"),
    ("\\247", "self-insert"),
    ("\\250", "self-insert"),
    ("\\251", "self-insert"),
    ("\\252", "self-insert"),
    ("\\253", "self-insert"),
    ("\\254", "self-insert"),
    ("\\255", "self-insert"),
    ("\\256", "self-insert"),
    ("\\257", "self-insert"),
    ("\\260", "self-insert"),
    ("\\261", "self-insert"),
    ("\\262", "self-insert"),
    ("\\263", "self-insert"),
    ("\\264", "self-insert"),
    ("\\265", "self-insert"),
    ("\\266", "self-insert"),
    ("\\267", "self-insert"),
    ("\\270", "self-insert"),
    ("\\271", "self-insert"),
    ("\\272", "self-insert"),
    ("\\273", "self-insert"),
    ("\\274", "self-insert"),
    ("\\275", "self-insert"),
    ("\\276", "self-insert"),
    ("\\277", "self-insert"),
    ("\\300", "self-insert"),
    ("\\301", "self-insert"),
    ("\\302", "self-insert"),
    ("\\303", "self-insert"),
    ("\\304", "self-insert"),
    ("\\305", "self-insert"),
    ("\\306", "self-insert"),
    ("\\307", "self-insert"),
    ("\\310", "self-insert"),
    ("\\311", "self-insert"),
    ("\\312", "self-insert"),
    ("\\313", "self-insert"),
    ("\\314", "self-insert"),
    ("\\315", "self-insert"),
    ("\\316", "self-insert"),
    ("\\317", "self-insert"),
    ("\\320", "self-insert"),
    ("\\321", "self-insert"),
    ("\\322", "self-insert"),
    ("\\323", "self-insert"),
    ("\\324", "self-insert"),
    ("\\325", "self-insert"),
    ("\\326", "self-insert"),
    ("\\327", "self-insert"),
    ("\\330", "self-insert"),
    ("\\331", "self-insert"),
    ("\\332", "self-insert"),
    ("\\333", "self-insert"),
    ("\\334", "self-insert"),
    ("\\335", "self-insert"),
    ("\\336", "self-insert"),
    ("\\337", "self-insert"),
    ("\\340", "self-insert"),
    ("\\341", "self-insert"),
    ("\\342", "self-insert"),
    ("\\343", "self-insert"),
    ("\\344", "self-insert"),
    ("\\345", "self-insert"),
    ("\\346", "self-insert"),
    ("\\347", "self-insert"),
    ("\\350", "self-insert"),
    ("\\351", "self-insert"),
    ("\\352", "self-insert"),
    ("\\353", "self-insert"),
    ("\\354", "self-insert"),
    ("\\355", "self-insert"),
    ("\\356", "self-insert"),
    ("\\357", "self-insert"),
    ("\\360", "self-insert"),
    ("\\361", "self-insert"),
    ("\\362", "self-insert"),
    ("\\363", "self-insert"),
    ("\\364", "self-insert"),
    ("\\365", "self-insert"),
    ("\\366", "self-insert"),
    ("\\367", "self-insert"),
    ("\\370", "self-insert"),
    ("\\371", "self-insert"),
    ("\\372", "self-insert"),
    ("\\373", "self-insert"),
    ("\\374", "self-insert"),
    ("\\375", "self-insert"),
    ("\\376", "self-insert"),
    ("\\377", "self-insert"),
];

/// The standalone emacs-meta keymap (the map `bind -m emacs-meta`
/// queries on its own: sequences WITHOUT the `\e` prefix the full
/// emacs table carries). emacs_meta_keymap (lib/readline/emacs_keymap.c)
/// plus readline's initialization, captured from the GNU 5.3.20
/// oracle's `bind -m emacs-meta -p` (rubash#2338), scan order.
pub const DEFAULT_BINDINGS_EMACS_META: &[(&str, &str)] = &[
    ("\\C-b", "shell-backward-word"),
    ("\\C-d", "shell-kill-word"),
    ("\\C-e", "shell-expand-line"),
    ("\\C-f", "shell-forward-word"),
    ("\\C-g", "abort"),
    ("\\C-h", "backward-kill-word"),
    ("\\C-i", "dynamic-complete-history"),
    ("\\C-l", "clear-display"),
    ("\\C-r", "revert-line"),
    ("\\C-t", "shell-transpose-words"),
    ("\\C-y", "yank-nth-arg"),
    ("\\e", "complete"),
    ("\\e[C", "forward-word"),
    ("\\e[D", "backward-word"),
    ("\\C-]", "character-search-backward"),
    (" ", "set-mark"),
    ("!", "complete-command"),
    ("#", "insert-comment"),
    ("$", "complete-variable"),
    ("&", "tilde-expand"),
    ("*", "insert-completions"),
    ("-", "digit-argument"),
    (".", "insert-last-argument"),
    (".", "yank-last-arg"),
    ("/", "complete-filename"),
    ("0", "digit-argument"),
    ("1", "digit-argument"),
    ("2", "digit-argument"),
    ("3", "digit-argument"),
    ("4", "digit-argument"),
    ("5", "digit-argument"),
    ("6", "digit-argument"),
    ("7", "digit-argument"),
    ("8", "digit-argument"),
    ("9", "digit-argument"),
    ("<", "beginning-of-history"),
    ("=", "possible-completions"),
    (">", "end-of-history"),
    ("?", "possible-completions"),
    ("@", "complete-hostname"),
    ("A", "do-lowercase-version"),
    ("B", "do-lowercase-version"),
    ("C", "do-lowercase-version"),
    ("D", "do-lowercase-version"),
    ("E", "do-lowercase-version"),
    ("F", "do-lowercase-version"),
    ("G", "do-lowercase-version"),
    ("H", "do-lowercase-version"),
    ("I", "do-lowercase-version"),
    ("J", "do-lowercase-version"),
    ("K", "do-lowercase-version"),
    ("L", "do-lowercase-version"),
    ("M", "do-lowercase-version"),
    ("N", "do-lowercase-version"),
    ("OA", "previous-history"),
    ("OB", "next-history"),
    ("OC", "forward-char"),
    ("OD", "backward-char"),
    ("OF", "end-of-line"),
    ("OH", "beginning-of-line"),
    ("P", "do-lowercase-version"),
    ("Q", "do-lowercase-version"),
    ("R", "do-lowercase-version"),
    ("S", "do-lowercase-version"),
    ("T", "do-lowercase-version"),
    ("U", "do-lowercase-version"),
    ("V", "do-lowercase-version"),
    ("W", "do-lowercase-version"),
    ("X", "do-lowercase-version"),
    ("Y", "do-lowercase-version"),
    ("Z", "do-lowercase-version"),
    ("[1;3C", "forward-word"),
    ("[1;3D", "backward-word"),
    ("[1;5C", "forward-word"),
    ("[1;5D", "backward-word"),
    ("[1~", "beginning-of-line"),
    ("[200~", "bracketed-paste-begin"),
    ("[2~", "quoted-insert"),
    ("[3;5~", "kill-word"),
    ("[3~", "delete-char"),
    ("[4~", "end-of-line"),
    ("[5C", "forward-word"),
    ("[5D", "backward-word"),
    ("[A", "previous-history"),
    ("[B", "next-history"),
    ("[C", "forward-char"),
    ("[D", "backward-char"),
    ("[F", "end-of-line"),
    ("[H", "beginning-of-line"),
    ("\\\\", "delete-horizontal-space"),
    ("^", "history-expand-line"),
    ("_", "insert-last-argument"),
    ("_", "yank-last-arg"),
    ("b", "backward-word"),
    ("c", "capitalize-word"),
    ("d", "kill-word"),
    ("f", "forward-word"),
    ("g", "glob-complete-word"),
    ("l", "downcase-word"),
    ("n", "non-incremental-forward-search-history"),
    ("p", "non-incremental-reverse-search-history"),
    ("r", "revert-line"),
    ("t", "transpose-words"),
    ("u", "upcase-word"),
    ("x", "execute-named-command"),
    ("y", "yank-pop"),
    ("{", "complete-into-braces"),
    ("~", "complete-username"),
    ("\\C-?", "backward-kill-word"),
];

/// The standalone emacs-ctlx keymap (`bind -m emacs-ctlx`: sequences
/// without the `\C-x` prefix). emacs_ctlx_keymap
/// (lib/readline/emacs_keymap.c), captured from the GNU 5.3.20
/// oracle's `bind -m emacs-ctlx -p` (rubash#2338), scan order.
pub const DEFAULT_BINDINGS_EMACS_CTLX: &[(&str, &str)] = &[
    ("\\C-e", "edit-and-execute-command"),
    ("\\C-g", "abort"),
    ("\\C-r", "re-read-init-file"),
    ("\\C-u", "undo"),
    ("\\C-v", "display-shell-version"),
    ("\\C-x", "exchange-point-and-mark"),
    ("!", "possible-command-completions"),
    ("$", "possible-variable-completions"),
    ("(", "start-kbd-macro"),
    (")", "end-kbd-macro"),
    ("*", "glob-expand-word"),
    ("/", "possible-filename-completions"),
    ("@", "possible-hostname-completions"),
    ("A", "do-lowercase-version"),
    ("B", "do-lowercase-version"),
    ("C", "do-lowercase-version"),
    ("D", "do-lowercase-version"),
    ("E", "do-lowercase-version"),
    ("F", "do-lowercase-version"),
    ("G", "do-lowercase-version"),
    ("H", "do-lowercase-version"),
    ("I", "do-lowercase-version"),
    ("J", "do-lowercase-version"),
    ("K", "do-lowercase-version"),
    ("L", "do-lowercase-version"),
    ("M", "do-lowercase-version"),
    ("N", "do-lowercase-version"),
    ("O", "do-lowercase-version"),
    ("P", "do-lowercase-version"),
    ("Q", "do-lowercase-version"),
    ("R", "do-lowercase-version"),
    ("S", "do-lowercase-version"),
    ("T", "do-lowercase-version"),
    ("U", "do-lowercase-version"),
    ("V", "do-lowercase-version"),
    ("W", "do-lowercase-version"),
    ("X", "do-lowercase-version"),
    ("Y", "do-lowercase-version"),
    ("Z", "do-lowercase-version"),
    ("e", "call-last-kbd-macro"),
    ("g", "glob-list-expansions"),
    ("s", "spell-correct-word"),
    ("~", "possible-username-completions"),
    ("\\C-?", "backward-kill-line"),
];

/// The vi-insert keymap's default function bindings
/// (vi_insertion_keymap, lib/readline/vi_keymap.c, plus the arrow
/// keys bind_arrow_keys_internal adds to it), captured from the GNU
/// 5.3.20 oracle's `bind -m vi-insert -p` (rubash#2338), scan order.
/// The bare `\e` row is vi-movement-mode: in the C map ESC is both
/// that function (the submap's ANYOTHERKEY slot) and the arrow-key
/// prefix; the flattened dump form keeps both rows.
pub const DEFAULT_BINDINGS_VI_INSERT: &[(&str, &str)] = &[
    ("\\C-a", "self-insert"),
    ("\\C-b", "self-insert"),
    ("\\C-c", "self-insert"),
    ("\\C-d", "vi-eof-maybe"),
    ("\\C-e", "self-insert"),
    ("\\C-f", "self-insert"),
    ("\\C-g", "self-insert"),
    ("\\C-h", "backward-delete-char"),
    ("\\C-i", "complete"),
    ("\\C-j", "accept-line"),
    ("\\C-k", "self-insert"),
    ("\\C-l", "self-insert"),
    ("\\C-m", "accept-line"),
    ("\\C-n", "menu-complete"),
    ("\\C-o", "self-insert"),
    ("\\C-p", "menu-complete-backward"),
    ("\\C-q", "self-insert"),
    ("\\C-r", "reverse-search-history"),
    ("\\C-s", "forward-search-history"),
    ("\\C-t", "transpose-chars"),
    ("\\C-u", "unix-line-discard"),
    ("\\C-v", "quoted-insert"),
    ("\\C-w", "vi-unix-word-rubout"),
    ("\\C-x", "self-insert"),
    ("\\C-y", "yank"),
    ("\\C-z", "self-insert"),
    ("\\e", "vi-movement-mode"),
    ("\\eOA", "previous-history"),
    ("\\eOB", "next-history"),
    ("\\eOC", "forward-char"),
    ("\\eOD", "backward-char"),
    ("\\eOF", "end-of-line"),
    ("\\eOH", "beginning-of-line"),
    ("\\e[1;3C", "forward-word"),
    ("\\e[1;3D", "backward-word"),
    ("\\e[1;5C", "forward-word"),
    ("\\e[1;5D", "backward-word"),
    ("\\e[200~", "bracketed-paste-begin"),
    ("\\e[3;5~", "kill-word"),
    ("\\e[A", "previous-history"),
    ("\\e[B", "next-history"),
    ("\\e[C", "forward-char"),
    ("\\e[D", "backward-char"),
    ("\\e[F", "end-of-line"),
    ("\\e[H", "beginning-of-line"),
    ("\\C-\\\\", "self-insert"),
    ("\\C-]", "self-insert"),
    ("\\C-^", "self-insert"),
    ("\\C-_", "vi-undo"),
    (" ", "self-insert"),
    ("!", "self-insert"),
    ("\\\"", "self-insert"),
    ("#", "self-insert"),
    ("$", "self-insert"),
    ("%", "self-insert"),
    ("&", "self-insert"),
    ("'", "self-insert"),
    ("(", "self-insert"),
    (")", "self-insert"),
    ("*", "self-insert"),
    ("+", "self-insert"),
    (",", "self-insert"),
    ("-", "self-insert"),
    (".", "self-insert"),
    ("/", "self-insert"),
    ("0", "self-insert"),
    ("1", "self-insert"),
    ("2", "self-insert"),
    ("3", "self-insert"),
    ("4", "self-insert"),
    ("5", "self-insert"),
    ("6", "self-insert"),
    ("7", "self-insert"),
    ("8", "self-insert"),
    ("9", "self-insert"),
    (":", "self-insert"),
    (";", "self-insert"),
    ("<", "self-insert"),
    ("=", "self-insert"),
    (">", "self-insert"),
    ("?", "self-insert"),
    ("@", "self-insert"),
    ("A", "self-insert"),
    ("B", "self-insert"),
    ("C", "self-insert"),
    ("D", "self-insert"),
    ("E", "self-insert"),
    ("F", "self-insert"),
    ("G", "self-insert"),
    ("H", "self-insert"),
    ("I", "self-insert"),
    ("J", "self-insert"),
    ("K", "self-insert"),
    ("L", "self-insert"),
    ("M", "self-insert"),
    ("N", "self-insert"),
    ("O", "self-insert"),
    ("P", "self-insert"),
    ("Q", "self-insert"),
    ("R", "self-insert"),
    ("S", "self-insert"),
    ("T", "self-insert"),
    ("U", "self-insert"),
    ("V", "self-insert"),
    ("W", "self-insert"),
    ("X", "self-insert"),
    ("Y", "self-insert"),
    ("Z", "self-insert"),
    ("[", "self-insert"),
    ("\\\\", "self-insert"),
    ("]", "self-insert"),
    ("^", "self-insert"),
    ("_", "self-insert"),
    ("`", "self-insert"),
    ("a", "self-insert"),
    ("b", "self-insert"),
    ("c", "self-insert"),
    ("d", "self-insert"),
    ("e", "self-insert"),
    ("f", "self-insert"),
    ("g", "self-insert"),
    ("h", "self-insert"),
    ("i", "self-insert"),
    ("j", "self-insert"),
    ("k", "self-insert"),
    ("l", "self-insert"),
    ("m", "self-insert"),
    ("n", "self-insert"),
    ("o", "self-insert"),
    ("p", "self-insert"),
    ("q", "self-insert"),
    ("r", "self-insert"),
    ("s", "self-insert"),
    ("t", "self-insert"),
    ("u", "self-insert"),
    ("v", "self-insert"),
    ("w", "self-insert"),
    ("x", "self-insert"),
    ("y", "self-insert"),
    ("z", "self-insert"),
    ("{", "self-insert"),
    ("|", "self-insert"),
    ("}", "self-insert"),
    ("~", "self-insert"),
    ("\\C-?", "backward-delete-char"),
    ("\\200", "self-insert"),
    ("\\201", "self-insert"),
    ("\\202", "self-insert"),
    ("\\203", "self-insert"),
    ("\\204", "self-insert"),
    ("\\205", "self-insert"),
    ("\\206", "self-insert"),
    ("\\207", "self-insert"),
    ("\\210", "self-insert"),
    ("\\211", "self-insert"),
    ("\\212", "self-insert"),
    ("\\213", "self-insert"),
    ("\\214", "self-insert"),
    ("\\215", "self-insert"),
    ("\\216", "self-insert"),
    ("\\217", "self-insert"),
    ("\\220", "self-insert"),
    ("\\221", "self-insert"),
    ("\\222", "self-insert"),
    ("\\223", "self-insert"),
    ("\\224", "self-insert"),
    ("\\225", "self-insert"),
    ("\\226", "self-insert"),
    ("\\227", "self-insert"),
    ("\\230", "self-insert"),
    ("\\231", "self-insert"),
    ("\\232", "self-insert"),
    ("\\233", "self-insert"),
    ("\\234", "self-insert"),
    ("\\235", "self-insert"),
    ("\\236", "self-insert"),
    ("\\237", "self-insert"),
    ("\\240", "self-insert"),
    ("\\241", "self-insert"),
    ("\\242", "self-insert"),
    ("\\243", "self-insert"),
    ("\\244", "self-insert"),
    ("\\245", "self-insert"),
    ("\\246", "self-insert"),
    ("\\247", "self-insert"),
    ("\\250", "self-insert"),
    ("\\251", "self-insert"),
    ("\\252", "self-insert"),
    ("\\253", "self-insert"),
    ("\\254", "self-insert"),
    ("\\255", "self-insert"),
    ("\\256", "self-insert"),
    ("\\257", "self-insert"),
    ("\\260", "self-insert"),
    ("\\261", "self-insert"),
    ("\\262", "self-insert"),
    ("\\263", "self-insert"),
    ("\\264", "self-insert"),
    ("\\265", "self-insert"),
    ("\\266", "self-insert"),
    ("\\267", "self-insert"),
    ("\\270", "self-insert"),
    ("\\271", "self-insert"),
    ("\\272", "self-insert"),
    ("\\273", "self-insert"),
    ("\\274", "self-insert"),
    ("\\275", "self-insert"),
    ("\\276", "self-insert"),
    ("\\277", "self-insert"),
    ("\\300", "self-insert"),
    ("\\301", "self-insert"),
    ("\\302", "self-insert"),
    ("\\303", "self-insert"),
    ("\\304", "self-insert"),
    ("\\305", "self-insert"),
    ("\\306", "self-insert"),
    ("\\307", "self-insert"),
    ("\\310", "self-insert"),
    ("\\311", "self-insert"),
    ("\\312", "self-insert"),
    ("\\313", "self-insert"),
    ("\\314", "self-insert"),
    ("\\315", "self-insert"),
    ("\\316", "self-insert"),
    ("\\317", "self-insert"),
    ("\\320", "self-insert"),
    ("\\321", "self-insert"),
    ("\\322", "self-insert"),
    ("\\323", "self-insert"),
    ("\\324", "self-insert"),
    ("\\325", "self-insert"),
    ("\\326", "self-insert"),
    ("\\327", "self-insert"),
    ("\\330", "self-insert"),
    ("\\331", "self-insert"),
    ("\\332", "self-insert"),
    ("\\333", "self-insert"),
    ("\\334", "self-insert"),
    ("\\335", "self-insert"),
    ("\\336", "self-insert"),
    ("\\337", "self-insert"),
    ("\\340", "self-insert"),
    ("\\341", "self-insert"),
    ("\\342", "self-insert"),
    ("\\343", "self-insert"),
    ("\\344", "self-insert"),
    ("\\345", "self-insert"),
    ("\\346", "self-insert"),
    ("\\347", "self-insert"),
    ("\\350", "self-insert"),
    ("\\351", "self-insert"),
    ("\\352", "self-insert"),
    ("\\353", "self-insert"),
    ("\\354", "self-insert"),
    ("\\355", "self-insert"),
    ("\\356", "self-insert"),
    ("\\357", "self-insert"),
    ("\\360", "self-insert"),
    ("\\361", "self-insert"),
    ("\\362", "self-insert"),
    ("\\363", "self-insert"),
    ("\\364", "self-insert"),
    ("\\365", "self-insert"),
    ("\\366", "self-insert"),
    ("\\367", "self-insert"),
    ("\\370", "self-insert"),
    ("\\371", "self-insert"),
    ("\\372", "self-insert"),
    ("\\373", "self-insert"),
    ("\\374", "self-insert"),
    ("\\375", "self-insert"),
    ("\\376", "self-insert"),
    ("\\377", "self-insert"),
];

/// The vi movement (command) keymap's default function bindings
/// (vi_movement_keymap, lib/readline/vi_keymap.c, plus arrow keys) —
/// the map `-m vi`, `-m vi-move` and `-m vi-command` all name.
/// Captured from the GNU 5.3.20 oracle's `bind -m vi-command -p`
/// (rubash#2338), scan order.
pub const DEFAULT_BINDINGS_VI_MOVEMENT: &[(&str, &str)] = &[
    ("\\C-d", "vi-eof-maybe"),
    ("\\C-g", "abort"),
    ("\\C-h", "backward-char"),
    ("\\C-j", "accept-line"),
    ("\\C-k", "kill-line"),
    ("\\C-l", "clear-screen"),
    ("\\C-m", "accept-line"),
    ("\\C-n", "next-history"),
    ("\\C-p", "previous-history"),
    ("\\C-q", "quoted-insert"),
    ("\\C-r", "reverse-search-history"),
    ("\\C-s", "forward-search-history"),
    ("\\C-t", "transpose-chars"),
    ("\\C-u", "unix-line-discard"),
    ("\\C-v", "quoted-insert"),
    ("\\C-w", "vi-unix-word-rubout"),
    ("\\C-y", "yank"),
    ("\\eOA", "previous-history"),
    ("\\eOB", "next-history"),
    ("\\eOC", "forward-char"),
    ("\\eOD", "backward-char"),
    ("\\eOF", "end-of-line"),
    ("\\eOH", "beginning-of-line"),
    ("\\e[1;3C", "forward-word"),
    ("\\e[1;3D", "backward-word"),
    ("\\e[1;5C", "forward-word"),
    ("\\e[1;5D", "backward-word"),
    ("\\e[3;5~", "kill-word"),
    ("\\e[A", "previous-history"),
    ("\\e[B", "next-history"),
    ("\\e[C", "forward-char"),
    ("\\e[D", "backward-char"),
    ("\\e[F", "end-of-line"),
    ("\\e[H", "beginning-of-line"),
    ("\\C-_", "vi-undo"),
    (" ", "forward-char"),
    ("#", "insert-comment"),
    ("$", "end-of-line"),
    ("%", "vi-match"),
    ("&", "vi-tilde-expand"),
    ("*", "bash-vi-complete"),
    ("+", "next-history"),
    (",", "vi-char-search"),
    ("-", "previous-history"),
    (".", "vi-redo"),
    ("/", "vi-search"),
    ("0", "beginning-of-line"),
    ("1", "vi-arg-digit"),
    ("2", "vi-arg-digit"),
    ("3", "vi-arg-digit"),
    ("4", "vi-arg-digit"),
    ("5", "vi-arg-digit"),
    ("6", "vi-arg-digit"),
    ("7", "vi-arg-digit"),
    ("8", "vi-arg-digit"),
    ("9", "vi-arg-digit"),
    (";", "vi-char-search"),
    ("=", "bash-vi-complete"),
    ("?", "vi-search"),
    ("A", "vi-append-eol"),
    ("B", "vi-prev-word"),
    ("C", "vi-change-to"),
    ("D", "vi-delete-to"),
    ("E", "vi-end-word"),
    ("F", "vi-char-search"),
    ("G", "vi-fetch-history"),
    ("I", "vi-insert-beg"),
    ("N", "vi-search-again"),
    ("P", "vi-put"),
    ("R", "vi-replace"),
    ("S", "vi-subst"),
    ("T", "vi-char-search"),
    ("U", "revert-line"),
    ("W", "vi-next-word"),
    ("X", "vi-rubout"),
    ("Y", "vi-yank-to"),
    ("\\\\", "bash-vi-complete"),
    ("^", "vi-first-print"),
    ("_", "vi-yank-arg"),
    ("`", "vi-goto-mark"),
    ("a", "vi-append-mode"),
    ("b", "vi-prev-word"),
    ("c", "vi-change-to"),
    ("d", "vi-delete-to"),
    ("e", "vi-end-word"),
    ("f", "vi-char-search"),
    ("h", "backward-char"),
    ("i", "vi-insertion-mode"),
    ("j", "next-history"),
    ("k", "previous-history"),
    ("l", "forward-char"),
    ("m", "vi-set-mark"),
    ("n", "vi-search-again"),
    ("p", "vi-put"),
    ("r", "vi-change-char"),
    ("s", "vi-subst"),
    ("t", "vi-char-search"),
    ("u", "vi-undo"),
    ("v", "vi-edit-and-execute-command"),
    ("w", "vi-next-word"),
    ("x", "vi-delete"),
    ("y", "vi-yank-to"),
    ("|", "vi-column"),
    ("~", "vi-change-case"),
];

/// The default function-binding table for one keymap name (the `-m`
/// names of bind.def:37-40, aliases folded by
/// [`crate::shell::bind_registry::normalize_keymap_name`]). `None` (no
/// `-m`) is the current keymap, emacs-standard in the default editing
/// mode. Every name in
/// [`crate::shell::bind_registry::KEYMAP_NAMES`] resolves here.
pub fn default_bindings_for_keymap(
    keymap: Option<&str>,
) -> &'static [(&'static str, &'static str)] {
    match crate::shell::bind_registry::normalize_keymap_name(keymap.unwrap_or("emacs-standard")) {
        "emacs-meta" => DEFAULT_BINDINGS_EMACS_META,
        "emacs-ctlx" => DEFAULT_BINDINGS_EMACS_CTLX,
        "vi-insert" => DEFAULT_BINDINGS_VI_INSERT,
        "vi-movement" => DEFAULT_BINDINGS_VI_MOVEMENT,
        _ => DEFAULT_FUNCTION_BINDINGS,
    }
}

/// Translate a written key sequence (`\C-b`, `\eOD`, `\e[D`, plain
/// characters) to the raw bytes it dispatches on — the inverse of
/// readline's `_rl_get_keyname` rendering, used to order sequences the
/// way rl_invoking_keyseqs_in_map's byte scan does (lib/readline/bind.c:
/// keys 0..255, submaps recursed in place, so plain byte order of the
/// translated sequence IS the listing order). Inputrc spellings the
/// dumpers never print are accepted too (`\d` for DEL, `\M-x` for the
/// ESC-prefixed meta form, the C-style letter escapes, `\xHH`), since
/// user-registered sequences arrive in the user's own spelling.
pub fn keyseq_bytes(written: &str) -> Vec<u8> {
    let chars: Vec<char> = written.chars().collect();
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0usize;
    // The base byte of a \C- or \M- argument: one character, or an
    // escape pair when the base is itself backslash/quote (\C-\\).
    let read_base = |chars: &[char], i: &mut usize| -> u32 {
        match chars.get(*i) {
            Some('\\') => {
                let base = match chars.get(*i + 1) {
                    Some('\\') => 0x5c,
                    Some('"') => 0x22,
                    _ => 0x5c,
                };
                *i += 2;
                base
            }
            Some(&c) => {
                *i += 1;
                c as u32
            }
            None => 0,
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            i += 1;
            continue;
        }
        match chars.get(i + 1) {
            Some('C') if chars.get(i + 2) == Some(&'-') => {
                i += 3;
                let base = read_base(&chars, &mut i);
                out.push(if base == '?' as u32 {
                    0x7f
                } else {
                    (base & 0x1f) as u8
                });
            }
            Some('M') if chars.get(i + 2) == Some(&'-') => {
                i += 3;
                let base = read_base(&chars, &mut i);
                out.push(0x1b);
                out.push(base as u8);
            }
            Some('e') => {
                out.push(0x1b);
                i += 2;
            }
            Some('d') => {
                out.push(0x7f);
                i += 2;
            }
            Some('\\') => {
                out.push(0x5c);
                i += 2;
            }
            Some('a') => {
                out.push(0x07);
                i += 2;
            }
            Some('b') => {
                out.push(0x08);
                i += 2;
            }
            Some('f') => {
                out.push(0x0c);
                i += 2;
            }
            Some('n') => {
                out.push(0x0a);
                i += 2;
            }
            Some('r') => {
                out.push(0x0d);
                i += 2;
            }
            Some('t') => {
                out.push(0x09);
                i += 2;
            }
            Some('v') => {
                out.push(0x0b);
                i += 2;
            }
            Some('x') => {
                let hex: String = chars[i + 2..].iter().take(2).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(byte) if !hex.is_empty() => {
                        out.push(byte);
                        i += 2 + hex.len();
                    }
                    _ => {
                        out.push(b'x');
                        i += 2;
                    }
                }
            }
            Some(d) if d.is_ascii_digit() => {
                let digits: String = chars[i + 1..]
                    .iter()
                    .take(3)
                    .take_while(|d| d.is_ascii_digit())
                    .collect();
                match u32::from_str_radix(&digits, 8) {
                    Ok(value) => {
                        out.push(value as u8);
                        i += 1 + digits.len();
                    }
                    Err(_) => {
                        out.push(*d as u8);
                        i += 2;
                    }
                }
            }
            Some(&other) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
                i += 2;
            }
            None => {
                out.push(0x5c);
                i += 1;
            }
        }
    }
    out
}

/// The errno text GNU's `bind -f` failure prints: `strerror(errno)`
/// (bind.def:274-276). std's io::Error Display appends ` (os error N)`
/// to that same text; strip the suffix so the diagnostic is GNU's
/// (rubash#2338 item 4).
pub fn cannot_read_reason(err: &std::io::Error) -> String {
    let text = err.to_string();
    match text.rfind(" (os error ") {
        Some(pos) if text.ends_with(')') => text[..pos].to_string(),
        _ => text,
    }
}

/// GNU 5.3's readline variable table in `bind -v` output order, with each
/// variable's default value (WT probe: WSL GNU bash 5.3.0 `bind -v`,
/// 48 lines). Booleans hold `on`/`off`, numbers decimal integers, strings
/// verbatim. `editing-mode` and `keymap` are the two variables GNU keeps
/// cross-consistent: `set editing-mode vi` also switches `keymap` to
/// `vi-insert` (GNU probe) — [`apply_readline_variable`] maintains that.
pub const READLINE_VARIABLE_DEFAULTS: &[(&str, &str)] = &[
    ("bind-tty-special-chars", "off"),
    ("blink-matching-paren", "off"),
    ("byte-oriented", "off"),
    ("colored-completion-prefix", "off"),
    ("colored-stats", "off"),
    ("completion-ignore-case", "off"),
    ("completion-map-case", "off"),
    ("convert-meta", "off"),
    ("disable-completion", "off"),
    ("echo-control-characters", "on"),
    ("enable-active-region", "off"),
    ("enable-bracketed-paste", "off"),
    ("enable-keypad", "off"),
    ("enable-meta-key", "on"),
    ("expand-tilde", "off"),
    ("force-meta-prefix", "off"),
    ("history-preserve-point", "off"),
    ("horizontal-scroll-mode", "off"),
    ("input-meta", "on"),
    ("mark-directories", "on"),
    ("mark-modified-lines", "off"),
    ("mark-symlinked-directories", "off"),
    ("match-hidden-files", "on"),
    ("menu-complete-display-prefix", "off"),
    ("meta-flag", "on"),
    ("output-meta", "on"),
    ("page-completions", "on"),
    ("prefer-visible-bell", "on"),
    ("print-completions-horizontally", "off"),
    ("revert-all-at-newline", "off"),
    ("search-ignore-case", "off"),
    ("show-all-if-ambiguous", "off"),
    ("show-all-if-unmodified", "off"),
    ("show-mode-in-prompt", "off"),
    ("skip-completed-text", "off"),
    ("visible-stats", "off"),
    ("bell-style", "audible"),
    ("comment-begin", "#"),
    ("completion-display-width", "-1"),
    ("completion-prefix-display-length", "0"),
    ("completion-query-items", "100"),
    ("editing-mode", "emacs"),
    ("emacs-mode-string", "@"),
    ("history-size", "-1"),
    ("keymap", "emacs"),
    ("keyseq-timeout", "500"),
    ("vi-cmd-mode-string", "(cmd)"),
    ("vi-ins-mode-string", "(ins)"),
];

/// How one `set NAME VALUE` resolved against the readline variable table
/// (readline bind.c rl_variable_bind):
/// - [`SetVariableOutcome::Applied`] — stored (booleans normalize
///   non-`on` values to `off` and numbers non-numeric text to `0`
///   silently, GNU probes: `set completion-ignore-case maybe` → `off`,
///   `set keyseq-timeout abc` → `0`; `set keymap bogus` keeps the old
///   keymap silently).
/// - [`SetVariableOutcome::UnknownVariable`] — GNU prints
///   ``readline: NAME: unknown variable name`` (probe) and continues.
/// - [`SetVariableOutcome::CouldNotSetValue`] — only `editing-mode`
///   rejects values loudly: ``readline: editing-mode: could not set
///   value to `bogus'`` (probe), variable unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetVariableOutcome {
    Applied,
    UnknownVariable,
    CouldNotSetValue,
}

/// Apply `set NAME VALUE` to a `bind -v` variable table (name → value, in
/// [`READLINE_VARIABLE_DEFAULTS`] order). Returns the outcome plus the
/// normalized value/keymap a caller may need to mirror (editing-mode
/// lowercases and switches `keymap`; GNU probe: `set editing-mode VI`
/// stores `vi` and `set editing-mode vi` stores `keymap vi-insert`).
pub fn apply_readline_variable(
    variables: &mut Vec<(String, String)>,
    name: &str,
    value: &str,
) -> (SetVariableOutcome, Option<(&'static str, &'static str)>) {
    let Some((_, kind)) = variable_kind(name) else {
        return (SetVariableOutcome::UnknownVariable, None);
    };
    match kind {
        VariableKind::EditingMode => {
            let lowered = value.to_ascii_lowercase();
            let mode = match lowered.as_str() {
                "emacs" => "emacs",
                "vi" => "vi",
                _ => {
                    return (SetVariableOutcome::CouldNotSetValue, None);
                }
            };
            let keymap = if mode == "vi" { "vi-insert" } else { "emacs" };
            store_variable(variables, name, mode);
            store_variable(variables, "keymap", keymap);
            (
                SetVariableOutcome::Applied,
                Some(("editing-mode", if mode == "vi" { "vi" } else { "emacs" })),
            )
        }
        VariableKind::Keymap => {
            // A bad keymap name keeps the current value silently (GNU
            // probe: `set keymap bogus` → keymap stays emacs, no message).
            if crate::shell::bind_registry::KEYMAP_NAMES.contains(&value) {
                store_variable(variables, name, value);
            }
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::Boolean => {
            let normalized = if value.eq_ignore_ascii_case("on") {
                "on"
            } else {
                "off"
            };
            store_variable(variables, name, normalized);
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::Number => {
            let normalized = strtol_i64(value);
            store_variable(variables, name, &normalized.to_string());
            (SetVariableOutcome::Applied, None)
        }
        VariableKind::String => {
            store_variable(variables, name, value);
            (SetVariableOutcome::Applied, None)
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VariableKind {
    Boolean,
    Number,
    String,
    EditingMode,
    Keymap,
}

fn variable_kind(name: &str) -> Option<(&'static str, VariableKind)> {
    READLINE_VARIABLE_DEFAULTS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(candidate, _)| (*candidate, kind_of(candidate)))
}

fn kind_of(name: &str) -> VariableKind {
    match name {
        "editing-mode" => VariableKind::EditingMode,
        "keymap" => VariableKind::Keymap,
        "bell-style" | "comment-begin" | "emacs-mode-string" | "vi-cmd-mode-string"
        | "vi-ins-mode-string" => VariableKind::String,
        "completion-display-width"
        | "completion-prefix-display-length"
        | "completion-query-items"
        | "history-size"
        | "keyseq-timeout" => VariableKind::Number,
        _ => VariableKind::Boolean,
    }
}

fn store_variable(variables: &mut Vec<(String, String)>, name: &str, value: &str) {
    match variables
        .iter_mut()
        .find(|(candidate, _)| candidate == name)
    {
        Some(slot) => slot.1 = value.to_string(),
        None => variables.push((name.to_string(), value.to_string())),
    }
}

/// strtol semantics readline uses for numeric variables: optional
/// whitespace and sign, leading decimal digits, `500abc` → 500, garbage →
/// 0 (GNU probe: `set keyseq-timeout abc` stores 0).
fn strtol_i64(text: &str) -> i64 {
    let trimmed = text.trim_start();
    let mut chars = trimmed.chars().peekable();
    let mut sign = 1i64;
    match chars.peek() {
        Some('+') => {
            chars.next();
        }
        Some('-') => {
            sign = -1;
            chars.next();
        }
        _ => {}
    }
    let mut value: i64 = 0;
    let mut any = false;
    for ch in chars {
        match ch.to_digit(10) {
            Some(digit) => {
                any = true;
                value = value.saturating_mul(10).saturating_add(digit as i64);
            }
            None => break,
        }
    }
    if any {
        sign * value
    } else {
        0
    }
}

/// Grammar failure of a `-x` or inputrc-form binding argument. `message`
/// is the GNU text without the `bind: ` prefix and without the shell's
/// `name: line N: ` diagnostic prefix.
///
/// The two argument channels report failures differently in GNU:
/// - `-x` (bind_keyseq_to_unix_command, bashline.c:4850) fails the
///   builtin with rc 1 under the shell's `bind: ` prefix (GNU probe:
///   `bind -x '"\C-g'` → `bash: bind: no closing `"' in "\C-g`, rc 1).
///   That is `message` alone.
/// - a plain positional goes through rl_parse_and_bind, whose errors
///   print as `readline: …` WITHOUT the shell prefix and do NOT fail the
///   builtin — bind.def:304-333 ignores the return value (GNU probes:
///   `bind '"\C-t'` → ``readline: "\C-t: no closing `"' in key binding``
///   rc 0; `bind '"\C-t"'` → ``readline: "\C-t": no key sequence
///   terminator`` rc 0). `readline_message` carries that text; the
///   caller prints it and leaves the status alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindSpecError {
    pub message: String,
    /// The quote character that never closed (no-closing errors).
    pub delimiter: Option<char>,
    /// The `readline: …` text for the plain-positional channel, when this
    /// error class reports there.
    pub readline_message: Option<String>,
}

impl BindSpecError {
    pub fn message(message: impl Into<String>) -> Self {
        BindSpecError {
            message: message.into(),
            delimiter: None,
            readline_message: None,
        }
    }
}

/// The inputrc-form binding value: an unquoted value names a readline
/// function, a quoted value is a macro (bind.def SHORT_DOC "keyseq:
/// readline-function or readline-command"; the wt100 GNU probe shows
/// `bind '"\C-t": ins-macro'` binds nothing — unknown function name,
/// silently ignored by rl_parse_and_bind — while the quoted form
/// `bind '"\C-t": "ins-macro"'` binds a macro `bind -s` lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingSpec {
    Function { name: String },
    Macro { text: String },
}

/// One parsed plain binding argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBinding {
    pub keyseq: String,
    pub spec: BindingSpec,
}

/// Port of bashline.c:4814 isolate_sequence: skip leading whitespace,
/// require `need_dquote` when told, find the closing unescaped delimiter
/// (`"`, `'`, or none), return the RAW interior plus the index just past
/// the closing quote. Callers decide which escape translation applies
/// (key sequences keep the written `\C-g` form for the host key parser;
/// macro text and whitespace-separated commands go through
/// [`unescape_macro_text`], matching rl_macro_bind's translate path at
/// bashline.c:4893-4896).
fn isolate_sequence(
    line: &str,
    start: usize,
    need_dquote: bool,
) -> Result<(String, usize), BindSpecError> {
    let bytes = line.as_bytes();
    let mut i = start;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() {
        if need_dquote {
            return Err(BindSpecError {
                message: format!("{line}: first non-whitespace character is not `\"'"),
                delimiter: None,
                readline_message: None,
            });
        }
        return Err(BindSpecError {
            message: format!("{line}: missing separator"),
            delimiter: None,
            readline_message: None,
        });
    }
    let delim = matches!(bytes[i], b'"' | b'\'').then(|| bytes[i]);
    if need_dquote && delim != Some(b'"') {
        return Err(BindSpecError {
            message: format!("{line}: first non-whitespace character is not `\"'"),
            delimiter: None,
            readline_message: None,
        });
    }
    let content_start;
    if let Some(d) = delim {
        i += 1;
        content_start = i;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == d {
                break;
            }
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != d {
            return Err(BindSpecError {
                message: format!("no closing `{}' in {line}", d as char),
                delimiter: Some(d as char),
                readline_message: None,
            });
        }
        Ok((line[content_start..i].to_string(), i + 1))
    } else {
        content_start = i;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            i += 1;
        }
        Ok((line[content_start..].to_string(), i))
    }
}

/// Resolve only the delimiter escapes a stored KEY SEQUENCE keeps
/// written: `\\` → `\`, `\"` → `"`; every other sequence (including
/// `\C-g`, `\e[A`, `\M-x`) stays in written form because the host key
/// parser (reedline bridge `parse_key_sequence`) reads that form. GNU
/// translates the sequence to raw bytes for matching and renders it back
/// in this same backslash form on dump (wt100 probe: `bind -X` prints
/// `"\C-g" "echo bound-c-g"`).
pub fn unescape_keyseq(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('\\') => {
                out.push('\\');
                chars.next();
            }
            Some('"') => {
                out.push('"');
                chars.next();
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// readline's \-escape translation for MACRO text and translated command
/// values (rl_translate_keyseq): \n \t \r \e \a \b \f \v, `\\` and quote
/// escapes, \C-x control characters, \M- ESC meta prefixes, and octal
/// \NNN. Unknown escapes keep the backslash.
pub fn unescape_macro_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let next = *chars.peek().unwrap_or(&'\\');
        match next {
            'n' => {
                out.push('\n');
                chars.next();
            }
            't' => {
                out.push('\t');
                chars.next();
            }
            'r' => {
                out.push('\r');
                chars.next();
            }
            'e' => {
                out.push('\x1b');
                chars.next();
            }
            'a' => {
                out.push('\x07');
                chars.next();
            }
            'b' => {
                out.push('\x08');
                chars.next();
            }
            'f' => {
                out.push('\x0c');
                chars.next();
            }
            'v' => {
                out.push('\x0b');
                chars.next();
            }
            '\\' => {
                out.push('\\');
                chars.next();
            }
            '"' => {
                out.push('"');
                chars.next();
            }
            '\'' => {
                out.push('\'');
                chars.next();
            }
            'C' => {
                // \C-x: control character (rl_translate_keyseq's CTL
                // handling). \C-? is DEL.
                chars.next();
                match chars.next() {
                    Some('-') => match chars.next() {
                        Some('?') => out.push('\x7f'),
                        Some(letter) => {
                            let upper = letter.to_ascii_uppercase();
                            out.push((upper as u8 & 0x1f) as char);
                        }
                        None => {}
                    },
                    other => {
                        // Not \C-x — keep written form.
                        out.push_str("\\C");
                        if let Some(ch) = other {
                            out.push(ch);
                        }
                    }
                }
            }
            'M' => {
                // \M-x: meta prefix = ESC before x.
                chars.next();
                match chars.next() {
                    Some('-') => {
                        out.push('\x1b');
                    }
                    other => {
                        out.push_str("\\M");
                        if let Some(ch) = other {
                            out.push(ch);
                        }
                    }
                }
            }
            '0'..='7' => {
                // \NNN octal.
                let mut value: u32 = 0;
                let mut digits = 0;
                while digits < 3 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + digit;
                            chars.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                if let Some(byte) = char::from_u32(value) {
                    out.push(byte);
                }
            }
            _ => out.push('\\'),
        }
    }
    out
}

/// Parse a `bind -x` argument: `"keyseq"[:|ws]["']command["']`
/// (bashline.c:4850 bind_keyseq_to_unix_command). The keyseq MUST be
/// double-quoted; colon or whitespace separates it from the command.
/// Colon separator stores the command verbatim (rl_generic_bind);
/// whitespace separator requires the command to be quoted and gets the
/// readline escape translation (rl_macro_bind path, translate=true at
/// bashline.c:4893-4896).
pub fn parse_unix_command_spec(line: &str) -> Result<(String, String), BindSpecError> {
    let (keyseq_raw, mut i) = isolate_sequence(line, 0, true)?;
    let keyseq = unescape_keyseq(&keyseq_raw);
    // Scan forward to the separator: colon or whitespace
    // (bashline.c:4877-4879).
    let bytes = line.as_bytes();
    while i < bytes.len() && bytes[i] != b':' && bytes[i] != b' ' && bytes[i] != b'\t' {
        i += 1;
    }
    if i >= bytes.len() {
        return Err(BindSpecError {
            message: format!("{line}: missing separator"),
            delimiter: None,
            readline_message: None,
        });
    }
    if bytes[i] == b':' {
        // Colon separator: the command is stored verbatim
        // (rl_generic_bind, bashline.c:4898-4900).
        let (command, _) = isolate_sequence(line, i + 1, false)?;
        Ok((keyseq, command))
    } else {
        // Whitespace separator: command must be quoted and gets the
        // readline escape translation (rl_macro_bind path,
        // bashline.c:4893-4896).
        let (command_raw, _) = isolate_sequence(line, i, true)?;
        Ok((keyseq, unescape_macro_text(&command_raw)))
    }
}

/// Parse a plain inputrc-form binding argument `"keyseq": value`
/// (rl_parse_and_bind). Quoted value → macro; unquoted value → function
/// name (GNU: `bind '"\C-t": ins-macro'` binds nothing — an unknown
/// function name is silently dropped; see wt100 probe). Returns None
/// when the argument is not this form (non-quote, non-`set` text —
/// rl_parse_and_bind ignores it silently, GNU probe `bind 'ls -l'` rc 0).
///
/// Failures carry `readline_message` (see [`BindSpecError`]): GNU shapes
/// from the wt100-era probes — ``"\C-t: no closing `"' in key binding``,
/// `"\C-t": no key sequence terminator`, ``'ins: missing closing quote
/// for macro`` — printed after `readline: ` with no effect on status.
pub fn parse_binding_line(line: &str) -> Option<Result<ParsedBinding, BindSpecError>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('"') {
        return None;
    }
    let (keyseq_raw, mut i) = match isolate_sequence(line, 0, true) {
        Ok(parsed) => parsed,
        Err(err) => {
            let readline_message = err
                .delimiter
                .map(|delimiter| format!("{line}: no closing `{delimiter}' in key binding"));
            return Some(Err(BindSpecError {
                message: err.message,
                delimiter: err.delimiter,
                readline_message,
            }));
        }
    };
    let keyseq = unescape_keyseq(&keyseq_raw);
    let bytes = line.as_bytes();
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b':' {
        i += 1;
    }
    // Quoted value → macro (rl_parse_and_bind's ISMACR path); anything
    // else is a function name, taken verbatim to end of line.
    let mut probe = i;
    while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
        probe += 1;
    }
    if probe < bytes.len() && (bytes[probe] == b'"' || bytes[probe] == b'\'') {
        let macro_text_start = probe;
        // isolate_sequence auto-detects the quote character (GNU passes
        // need_dquote=0 here; either quote works as the macro delimiter).
        let (text_raw, _) = match isolate_sequence(line, i, false) {
            Ok(parsed) => parsed,
            Err(err) => {
                let readline_message = if err.delimiter.is_some() {
                    Some(format!(
                        "{}: missing closing quote for macro",
                        line[macro_text_start..].trim_end()
                    ))
                } else {
                    None
                };
                return Some(Err(BindSpecError {
                    message: err.message,
                    delimiter: err.delimiter,
                    readline_message,
                }));
            }
        };
        Some(Ok(ParsedBinding {
            keyseq,
            spec: BindingSpec::Macro {
                text: unescape_macro_text(&text_raw),
            },
        }))
    } else {
        let (name, _) = match isolate_sequence(line, i, false) {
            Ok(parsed) => parsed,
            Err(err) => {
                // The keyseq closed but no `: value` followed — GNU
                // readline: `"\C-t": no key sequence terminator`.
                let readline_message = (err.message.ends_with("missing separator"))
                    .then(|| format!("{line}: no key sequence terminator"));
                return Some(Err(BindSpecError {
                    message: err.message,
                    delimiter: None,
                    readline_message,
                }));
            }
        };
        Some(Ok(ParsedBinding {
            keyseq,
            spec: BindingSpec::Function { name },
        }))
    }
}

/// True when `name` is in the supported function table.
pub fn is_known_function(name: &str) -> bool {
    READLINE_FUNCTIONS.contains(&name)
}

/// `bind` usage text (builtins/bind.def SHORT_DOC).
pub fn usage_line() -> &'static str {
    "bind: usage: bind [-lpsvPSVX] [-m keymap] [-f filename] [-q name] [-u name] [-r keyseq] [-x keyseq:shell-command] [keyseq:readline-function or readline-command]"
}

/// Write the usage line (bind.def default: builtin_usage → EX_USAGE).
pub fn write_usage<E>(stderr: &mut E) -> io::Result<()>
where
    E: Write,
{
    writeln!(stderr, "{}", usage_line())
}

/// `bind -X` line: `"\C-g" "command"` (print_unix_command,
/// bashline.c:4720-4727, readable form).
pub fn format_unix_command_readable(keyseq: &str, command: &str) -> String {
    format!("\"{keyseq}\" \"{command}\"")
}

/// `bind -s` line: `"\C-t": "ins-macro"` (rl_macro_dumper readable).
pub fn format_macro_readable(keyseq: &str, text: &str) -> String {
    format!("\"{keyseq}\": \"{text}\"")
}

/// `bind -S` line: `\C-t outputs ins-macro` (GNU probe; rl_macro_dumper
/// plain form).
pub fn format_macro_plain(keyseq: &str, text: &str) -> String {
    format!("{keyseq} outputs {text}")
}

/// `bind -p` line: `"\C-a": beginning-of-line` (rl_function_dumper(1)).
pub fn format_function_readable(keyseq: &str, name: &str) -> String {
    format!("\"{keyseq}\": {name}")
}

/// `bind -v` line: `set completion-ignore-case on`
/// (rl_variable_dumper(1) readable form; WSL GNU 5.3 probe).
pub fn format_variable_readable(name: &str, value: &str) -> String {
    format!("set {name} {value}")
}

/// `bind -V` line: ``completion-ignore-case is set to `off'``
/// (rl_variable_dumper(0) plain form; WSL GNU 5.3 probe).
pub fn format_variable_plain(name: &str, value: &str) -> String {
    format!("{name} is set to `{value}'")
}

/// `bind -P` line (rl_print_keybinding, print_readably = 0): at most
/// five sequences, `, ...` when truncated, `.` only when the list is
/// complete: `abort can be found on "\C-g", "\C-x\C-g".`
pub fn format_function_plain(name: &str, keyseqs: &[String]) -> String {
    let quoted: Vec<String> = keyseqs
        .iter()
        .take(5)
        .map(|seq| format!("\"{seq}\""))
        .collect();
    if keyseqs.len() > 5 {
        format!("{name} can be found on {}, ...", quoted.join(", "))
    } else {
        format!("{name} can be found on {}.", quoted.join(", "))
    }
}

/// `bind -P` line for a function with no bindings in the dumped keymap
/// (rl_print_keybinding, print_readably = 0, no invokers): NO trailing
/// period, unlike the `-q` form.
pub fn format_function_plain_unbound(name: &str) -> String {
    format!("{name} is not bound to any keys")
}

/// `bind -p` comment line for a function with no bindings in the dumped
/// keymap (rl_print_keybinding, print_readably = 1, no invokers).
pub fn format_function_readable_unbound(name: &str) -> String {
    format!("# {name} (not bound)")
}

/// `bind -q` positive answer (query_bindings, bind.def:344-367): at most
/// five key sequences, `...` when truncated.
pub fn format_query(name: &str, keyseqs: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let shown = keyseqs.len().min(5);
    for seq in keyseqs.iter().take(shown) {
        parts.push(format!("\"{seq}\""));
    }
    let mut out = format!("{name} can be invoked via ");
    if keyseqs.len() > shown {
        out.push_str(&parts.join(", "));
        out.push_str(", ...");
    } else {
        out.push_str(&parts.join(", "));
        out.push('.');
    }
    out
}

/// `bind -q` negative answer: `NAME is not bound to any keys.`
pub fn format_query_unbound(name: &str) -> String {
    format!("{name} is not bound to any keys.")
}

/// ``bind: `NAME': unknown function name`` (bind.def:351).
pub fn format_unknown_function(name: &str) -> String {
    format!("`{name}': unknown function name")
}

/// ``bind: `NAME': invalid keymap name`` (bind.def:255).
pub fn format_invalid_keymap(name: &str) -> String {
    format!("`{name}': invalid keymap name")
}

/// `bind: FILE: cannot read: ERRNO` (bind.def:274-276).
pub fn format_cannot_read(file: &str, reason: &str) -> String {
    format!("{file}: cannot read: {reason}")
}

/// Parse one positional binding argument. `bind -x` values arrive through
/// [`parse_unix_command_spec`]; this handles the plain inputrc form.
pub fn parse_positional_binding(arg: &str) -> Result<ParsedBinding, BindSpecError> {
    match parse_binding_line(arg) {
        Some(Ok(parsed)) => Ok(parsed),
        Some(Err(err)) => Err(err),
        None => Err(BindSpecError::message(format!("{arg}: missing separator"))),
    }
}

/// Strip a case-insensitive `set` keyword with its separating whitespace
/// (readline matches the keyword case-insensitively); `None` when the
/// line is not a variable assignment.
pub fn strip_set_keyword(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.len() >= 3 && trimmed[..3].eq_ignore_ascii_case("set") {
        let rest = &trimmed[3..];
        if rest.starts_with(char::is_whitespace) {
            return Some(rest);
        }
    }
    None
}

/// The environment an inputrc file's conditionals evaluate against:
/// `$if term=VALUE` compares `$TERM` verbatim (case-sensitively — GNU
/// probe: `$if term=XTERM` does not match TERM=xterm) and `$if mode=VALUE`
/// compares the current editing mode (`set editing-mode` lines earlier in
/// the same file update it for later conditionals — GNU probe: a
/// `mode=vi` block after `set editing-mode vi` applies). A bare
/// `$if word` or unknown key is an error whose block never applies
/// (GNU probe: `readline: ...: equality comparison operator expected,
/// found `end-of-line'`).
#[derive(Debug, Clone, Default)]
pub struct InputrcEnv<'a> {
    pub term: Option<&'a str>,
    pub editing_mode: Option<&'a str>,
}

/// One `readline: ...` diagnostic from parsing. `file` is the file the
/// line came from ($include files carry their own name), `line` its
/// 1-based number, and `core` the message body GNU prints after the
/// `file: line N: ` middle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputrcDiagnostic {
    pub file: String,
    pub line: usize,
    pub core: String,
}

/// Everything one inputrc parse produced, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputrcParsed {
    /// `"keyseq": value` bindings from active regions.
    pub bindings: Vec<(String, BindingSpec)>,
    /// Validated `set NAME VALUE` lines (value normalized the way
    /// [`apply_readline_variable`] normalizes; `editing-mode` normalized
    /// lowercase).
    pub variables: Vec<(String, String)>,
    pub diagnostics: Vec<InputrcDiagnostic>,
}

/// Parse an inputrc body (`bind -f` / the startup ~/.inputrc read):
/// bindings, `set NAME VALUE` variables, `$include FILE`, and
/// `$if term=…/mode=…` / `$else` / `$endif` conditionals (nesting
/// supported; a `$if`-family line is recognized even inside inactive
/// regions so its `$endif` still matches). `load` fetches `$include`
/// content — a missing include is silently skipped (GNU probe:
/// `bind -f` over a file with `$include nope.rc` proceeds, rc 0).
pub fn parse_inputrc_with_env(
    text: &str,
    file: &str,
    env: &InputrcEnv,
    load: &mut dyn FnMut(&str) -> Option<String>,
) -> InputrcParsed {
    let mut state = InputrcParseState {
        result: InputrcParsed::default(),
        env: env.clone(),
        file: file.to_string(),
        // GNU/readline caps conditional nesting (INIT_NEST_DEPTH 10 in
        // readline's encapsulations); deeper `$if`s are ignored.
        conditionals: Vec::new(),
        include_depth: 0,
    };
    state.parse_text(text, load);
    state.result
}

struct InputrcParseState<'a> {
    result: InputrcParsed,
    env: InputrcEnv<'a>,
    file: String,
    conditionals: Vec<ConditionalRegion>,
    include_depth: u8,
}

#[derive(Clone, Copy)]
struct ConditionalRegion {
    /// Whether lines in this region are active.
    active: bool,
    /// Whether any branch of this region has already been active (an
    /// `$else` is only active when no earlier branch was).
    taken: bool,
}

impl<'a> InputrcParseState<'a> {
    fn active(&self) -> bool {
        self.conditionals.iter().all(|region| region.active)
    }

    fn diagnostic(&mut self, line: usize, core: String) {
        self.result.diagnostics.push(InputrcDiagnostic {
            file: self.file.clone(),
            line,
            core,
        });
    }

    fn parse_text(&mut self, text: &str, load: &mut dyn FnMut(&str) -> Option<String>) {
        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            let number = index + 1;
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('$') {
                self.parse_directive(rest.trim(), number, load);
                continue;
            }
            if !self.active() {
                continue;
            }
            if let Some(rest) = strip_set_keyword(line) {
                self.parse_set_line(rest, number);
                continue;
            }
            match parse_binding_line(line) {
                Some(Ok(parsed)) => self.result.bindings.push((parsed.keyseq, parsed.spec)),
                // rl_parse_and_bind errors do not fail the bind builtin
                // (bind.def:304-333 ignores the return value; GNU probe:
                // `bind '"\C-t'` prints
                // `readline: "\C-t: no closing `"' in key binding`, rc 0).
                Some(Err(_)) => {}
                // Other non-quote text is silently ignored (GNU probe:
                // `bind 'ls -l'` → rc 0, no output).
                None => {}
            }
        }
    }

    fn parse_directive(
        &mut self,
        rest: &str,
        number: usize,
        load: &mut dyn FnMut(&str) -> Option<String>,
    ) {
        if let Some(condition) = rest.strip_prefix("if") {
            let condition = condition.trim();
            let active = if let Some((key, value)) = condition.split_once('=') {
                match key.trim() {
                    // `$if term=…` compares $TERM verbatim (case
                    // sensitive, GNU probe: XTERM ≠ xterm).
                    "term" => self.env.term == Some(value),
                    // `$if mode=…` compares the CURRENT editing mode —
                    // a `set editing-mode` earlier in the file already
                    // updated env.editing_mode (GNU probe: the mode=vi
                    // block after `set editing-mode vi` applies).
                    "mode" => self.env.editing_mode == Some(value),
                    _ => false,
                }
            } else {
                self.diagnostic(
                    number,
                    "equality comparison operator expected, found `end-of-line'".to_string(),
                );
                false
            };
            self.conditionals.push(ConditionalRegion {
                active,
                taken: active,
            });
            return;
        }
        if rest == "else" {
            if let Some(region) = self.conditionals.last_mut() {
                region.active = !region.taken;
                region.taken |= region.active;
            }
            return;
        }
        if rest == "endif" {
            self.conditionals.pop();
            return;
        }
        if let Some(name) = rest.strip_prefix("include") {
            let name = name.trim();
            if self.active() && self.include_depth < 10 {
                if let Some(content) = load(name) {
                    let saved_file = std::mem::replace(&mut self.file, name.to_string());
                    self.include_depth += 1;
                    self.parse_text(&content, load);
                    self.include_depth -= 1;
                    self.file = saved_file;
                }
            }
        }
    }

    fn parse_set_line(&mut self, rest: &str, number: usize) {
        let rest = rest.trim_start();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or_default().trim();
        let value = parts.next().unwrap_or_default().trim();
        if name.is_empty() {
            return;
        }
        // Validate against the variable table now (the diagnostics carry
        // the file:line position GNU reports), store the normalized pair.
        let mut scratch: Vec<(String, String)> = Vec::new();
        let (outcome, editing_mode) = apply_readline_variable(&mut scratch, name, value);
        match outcome {
            SetVariableOutcome::Applied => {
                for (stored, normalized) in scratch {
                    store_variable(&mut self.result.variables, &stored, &normalized);
                }
                if let Some((mode_name, mode_value)) = editing_mode {
                    // Later `mode=` conditionals in this file see the new
                    // mode (GNU probe).
                    if mode_name == "editing-mode" {
                        self.env.editing_mode = Some(mode_value);
                    }
                }
            }
            SetVariableOutcome::UnknownVariable => {
                self.diagnostic(number, format!("{name}: unknown variable name"))
            }
            SetVariableOutcome::CouldNotSetValue => {
                self.diagnostic(number, format!("{name}: could not set value to `{value}'"))
            }
        }
    }
}

/// Parse an inputrc file body (`bind -f`): returns the keyseq bindings
/// and the `set editing-mode` value. Compatibility wrapper over
/// [`parse_inputrc_with_env`] with no term and no include loading.
pub fn parse_inputrc(text: &str) -> (Vec<(String, BindingSpec)>, Option<String>) {
    let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
    (
        parsed.bindings,
        parsed
            .variables
            .iter()
            .find(|(name, _)| name == "editing-mode")
            .map(|(_, value)| value.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyseq_bytes_orders_like_the_readline_scan() {
        // rubash#2338: byte order of the translated sequence is the
        // rl_invoking_keyseqs_in_map listing order.
        assert_eq!(keyseq_bytes(r"\C-b"), vec![0x02]);
        assert_eq!(keyseq_bytes(r"\eOD"), vec![0x1b, b'O', b'D']);
        assert_eq!(keyseq_bytes(r"\e[D"), vec![0x1b, b'[', b'D']);
        assert!(keyseq_bytes(r"\C-b") < keyseq_bytes(r"\eOD"));
        assert!(keyseq_bytes(r"\eOD") < keyseq_bytes(r"\e[D"));
        assert_eq!(keyseq_bytes(r"\C-@"), vec![0x00]);
        assert_eq!(keyseq_bytes(r"\C-?"), vec![0x7f]);
        assert_eq!(keyseq_bytes(r"\C-\\"), vec![0x1c]);
        assert_eq!(keyseq_bytes(r"\e "), vec![0x1b, b' ']);
        assert_eq!(keyseq_bytes("\\\\"), vec![0x5c]);
        assert_eq!(keyseq_bytes("\\\""), vec![0x22]);
        assert_eq!(keyseq_bytes(r"\200"), vec![0x80]);
        assert_eq!(keyseq_bytes("h"), vec![b'h']);
    }

    #[test]
    fn function_tables_match_gnu_counts() {
        // GNU 5.3.20 oracle: `bind -l` = 176 names; the per-keymap
        // dumps carry 403/109/44/273/104 bound rows (rubash#2338).
        assert_eq!(READLINE_FUNCTIONS.len(), 176);
        assert_eq!(DEFAULT_FUNCTION_BINDINGS.len(), 403);
        assert_eq!(DEFAULT_BINDINGS_EMACS_META.len(), 109);
        assert_eq!(DEFAULT_BINDINGS_EMACS_CTLX.len(), 44);
        assert_eq!(DEFAULT_BINDINGS_VI_INSERT.len(), 273);
        assert_eq!(DEFAULT_BINDINGS_VI_MOVEMENT.len(), 104);
        assert!(is_known_function("complete"));
        assert!(is_known_function("history-search-backward"));
    }

    #[test]
    fn plain_dump_formats_match_rl_print_keybinding() {
        let seqs: Vec<String> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            format_function_plain("fn", &seqs),
            "fn can be found on \"a\", \"b\", \"c\", \"d\", \"e\", ..."
        );
        assert_eq!(
            format_function_plain("fn", &seqs[..2]),
            "fn can be found on \"a\", \"b\"."
        );
        // The unbound plain form has NO trailing period (unlike -q's).
        assert_eq!(
            format_function_plain_unbound("fn"),
            "fn is not bound to any keys"
        );
        assert_eq!(format_function_readable_unbound("fn"), "# fn (not bound)");
    }

    #[test]
    fn parse_unix_command_spec_colon_form() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-g": echo bound-c-g"#).expect("parses");
        assert_eq!(keyseq, "\\C-g");
        assert_eq!(command, "echo bound-c-g");
    }

    #[test]
    fn parse_unix_command_spec_space_form_with_quotes() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-r" "fzf --query $READLINE_LINE""#).expect("parses");
        assert_eq!(keyseq, "\\C-r");
        assert_eq!(command, "fzf --query $READLINE_LINE");
    }

    #[test]
    fn parse_unix_command_spec_keeps_nested_quotes() {
        let (keyseq, command) =
            parse_unix_command_spec(r#""\C-g": sh -c "echo inner""#).expect("parses");
        assert_eq!(command, r#"sh -c "echo inner""#);
    }

    #[test]
    fn parse_unix_command_spec_errors_match_gnu() {
        let err = parse_unix_command_spec("noquotes").expect_err("rejected");
        assert_eq!(
            err.message,
            "noquotes: first non-whitespace character is not `\"'"
        );
        let err = parse_unix_command_spec(r#""\C-g"#).expect_err("rejected");
        assert!(err.message.contains("no closing"));
    }

    #[test]
    fn parse_binding_line_macro_and_function() {
        let parsed = parse_binding_line(r#""\C-t": "ins-macro""#)
            .expect("this form")
            .expect("parses");
        assert_eq!(parsed.keyseq, "\\C-t");
        assert_eq!(
            parsed.spec,
            BindingSpec::Macro {
                text: "ins-macro".to_string()
            }
        );
        let parsed = parse_binding_line(r#""\C-o": accept-line"#)
            .expect("this form")
            .expect("parses");
        assert_eq!(
            parsed.spec,
            BindingSpec::Function {
                name: "accept-line".to_string()
            }
        );
    }

    #[test]
    fn formats_match_gnu_probe_shapes() {
        assert_eq!(
            format_unix_command_readable("\\C-g", "echo bound-c-g"),
            "\"\\C-g\" \"echo bound-c-g\""
        );
        assert_eq!(
            format_macro_readable("\\C-t", "ins-macro"),
            "\"\\C-t\": \"ins-macro\""
        );
        assert_eq!(
            format_macro_plain("\\C-t", "ins-macro"),
            "\\C-t outputs ins-macro"
        );
        assert_eq!(
            format_function_plain("abort", &["\\C-g".to_string(), "\\C-x\\C-g".to_string()]),
            "abort can be found on \"\\C-g\", \"\\C-x\\C-g\"."
        );
        assert_eq!(
            format_query("self-insert", &[" ".to_string()]),
            "self-insert can be invoked via \" \"."
        );
        assert_eq!(
            format_query("self-insert", &[" ".to_string(), "!".to_string()]),
            "self-insert can be invoked via \" \", \"!\"."
        );
        assert_eq!(
            format_query(
                "self-insert",
                &[" ".to_string(), "!".to_string(), "?".to_string()]
            ),
            "self-insert can be invoked via \" \", \"!\", \"?\"."
        );
        assert_eq!(
            format_query_unbound("vi-editing-mode"),
            "vi-editing-mode is not bound to any keys."
        );
    }

    #[test]
    fn query_truncates_at_five_with_ellipsis() {
        let seqs: Vec<String> = (0..7).map(|i| format!("\\C-{i}")).collect();
        let out = format_query("self-insert", &seqs);
        assert!(out.ends_with(", ..."), "got: {out}");
        assert!(out.contains("\\C-4"));
        assert!(!out.contains("\\C-5"), "only five shown: {out}");
    }

    #[test]
    fn inputrc_parsing_extracts_bindings_and_editing_mode() {
        let text = "\n# comment\n\"\\C-k\": kill-whole-line\n\"\\C-t\": \"macro text\"\nset editing-mode vi\nset blink-matching-paren off\n$if term=xterm\n\"\\C-x\": beginning-of-line\n$endif\n";
        let (bindings, mode) = parse_inputrc(text);
        // `$if term=xterm` with no TERM in the wrapper env: the block is
        // skipped (GNU probe: term conditions compare $TERM verbatim).
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].0, "\\C-k");
        assert_eq!(mode, Some("vi".to_string()));
    }

    #[test]
    fn inputrc_term_condition_applies_when_term_matches() {
        let text = "$if term=xterm\n\"\\C-x\": beginning-of-line\n$else\n\"\\C-y\": yank\n$endif\n";
        let env = InputrcEnv {
            term: Some("xterm"),
            editing_mode: None,
        };
        let parsed = parse_inputrc_with_env(text, "", &env, &mut |_| None);
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.bindings[0].0, "\\C-x");
        // $else branch when the term does not match (case-sensitive: GNU
        // probe `term=XTERM` does not match TERM=xterm).
        let env = InputrcEnv {
            term: Some("XTERM"),
            editing_mode: None,
        };
        let parsed = parse_inputrc_with_env(text, "", &env, &mut |_| None);
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.bindings[0].0, "\\C-y");
    }

    #[test]
    fn inputrc_mode_condition_follows_in_file_set() {
        let text = "set editing-mode vi\n$if mode=vi\n\"\\C-m\": vi-editing-mode\n$endif\n";
        let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
        // The `set editing-mode vi` earlier in the file arms the later
        // mode= conditional (GNU probe).
        assert_eq!(parsed.bindings.len(), 1);
        assert_eq!(parsed.variables[0].0, "editing-mode");
        assert_eq!(parsed.variables[0].1, "vi");
        assert_eq!(parsed.variables[1].0, "keymap");
        assert_eq!(parsed.variables[1].1, "vi-insert");
    }

    #[test]
    fn inputrc_set_variables_validate_and_normalize() {
        let text = "set completion-ignore-case on\nset completion-ignore-case maybe\nset keyseq-timeout abc\nset no-such-var on\nset editing-mode bogus\nset editing-mode VI\n";
        let parsed = parse_inputrc_with_env(text, "rc", &InputrcEnv::default(), &mut |_| None);
        // Non-`on` booleans normalize to `off`, garbage numbers to 0
        // (GNU probes: silent). `editing-mode VI` lowercases.
        let value = |name: &str| {
            parsed
                .variables
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert_eq!(value("completion-ignore-case"), "off");
        assert_eq!(value("keyseq-timeout"), "0");
        assert_eq!(value("editing-mode"), "vi");
        // Diagnostics: the unknown name and the bad editing-mode value
        // (`readline: rc: line N: …` shaped by the caller).
        assert_eq!(parsed.diagnostics.len(), 2);
        assert_eq!(parsed.diagnostics[0].line, 4);
        assert_eq!(
            parsed.diagnostics[0].core,
            "no-such-var: unknown variable name"
        );
        assert_eq!(parsed.diagnostics[1].line, 5);
        assert_eq!(
            parsed.diagnostics[1].core,
            "editing-mode: could not set value to `bogus'"
        );
    }

    #[test]
    fn inputrc_include_and_nesting() {
        let text = "$if term=vt100\n$if term=vt100\n\"\\C-a\": beginning-of-line\n$endif\n$endif\n\"\\C-b\": backward-char\n";
        let parsed = parse_inputrc_with_env(text, "", &InputrcEnv::default(), &mut |_| None);
        // Nested $if with no TERM: inactive; the outer binding applies.
        assert_eq!(
            parsed
                .bindings
                .iter()
                .map(|(seq, _)| seq.as_str())
                .collect::<Vec<_>>(),
            vec!["\\C-b"]
        );
    }

    #[test]
    fn variable_table_defaults_match_gnu_dump() {
        // `bind -v` order and defaults pinned against WSL GNU bash 5.3.0
        // (48 lines).
        assert_eq!(READLINE_VARIABLE_DEFAULTS.len(), 48);
        assert_eq!(
            READLINE_VARIABLE_DEFAULTS[0],
            ("bind-tty-special-chars", "off")
        );
        assert_eq!(READLINE_VARIABLE_DEFAULTS[35], ("visible-stats", "off"));
        assert_eq!(READLINE_VARIABLE_DEFAULTS[36], ("bell-style", "audible"));
        assert_eq!(
            READLINE_VARIABLE_DEFAULTS[47],
            ("vi-ins-mode-string", "(ins)")
        );
        let registry = crate::shell::bind_registry::BindRegistry::default();
        assert_eq!(registry.variable_value("editing-mode"), Some("emacs"));
        assert_eq!(registry.variable_value("keymap"), Some("emacs"));
    }

    #[test]
    fn set_variable_outcomes_match_gnu_probes() {
        let mut registry = crate::shell::bind_registry::BindRegistry::default();
        // `bind 'set completion-ignore-case on'` → visible in `bind -v`.
        assert_eq!(
            registry.set_readline_variable("completion-ignore-case", "on"),
            SetVariableOutcome::Applied
        );
        assert_eq!(
            registry.variable_value("completion-ignore-case"),
            Some("on")
        );
        // Unknown name: ``readline: no-such-var: unknown variable name``,
        // command keeps running.
        assert_eq!(
            registry.set_readline_variable("no-such-var", "on"),
            SetVariableOutcome::UnknownVariable
        );
        // Bad editing-mode value: loud, variable unchanged.
        assert_eq!(
            registry.set_readline_variable("editing-mode", "bogus"),
            SetVariableOutcome::CouldNotSetValue
        );
        assert_eq!(registry.variable_value("editing-mode"), Some("emacs"));
        // Valid vi switches editing-mode AND keymap (GNU probe).
        assert_eq!(
            registry.set_readline_variable("editing-mode", "vi"),
            SetVariableOutcome::Applied
        );
        assert_eq!(registry.variable_value("editing-mode"), Some("vi"));
        assert_eq!(registry.variable_value("keymap"), Some("vi-insert"));
        assert_eq!(registry.editing_mode.as_deref(), Some("vi"));
        // Bad keymap keeps the old value silently.
        assert_eq!(
            registry.set_readline_variable("keymap", "bogus"),
            SetVariableOutcome::Applied
        );
        assert_eq!(registry.variable_value("keymap"), Some("vi-insert"));
    }

    #[test]
    fn positional_errors_carry_readline_channel() {
        // Unclosed keyseq quote: `readline: …` text, no status effect.
        let err = parse_binding_line(r#""\C-t"#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("\"\\C-t: no closing `\"' in key binding")
        );
        // Keyseq with no `: value`: no key sequence terminator.
        let err = parse_binding_line(r#""\C-t""#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("\"\\C-t\": no key sequence terminator")
        );
        // Unclosed macro quote.
        let err = parse_binding_line(r#""\C-t": 'ins"#)
            .expect("this form")
            .expect_err("rejected");
        assert_eq!(
            err.readline_message.as_deref(),
            Some("'ins: missing closing quote for macro")
        );
        // -x errors stay in the `bind:` channel (rc 1 there).
        let err = parse_unix_command_spec("noquotes").expect_err("rejected");
        assert!(err.readline_message.is_none());
        // Non-quote non-set text: not this form at all.
        assert!(parse_binding_line("ls -l").is_none());
    }

    #[test]
    fn known_function_table_covers_common_names() {
        for name in [
            "self-insert",
            "accept-line",
            "beginning-of-line",
            "history-incremental-search-backward",
        ] {
            assert!(is_known_function(name), "{name} should be known");
        }
        assert!(!is_known_function("no-such-fn"));
    }
}
