use super::*;

impl Executor {
    pub(in crate::executor) fn read_redirected_fd(
        &mut self,
        cmd: &CommandNode,
        fd: u32,
        delimiter: char,
        char_limit: Option<usize>,
        exact_char_limit: bool,
    ) -> Option<String> {
        let redirect = cmd.redirect_in.as_ref()?;
        if redirect.fd != Some(fd) {
            return None;
        }

        if let Some(source) = redirect
            .target
            .strip_prefix("<(")
            .and_then(|target| target.strip_suffix(')'))
        {
            let output = self.process_substitution_output(source)?;
            return Some(trim_read_input(
                output,
                delimiter,
                char_limit,
                exact_char_limit,
            ));
        }

        let target = self.expand_word(&redirect.target);
        let path = shell_path_to_windows(&target, &self.env_vars);
        if redirect.append {
            let _ = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path);
        }
        let input = fs::read_to_string(path).ok()?;
        Some(trim_read_input(
            input,
            delimiter,
            char_limit,
            exact_char_limit,
        ))
    }
}
