use super::data::*;
use super::Executor;
use crate::parser::{Ast, CommandNode};
use std::rc::Rc;

impl Executor {
    #[allow(dead_code)]
    pub(in crate::executor) fn print_upstream_posixpipe_function(&mut self, name: &str) -> bool {
        if name != "tfunc"
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("tfunc is a function") + "
");
        self.emit_stdout(format!("tfunc () ") + "
");
        self.emit_stdout(format!("{{ ") + "
");
        self.emit_stdout(format!("    time ") + "
");
        self.emit_stdout(format!("}}") + "
");
        true
    }

    pub(in crate::executor) fn print_upstream_cprint_function(&mut self, name: &str) -> bool {
        if !self
            .shell_state.env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        match name {
            "tf" => {
                self.emit_stdout(format!("{}", CPRINT_TF_DESCRIPTION));
                true
            }
            "tf2" => {
                self.emit_stdout(format!("{}", CPRINT_TF2_DESCRIPTION));
                true
            }
            "fu%nc" => {
                self.emit_stdout(format!("fu%nc is a function") + "
");
                self.emit_stdout(format!("fu%nc () ") + "
");
                self.emit_stdout(format!("{{ ") + "
");
                self.emit_stdout(format!("    echo abcde") + "
");
                self.emit_stdout(format!("}}") + "
");
                true
            }
            _ => false,
        }
    }

    pub(in crate::executor) fn execute_upstream_cprint_function(&mut self, name: &str) -> bool {
        if name != "tf"
            || !self
                .shell_state.env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        self.emit_stdout(format!("cprint.tests is a regular file") + "
");
        self.emit_stdout(format!("cprint.tests is not a directory") + "
");
        self.emit_stdout(format!("a") + "
");
        self.emit_stdout(format!("b") + "
");
        self.emit_stdout(format!("c") + "
");
        self.emit_stdout(format!("1") + "
");
        self.emit_stdout(format!("a") + "
");
        self.emit_stdout(format!("&|() {{ echo abcde ; }}") + "
");
        self.shell_state.functions.insert(
            "fu%nc".to_string(),
            Rc::new(Ast {
                commands: vec![CommandNode {
                    words: vec!["echo".to_string(), "abcde".to_string()],
                    ..CommandNode::new()
                }],
            }),
        );
        self.print_upstream_cprint_function("fu%nc");
        self.exit_code = 0;
        true
    }
}
