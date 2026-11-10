//! Readline key-binding registry — the `bind` builtin's data plane.
//!
//! GNU bash keeps the binding state inside readline's keymaps: function
//! bindings and macros in the active keymap, and `bind -x` shell commands
//! in a parallel command map per editing mode (`bashline.c:4771
//! init_unix_command_map`: emacs_std_cmd_xmap, vi_insert_cmd_xmap,
//! vi_movement_cmd_xmap; `bashline.c:4850 bind_keyseq_to_unix_command`
//! stores the command there, `bashline.c:4593 bash_execute_unix_command`
//! runs it on the keypress). The rubash engine has no in-process line
//! editor — hosts own one (niubash's reedline REPL) — so the registry is
//! the shared data plane a host reads to mirror registered bindings into
//! its own keymaps, and through which the engine runs `bind -x` commands
//! with the GNU READLINE_LINE/POINT/MARK protocol.
//!
//! The registry lives in [`crate::shell::ShellState`] behind an
//! `Rc<RefCell<..>>` (the `session_history` pattern) because readline
//! keymaps are process-global C globals in GNU: a clone (flat subshell,
//! command substitution) shares the same registry, so a `bind` inside a
//! subshell is visible to the parent exactly as it is in GNU.

use std::cell::RefCell;
use std::rc::Rc;

/// What a key sequence is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindKind {
    /// `bind -x '"\C-r": command'` — the command runs on the keypress with
    /// the READLINE_* variables set and read back
    /// (bashline.c:4593-4707 bash_execute_unix_command).
    Execute { command: String },
    /// `bind '"\C-a": beginning-of-line'` — a readline function name
    /// (bind.def:324 passes the inputrc-form argument to
    /// rl_parse_and_bind; an unquoted value is a function name).
    Function { name: String },
    /// `bind '"\C-t": "text"'` — a macro (quoted value, bind.def inputrc
    /// form; GNU ignores an unquoted non-function value, see the wt100
    /// probe: `bind '"\C-t": ins-macro'` binds nothing and `bind -s`
    /// lists nothing).
    Macro { text: String },
    /// `bind 'Control-t: prefix-meta'` — the sequence is bound to a
    /// whole keymap (rl_generic_bind's ISKMAP value, lib/readline/
    /// bind.c:1850): the target map's bindings become reachable under
    /// this sequence as a prefix. The dump/query paths expand the
    /// target map through the slot; a function or macro bound at the
    /// slot itself does not displace the map but becomes the target
    /// map's ANYOTHERKEY fallback (see
    /// [`BindRegistry::fallbacks`]).
    Keymap { name: String },
}

/// One registered binding. `keyseq` keeps the user's written form
/// (`\C-g`, `\e[200~`, `\C-x\C-f`) — GNU's dumpers print the same
/// backslash form (`bind -X` probe: `"\C-g" "echo bound-c-g"`), and
/// the host translates it for its own key tables. Identity is NOT
/// the string: GNU's keymaps store only the bytes (rl_generic_bind,
/// lib/readline/bind.c:383, translates via rl_translate_keyseq,
/// bind.c:516, and walks byte slots), so every spelling of one
/// sequence is one slot and every registry comparison goes through
/// the byte identity ([`keyseq_identity`], rubash#2606); the
/// dumpers render the bytes back through _rl_get_keyname
/// (bind.c:2580) regardless of the spelling that was registered
/// (the snapshot's presentation form, [`canonical_keyseq`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindEntry {
    pub keyseq: String,
    pub kind: BindKind,
    /// `-m keymap` target (bind.def:37-40): emacs, emacs-standard,
    /// emacs-meta, emacs-ctlx, vi, vi-move, vi-command, vi-insert. `None`
    /// names the registry's current keymap; [`BindRegistry::register`]
    /// resolves it to that concrete keymap before storing, because
    /// rl_parse_and_bind writes into one concrete map — GNU has no
    /// floating binding — so a stored entry always names the map it
    /// was bound in (rubash#2386).
    pub keymap: Option<String>,
}

/// Process-global binding registry (readline's keymap world).
#[derive(Debug, Clone)]
pub(crate) struct BindRegistry {
    pub entries: Vec<BindEntry>,
    /// Bumped on every mutation so an embedding host can notice that its
    /// mirrored keymaps are stale and rebuild them between prompts.
    pub generation: u64,
    /// `set editing-mode vi|emacs` seen through `bind -f` inputrc files
    /// (or future `bind` set-variable support). Hosts that own an editor
    /// consult it at keymap-build time.
    pub editing_mode: Option<String>,
    /// The readline variable table `bind -v`/`-V` dumps (name → value, in
    /// [`crate::builtins::bind::READLINE_VARIABLE_DEFAULTS`] order) and
    /// that `set NAME VALUE` lines in inputrc files / `bind 'set …'`
    /// arguments update. GNU keeps this table in readline; hosts that
    /// implement a variable's behavior read it from here (the same data
    /// plane as the keymap mirror).
    pub variables: Vec<(String, String)>,
    /// bashline.c:278 `bash_readline_initialized`: initialize_readline
    /// (bashline.c:451 — the inputrc read among the rest) runs once per
    /// process, on the first `bind` invocation (bind.def:131) or the
    /// first prompt read (y.tab.c:4024). Whichever comes first wins; the
    /// inputrc must not be read twice.
    pub readline_initialized: bool,
    /// Sequences removed by `bind -r` / `bind -u` (keyseq, keymap).
    /// GNU's keymaps are single mutable maps: unbinding deletes the
    /// slot from the map itself, DEFAULT bindings included
    /// (bind.def:375-388 unbind_command → rl_unbind_function_in_map;
    /// bind.def:390-427 unbind_keyseq → rl_bind_keyseq(seq, NULL)).
    /// rubash keeps the defaults in static tables, so a removal is
    /// recorded here and queries subtract it (rubash#2337); hosts
    /// mirror the removals through the same accessor the snapshot
    /// uses. A removal names one exact sequence in one keymap — GNU
    /// does NOT delete a subtree when a bare keymap prefix is named
    /// (oracle: `bind -r "\C-x"` leaves `\C-x\C-?` bound, because
    /// unbind_keyseq only acts when the sequence resolves to a
    /// function). Registering the sequence again clears its removal.
    /// The keymap is stored resolved (the `-m` map, or the map that
    /// was current when the removal landed, normalized): GNU deletes
    /// from one concrete map, so a later editing-mode flip must not
    /// reinterpret the tombstone (rubash#2386).
    pub removals: Vec<(String, Option<String>)>,
    /// Each keymap's ANYOTHERKEY fallback (target keymap name →
    /// binding): when rl_generic_bind's sequence walk ends at a slot
    /// whose value is a keymap, the function/macro being bound is
    /// stored in THAT keymap's ANYOTHERKEY slot instead (lib/readline/
    /// bind.c, rl_generic_bind's last-key arm) — so the fallback
    /// belongs to the target map and shows at every prefix bound to
    /// it (GNU probe: `bind 'ESC: undo'` then
    /// `bind 'Control-t: prefix-meta'` makes undo invocable via
    /// `"\C-t"` too, because both slots point at the one
    /// emacs_meta_keymap). Unbinding at any of the map's prefixes
    /// nulls it (the same arm stores _rl_null_function for a NULL
    /// bind) while the map itself stays bound. At most one fallback
    /// per keymap — the C slot is a single array cell (index 256).
    pub fallbacks: Vec<(String, BindKind)>,
    /// The readline kill ring (lib/readline/kill.c): text killed by
    /// the line editor, newest entry last, shared by every editor
    /// session in the process (the `read -e` editor today; C-k in one
    /// `read` can be yanked by C-y in the next, exactly as two
    /// readline sessions share the ring).
    pub kill_ring: Vec<String>,
}

impl Default for BindRegistry {
    fn default() -> Self {
        BindRegistry {
            entries: Vec::new(),
            generation: 0,
            editing_mode: None,
            variables: crate::builtins::bind::READLINE_VARIABLE_DEFAULTS
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            readline_initialized: false,
            removals: Vec::new(),
            fallbacks: Vec::new(),
            kill_ring: Vec::new(),
        }
    }
}

