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
        let info = self.shell_state.function_def_infos.get(name);
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
}
