use super::*;

impl Executor {
    pub(in crate::executor) fn parse_mapfile_usize(
        &self,
        command_name: &str,
        value: &str,
        diagnostic: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        value.parse::<usize>().map_err(|_| {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: {diagnostic}",
                self.diagnostic_prefix()
            );
            1
        })
    }

    pub(in crate::executor) fn parse_mapfile_callback_quantum(
        &self,
        command_name: &str,
        value: &str,
        stderr: &mut Vec<u8>,
    ) -> Result<usize, i32> {
        let quantum =
            self.parse_mapfile_usize(command_name, value, "invalid callback quantum", stderr)?;
        if quantum == 0 {
            let _ = writeln!(
                stderr,
                "{}{command_name}: {value}: invalid callback quantum",
                self.diagnostic_prefix()
            );
            return Err(1);
        }
        Ok(quantum)
    }

    pub(in crate::executor) fn mapfile_missing_option_argument(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: &str,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: option requires an argument",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn mapfile_invalid_option(
        &mut self,
        cmd: &CommandNode,
        command_name: &str,
        option: char,
        stderr: &mut Vec<u8>,
    ) -> i32 {
        let _ = writeln!(
            stderr,
            "{}{command_name}: -{option}: invalid option",
            self.diagnostic_prefix()
        );
        self.print_mapfile_usage(command_name, stderr);
        self.finish_mapfile_error(cmd, stderr, 2)
    }

    pub(in crate::executor) fn print_mapfile_usage(
        &self,
        command_name: &str,
        stderr: &mut Vec<u8>,
    ) {
        let _ = writeln!(
            stderr,
            "{command_name}: usage: {command_name} [-d delim] [-n count] [-O origin] [-s count] [-t] [-u fd] [-C callback] [-c quantum] [array]"
        );
    }

    pub(in crate::executor) fn finish_mapfile_error(
        &mut self,
        cmd: &CommandNode,
        stderr: &[u8],
        status: i32,
    ) -> i32 {
        if self
            .write_buffered_builtin_output(cmd, &[], stderr)
            .is_err()
        {
            return 1;
        }
        status
    }

    pub(in crate::executor) fn execute_mapfile_callback(
        &mut self,
        callback: &str,
        index: usize,
        value: &str,
    ) -> Result<(), ExecuteError> {
        let mut words = callback
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if words.is_empty() {
            return Ok(());
        }
        words.push(index.to_string());
        words.push(value.to_string());

        let mut callback_cmd = CommandNode::new();
        callback_cmd.words = words;
        self.execute_command(&callback_cmd)
    }
}
