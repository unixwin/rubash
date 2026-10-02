//! Parser Module - Bash Parser
//!
//! Transforms tokens into an AST.

mod arithmetic_command;
mod arithmetic_for;
mod array_element_assignment;
pub mod assignment;
pub mod ast_print;
mod brace_command;
mod case_command;
mod command_substitution;
mod conditional_command;
mod coproc_command;
mod extglob_pattern;
mod for_command;
mod function_command;
mod if_command;
mod loop_command;
mod nodes;
mod parse_loop;
pub(crate) mod pathname_pattern;
mod process_substitution;
mod redirect_assign;
mod redirections;
mod select_command;
mod subshell_command;
mod support;
mod token_actions;
mod word_quote;

/// Executor-facing wrapper for the parse-time command-substitution scan:
/// the print_comsub text serializer (rubash#274) re-derives the `$()` spans
/// of a word the same way `record_command_substitutions_for_word` does at
/// parse time, so both see identical span boundaries.
pub fn command_substitutions_in_word_public(word: &str) -> Vec<CommandSubstitutionNode> {
    command_substitution::command_substitutions_in_word(word)
}

#[cfg(test)]
mod tests;

pub(crate) use function_command::find_body_parse_error;
pub use nodes::*;
pub use parse_loop::{parse, parse_with_options, ParseLoopOptions};
pub(crate) use process_substitution::raw_word_has_unquoted_process_substitution;

use arithmetic_command::*;
use arithmetic_for::*;
use array_element_assignment::*;
use assignment::*;
use brace_command::*;
use case_command::*;
use command_substitution::*;
use conditional_command::*;
use coproc_command::*;
use extglob_pattern::*;
use for_command::*;
use function_command::*;
use if_command::*;
use loop_command::*;
use pathname_pattern::*;
use process_substitution::*;
use redirect_assign::*;
use redirections::*;
use select_command::*;
use subshell_command::*;
use support::*;
use token_actions::*;
use word_quote::*;
