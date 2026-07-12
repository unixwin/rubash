//! Parser Module - Bash Parser
//!
//! Transforms tokens into an AST.

mod arithmetic_command;
mod arithmetic_expansion;
mod arithmetic_for;
mod assignment;
mod brace_command;
mod brace_expansion;
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
mod parameter_expansion;
mod parse_loop;
mod pathname_pattern;
mod process_substitution;
mod redirect_assign;
mod redirections;
mod select_command;
mod subshell_command;
mod support;
mod tilde_expansion;
mod token_actions;
mod word_quote;

#[cfg(test)]
mod tests;

pub use nodes::*;
pub use parse_loop::parse;

use arithmetic_command::*;
use arithmetic_expansion::*;
use arithmetic_for::*;
use assignment::*;
use brace_command::*;
use brace_expansion::*;
use case_command::*;
use command_substitution::*;
use conditional_command::*;
use coproc_command::*;
use extglob_pattern::*;
use for_command::*;
use function_command::*;
use if_command::*;
use loop_command::*;
use parameter_expansion::*;
use pathname_pattern::*;
use process_substitution::*;
use redirect_assign::*;
use redirections::*;
use select_command::*;
use subshell_command::*;
use support::*;
use tilde_expansion::*;
use token_actions::*;
use word_quote::*;
