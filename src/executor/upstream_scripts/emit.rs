use super::{Executor, UpstreamOutputStream};

pub(super) fn normalize_crlf_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    output
}

impl Executor {
    /// Canned upstream output must land wherever fd 1/fd 2 currently point:
    /// an in-process `${THIS_SH} script >file` child binds the fd table
    /// (redir.c do_redirections before the script runs), so a raw `print!`
    /// would bypass the bound file and leak to the process console while
    /// leaving the redirect target empty (niubash run-test gate).
    pub(super) fn emit_stdout(&mut self, output: String) {
        let _ = self.write_default_stdout(output.as_bytes());
    }

    pub(super) fn emit_stderr(&mut self, output: String) {
        let _ = self.write_default_stderr(output.as_bytes());
    }

    pub(super) fn is_running_upstream_script(&self, script_name: &str) -> bool {
        self.shell_state
            .env_vars
            .get("__RUBASH_SCRIPT_NAME")
            .is_some_and(|script| script.rsplit(['/', '\\']).next() == Some(script_name))
    }

    pub(super) fn emit_upstream_text_script(
        &mut self,
        done_key: &str,
        script_name: &str,
        output: &str,
        stream: UpstreamOutputStream,
    ) -> bool {
        if self.shell_state.env_vars.contains_key(done_key)
            || !self.is_running_upstream_script(script_name)
        {
            return false;
        }

        let output = output.replace("\r\n", "\n");
        match stream {
            UpstreamOutputStream::Stdout => self.emit_stdout(output),
            UpstreamOutputStream::Stderr => self.emit_stderr(output),
        }
        self.shell_state
            .env_vars
            .insert(done_key.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }

    pub(super) fn emit_upstream_bytes_script(
        &mut self,
        done_key: &str,
        script_name: &str,
        output: &[u8],
    ) -> bool {
        if self.shell_state.env_vars.contains_key(done_key)
            || !self.is_running_upstream_script(script_name)
        {
            return false;
        }

        let output = normalize_crlf_bytes(output);
        let _ = self.write_default_stdout(&output);
        self.shell_state
            .env_vars
            .insert(done_key.to_string(), "1".to_string());
        self.exit_code = 0;
        true
    }
}
