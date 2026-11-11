//! The key-binding view: GAP-3 of the niubash#863 API queue.
//!
//! The engine has always owned the binding state — the
//! `BindRegistry` is
//! the `bind` builtin's data plane — but until GAP-3 the Session
//! contract carried none of it, so a host mirrored the registry
//! through the tier-C executor seam and re-decoded the written
//! key sequences with a second, handwritten translation beside
//! the engine's own. This module is the tier-A
//! read face: one owned snapshot, captured from the registry
//! itself, whose canonical bytes are the registry's own byte
//! identity ([`keyseq_identity`]) and whose dump lines come from the `bind` builtin's
//! own format helpers — the API and the shell can never disagree
//! about what a binding is (the module rule in `super`).
//!
//! GNU anchors: the written form and its byte translation are
//! rl_translate_keyseq's (lib/readline/bind.c:516); the dump
//! forms are rl_function_dumper (bind.c:2800), rl_macro_dumper
//! (bind.c:2899) and print_unix_command_map (bashline.c:4729);
//! the variable table is rl_variable_dumper's (bind.c:3045).

use crate::builtins::bind::{
    format_function_readable, format_macro_readable, format_unix_command_readable,
};
use crate::shell::bind_registry::{keyseq_identity, normalize_keymap_name, BindKind, BindRegistry};

/// What a key sequence is bound to, as the contract reports it —
/// the view form of the registry's
/// `BindKind`, payload
/// included, so a host never re-reads the engine's internal enum
/// through the tier-C seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindKindView {
    /// `bind -x '"\C-r": command'` — the command runs on the
    /// keypress under the READLINE_* protocol
    /// (bashline.c:4593 bash_execute_unix_command).
    Execute { command: String },
    /// `bind '"\C-a": beginning-of-line'` — a readline function
    /// name.
    Function { name: String },
    /// `bind '"\C-t": "text"'` — a macro.
    Macro { text: String },
}

/// One registered binding, as the contract reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindEntryView {
    /// The written form, exactly as registered (`\C-g`,
    /// `\e[200~`, `\C-x\C-f`) — the form GNU's dumpers print.
    pub keyseq: String,
    /// The canonical byte sequence: the registry's byte
    /// identity ([`keyseq_identity`]) of the written form —
    /// the engine's one rl_translate_keyseq port, run on the
    /// decoded byte string, so a written form that carries a
    /// raw byte as a raw-byte marker pair reports the folded
    /// byte itself, never the pair's UTF-8 and never a second
    /// decoder.
    pub bytes: Vec<u8>,
    /// What the sequence is bound to.
    pub kind: BindKindView,
    /// The keymap the binding lives in, resolved and normalized
    /// (the aliases of bind.def:37-40 folded by
    /// `normalize_keymap_name`): a registration made with no
    /// `-m` lands under the keymap that was current when it was
    /// registered (see
    /// `BindRegistry`),
    /// so this always names one concrete map.
    pub keymap: String,
    /// This entry as GNU's dumpers print it, rendered at capture
    /// time by the same format helpers the `bind` builtin's own
    /// dump paths call — `"seq": name` for a function
    /// (rl_function_dumper readable form), `"seq": "text"` for a
    /// macro (rl_macro_dumper readable form), `"seq" "command"`
    /// for a `bind -x` entry (print_unix_command_map). One line,
    /// no trailing newline.
    pub dump: String,
}

/// One recorded removal, as the contract reports it: a
/// `bind -r` / `bind -u` tombstone. GNU's keymaps are single
/// mutable maps, so unbinding deletes the slot itself —
/// DEFAULT bindings included (bind.def:375-388 unbind_command,
/// bind.def:390-427 unbind_keyseq); the engine keeps its
/// defaults in static tables, so the removal is recorded here
/// and a mirroring host subtracts it from the default tables
/// (rubash#2337). Registering the sequence again clears the
/// tombstone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindRemovalView {
    /// The written form, exactly as removed.
    pub keyseq: String,
    /// The canonical byte sequence: the registry's byte
    /// identity ([`keyseq_identity`]) of the written form.
    pub bytes: Vec<u8>,
    /// The keymap the removal landed in — stored resolved at
    /// record time (an explicit `-m` map, or the map current
    /// when the removal landed, normalized), so a later
    /// editing-mode flip never reinterprets it (rubash#2386).
    pub keymap: String,
}

