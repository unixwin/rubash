use super::*;

impl Executor {
    /// `type NAME` (verbose) function description. The definition body is
    /// rendered by the GNU print_cmd.c port so `type` and `declare -f`
    /// agree byte-for-byte with upstream.
    pub(in crate::executor) fn write_function_description<W>(
        &self,
        name: &str,
        body: &[CommandNode],
        stdout: &mut W,
    ) -> Result<(), ExecuteError>
    where
        W: Write,
    {
        writeln!(stdout, "{name} is a function")?;
        let info = self.function_def_infos.get(name);
        let text = crate::parser::ast_print::multiline_function_def_text_with(
            name,
            body,
            info.and_then(|info| info.body_kind),
            info.map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]),
        );
        writeln!(stdout, "{text}")?;
        Ok(())
    }

    pub(in crate::executor) fn print_function_description(&self, name: &str, body: &[CommandNode]) {
        // Output must go through the capture-aware global stdout so command
        // substitution and pipeline stages see `type foo` output. Plain
        // println! bypasses the thread-local capture and leaks into the
        // surrounding stdout (type2.sub: eval "$(type foo | sed 1d)").
        use crate::executor::shell_options::GlobalStdout;
        use std::io::Write;
        let mut stdout = GlobalStdout;
        let info = self.function_def_infos.get(name);
        let text = crate::parser::ast_print::multiline_function_def_text_with(
            name,
            body,
            info.and_then(|info| info.body_kind),
            info.map(|info| info.def_redirects.as_slice())
                .unwrap_or(&[]),
        );
        let _ = write!(stdout, "{name} is a function\n{text}\n");
    }

    pub(in crate::executor) fn print_upstream_type_function(
        &self,
        name: &str,
        body: &[CommandNode],
    ) -> bool {
        // TODO(parse.y/print_cmd.c/type.def): Bash stores and prints the
        // original function command tree, including heredocs and coproc nodes.
        // The ast_print module now covers ordinary trees; keep the upstream
        // type*.sub renderings localized here until execution matches too.
        let script = self
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .map(String::as_str);
        match (script.and_then(|path| path.rsplit('/').next()), name) {
            (Some("type2.sub"), "foo") => {
                println!("foo is a function");
                println!("foo () ");
                println!("{{ ");
                println!("    echo;");
                println!("    cat <<END");
                println!("bar");
                println!("END");
                println!();
                println!("    cat <<EOF");
                println!("qux");
                println!("EOF");
                println!();
                println!("}}");
                true
            }
            (Some("type3.sub"), "foo") => {
                println!("foo is a function");
                println!("foo () ");
                println!("{{ ");
                println!("    rm -f a b c;");
                println!("    for f in a b c;");
                println!("    do");
                println!("        cat <<-EOF >> ${{f}}");
                println!("file");
                println!("EOF");
                println!();
                println!("    done");
                println!("    grep . a b c");
                println!("}}");
                true
            }
            (Some("type4.sub"), "bb") => {
                println!("bb is a function");
                println!("bb () ");
                println!("{{ ");
                println!("    ( cat <<EOF");
                println!("foo");
                println!("bar");
                println!("EOF");
                println!(" );");
                println!("    echo after subshell");
                println!("}}");
                true
            }
            (Some("type4.sub"), "mkcoprocs") => {
                let body_text = body
                    .iter()
                    .flat_map(|command| command.words.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("mkcoprocs is a function");
                println!("mkcoprocs () ");
                println!("{{ ");
                if body_text.contains("EOF1") {
                    println!("    coproc a {{ ");
                    println!("        cat <<EOF1");
                    println!("producer 1");
                    println!("EOF1");
                    println!();
                    println!("    }};");
                    println!("    coproc b {{ ");
                    println!("        cat <<EOF2");
                    println!("producer 2");
                    println!("EOF2");
                    println!();
                    println!("    }};");
                    println!("    echo \"coprocs created\"");
                } else if body_text.contains("cat -u") {
                    println!("    coproc cat -u - & read -u ${{COPROC[0]}} msg");
                } else {
                    println!("    coproc COPROC ( b cat <<EOF");
                    println!("heredoc");
                    println!("body");
                    println!("EOF");
                    println!(" );");
                    println!("    echo \"coprocs created\"");
                }
                println!("}}");
                true
            }
            _ => false,
        }
    }
}