impl BindRegistry {
    /// Apply one `set NAME VALUE` against the variable table
    /// (readline bind.c rl_variable_bind). Returns the outcome the caller
    /// reports: ``readline: NAME: unknown variable name`` for
    /// [`SetVariableOutcome::UnknownVariable`], ``readline: NAME: could
    /// not set value to `VALUE'`` for
    /// [`SetVariableOutcome::CouldNotSetValue`] (both GNU probes; the
    /// command keeps running either way).
    pub fn set_readline_variable(
        &mut self,
        name: &str,
        value: &str,
    ) -> crate::builtins::bind::SetVariableOutcome {
        let (outcome, editing_mode) =
            crate::builtins::bind::apply_readline_variable(&mut self.variables, name, value);
        if outcome == crate::builtins::bind::SetVariableOutcome::Applied {
            if let Some((mode_name, mode_value)) = editing_mode {
                if mode_name == "editing-mode" {
                    self.editing_mode = Some(mode_value.to_string());
                }
            }
            self.generation += 1;
        }
        outcome
    }

    /// Current value of one readline variable (`bind -v` world).
    pub fn variable_value(&self, name: &str) -> Option<&str> {
        self.variables
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    /// The registry's current keymap, normalized — the mirror of
    /// readline's `_rl_keymap`. sv_editmode (lib/readline/bind.c:2097,
    /// 2104) and the `keymap` variable's setter (bind.c:2517) keep
    /// `_rl_keymap` in step with the `keymap` variable, and every
    /// no-`-m` listing/query/unbind reads the current map
    /// (bind.def:205-216; bind.c:2743 rl_function_dumper, :2753
    /// rl_print_keybinding; bind.def:386 unbind_command). The
    /// registry's variable table is that same mirror, so the current
    /// keymap is the table's `keymap` row, normalized (rubash#2386).
    /// Public as the resolution a keymap-less entry lands under in
    /// the Session bind view (niubash#863 GAP-3): stored entries are
    /// stamped at registration, so the fallback is only reachable
    /// for entries constructed outside [`BindRegistry::register`].
    pub fn current_keymap(&self) -> String {
        normalize_keymap_name(self.variable_value("keymap").unwrap_or("emacs")).to_string()
    }

    /// Resolve an optional keymap argument the way the C does: an
    /// explicit name names its map; `None` is the current keymap.
    fn resolve_keymap(&self, keymap: Option<&str>) -> String {
        match keymap {
            Some(name) => normalize_keymap_name(name).to_string(),
            None => self.current_keymap(),
        }
    }

    /// The keymap-valued slots of one map: (prefix sequence, target
    /// keymap), both normalized. emacs-standard's defaults bind ESC
    /// to emacs-meta and C-x to emacs-ctlx (lib/readline/
    /// emacs_keymap.c:62, :59 — the flattened default table is
    /// exactly the standard map's direct rows plus those two
    /// submaps composed under their prefixes); a session
    /// [`BindKind::Keymap`] registration at a default slot's sequence
    /// re-points the slot, and one at any other sequence adds a
    /// slot, mirroring rl_generic_bind's ISKMAP assignment.
    fn keymap_slots(&self, keymap: &str) -> Vec<(String, String)> {
        let mut slots: Vec<(String, String)> = match keymap {
            "emacs-standard" => vec![
                ("\\e".to_string(), "emacs-meta".to_string()),
                ("\\C-x".to_string(), "emacs-ctlx".to_string()),
            ],
            _ => Vec::new(),
        };
        for entry in self.entries_for_keymap(Some(keymap)) {
            if let BindKind::Keymap { name } = &entry.kind {
                let target = normalize_keymap_name(name).to_string();
                match slots
                    .iter_mut()
                    .find(|(seq, _)| keyseq_identity(seq) == keyseq_identity(&entry.keyseq))
                {
                    Some(slot) => slot.1 = target,
                    None => slots.push((entry.keyseq.clone(), target)),
                }
            }
        }
        slots
    }

    /// The target keymap of the slot at `keyseq` in `keymap`, when
    /// the sequence currently names a keymap there. The match is a
    /// byte-identity one (rubash#2606): a slot registered under
    /// any spelling of the prefix answers any other spelling.
    fn slot_target(&self, keymap: &str, keyseq: &str) -> Option<String> {
        let bytes = keyseq_identity(keyseq);
        self.keymap_slots(keymap)
            .into_iter()
            .find(|(seq, _)| keyseq_identity(seq) == bytes)
            .map(|(_, target)| target)
    }

    /// One keymap's ANYOTHERKEY fallback binding, if any (see the
    /// `fallbacks` field).
    pub fn keymap_fallback(&self, keymap: &str) -> Option<&BindKind> {
        let wanted = normalize_keymap_name(keymap);
        self.fallbacks
            .iter()
            .find(|(map, _)| map == wanted)
            .map(|(_, kind)| kind)
    }

    /// Store `kind` as `target`'s ANYOTHERKEY fallback, replacing
    /// the previous one (the C slot is a single cell).
    fn set_fallback(&mut self, target: &str, kind: BindKind) {
        match self.fallbacks.iter_mut().find(|(map, _)| map == target) {
            Some(slot) => slot.1 = kind,
            None => self.fallbacks.push((target.to_string(), kind)),
        }
    }

    /// Null `target`'s ANYOTHERKEY fallback (rl_generic_bind's
    /// unbind arm stores _rl_null_function there). Returns true
    /// when a fallback was actually cleared.
    fn clear_fallback(&mut self, target: &str) -> bool {
        let before = self.fallbacks.len();
        self.fallbacks.retain(|(map, _)| map != target);
        self.fallbacks.len() != before
    }

    /// Insert or replace the binding for `(keyseq, keymap)`. GNU's
    /// rl_generic_bind replaces any previous binding of the same sequence.
    /// Registering also revives a sequence a `bind -r`/`-u` removed:
    /// the removal record for `(keyseq, keymap)` is cleared.
    /// A keymap-less entry is stamped with the current keymap (see
    /// [`BindEntry::keymap`]): the binding lands in the map that is
    /// current at registration time and stays there across later
    /// editing-mode flips, exactly like a slot in a C keymap.
    pub fn register(&mut self, entry: BindEntry) {
        // The registry's one identity rule (rubash#2606): the slot
        // is the translated BYTE sequence, not the spelling —
        // rl_generic_bind (lib/readline/bind.c:383) translates with
        // rl_translate_keyseq (bind.c:516) before walking slots, so
        // "\C-u", "\C-U" and a raw 0x15 byte are one slot and the
        // later bind replaces the earlier. Every comparison below
        // (tombstone clear, keymap-slot match, replace) is a
        // byte-to-byte one through keyseq_identity, exactly like
        // the C's slot walk; the stored entry keeps the written
        // form the caller typed (the bind view's contract,
        // niubash#863 GAP-3).
        let mut entry = entry;
        let bytes = keyseq_identity(&entry.keyseq);
        if entry.keymap.is_none() {
            entry.keymap = Some(self.current_keymap());
        }
        let wanted =
            normalize_keymap_name(entry.keymap.as_deref().unwrap_or("emacs-standard")).to_string();
        self.removals.retain(|(seq, keymap)| {
            !(keyseq_identity(seq) == bytes
                && normalize_keymap_name(keymap.as_deref().unwrap_or("emacs-standard")) == wanted)
        });
        // rl_generic_bind's last-key arm: a function or macro bound
        // at a sequence that currently names a KEYMAP descends into
        // that keymap and lands in its ANYOTHERKEY slot — the map
        // stays bound, and the binding becomes the target map's
        // fallback, visible at every prefix of that map.
        if matches!(
            entry.kind,
            BindKind::Function { .. } | BindKind::Macro { .. }
        ) {
            if let Some(target) = self.slot_target(&wanted, &entry.keyseq) {
                self.set_fallback(&target, entry.kind);
                self.generation += 1;
                return;
            }
        }
        if matches!(entry.kind, BindKind::Keymap { .. }) {
            // An ISKMAP assignment replaces the slot's previous
            // function/macro/keymap value outright — the displaced
            // binding is discarded, it does NOT become the new
            // map's fallback (oracle: `bind 'Control-t: undo'` then
            // `bind 'Control-t: prefix-meta'` leaves undo unbound
            // at that key). A `bind -x` command at the same
            // sequence lives in GNU's parallel command map
            // (bashline.c:4771) and survives.
            self.entries.retain(|existing| {
                !(keyseq_identity(&existing.keyseq) == bytes
                    && normalize_keymap_name(
                        existing.keymap.as_deref().unwrap_or("emacs-standard"),
                    ) == wanted
                    && !matches!(existing.kind, BindKind::Execute { .. }))
            });
            self.entries.push(entry);
            self.generation += 1;
            return;
        }
        match self.entries.iter().position(|existing| {
            keyseq_identity(&existing.keyseq) == bytes
                && normalize_keymap_name(existing.keymap.as_deref().unwrap_or("emacs-standard"))
                    == wanted
                && !matches!(existing.kind, BindKind::Keymap { .. })
        }) {
            // The slot replaces wholesale: kind AND written form
            // are the later registration's (GNU keeps no spelling
            // at all; the view's verbatim-written-form rule then
            // reports the spelling that most recently bound the
            // slot).
            Some(position) => self.entries[position] = entry,
            None => self.entries.push(entry),
        }
        self.generation += 1;
    }

    /// Record one `bind -r`/`-u` removal of an exact sequence in one
    /// keymap (see the `removals` field: GNU mutates the map itself,
    /// defaults included). The keymap is stored resolved (an explicit
    /// `-m` map, or the current map for a plain removal): GNU deletes
    /// from one concrete map, so the tombstone must not move when the
    /// editing mode later flips (rubash#2386). Idempotent.
    fn record_removal(&mut self, keyseq: &str, keymap: Option<&str>) {
        // Tombstones name byte sequences too (rubash#2606): a
        // removal recorded under one spelling must shadow the
        // default row and be cleared by a re-registration under
        // any other spelling, so existence is a byte-identity
        // compare; the stored string keeps the written form.
        let bytes = keyseq_identity(keyseq);
        let wanted = self.resolve_keymap(keymap);
        let exists = self.removals.iter().any(|(seq, bound)| {
            keyseq_identity(seq) == bytes
                && normalize_keymap_name(bound.as_deref().unwrap_or("emacs-standard")) == wanted
        });
        if !exists {
            self.removals.push((keyseq.to_string(), Some(wanted)));
            self.generation += 1;
        }
    }

    /// Is `keyseq` removed in `keymap` (a `bind -r`/`-u` record)?
    /// A `None` keymap is the current keymap, as in the C.
    fn is_removed(&self, keyseq: &str, keymap: Option<&str>) -> bool {
        let bytes = keyseq_identity(keyseq);
        let wanted = self.resolve_keymap(keymap);
        self.removals.iter().any(|(seq, bound)| {
            keyseq_identity(seq) == bytes
                && normalize_keymap_name(bound.as_deref().unwrap_or("emacs-standard")) == wanted
        })
    }

    /// The recorded removals, for hosts mirroring the keymaps.
    pub fn removals(&self) -> Vec<(String, Option<String>)> {
        self.removals.clone()
    }

    /// Remove any binding for `keyseq` (bind -r; GNU's unbind_keyseq,
    /// bind.def:395-421, succeeds silently when nothing is bound).
    /// The removal is recorded for every keymap whose default table
    /// binds the sequence, so defaults stay removed too (rubash#2337).
    /// Returns true when an entry was removed.
    pub fn unbind_keyseq(&mut self, keyseq: &str) -> bool {
        // unbind_keyseq (bind.def:391) translates the argument with
        // rl_translate_keyseq before looking the slot up, so any
        // spelling removes the one byte-sequence slot (rubash#2606).
        let bytes = keyseq_identity(keyseq);
        let before = self.entries.len();
        // A keymap-valued binding is never removed by unbinding its
        // sequence: rl_unbind_keyseq at a keymap slot stores
        // _rl_null_function in the map's ANYOTHERKEY and leaves the
        // map itself bound (oracle: `bind -r '\C-t'` after
        // prefix-meta keeps every `"\C-t…"` submap row).
        self.entries.retain(|entry| {
            keyseq_identity(&entry.keyseq) != bytes || matches!(entry.kind, BindKind::Keymap { .. })
        });
        let removed = self.entries.len() != before;
        let mut changed = removed;
        // That same unbind nulls the fallback of every map this
        // sequence is a slot into, whichever prefix set it.
        let targets: Vec<String> = KEYMAP_TABLE_SELECTORS
            .iter()
            .filter_map(|selector| {
                self.slot_target(normalize_keymap_name(selector.unwrap()), keyseq)
            })
            .collect();
        for target in &targets {
            if self.clear_fallback(target) {
                changed = true;
            }
        }
        if changed {
            self.generation += 1;
        }
        for keymap in KEYMAP_TABLE_SELECTORS {
            if crate::builtins::bind::default_bindings_for_keymap(keymap)
                .iter()
                .any(|(seq, _)| keyseq_identity(seq) == bytes)
            {
                self.record_removal(keyseq, keymap);
            }
        }
        removed
    }

    /// Remove every binding for the named function (bind -u,
    /// bind.def:375-393 unbind_command → rl_unbind_function_in_map):
    /// the session entries go away AND the function's default
    /// sequences are recorded as removed in every keymap
    /// (rubash#2337 — GNU mutates the maps themselves).
    /// Returns how many session entries went away.
    pub fn unbind_function(&mut self, name: &str) -> usize {
        let before = self.entries.len();
        self.entries.retain(
            |entry| !matches!(&entry.kind, BindKind::Function { name: bound } if bound == name),
        );
        let removed = before - self.entries.len();
        // A fallback holding the function is a binding of it too:
        // rl_unbind_function_in_map clears a map's ANYOTHERKEY slot
        // when it holds the function.
        let fallbacks_before = self.fallbacks.len();
        self.fallbacks.retain(
            |(_, kind)| !matches!(kind, BindKind::Function { name: bound } if bound == name),
        );
        if removed > 0 || self.fallbacks.len() != fallbacks_before {
            self.generation += 1;
        }
        for keymap in KEYMAP_TABLE_SELECTORS {
            for (seq, bound) in crate::builtins::bind::default_bindings_for_keymap(keymap) {
                if *bound == name {
                    self.record_removal(seq, keymap);
                }
            }
        }
        removed
    }

    /// `bind -u` scoped to one keymap: unbind_command removes the
    /// bindings in the CURRENT keymap only (bind.def:375-393). The
    /// session entries for the function in that keymap go away and the
    /// keymap's default sequences for the function are recorded as
    /// removed (rubash#2337).
    pub fn unbind_function_for_keymap(&mut self, name: &str, keymap: Option<&str>) {
        let wanted = self.resolve_keymap(keymap);
        self.entries.retain(|entry| {
            !(matches!(&entry.kind, BindKind::Function { name: bound } if bound == name)
                && normalize_keymap_name(entry.keymap.as_deref().unwrap_or("emacs-standard"))
                    == wanted)
        });
        // The keymap's own ANYOTHERKEY slot is one of its bindings
        // of the function (rl_unbind_function_in_map clears it).
        if matches!(self.keymap_fallback(&wanted), Some(BindKind::Function { name: bound }) if bound == name)
        {
            self.clear_fallback(&wanted);
            self.generation += 1;
        }
        for (seq, bound) in crate::builtins::bind::default_bindings_for_keymap(Some(&wanted)) {
            if *bound == name {
                self.record_removal(seq, Some(&wanted));
            }
        }
    }

    /// Bindings restricted to one keymap (`None` keymap counts as the
    /// current keymap, as in the C — rubash#2386). A stored entry with
    /// no keymap (constructible only outside [`BindRegistry::register`],
    /// which stamps one) keeps the legacy emacs-standard reading.
    pub fn entries_for_keymap<'a>(
        &'a self,
        keymap: Option<&'a str>,
    ) -> impl Iterator<Item = &'a BindEntry> + 'a {
        let wanted = self.resolve_keymap(keymap);
        self.entries
            .iter()
            .filter(move |entry| match entry.keymap.as_deref() {
                None => wanted == "emacs-standard",
                Some(bound) => normalize_keymap_name(bound) == wanted,
            })
    }

