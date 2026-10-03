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

/// Executor-facing wrapper for the command-substitution scan: the
/// pretty-print renderer and the print_comsub text serializer (rubash#274)
/// re-derive a word's `$()` spans on demand (wt44/parse4: the parse-time
/// word-intake scan is gone; GNU parse.y:5305 read_token_word stores the
/// word once and subst.c analyzes at use).
pub fn command_substitutions_in_word_public(word: &str) -> Vec<CommandSubstitutionNode> {
    command_substitution::command_substitutions_in_word(word)
}

/// Executor-facing wrapper for the word-quote scan: the conditional `=~`
/// operand-quoting check (GNU parse.y cond.c / subst.c quoted-regex
/// semantics) re-derives a raw word's quote spans on demand (wt44/parse4).
pub fn word_quotes_in_raw_public(raw: &str) -> Vec<WordQuote> {
    word_quote::word_quotes_in_raw(raw)
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
