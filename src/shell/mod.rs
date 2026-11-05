//! Shell semantic state owners.

pub mod arrays;
pub(crate) mod bind_registry;
pub mod state;
pub mod var_table;
pub mod variables;

pub(crate) use state::BuiltinDispatchClass;
pub use state::ShellState;
pub use variables::{ShellValue, Variable, VariableStore};
