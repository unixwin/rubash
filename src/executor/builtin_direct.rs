use super::*;

impl Executor {
    pub(in crate::executor) fn execute_builtin_direct(
        &mut self,
        args: &[String],
    ) -> Result<(), ExecuteError> {
        // TODO(builtins/builtin.def): Bash `builtin` invokes shell builtins
        // while bypassing functions. This narrow implementation covers the
        // upstream builtins tests and should grow with the builtin table.
        let Some(name) = args.first() else {
            self.exit_code = 0;
            return Ok(());
        };

        if crate::builtins::enable::is_disabled(&self.env_vars, name) {
            eprintln!(
                "{}builtin: {name}: not a shell builtin",
                self.diagnostic_prefix()
            );
            self.exit_code = 1;
            return Ok(());
        }

        match name.as_str() {
            "echo" => {
                crate::builtins::echo::execute(&args[1..])?;
                self.exit_code = 0;
                Ok(())
            }
            "printf" => {
                self.exit_code = crate::builtins::printf::execute(&args[1..], &mut self.env_vars)?;
                Ok(())
            }
            "pwd" => {
                if args.len() == 1 || args.get(1).map(String::as_str) == Some("-L") {
                    if let Some(pwd) = self.env_vars.get("PWD") {
                        if pwd.starts_with('/') {
                            println!("{pwd}");
                            self.exit_code = 0;
                            return Ok(());
                        }
                    }
                }
                self.exit_code = crate::builtins::pwd::execute(&args[1..])?;
                Ok(())
            }
            "cd" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_cd(&command)?;
                Ok(())
            }
            "set" => {
                let command = command_node_from_args(args);
                self.execute_set_command(&command)
            }
            "getopts" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_getopts_command(&command)?;
                Ok(())
            }
            "shopt" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_shopt(&command)?;
                Ok(())
            }
            "enable" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_enable(&command)?;
                Ok(())
            }
            "exec" => {
                let command = command_node_from_args(args);
                self.execute_exec_command(&command)
            }
            "logout" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_logout(&command)?;
                Ok(())
            }
            "source" | "." => crate::builtins::source::execute_named(self, &args[0], &args[1..]),
            "return" => {
                let command = command_node_from_args(args);
                self.execute_return(&command)
            }
            "break" => {
                let command = command_node_from_args(args);
                self.execute_loop_control(&command, LoopControlKind::Break)
            }
            "continue" => {
                let command = command_node_from_args(args);
                self.execute_loop_control(&command, LoopControlKind::Continue)
            }
            "command" => {
                let command = command_node_from_args(args);
                self.execute_command_without_aliases(&command)
            }
            ":" => {
                self.exit_code = crate::builtins::colon::colon();
                Ok(())
            }
            "true" => {
                self.exit_code = crate::builtins::colon::true_builtin();
                Ok(())
            }
            "false" => {
                self.exit_code = crate::builtins::colon::false_builtin();
                Ok(())
            }
            "eval" => match crate::builtins::eval::execute(&args[1..])? {
                crate::builtins::eval::EvalAction::Complete(status) => {
                    self.exit_code = status;
                    Ok(())
                }
                crate::builtins::eval::EvalAction::Execute(source) => {
                    let tokens = crate::lexer::tokenize(&source);
                    let ast = crate::parser::parse(&tokens);
                    self.execute_ast(&ast)
                }
            },
            "hash" => {
                self.exit_code = crate::builtins::hash::execute(&args[1..], &mut self.env_vars)?;
                Ok(())
            }
            "help" => {
                self.exit_code = crate::builtins::help::execute(&args[1..])?;
                Ok(())
            }
            "kill" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_kill(&command)?;
                Ok(())
            }
            "alias" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_alias(&command)?;
                Ok(())
            }
            "unalias" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_unalias(&command)?;
                Ok(())
            }
            "export" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_export(&command)?;
                Ok(())
            }
            "readonly" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_readonly(&command)?;
                Ok(())
            }
            "declare" | "typeset" => {
                let command = command_node_from_args(args);
                self.execute_declare_command(&command)
            }
            "local" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_local(&command)?;
                Ok(())
            }
            "unset" => {
                let command = command_node_from_args(args);
                self.exit_code = self.execute_unset(&command)?;
                Ok(())
            }
            other => self.execute_builtin_direct_late(args, other),
        }
    }
}

pub(in crate::executor) fn command_node_from_args(args: &[String]) -> CommandNode {
    let mut command = CommandNode::new();
    command.words = args.to_vec();
    command
}
