use std::collections::HashMap;

use crate::parser::Ast;

/// Maximum number of cached runtime ASTs per executor.
pub(crate) const RUNTIME_PARSE_CACHE_MAX_ENTRIES: usize = 128;

/// Maximum source byte length eligible for caching so large one-off scripts
/// do not retain unbounded memory.
const RUNTIME_PARSE_CACHE_MAX_SOURCE_BYTES: usize = 256 * 1024;

/// Discriminator for runtime text parse entrypoints whose tokenization or
/// parser options differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RuntimeParseKind {
    Eval,
    Trap,
    Source,
}

/// Content-and-parser-state key for cached runtime ASTs (`eval`, `trap`, `source`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RuntimeParseCacheKey {
    pub(crate) kind: RuntimeParseKind,
    pub(crate) source: String,
    pub(crate) diagnostic_text: Option<String>,
    pub(crate) line_offset: usize,
    pub(crate) posix_mode: bool,
    pub(crate) extglob: bool,
    pub(crate) alias_expansion_enabled: bool,
    pub(crate) alias_generation: u64,
    pub(crate) stray_close_is_error: bool,
}

impl RuntimeParseCacheKey {
    pub(crate) fn eval(
        source: &str,
        line_offset: usize,
        posix_mode: bool,
        extglob: bool,
        alias_expansion_enabled: bool,
        alias_generation: u64,
    ) -> Self {
        Self {
            kind: RuntimeParseKind::Eval,
            source: source.to_string(),
            diagnostic_text: None,
            line_offset,
            posix_mode,
            extglob,
            alias_expansion_enabled,
            alias_generation,
            stray_close_is_error: true,
        }
    }

    pub(crate) fn trap(
        source: &str,
        line_offset: usize,
        posix_mode: bool,
        extglob: bool,
        alias_expansion_enabled: bool,
        alias_generation: u64,
    ) -> Self {
        Self {
            kind: RuntimeParseKind::Trap,
            source: source.to_string(),
            diagnostic_text: None,
            line_offset,
            posix_mode,
            extglob,
            alias_expansion_enabled,
            alias_generation,
            stray_close_is_error: false,
        }
    }

    pub(crate) fn source(
        source: &str,
        diagnostic_text: Option<&str>,
        line_offset: usize,
        posix_mode: bool,
        extglob: bool,
        alias_expansion_enabled: bool,
        alias_generation: u64,
    ) -> Self {
        Self {
            kind: RuntimeParseKind::Source,
            source: source.to_string(),
            diagnostic_text: diagnostic_text.map(str::to_string),
            line_offset,
            posix_mode,
            extglob,
            alias_expansion_enabled,
            alias_generation,
            stray_close_is_error: true,
        }
    }
}

/// Key for the single-slot zero-clone DEBUG trap fast-path cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugTrapCacheKey {
    pub(crate) action: String,
    pub(crate) extglob: bool,
    pub(crate) alias_expansion_enabled: bool,
    pub(crate) alias_generation: u64,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    ast: Ast,
    /// The ambient `PARSE_EXTENDED_GLOB` gate left behind by the miss
    /// path's tokenize+parse. The lexer's line loop flips the global when
    /// the runtime text itself contains a top-level `shopt -s/-u extglob`
    /// (its parse-execute simulation), so a hit that skips the re-lex must
    /// replay the net effect to stay identical to the uncached path.
    post_extglob_gate: bool,
    last_used: u64,
}

/// Bounded LRU cache of parsed runtime ASTs keyed by text and parser/alias state.
#[derive(Debug, Default)]
pub(crate) struct RuntimeParseCache {
    entries: HashMap<RuntimeParseCacheKey, CacheEntry>,
    tick: u64,
    hits: u64,
    misses: u64,
}

impl RuntimeParseCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A hit returns a fresh clone of the stored AST plus the extglob gate
    /// the miss path left behind (callers re-apply it via
    /// `set_parse_extended_glob` so hit and miss leave the same ambient).
    pub(crate) fn get(&mut self, key: &RuntimeParseCacheKey) -> Option<(Ast, bool)> {
        if let Some(entry) = self.entries.get_mut(key) {
            self.tick = self.tick.wrapping_add(1);
            entry.last_used = self.tick;
            self.hits = self.hits.wrapping_add(1);
            return Some((entry.ast.clone(), entry.post_extglob_gate));
        }
        self.misses = self.misses.wrapping_add(1);
        None
    }

    pub(crate) fn insert(&mut self, key: RuntimeParseCacheKey, ast: Ast, post_extglob_gate: bool) {
        if key.source.len() > RUNTIME_PARSE_CACHE_MAX_SOURCE_BYTES {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        if !self.entries.contains_key(&key) && self.entries.len() >= RUNTIME_PARSE_CACHE_MAX_ENTRIES
        {
            if let Some(evict_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&evict_key);
            }
        }
        self.entries.insert(
            key,
            CacheEntry {
                ast,
                post_extglob_gate,
                last_used: self.tick,
            },
        );
    }

    pub(crate) fn stats(&self) -> (usize, usize, u64, u64) {
        (
            self.entries.len(),
            RUNTIME_PARSE_CACHE_MAX_ENTRIES,
            self.hits,
            self.misses,
        )
    }
}