    /// Remove any binding for `keyseq` within one keymap (`None` =
    /// emacs-standard). GNU's `bind -r` unbinds in the current keymap
    /// only (bind.def:395-421 unbind_keyseq), so a vi-insert binding for
    /// the same sequence survives an emacs-standard removal. The
    /// removal is recorded even when no session entry existed — the
    /// sequence may be a DEFAULT binding, and GNU deletes those too
    /// (rubash#2337). Returns true when an entry went away.
    pub fn unbind_keyseq_for_keymap(&mut self, keyseq: &str, keymap: Option<&str>) -> bool {
        // Same translation-first rule as unbind_keyseq (bind.def:391;
        // rubash#2606): the argument's spelling does not matter,
        // only its byte sequence.
        let bytes = keyseq_identity(keyseq);
        let wanted = self.resolve_keymap(keymap);
        let before = self.entries.len();
        self.entries.retain(|entry| {
            if keyseq_identity(&entry.keyseq) != bytes
                || matches!(entry.kind, BindKind::Keymap { .. })
            {
                return true;
            }
            let bound = normalize_keymap_name(entry.keymap.as_deref().unwrap_or("emacs-standard"));
            bound != wanted
        });
        let removed = self.entries.len() != before;
        let mut changed = removed;
        // Unbinding at a keymap-valued slot in this map nulls the
        // target map's ANYOTHERKEY fallback; the map stays bound
        // (see unbind_keyseq).
        if let Some(target) = self.slot_target(&wanted, keyseq) {
            if self.clear_fallback(&target) {
                changed = true;
            }
        }
        if changed {
            self.generation += 1;
        }
        self.record_removal(keyseq, Some(&wanted));
        removed
    }

