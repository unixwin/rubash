//! TokStr — small-string-optimized owned text for lexer tokens.
//!
//! The parse-throughput attribution (issue #241 lane, 2026-10-10, Windows
//! host) measured the process allocator as ~75-80% of `nvm.sh -n` wall:
//! 459k allocations for a 173 KB script, dominated by the two per-token
//! `String`s (value + raw) plus the per-token reader-state clone. A
//! never-free bump-allocator A/B bounded the allocator+free share at
//! ~120 ms of a ~150 ms wall. Most shell tokens (operators, keywords,
//! short words, `;;`, `$1`, `v0=0`) are <= 22 UTF-8 bytes, so an inline
//! representation erases their heap traffic without changing any value
//! bytes: `TokStr` derefs to `str`, compares/hashes as `str`, and
//! converts to `String` at the AST boundary exactly as before.
//!
//! Representation: `Inline` carries up to 22 bytes in place (32-byte
//! total footprint, no allocation); `Heap` boxes longer texts as a
//! `Vec<u8>` known to hold UTF-8 (construction-time invariant, trusted
//! on the way out — same discipline the `raw` bytes already enjoy).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

const INLINE_CAP: usize = 22;
const HEAP_TAG: u8 = 0xFF;

#[derive(Clone)]
pub struct TokStr {
    repr: Repr,
}

#[derive(Clone)]
enum Repr {
    /// `len` <= INLINE_CAP bytes, stored in place.
    Inline { len: u8, bytes: [u8; INLINE_CAP] },
    /// UTF-8 bytes owned on the heap (len > INLINE_CAP).
    Heap(Vec<u8>),
}

impl TokStr {
    #[inline]
    pub fn empty() -> Self {
        Self {
            repr: Repr::Inline {
                len: 0,
                bytes: [0; INLINE_CAP],
            },
        }
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        match &self.repr {
            Repr::Inline { len, .. } => *len == 0,
            Repr::Heap(bytes) => bytes.is_empty(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match &self.repr {
            Repr::Inline { len, .. } => *len as usize,
            Repr::Heap(bytes) => bytes.len(),
        }
    }
}

impl Default for TokStr {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl From<&str> for TokStr {
    #[inline]
    fn from(value: &str) -> Self {
        let bytes = value.as_bytes();
        if bytes.len() <= INLINE_CAP {
            let mut inline = [0u8; INLINE_CAP];
            inline[..bytes.len()].copy_from_slice(bytes);
            Self {
                repr: Repr::Inline {
                    len: bytes.len() as u8,
                    bytes: inline,
                },
            }
        } else {
            Self {
                repr: Repr::Heap(bytes.to_vec()),
            }
        }
    }
}

impl From<String> for TokStr {
    #[inline]
    fn from(value: String) -> Self {
        // Preserve the incoming heap allocation for long texts; short
        // texts are copied inline and the incoming buffer freed.
        if value.len() <= INLINE_CAP {
            Self::from(value.as_str())
        } else {
            Self {
                repr: Repr::Heap(value.into_bytes()),
            }
        }
    }
}

impl From<&String> for TokStr {
    #[inline]
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<TokStr> for String {
    #[inline]
    fn from(value: TokStr) -> Self {
        match value.repr {
            Repr::Inline { len, bytes } => {
                // Constructed only from &str sources, so the bytes are
                // valid UTF-8 by construction.
                unsafe { String::from_utf8_unchecked(bytes[..len as usize].to_vec()) }
            }
            Repr::Heap(bytes) => unsafe { String::from_utf8_unchecked(bytes) },
        }
    }
}

impl Deref for TokStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        match &self.repr {
            Repr::Inline { len, bytes } => unsafe {
                std::str::from_utf8_unchecked(&bytes[..*len as usize])
            },
            Repr::Heap(bytes) => unsafe { std::str::from_utf8_unchecked(bytes) },
        }
    }
}

impl PartialEq for TokStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.deref() == other.deref()
    }
}

impl Eq for TokStr {}

impl PartialEq<str> for TokStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.deref() == other
    }
}

impl PartialEq<&str> for TokStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.deref() == *other
    }
}

impl PartialEq<String> for TokStr {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.deref() == other.as_str()
    }
}

impl PartialOrd for TokStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TokStr {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.deref().cmp(other.deref())
    }
}

impl Hash for TokStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.deref().hash(state)
    }
}

impl fmt::Display for TokStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}

impl fmt::Debug for TokStr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.deref(), f)
    }
}

impl AsRef<str> for TokStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::borrow::Borrow<str> for TokStr {
    #[inline]
    fn borrow(&self) -> &str {
        self
    }
}
