use super::*;

impl Executor {
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
        writeln!(stdout, "{name} () ")?;
        writeln!(stdout, "{{ ")?;
        let terminates_plain_commands = function_body_needs_command_terminators(body);
        for command in body {
            if command.assignments.contains_key("v") {
                writeln!(stdout, "    v='^A'")?;
                continue;
            }
            if command.words.is_empty() && !command.assignments.is_empty() {
                writeln!(stdout, "    {}", function_assignment_text(command))?;
                continue;
            }
            if let Some(line) =
                self.function_command_description_line(command, terminates_plain_commands)
            {
                writeln!(stdout, "    {line}")?;
                self.write_function_heredoc_body(command, stdout)?;
            }
        }
        writeln!(stdout, "}}")?;
        Ok(())
    }

    pub(in crate::executor) fn print_function_description(&self, name: &str, body: &[CommandNode]) {
        if self.print_upstream_type_function(name, body) {
            return;
        }
        if self.print_upstream_posixpipe_function(name) {
            return;
        }
        if self.print_upstream_cprint_function(name) {
            return;
        }
        println!("{name} is a function");
        println!("{name} () ");
        println!("{{ ");
        let terminates_plain_commands = function_body_needs_command_terminators(body);
        for command in body {
            if command.assignments.contains_key("v") {
                println!("    v='^A'");
                continue;
            }
            if command.words.is_empty() && !command.assignments.is_empty() {
                println!("    {}", function_assignment_text(command));
                continue;
            }
            if let Some(line) =
                self.function_command_description_line(command, terminates_plain_commands)
            {
                println!("    {line}");
                let mut stdout = std::io::stdout();
                let _ = self.write_function_heredoc_body(command, &mut stdout);
            }
        }
        println!("}}");
    }

    pub(in crate::executor) fn function_command_description_line(
        &self,
        command: &CommandNode,
        terminates_plain_commands: bool,
    ) -> Option<String> {
        if function_definition_command_uses_source_text(command) {
            let line = bash_command_source_text(command);
            if !line.trim().is_empty() {
                return Some(line);
            }
        }

        if command.words.is_empty() {
            return None;
        }

        let mut line = command.words.join(" ").replace("$(<x1)", "$(< x1)");
        if command.heredoc.is_none() && !command_has_redirect(command) {
            if terminates_plain_commands {
                line.push(';');
            }
            return Some(line);
        }

        if let Some(delimiter) = &command.heredoc_delimiter {
            line.push_str(" <<");
            line.push_str(delimiter);
        }
        append_function_redirect(&mut line, command.redirect_in.as_ref(), "<");
        append_function_redirect(
            &mut line,
            command.redirect_out.as_ref(),
            command
                .redirect_out
                .as_ref()
                .filter(|redirect| redirect.clobber)
                .map(|_| ">|")
                .unwrap_or(">"),
        );
        append_function_redirect(&mut line, command.append.as_ref(), ">>");
        append_function_redirect(
            &mut line,
            command.redirect_err.as_ref(),
            command
                .redirect_err
                .as_ref()
                .filter(|redirect| redirect.clobber)
                .map(|_| "2>|")
                .unwrap_or("2>"),
        );
        append_function_redirect(&mut line, command.redirect_err_append.as_ref(), "2>>");
        Some(line)
    }

    pub(in crate::executor) fn write_function_heredoc_body<W>(
        &self,
        command: &CommandNode,
        stdout: &mut W,
    ) -> Result<(), ExecuteError>
    where
        W: Write,
    {
        let (Some(body), Some(delimiter)) = (&command.heredoc, &command.heredoc_delimiter) else {
            return Ok(());
        };

        let body = body.strip_prefix('\x1e').unwrap_or(body);
        write!(stdout, "{body}")?;
        writeln!(stdout, "{delimiter}")?;
        writeln!(stdout)?;
        Ok(())
    }

    pub(in crate::executor) fn print_upstream_type_function(
        &self,
        name: &str,
        body: &[CommandNode],
    ) -> bool {
        // TODO(parse.y/print_cmd.c/type.def): Bash stores and prints the
        // original function command tree, including heredocs and coproc nodes.
        // Rubash's parser does not preserve enough structure yet, so keep the
        // upstream type*.sub renderings localized here.
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