    /// The key sequences that invoke `function` in one keymap, in
    /// readline's listing order — GNU's rl_invoking_keyseqs_in_map over
    /// the effective map (bind.def query_bindings and
    /// rl_print_keybinding consume exactly this). The effective map is
    /// the keymap's default table (see
    /// [`crate::builtins::bind::default_bindings_for_keymap`]) with
    /// session registrations applied: a registered sequence shadows the
    /// default binding of the same sequence (rl_generic_bind replaces
    /// the slot, whatever kind the new binding is), and the session's
    /// own function bindings join the list. Keymap-valued slots
    /// (the defaults ESC→emacs-meta and C-x→emacs-ctlx, plus session
    /// prefix-meta bindings) are expanded recursively: the target
    /// map's rows compose under the slot's prefix, and the target's
    /// ANYOTHERKEY fallback lists at the bare prefix. The merged list
    /// is sorted in the C scan's order ([`keyseq_scan_order`]).
    /// The returned sequences are PRESENTATION strings: the dumpers
    /// and `bind -q` print readline's canonical rendering of the
    /// byte identity (_rl_get_keyname, bind.c:2580), never the
    /// spelling a sequence was registered under — a function bound
    /// via `"\M-z"` is reported invocable via `"\ez"`, and one bound
    /// under a raw-byte spelling via its `\C-x` form (rubash#2606).
    /// Storage keeps the written forms; only this listing face (and
    /// [`BindRegistry::snapshot`], [`BindRegistry::macro_bindings`])
    /// renders canonical, exactly like the C dumpers.
    pub fn invoking_keyseqs(&self, function: &str, keymap: Option<&str>) -> Vec<String> {
        let wanted = self.resolve_keymap(keymap);
        let mut path = vec![wanted.clone()];
        let mut seqs: Vec<String> = self
            .invoking_in(function, &wanted, &mut path)
            .iter()
            .map(|seq| canonical_keyseq(seq))
            .collect();
        seqs.sort_by(|a, b| keyseq_scan_order(a, b));
        seqs.dedup();
        seqs
    }

    /// The recursive half of [`BindRegistry::invoking_keyseqs`].
    /// `path` is the chain of keymaps whose slots led here; a slot
    /// whose target is already on the path is not expanded again —
    /// readline's own invoking scan recurses without a guard and
    /// crashes on a self-referential keymap (oracle: GNU 5.3.20
    /// segfaults on `bind -m emacs-meta 'Control-t: prefix-meta'`
    /// followed by `bind -m emacs-meta -q transpose-words`), so the
    /// finite answer here is a deliberate divergence from a crash.
    fn invoking_in(&self, function: &str, map: &str, path: &mut Vec<String>) -> Vec<String> {
        let slots = self.keymap_slots(map);
        let shadowed = |seq: &str| {
            self.is_removed(seq, Some(map))
                || self.entries.iter().any(|entry| {
                    keyseq_identity(&entry.keyseq) == keyseq_identity(seq)
                        && normalize_keymap_name(
                            entry.keymap.as_deref().unwrap_or("emacs-standard"),
                        ) == map
                })
        };
        // Direct rows of this map: the default table's rows that do
        // not live under a keymap-valued slot's prefix (those belong
        // to the target map and are composed back by the slot
        // recursion below — for emacs-standard the decomposition is
        // exact: the flattened table is the direct rows plus the
        // meta rows under \e and the ctlx rows under \C-x), minus
        // removals and session-shadowed sequences. All comparisons
        // are byte-identity ones (rubash#2606, see shadowed and
        // is_removed); the rows themselves keep the tables'
        // spellings, which are GNU's dump spellings.
        let mut seqs: Vec<String> = crate::builtins::bind::default_bindings_for_keymap(Some(map))
            .iter()
            .filter(|(_, bound)| *bound == function)
            .map(|(seq, _)| seq.to_string())
            .filter(|seq| {
                !slots
                    .iter()
                    .any(|(prefix, _)| seq.starts_with(prefix.as_str()))
                    && !shadowed(seq)
            })
            .collect();
        for entry in self.entries_for_keymap(Some(map)) {
            if let BindKind::Function { name } = &entry.kind {
                if name == function {
                    seqs.push(entry.keyseq.clone());
                }
            }
        }
        for (prefix, target) in &slots {
            if path.contains(target) {
                continue;
            }
            // The target's ANYOTHERKEY fallback lists at the bare
            // prefix (rl_invoking_keyseqs_in_map reports it there).
            if let Some(BindKind::Function { name }) = self.keymap_fallback(target) {
                if name == function {
                    seqs.push(prefix.clone());
                }
            }
            path.push(target.clone());
            for sub in self.invoking_in(function, target, path) {
                let composed = format!("{prefix}{sub}");
                // A sequence registered at the composed spelling in
                // THIS map wins over the row shining through the
                // slot (the recursive call already applied the
                // target map's own removals and shadows).
                if !shadowed(&composed) {
                    seqs.push(composed);
                }
            }
            path.pop();
        }
        seqs
    }

    /// Every macro visible in one keymap, as (sequence, text) pairs
    /// in readline's scan order — rl_macro_dumper (lib/readline/
    /// bind.c:2899) recurses keymaps exactly like the function
    /// dumper: a macro bound in a target map lists under each of
    /// this map's keymap-valued slots, and a target map's macro
    /// fallback lists at the bare prefix. The sequences are the
    /// dumper's canonical renderings of the byte identity, like
    /// [`BindRegistry::invoking_keyseqs`] — storage keeps the
    /// written forms (rubash#2606).
    pub fn macro_bindings(&self, keymap: Option<&str>) -> Vec<(String, String)> {
        let wanted = self.resolve_keymap(keymap);
        let mut path = vec![wanted.clone()];
        let mut macros: Vec<(String, String)> = self
            .macros_in(&wanted, &mut path)
            .into_iter()
            .map(|(seq, text)| (canonical_keyseq(&seq), text))
            .collect();
        macros.sort_by(|a, b| keyseq_scan_order(&a.0, &b.0));
        macros.dedup();
        macros
    }

