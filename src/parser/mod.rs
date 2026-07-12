//! Parser Module - Bash Parser
//!
//! Transforms tokens into an AST.

mod arithmetic_command;
mod arithmetic_for;
mod assignment;
mod brace_command;
mod case_command;
mod conditional_command;
mod coproc_command;
mod for_command;
mod function_command;
mod if_command;
mod loop_command;
mod nodes;
mod parse_loop;
mod process_substitution;
mod redirect_assign;
mod redirections;
mod select_command;
mod support;
mod token_actions;

#[cfg(test)]
mod tests;

pub use nodes::*;
pub use parse_loop::parse;

use arithmetic_command::*;
use arithmetic_for::*;
use assignment::*;
use brace_command::*;
use case_command::*;
use conditional_command::*;
use coproc_command::*;
use for_command::*;
use function_command::*;
use if_command::*;
use loop_command::*;
use process_substitution::*;
use redirect_assign::*;
use redirections::*;
use select_command::*;
use support::*;
use token_actions::*;