impl crate::executor::Executor {
    /// The parser-affecting ambient state a runtime-parse key carries:
    /// live posix mode, the lexer extglob gate, `expand_aliases`, and the
    /// alias-table generation (`shell_state.alias_generation`, bumped by
    /// every `alias`/`unalias`/BASH_ALIASES write).
    pub(crate) fn runtime_parse_state(&self) -> (bool, bool, bool, u64) {
        (
            self.posix_mode_enabled(),
            crate::lexer::parse_extended_glob(),
            self.alias_expansion_enabled(),
            self.shell_state.alias_generation,
        )
    }

    /// Consult the bounded runtime parse cache; on a hit the stored
    /// post-parse extglob gate is replayed so a skipped re-lex leaves the
    /// same ambient state the miss path did.
    pub(crate) fn runtime_parse_lookup(
        &self,
        key: &RuntimeParseCacheKey,
    ) -> Option<crate::parser::Ast> {
        self.runtime_parse_cache
            .borrow_mut()
            .get(key)
            .map(|(ast, post_gate)| {
                crate::lexer::set_parse_extended_glob(post_gate);
                ast
            })
    }

    /// Store the AST just parsed under `key`, recording the ambient
    /// extglob gate the tokenize+parse left behind.
    pub(crate) fn runtime_parse_store(&self, key: RuntimeParseCacheKey, ast: &crate::parser::Ast) {
        self.runtime_parse_cache.borrow_mut().insert(
            key,
            ast.clone(),
            crate::lexer::parse_extended_glob(),
        );
    }

    /// rubash#2828 graft: env-gated observability for the runtime parse
    /// cache. `RUBASH_PARSE_CACHE_STATS=1` prints these counters to
    /// stderr at exit (see `report_runtime_parse_cache_stats` in
    /// main.rs) so integration tests can assert the cache actually
    /// fires; without the variable nothing is read and nothing prints.
    /// Returns (entries, cap, hits, misses).
    /// `pub` (not `pub(crate)`) because the only caller is the rubash
    /// binary crate (main.rs), a separate crate from this library;
    /// not consumed by niubash, so no api-ledger entry is owed
    /// (the ledger censuses niubash-consumed paths only).
    pub fn runtime_parse_cache_stats(&self) -> (usize, usize, u64, u64) {
        self.runtime_parse_cache.borrow().stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_key(text: &str, alias_generation: u64) -> RuntimeParseCacheKey {
        RuntimeParseCacheKey::eval(text, 0, false, false, true, alias_generation)
    }

    fn empty_ast() -> Ast {
        Ast {
            commands: Vec::new(),
        }
    }

    #[test]
    fn cache_stays_bounded_under_distinct_text_flood() {
        let mut cache = RuntimeParseCache::new();
        for i in 0..(RUNTIME_PARSE_CACHE_MAX_ENTRIES * 3) {
            cache.insert(eval_key(&format!("echo flood-{i}"), 0), empty_ast(), false);
        }
        assert_eq!(
            cache.entries.len(),
            RUNTIME_PARSE_CACHE_MAX_ENTRIES,
            "a flood of distinct texts must leave the cache at its cap"
        );
        // The oldest entries were evicted; the newest ones remain.
        assert!(cache
            .get(&eval_key(
                &format!("echo flood-{}", RUNTIME_PARSE_CACHE_MAX_ENTRIES * 3 - 1),
                0
            ))
            .is_some());
        assert!(cache.get(&eval_key("echo flood-0", 0)).is_none());
    }

    #[test]
    fn alias_generation_change_invalidates_same_text() {
        let mut cache = RuntimeParseCache::new();
        cache.insert(eval_key("myalias x", 7), empty_ast(), false);
        assert!(cache.get(&eval_key("myalias x", 7)).is_some());
        // Same text, later alias-table generation: never a stale hit.
        assert!(cache.get(&eval_key("myalias x", 8)).is_none());
    }

    #[test]
    fn lru_hit_protects_from_eviction() {
        let mut cache = RuntimeParseCache::new();
        cache.insert(eval_key("echo hot", 0), empty_ast(), false);
        // Fill to exactly the cap without evicting.
        for i in 0..(RUNTIME_PARSE_CACHE_MAX_ENTRIES - 1) {
            cache.insert(eval_key(&format!("echo filler-{i}"), 0), empty_ast(), false);
        }
        // `hot` is the oldest entry; a hit refreshes it, so the next
        // insert must evict filler-0 instead.
        assert!(cache.get(&eval_key("echo hot", 0)).is_some());
        cache.insert(eval_key("echo tail", 0), empty_ast(), false);
        assert_eq!(cache.entries.len(), RUNTIME_PARSE_CACHE_MAX_ENTRIES);
        assert!(cache.get(&eval_key("echo hot", 0)).is_some());
        assert!(cache.get(&eval_key("echo filler-0", 0)).is_none());
    }
}