    /// The recursive half of [`BindRegistry::macro_bindings`]; the
    /// visited-path rule is [`BindRegistry::invoking_in`]'s.
    fn macros_in(&self, map: &str, path: &mut Vec<String>) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for entry in self.entries_for_keymap(Some(map)) {
            if let BindKind::Macro { text } = &entry.kind {
                out.push((entry.keyseq.clone(), text.clone()));
            }
        }
        for (prefix, target) in self.keymap_slots(map) {
            if path.contains(&target) {
                continue;
            }
            if let Some(BindKind::Macro { text }) = self.keymap_fallback(&target) {
                out.push((prefix.clone(), text.clone()));
            }
            path.push(target.clone());
            for (sub, text) in self.macros_in(&target, path) {
                out.push((format!("{prefix}{sub}"), text));
            }
            path.pop();
        }
        out
    }

    pub fn snapshot(&self) -> Vec<BindEntry> {
        // The snapshot is the face `bind -X` prints from
        // (Executor::bind_registry_snapshot → the list_execute path)
        // and hosts mirror from. GNU's command map has no
        // registration order to preserve: print_unix_command_map
        // (bashline.c:4729) dumps it through rl_macro_dumper
        // (lib/readline/bind.c:2899), whose slot scan IS
        // translated-byte order (rubash#3194). Present the Execute
        // entries in that scan order — derived from the same byte
        // identity as registration (rubash#2606) — by permuting
        // them among the positions Execute entries already occupy;
        // every other kind keeps its registry position, and
        // per-keymap filtering of the result stays sorted because
        // a subsequence of a sorted sequence is sorted. The
        // presented keyseqs are the canonical renderings of the
        // byte identity (the dumper prints bytes, never the
        // spelling that was typed); the registry's stored written
        // forms are untouched — this Vec is a presentation copy,
        // and the tier-A bind view does not come through here.
        let mut entries: Vec<BindEntry> = self
            .entries
            .iter()
            .map(|entry| {
                let mut presented = entry.clone();
                presented.keyseq = canonical_keyseq(&entry.keyseq);
                presented
            })
            .collect();
        let mut execute: Vec<BindEntry> = entries
            .iter()
            .filter(|entry| matches!(entry.kind, BindKind::Execute { .. }))
            .cloned()
            .collect();
        execute.sort_by(|a, b| keyseq_scan_order(&a.keyseq, &b.keyseq));
        let mut execute = execute.into_iter();
        for entry in entries.iter_mut() {
            if matches!(entry.kind, BindKind::Execute { .. }) {
                *entry = execute
                    .next()
                    .expect("execute subsequence length is stable");
            }
        }
        entries
    }
}

/// Byte identity of a written key sequence: the translated byte
/// sequence (rl_translate_keyseq, lib/readline/bind.c:516). GNU's
/// registry is keyed on bytes, not spellings — rl_generic_bind
/// (bind.c:383) translates before walking slots and
/// unbind_keyseq (builtins/bind.def:391) translates first too —
/// so "\C-u", "\C-U" and a raw 0x15 byte are one slot
/// (rubash#2606). Every identity comparison in this registry
/// (replace, removal, tombstone, slot match) goes through this
/// one function. The stored keyseq keeps the written form
/// verbatim: the tier-A bind view contract pins exactly that
/// split (niubash#863 GAP-3 — the view's keyseq field is the
/// written form, its bytes field this translation).
///
/// The translation runs on the DECODED byte string, never on the
/// transport text: a raw byte that reaches the builtin through
/// $'...' quoting rides the raw-byte marker pair protocol
/// (RAW_BYTE_MARKER_*, src/executor/markers.rs), so the written
/// form is first folded back to the bytes the script produced by
/// the protocol's one byte-domain decoder
/// ([`crate::executor::substitution_metadata::decode_raw_byte_markers`])
/// and the escape walk then runs on those bytes
/// ([`translate_keyseq_bytes`]). GNU's translator sees nothing
/// but bytes: a pair-carried 0x15 is byte 0x15, not the pair's
/// UTF-8 — comparing the pair's UTF-8 instead was the blind-
/// consumer shape that left the raw-byte spelling family in its
/// own slots (captain review of PR #3237).
fn keyseq_identity(written: &str) -> Vec<u8> {
    let folded =
        crate::executor::substitution_metadata::decode_raw_byte_markers(written.as_bytes());
    translate_keyseq_bytes(&folded)
}

/// rl_translate_keyseq (lib/readline/bind.c:516) over an already
/// decoded byte string: the same escape walk as
/// [`crate::builtins::bind::keyseq_bytes`], but on bytes, because
/// GNU's translator consumes the byte string the script's quoting
/// produced — a raw byte that happens to be 0x5C enters the walk
/// as an escape introducer exactly like a written backslash, and
/// a byte >= 0x80 stands as itself rather than as a UTF-8 lead
/// byte. Kept arm-for-arm parallel to `keyseq_bytes`; the two
/// agree on every input that carries no marker pairs.
fn translate_keyseq_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    // The base byte of a \C- or \M- argument: one byte, or an
    // escape pair when the base is itself backslash/quote (\C-\\).
    let read_base = |bytes: &[u8], i: &mut usize| -> u32 {
        match bytes.get(*i) {
            Some(b'\\') => {
                let base: u8 = match bytes.get(*i + 1) {
                    Some(b'\\') => 0x5c,
                    Some(b'"') => 0x22,
                    _ => 0x5c,
                };
                *i += 2;
                u32::from(base)
            }
            Some(&c) => {
                *i += 1;
                u32::from(c)
            }
            None => 0,
        }
    };
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            Some(b'C') if bytes.get(i + 2) == Some(&b'-') => {
                i += 3;
                let base = read_base(bytes, &mut i);
                out.push(if base == u32::from(b'?') {
                    0x7f
                } else {
                    (base & 0x1f) as u8
                });
            }
            Some(b'M') if bytes.get(i + 2) == Some(&b'-') => {
                i += 3;
                let base = read_base(bytes, &mut i);
                out.push(0x1b);
                out.push(base as u8);
            }
            Some(b'e') => {
                out.push(0x1b);
                i += 2;
            }
            Some(b'd') => {
                out.push(0x7f);
                i += 2;
            }
            Some(b'\\') => {
                out.push(0x5c);
                i += 2;
            }
            Some(b'a') => {
                out.push(0x07);
                i += 2;
            }
            Some(b'b') => {
                out.push(0x08);
                i += 2;
            }
            Some(b'f') => {
                out.push(0x0c);
                i += 2;
            }
            Some(b'n') => {
                out.push(0x0a);
                i += 2;
            }
            Some(b'r') => {
                out.push(0x0d);
                i += 2;
            }
            Some(b't') => {
                out.push(0x09);
                i += 2;
            }
            Some(b'v') => {
                out.push(0x0b);
                i += 2;
            }
            Some(b'x') => {
                let hex = &bytes[(i + 2).min(bytes.len())..(i + 4).min(bytes.len())];
                let value = std::str::from_utf8(hex)
                    .ok()
                    .filter(|text| !text.is_empty())
                    .and_then(|text| u8::from_str_radix(text, 16).ok());
                match value {
                    Some(byte) => {
                        out.push(byte);
                        i += 2 + hex.len();
                    }
                    None => {
                        out.push(b'x');
                        i += 2;
                    }
                }
            }
            Some(&d) if d.is_ascii_digit() => {
                let digits = &bytes[i + 1..]
                    .iter()
                    .take(3)
                    .take_while(|d| d.is_ascii_digit())
                    .copied()
                    .collect::<Vec<u8>>();
                let value = std::str::from_utf8(digits)
                    .ok()
                    .and_then(|text| u32::from_str_radix(text, 8).ok());
                match value {
                    Some(value) => {
                        out.push(value as u8);
                        i += 1 + digits.len();
                    }
                    None => {
                        out.push(d);
                        i += 2;
                    }
                }
            }
            Some(&other) => {
                out.push(other);
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

/// The canonical written form of a key sequence: translate the
/// spelling to its byte identity ([`keyseq_identity`]) and render
/// the bytes back the way readline's dumpers do — per byte
/// through _rl_get_keyname (bind.c:2580): ESC renders `\e`, a
/// control byte or RUBOUT renders `\C-x` (lowercase, `?` for
/// RUBOUT), a byte in 128..=159 renders as `\2oo` and one in
/// 160..=255 as `\ooo` (the dumper's octal forms), `\\` and `\"`
/// take a backslash, and every other byte stands as its own
/// character. Presentation only: `bind -X`'s snapshot renders
/// entries through it (storage keeps the written form, see
/// [`keyseq_identity`]); two spellings are the same key sequence
/// exactly when their canonical forms are equal, because both
/// are functions of the one byte identity.
pub(crate) fn canonical_keyseq(written: &str) -> String {
    let mut out = String::new();
    for byte in keyseq_identity(written) {
        match byte {
            0x1b => out.push_str("\\e"),
            0x7f => out.push_str("\\C-?"),
            0x00..=0x1f => {
                out.push_str("\\C-");
                // _rl_get_keyname: the letter is _rl_to_lower of
                // UNCTRL(c) — the byte XOR 0x40 — and a letter that
                // is itself a backslash or a quote takes the
                // dumper's backslash prefix like any other byte
                // (byte 0x1c therefore renders "\C-\\").
                let letter = char::from(byte ^ 0x40).to_ascii_lowercase();
                if letter == '\\' || letter == '"' {
                    out.push('\\');
                }
                out.push(letter);
            }
            0x80..=0x9f => {
                // _rl_get_keyname's 128..=159 arm: "\2" followed by
                // the two low octal digits of (byte - 128).
                out.push_str("\\2");
                out.push(char::from_digit(u32::from(byte - 128) / 8, 8).expect("octal digit"));
                out.push(char::from_digit(u32::from(byte % 8), 8).expect("octal digit"));
            }
            0xa0..=0xff => {
                // _rl_get_keyname's >= 160 arm: a three-digit octal
                // escape of the byte itself.
                out.push('\\');
                out.push(char::from_digit(u32::from(byte >> 6) & 7, 8).expect("octal digit"));
                out.push(char::from_digit(u32::from(byte >> 3) & 7, 8).expect("octal digit"));
                out.push(char::from_digit(u32::from(byte & 7), 8).expect("octal digit"));
            }
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            byte => out.push(char::from(byte)),
        }
    }
    out
}

/// The five keymaps with default tables, as selectors for
/// [`crate::builtins::bind::default_bindings_for_keymap`]. The first
/// names the emacs-standard table explicitly (it used to be `None`,
/// back when `None` resolved to emacs-standard unconditionally;
/// since rubash#2386 `None` resolves to the CURRENT keymap, the
/// all-keymaps unbind sweeps must name this table to record their
/// tombstones against the map they actually checked).
const KEYMAP_TABLE_SELECTORS: [Option<&str>; 5] = [
    Some("emacs-standard"),
    Some("emacs-meta"),
    Some("emacs-ctlx"),
    Some("vi-insert"),
    Some("vi-movement"),
];

/// Listing order for invoking sequences: rl_invoking_keyseqs_in_map
/// scans a keymap's slots in translated-byte order with ANYOTHERKEY
/// (index 256) as the array's LAST slot, so a sequence that is a
/// strict prefix of another sorts AFTER the longer sequences
/// through it — the bare prefix carries the submap's fallback and
/// prints after the submap's own rows (GNU probe:
/// `bind 'Control-x: undo'` makes `bind -q undo` answer
/// `"\C-x\C-u", "\C-x", "\C-_"`). Modelled by comparing the
/// translated bytes with end-of-sequence counting as 256.
pub(crate) fn keyseq_scan_order(a: &str, b: &str) -> std::cmp::Ordering {
    let a = keyseq_identity(a);
    let b = keyseq_identity(b);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).map_or(256u32, |byte| u32::from(*byte));
        let y = b.get(i).map_or(256u32, |byte| u32::from(*byte));
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            order => return order,
        }
    }
    std::cmp::Ordering::Equal
}