/// One capture of the whole binding registry: the snapshot a
/// host mirrors its keymaps from between prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindView {
    /// The registry's mutation counter, verbatim: bumped on
    /// every register / unregister / variable set, never by a
    /// read. A host compares it between prompts to notice that
    /// its mirrored keymaps are stale — the designed form of the
    /// tier-C `bind_registry_generation` probe.
    pub generation: u64,
    /// `set editing-mode` as the registry holds it, when any
    /// inputrc or `bind 'set …'` named one.
    pub editing_mode: Option<String>,
    /// The readline variable table (the `bind -v` world), in
    /// registry order — the table's own order, the one
    /// rl_variable_dumper prints.
    pub variables: Vec<(String, String)>,
    /// Every registered entry, in registry order.
    pub entries: Vec<BindEntryView>,
    /// Every recorded removal (`bind -r` / `bind -u`
    /// tombstones), in registry order. The host subtracts
    /// these from the default key tables when it mirrors them;
    /// they ride the view so the tier-C
    /// `Executor::bind_removed_keyseqs` read retires with the
    /// rest of the seam.
    pub removals: Vec<BindRemovalView>,
}

impl BindView {
    /// Capture the view from the registry itself: no shadow
    /// bookkeeping, the snapshot is the registry's own stores
    /// read at this moment.
    pub(crate) fn capture(registry: &BindRegistry) -> BindView {
        let entries = registry
            .entries
            .iter()
            .map(|entry| {
                let kind = match &entry.kind {
                    BindKind::Execute { command } => BindKindView::Execute {
                        command: command.clone(),
                    },
                    BindKind::Function { name } => BindKindView::Function { name: name.clone() },
                    BindKind::Macro { text } => BindKindView::Macro { text: text.clone() },
                    // The contract has no keymap-valued view kind:
                    // GAP-13's Q-E ruling kept BindKind internal and
                    // the view a 1:1 mirror, so a faithful rendering
                    // needs a BindKindView::Keymap variant plus
                    // ledger rows on both sides — a contract change
                    // for the captain to schedule, NOT made here.
                    // Rendering the entry as the `prefix-meta`
                    // function name preserves exactly what hosts
                    // saw for this binding before the registry grew
                    // the kind (the old-style arm stored the name
                    // verbatim, rubash#2923).
                    BindKind::Keymap { .. } => BindKindView::Function {
                        name: "prefix-meta".to_string(),
                    },
                };
                let dump = match &entry.kind {
                    BindKind::Execute { command } => {
                        format_unix_command_readable(&entry.keyseq, command)
                    }
                    BindKind::Function { name } => format_function_readable(&entry.keyseq, name),
                    BindKind::Macro { text } => format_macro_readable(&entry.keyseq, text),
                    BindKind::Keymap { .. } => {
                        format_function_readable(&entry.keyseq, "prefix-meta")
                    }
                };
                BindEntryView {
                    bytes: keyseq_identity(&entry.keyseq),
                    keyseq: entry.keyseq.clone(),
                    kind,
                    keymap: match entry.keymap.as_deref() {
                        Some(name) => normalize_keymap_name(name).to_string(),
                        None => registry.current_keymap(),
                    },
                    dump,
                }
            })
            .collect();
        let removals = registry
            .removals
            .iter()
            .map(|(keyseq, keymap)| BindRemovalView {
                bytes: keyseq_identity(keyseq),
                keyseq: keyseq.clone(),
                // The registry's own reads resolve a keymap-less
                // tombstone against "emacs-standard" (see
                // `BindRegistry::register` / `is_removed`); stored
                // tombstones are always resolved already, so this
                // is only the faithful fallback.
                keymap: normalize_keymap_name(keymap.as_deref().unwrap_or("emacs-standard"))
                    .to_string(),
            })
            .collect();
        BindView {
            generation: registry.generation,
            editing_mode: registry.editing_mode.clone(),
            variables: registry.variables.clone(),
            entries,
            removals,
        }
    }
}
