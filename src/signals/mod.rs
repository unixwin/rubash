//! Shell signal runtime infrastructure.
//!
//! This module owns the signal tables, cross-process mailbox, kernel
//! dispositions, and delivery machinery factored out of the `kill` builtin.
//! It is crate-private: `builtins::kill` re-exports the existing public and
//! crate-internal faces so established callers keep their paths unchanged.

pub mod delivery;
pub mod dispositions;
pub mod mailbox;
pub mod table;

pub use delivery::*;
pub use dispositions::*;
pub use mailbox::*;
pub use table::*;