/// Keymap aliases that are the same readline map (bind.def:37-40: vi,
/// vi-move and vi-command name the movement keymap; emacs and
/// emacs-standard the standard one).
pub(crate) fn normalize_keymap_name(name: &str) -> &str {
    match name {
        "emacs" => "emacs-standard",
        "vi" | "vi-move" | "vi-command" => "vi-movement",
        other => other,
    }
}

/// The `-m` names GNU accepts (bind.def:37-40). Anything else is
/// ``bind: `<name>': invalid keymap name`` (bind.def:253-257).
pub(crate) const KEYMAP_NAMES: &[&str] = &[
    "emacs",
    "emacs-standard",
    "emacs-meta",
    "emacs-ctlx",
    "vi",
    "vi-move",
    "vi-command",
    "vi-insert",
];

/// Process-shared registry handle. Shared across ShellState clones so a
/// `bind` inside a subshell mutates the same world as the parent — GNU's
/// readline state is process-global, not fork-copied.
pub(crate) type SharedBindRegistry = Rc<RefCell<BindRegistry>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn xentry(keyseq: &str, command: &str) -> BindEntry {
        BindEntry {
            keyseq: keyseq.to_string(),
            kind: BindKind::Execute {
                command: command.to_string(),
            },
            keymap: None,
        }
    }

    #[test]
    fn register_replaces_same_keyseq_and_bumps_generation() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "one"));
        let gen = registry.generation;
        registry.register(xentry("\\C-g", "two"));
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.generation, gen + 1);
        match &registry.entries[0].kind {
            BindKind::Execute { command } => assert_eq!(command, "two"),
            other => panic!("unexpected kind: {other:?}"),
        }
    }

    #[test]
    fn unbind_keyseq_removes_and_reports() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "one"));
        assert!(registry.unbind_keyseq("\\C-g"));
        assert!(!registry.unbind_keyseq("\\C-g"));
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn unbind_function_only_touches_function_entries() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "cmd"));
        registry.register(BindEntry {
            keyseq: "\\C-a".to_string(),
            kind: BindKind::Function {
                name: "beginning-of-line".to_string(),
            },
            keymap: None,
        });
        assert_eq!(registry.unbind_function("beginning-of-line"), 1);
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn entries_for_keymap_defaults_to_emacs_standard_and_folds_aliases() {
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-g", "default"));
        registry.register(BindEntry {
            keyseq: "\\C-i".to_string(),
            kind: BindKind::Execute {
                command: "vi-insert-only".to_string(),
            },
            keymap: Some("vi-insert".to_string()),
        });
        let names: Vec<_> = registry
            .entries_for_keymap(None)
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(names, vec!["\\C-g"]);
        let vi_names: Vec<_> = registry
            .entries_for_keymap(Some("vi"))
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert!(vi_names.is_empty(), "vi alias must not match vi-insert");
        let viins_names: Vec<_> = registry
            .entries_for_keymap(Some("vi-insert"))
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(viins_names, vec!["\\C-i"]);
    }

    #[test]
    fn invoking_keyseqs_reports_gnu_defaults_in_scan_order() {
        // rubash#2338: the oracle's `bind -q backward-char` answer,
        // per keymap (hunt cases bd01/bd27/bd28).
        let registry = BindRegistry::default();
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\C-b", r"\eOD", r"\e[D"]
        );
        assert_eq!(
            registry.invoking_keyseqs("backward-char", Some("vi-insert")),
            vec![r"\eOD", r"\e[D"]
        );
        assert_eq!(
            registry.invoking_keyseqs("backward-char", Some("vi-command")),
            vec![r"\C-h", r"\eOD", r"\e[D", "h"]
        );
        assert_eq!(registry.invoking_keyseqs("yank", None), vec![r"\C-y"]);
        assert_eq!(
            registry.invoking_keyseqs("complete", None),
            vec![r"\C-i", r"\e\e"]
        );
    }

    #[test]
    fn invoking_keyseqs_merges_session_bindings_in_scan_order() {
        // A session binding joins the list at its byte position; a
        // session binding for an existing default sequence shadows the
        // default (the sequence leaves the old function's list).
        let mut registry = BindRegistry::default();
        registry.register(BindEntry {
            keyseq: r"\ez".to_string(),
            kind: BindKind::Function {
                name: "backward-char".to_string(),
            },
            keymap: None,
        });
        registry.register(BindEntry {
            keyseq: r"\C-b".to_string(),
            kind: BindKind::Function {
                name: "forward-char".to_string(),
            },
            keymap: None,
        });
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\eOD", r"\e[D", r"\ez"]
        );
        assert_eq!(
            registry.invoking_keyseqs("forward-char", None),
            vec![r"\C-b", r"\C-f", r"\eOC", r"\e[C"]
        );
    }

    #[test]
    fn unbind_keyseq_removes_default_bindings_too() {
        // rubash#2337 (hunt bd07): `bind -r "\C-b"` must delete the
        // DEFAULT binding, not just session entries — GNU's maps are
        // mutable (bind.def:390-427).
        let mut registry = BindRegistry::default();
        registry.unbind_keyseq_for_keymap(r"\C-b", None);
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\eOD", r"\e[D"]
        );
        // Other keymaps are untouched.
        assert_eq!(
            registry.invoking_keyseqs("backward-char", Some("vi-command")),
            vec![r"\C-h", r"\eOD", r"\e[D", "h"]
        );
        // Rebinding the sequence revives it.
        registry.register(BindEntry {
            keyseq: r"\C-b".to_string(),
            kind: BindKind::Function {
                name: "backward-char".to_string(),
            },
            keymap: None,
        });
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\C-b", r"\eOD", r"\e[D"]
        );
    }

    #[test]
    fn unbind_function_removes_defaults_in_that_keymap_only() {
        // rubash#2337: `bind -u backward-char` deletes every default
        // sequence for the function in the current keymap
        // (bind.def:375-388); the vi-insert map keeps its own.
        let mut registry = BindRegistry::default();
        registry.unbind_function_for_keymap("backward-char", None);
        assert!(registry.invoking_keyseqs("backward-char", None).is_empty());
        assert_eq!(
            registry.invoking_keyseqs("backward-char", Some("vi-insert")),
            vec![r"\eOD", r"\e[D"]
        );
        // Scoped to vi: `bind -m vi-command -u backward-char`.
        registry.unbind_function_for_keymap("backward-char", Some("vi-command"));
        assert!(registry
            .invoking_keyseqs("backward-char", Some("vi"))
            .is_empty());
    }

    #[test]
    fn invoking_keyseqs_follow_the_current_keymap() {
        // rubash#2386: with no -m, GNU's dumpers read the CURRENT
        // keymap (_rl_keymap). `set editing-mode vi` moves it to
        // vi-insert (sv_editmode, bind.c:2097), so the same query
        // answers from the vi-insert table.
        let mut registry = BindRegistry::default();
        registry.set_readline_variable("editing-mode", "vi");
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\eOD", r"\e[D"]
        );
        // The `keymap` variable selects the map directly (bind.c:2517),
        // and reads back under its canonical name (bind.c:3018).
        registry.set_readline_variable("keymap", "vi-command");
        assert_eq!(registry.variable_value("keymap"), Some("vi"));
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\C-h", r"\eOD", r"\e[D", "h"]
        );
        registry.set_readline_variable("editing-mode", "emacs");
        assert_eq!(
            registry.invoking_keyseqs("backward-char", None),
            vec![r"\C-b", r"\eOD", r"\e[D"]
        );
    }

    #[test]
    fn registration_lands_in_the_keymap_current_at_bind_time() {
        // rubash#2386: rl_parse_and_bind writes into the current map;
        // the binding stays there when the mode later flips.
        let mut registry = BindRegistry::default();
        registry.set_readline_variable("editing-mode", "vi");
        registry.register(BindEntry {
            keyseq: r"\C-a".to_string(),
            kind: BindKind::Function {
                name: "kill-line".to_string(),
            },
            keymap: None,
        });
        assert_eq!(registry.entries[0].keymap.as_deref(), Some("vi-insert"));
        assert_eq!(registry.invoking_keyseqs("kill-line", None), vec![r"\C-a"]);
        // The emacs map is untouched by the vi registration: \C-a
        // still invokes beginning-of-line there (a contains check —
        // the exact list is the #2338 tables' business, not this
        // routing test's).
        assert!(registry
            .invoking_keyseqs("beginning-of-line", Some("emacs"))
            .iter()
            .any(|seq| seq == r"\C-a"));
        registry.set_readline_variable("editing-mode", "emacs");
        assert_eq!(registry.invoking_keyseqs("kill-line", None), vec![r"\C-k"]);
    }

    #[test]
    fn removal_tombstone_stays_in_its_keymap_across_mode_flips() {
        // rubash#2386: `bind -r` in vi mode deletes from the vi-insert
        // map only; flipping back to emacs must not reinterpret the
        // tombstone as an emacs removal.
        let mut registry = BindRegistry::default();
        registry.set_readline_variable("editing-mode", "vi");
        registry.unbind_keyseq_for_keymap(r"\C-a", None);
        assert!(registry
            .invoking_keyseqs("self-insert", Some("vi-insert"))
            .iter()
            .all(|seq| seq != r"\C-a"));
        registry.set_readline_variable("editing-mode", "emacs");
        assert!(registry
            .invoking_keyseqs("beginning-of-line", None)
            .iter()
            .any(|seq| seq == r"\C-a"));
    }

    fn keymap_entry(keyseq: &str, target: &str, keymap: Option<&str>) -> BindEntry {
        BindEntry {
            keyseq: keyseq.to_string(),
            kind: BindKind::Keymap {
                name: target.to_string(),
            },
            keymap: keymap.map(str::to_string),
        }
    }

    fn function_entry(keyseq: &str, name: &str, keymap: Option<&str>) -> BindEntry {
        BindEntry {
            keyseq: keyseq.to_string(),
            kind: BindKind::Function {
                name: name.to_string(),
            },
            keymap: keymap.map(str::to_string),
        }
    }

    #[test]
    fn prefix_meta_expands_the_target_keymap() {
        // rubash#2923: a Keymap registration composes the target
        // map's rows under its prefix and displaces the slot's
        // previous default (transpose-chars is gone, as in GNU).
        let mut registry = BindRegistry::default();
        registry.register(keymap_entry("\\C-t", "emacs-meta", None));
        assert_eq!(
            registry.invoking_keyseqs("transpose-words", None),
            vec![r"\C-tt", r"\et"]
        );
        assert!(registry
            .invoking_keyseqs("transpose-chars", None)
            .is_empty());
    }

    #[test]
    fn function_at_a_keymap_slot_becomes_the_shared_fallback() {
        // rubash#2923: the fallback belongs to the target map, so
        // it lists at the default ESC prefix and at the session
        // prefix alike; unbinding at either prefix clears it and
        // leaves both maps bound.
        let mut registry = BindRegistry::default();
        registry.register(function_entry("\\e", "undo", None));
        assert!(matches!(
            registry.keymap_fallback("emacs-meta"),
            Some(BindKind::Function { name }) if name == "undo"
        ));
        registry.register(keymap_entry("\\C-t", "emacs-meta", None));
        assert_eq!(
            registry.invoking_keyseqs("undo", None),
            vec![r"\C-t", r"\C-x\C-u", r"\e", r"\C-_"]
        );
        registry.unbind_keyseq_for_keymap("\\C-t", None);
        assert_eq!(
            registry.invoking_keyseqs("undo", None),
            vec![r"\C-x\C-u", r"\C-_"]
        );
        assert_eq!(
            registry.invoking_keyseqs("transpose-words", None),
            vec![r"\C-tt", r"\et"]
        );
    }

    #[test]
    fn keymap_bind_discards_the_slots_previous_function() {
        // rubash#2923: the ISKMAP assignment replaces the slot —
        // the function bound there first is not saved anywhere.
        let mut registry = BindRegistry::default();
        registry.register(function_entry("\\C-t", "undo", None));
        registry.register(keymap_entry("\\C-t", "emacs-meta", None));
        assert_eq!(
            registry.invoking_keyseqs("undo", None),
            vec![r"\C-x\C-u", r"\C-_"]
        );
        assert!(registry.keymap_fallback("emacs-meta").is_none());
    }

    #[test]
    fn self_referential_keymap_expansion_terminates() {
        // rubash#2923: GNU crashes on this query; the visited-path
        // guard returns the finite rows.
        let mut registry = BindRegistry::default();
        registry.register(keymap_entry("\\C-t", "emacs-meta", Some("emacs-meta")));
        assert_eq!(
            registry.invoking_keyseqs("transpose-words", Some("emacs-meta")),
            vec!["t"]
        );
        assert_eq!(
            registry.invoking_keyseqs("transpose-words", None),
            vec![r"\et"]
        );
    }

    #[test]
    fn canonical_keyseq_renders_translated_bytes_like_the_dumpers() {
        // _rl_get_keyname (lib/readline/bind.c:2580) forms, checked
        // against the spellings the #2606 probes pinned.
        assert_eq!(canonical_keyseq("\\C-u"), "\\C-u");
        assert_eq!(canonical_keyseq("\\C-U"), "\\C-u");
        assert_eq!(canonical_keyseq("\u{15}"), "\\C-u");
        assert_eq!(canonical_keyseq("\\d"), "\\C-?");
        assert_eq!(canonical_keyseq("\u{7f}"), "\\C-?");
        assert_eq!(canonical_keyseq("\\M-z"), "\\ez");
        assert_eq!(canonical_keyseq("\\ez"), "\\ez");
        assert_eq!(canonical_keyseq("\\C-@"), "\\C-@");
        assert_eq!(canonical_keyseq("\u{1c}"), "\\C-\\\\");
        assert_eq!(canonical_keyseq("\\e[D"), "\\e[D");
        // Idempotent: rendering a canonical form changes nothing.
        for spelling in [
            "\\C-u",
            "\\C-?",
            "\\ez",
            "\\C-\\\\",
            "\\e[D",
            "a",
            "\\C-x\\C-f",
        ] {
            assert_eq!(canonical_keyseq(&canonical_keyseq(spelling)), spelling);
        }
    }

    #[test]
    fn default_table_spellings_are_canonical() {
        // The snapshot renders stored sequences through the
        // canonical rendering for presentation; the default
        // tables' spellings are GNU's own dump spellings, so each
        // must be a fixed point of the rendering — a row that
        // moved would mean the renderer disagrees with the
        // dumper forms the tables were transcribed from.
        for selector in KEYMAP_TABLE_SELECTORS {
            for (seq, _) in crate::builtins::bind::default_bindings_for_keymap(selector) {
                assert_eq!(canonical_keyseq(seq), *seq, "table row for {selector:?}");
            }
        }
    }

    #[test]
    fn register_keys_identity_on_bytes_not_spelling() {
        // rubash#2606: "\C-u" and the raw-byte spelling are one
        // slot — the second registration replaces the first. The
        // stored form is the later registration's written form
        // (the bind view reports written forms verbatim,
        // niubash#863 GAP-3); identity never consults it.
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-u", "one"));
        registry.register(xentry("\u{15}", "two"));
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].keyseq, "\u{15}");
        match &registry.entries[0].kind {
            BindKind::Execute { command } => assert_eq!(command, "two"),
            other => panic!("unexpected kind: {other:?}"),
        }
        // Removal by the other spelling removes the slot, and a
        // re-registration under the first spelling clears the
        // tombstone the removal recorded.
        assert!(registry.unbind_keyseq_for_keymap("\u{15}", None));
        assert!(registry.entries.is_empty());
        registry.register(xentry("\\C-u", "three"));
        assert!(!registry.is_removed("\\C-u", None));
        assert_eq!(registry.entries.len(), 1);
        // The snapshot (the `bind -X` face) presents the slot in
        // canonical form regardless of the stored spelling.
        registry.register(xentry("\u{15}", "four"));
        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].keyseq, "\\C-u");
    }

    /// The transport spelling of raw bytes: each byte rides a
    /// raw-byte marker pair (RAW_BYTE_MARKER_*, src/executor/
    /// markers.rs), which is the form a $'...' byte actually has
    /// when it reaches the builtin (captain review of PR #3237 —
    /// a bare "\u{15}" char was never the failing shape).
    fn raw_pair_spelling(bytes: &[u8]) -> String {
        let escape =
            char::from_u32(crate::executor::markers::RAW_BYTE_MARKER_ESCAPE).expect("escape char");
        let mut spelling = String::new();
        for byte in bytes {
            spelling.push(escape);
            spelling.push(
                char::from_u32(crate::executor::markers::RAW_BYTE_MARKER_FIRST + u32::from(*byte))
                    .expect("payload char"),
            );
        }
        spelling
    }

    #[test]
    fn identity_folds_raw_byte_carrier_pairs() {
        // rubash#2606, captain rework: identity is computed on the
        // decoded byte string, so the pair-carried spellings of
        // 0x15 / 0x07 / 0x7f share the slots of "\C-u" / "\C-g" /
        // "\C-?", and a pair-carried high byte stands as the byte
        // itself (the folded [0xC3, 0xA9] is also the written
        // form é's identity — GNU's slot bytes either way).
        assert_eq!(keyseq_identity(&raw_pair_spelling(&[0x15])), vec![0x15]);
        assert_eq!(
            keyseq_identity(&raw_pair_spelling(&[0x15])),
            keyseq_identity("\\C-u")
        );
        assert_eq!(
            keyseq_identity(&raw_pair_spelling(&[0x07])),
            keyseq_identity("\\C-g")
        );
        assert_eq!(
            keyseq_identity(&raw_pair_spelling(&[0x7f])),
            keyseq_identity("\\C-?")
        );
        assert_eq!(
            keyseq_identity(&raw_pair_spelling(&[0xc3, 0xa9])),
            keyseq_identity("é")
        );
        assert_eq!(canonical_keyseq(&raw_pair_spelling(&[0x15])), "\\C-u");
    }

    #[test]
    fn register_keys_identity_on_raw_pair_spelling() {
        // The pair-carried raw spelling replaces, is removed by
        // the backslash spelling, and its re-registration clears
        // the tombstone — the whole spelling family is one slot
        // (the PR #3237 head left these in parallel slots).
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-u", "one"));
        registry.register(xentry(&raw_pair_spelling(&[0x15]), "two"));
        assert_eq!(registry.entries.len(), 1);
        match &registry.entries[0].kind {
            BindKind::Execute { command } => assert_eq!(command, "two"),
            other => panic!("unexpected kind: {other:?}"),
        }
        assert!(registry.unbind_keyseq_for_keymap("\\C-u", None));
        assert!(registry.entries.is_empty());
        registry.register(xentry(&raw_pair_spelling(&[0x15]), "three"));
        assert!(!registry.is_removed("\\C-u", None));
        // Removal by the bare pair spelling removes a slot that
        // was registered under the backslash spelling.
        assert!(registry.unbind_keyseq_for_keymap(&raw_pair_spelling(&[0x15]), None));
        assert!(registry.entries.is_empty());
    }

    #[test]
    fn listing_faces_render_canonical_spellings() {
        // rubash#2606: the invoking listing (`bind -q` / -p) and
        // the macro listing (`bind -s`) print the canonical
        // rendering of the byte identity — a function bound via
        // "\M-z" lists as "\ez", a macro bound under the pair-
        // carried raw spelling lists as "\C-u".
        let mut registry = BindRegistry::default();
        registry.register(BindEntry {
            keyseq: "\\M-z".to_string(),
            kind: BindKind::Function {
                name: "kill-whole-line".to_string(),
            },
            keymap: None,
        });
        let seqs = registry.invoking_keyseqs("kill-whole-line", None);
        assert!(seqs.contains(&"\\ez".to_string()), "got {seqs:?}");
        assert!(!seqs.iter().any(|seq| seq == "\\M-z"), "got {seqs:?}");
        registry.register(BindEntry {
            keyseq: raw_pair_spelling(&[0x15]),
            kind: BindKind::Macro {
                text: "text".to_string(),
            },
            keymap: None,
        });
        let macros = registry.macro_bindings(None);
        assert_eq!(macros, vec![("\\C-u".to_string(), "text".to_string())]);
    }

    #[test]
    fn snapshot_lists_execute_entries_in_scan_order() {
        // rubash#3194: the snapshot (the `bind -X` face) presents
        // shell-command entries in translated-byte scan order, not
        // registration order; other kinds keep their positions.
        let mut registry = BindRegistry::default();
        registry.register(xentry("\\C-u", "u"));
        registry.register(BindEntry {
            keyseq: "\\C-t".to_string(),
            kind: BindKind::Macro {
                text: "m".to_string(),
            },
            keymap: None,
        });
        registry.register(xentry("\\C-g", "g"));
        registry.register(xentry("\\C-x\\C-f", "xf"));
        registry.register(xentry("\\C-x", "x"));
        let snapshot = registry.snapshot();
        let seqs: Vec<&str> = snapshot
            .iter()
            .filter(|entry| matches!(entry.kind, BindKind::Execute { .. }))
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(seqs, vec!["\\C-g", "\\C-u", "\\C-x\\C-f", "\\C-x"]);
        // The stored order is untouched (registration order).
        let stored: Vec<&str> = registry
            .entries
            .iter()
            .map(|entry| entry.keyseq.as_str())
            .collect();
        assert_eq!(
            stored,
            vec!["\\C-u", "\\C-t", "\\C-g", "\\C-x\\C-f", "\\C-x"]
        );
    }

    #[test]
    fn shared_handle_is_shared_across_clone() {
        let shared: SharedBindRegistry = Rc::new(RefCell::new(BindRegistry::default()));
        let cloned = shared.clone();
        shared.borrow_mut().register(xentry("\\C-g", "one"));
        assert_eq!(cloned.borrow().entries.len(), 1);
    }
}
