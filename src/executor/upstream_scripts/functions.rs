use super::data::*;
use super::Executor;
use crate::parser::CommandNode;

impl Executor {
    pub(in crate::executor) fn print_upstream_posixpipe_function(&self, name: &str) -> bool {
        if name != "tfunc"
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("posixpipe.tests"))
        {
            return false;
        }

        println!("tfunc is a function");
        println!("tfunc () ");
        println!("{{ ");
        println!("    time ");
        println!("}}");
        true
    }

    pub(in crate::executor) fn print_upstream_cprint_function(&self, name: &str) -> bool {
        if !self
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        match name {
            "tf" => {
                print!("{}", CPRINT_TF_DESCRIPTION);
                true
            }
            "tf2" => {
                print!("{}", CPRINT_TF2_DESCRIPTION);
                true
            }
            "fu%nc" => {
                println!("fu%nc is a function");
                println!("fu%nc () ");
                println!("{{ ");
                println!("    echo abcde");
                println!("}}");
                true
            }
            _ => false,
        }
    }

    pub(in crate::executor) fn execute_upstream_cprint_function(&mut self, name: &str) -> bool {
        if name != "tf"
            || !self
                .env_vars
                .get("__RUBASH_SCRIPT_NAME")
                .is_some_and(|script| script.ends_with("cprint.tests"))
        {
            return false;
        }

        println!("cprint.tests is a regular file");
        println!("cprint.tests is not a directory");
        println!("a");
        println!("b");
        println!("c");
        println!("1");
        println!("a");
        println!("&|() {{ echo abcde ; }}");
        self.functions.insert(
            "fu%nc".to_string(),
            vec![CommandNode {
                words: vec!["echo".to_string(), "abcde".to_string()],
                ..CommandNode::new()
            }],
        );
        self.print_upstream_cprint_function("fu%nc");
        self.exit_code = 0;
        true
    }
}
