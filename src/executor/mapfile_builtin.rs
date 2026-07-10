use super::*;

impl Executor {
    pub(in crate::executor) fn execute_mapfile(&mut self, cmd: &CommandNode) -> i32 {
        // TODO(builtins/mapfile.def/subst.c/redir.c): Implement the full option
        // set, callbacks, origin/count handling, and newline-preserving storage.
        let command_name = cmd.words.first().map(String::as_str).unwrap_or("mapfile");
        let mut trim_newline = false;
        let mut count = None;
        let mut delimiter = None;
        let mut origin = None;
        let mut skip = 0;
        let mut callback = None;
        let mut callback_quantum = 5000usize;
        let mut array_name = None;
        let mut index = 1;
        let mut stderr = Vec::new();
        while index < cmd.words.len() {
            match cmd.words[index].as_str() {
                "-t" => {
                    trim_newline = true;
                    index += 1;
                }
                "-d" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "d",
                            &mut stderr,
                        );
                    };
                    delimiter = Some(word.chars().next().unwrap_or('\0'));
                    index += 2;
                }
                "-n" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "n",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_usize(
                        command_name,
                        word,
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => count = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 2;
                }
                "-O" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "O",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_usize(
                        command_name,
                        word,
                        "invalid array origin",
                        &mut stderr,
                    ) {
                        Ok(value) => origin = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 2;
                }
                "-s" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "s",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_usize(
                        command_name,
                        word,
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => skip = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 2;
                }
                "-C" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "C",
                            &mut stderr,
                        );
                    };
                    callback = Some(word.clone());
                    index += 2;
                }
                "-c" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "c",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_callback_quantum(command_name, word, &mut stderr) {
                        Ok(value) => callback_quantum = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 2;
                }
                word if word.starts_with("-d") && word.len() > 2 => {
                    delimiter = Some(word[2..].chars().next().unwrap_or('\0'));
                    index += 1;
                }
                word if word.starts_with("-n") && word.len() > 2 => {
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[2..],
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => count = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                word if word.starts_with("-O") && word.len() > 2 => {
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[2..],
                        "invalid array origin",
                        &mut stderr,
                    ) {
                        Ok(value) => origin = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                word if word.starts_with("-s") && word.len() > 2 => {
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[2..],
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => skip = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                word if word.starts_with("-C") && word.len() > 2 => {
                    callback = Some(word[2..].to_string());
                    index += 1;
                }
                word if word.starts_with("-c") && word.len() > 2 => {
                    match self.parse_mapfile_callback_quantum(command_name, &word[2..], &mut stderr)
                    {
                        Ok(value) => callback_quantum = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                word if word.starts_with('-') => {
                    let option = word.trim_start_matches('-').chars().next().unwrap_or('-');
                    return self.mapfile_invalid_option(cmd, command_name, option, &mut stderr);
                }
                word if is_shell_name(word) => {
                    array_name = Some(word.to_string());
                    index += 1;
                }
                _ => {
                    index += 1;
                }
            }
        }

        let name = array_name.unwrap_or_else(|| "MAPFILE".to_string());
        if let Some(input) = self.stdin_string_for_command(cmd) {
            let mut values = split_mapfile_input(&input, delimiter, trim_newline)
                .into_iter()
                .skip(skip)
                .collect::<Vec<_>>();
            if let Some(count) = count.filter(|count| *count > 0) {
                values.truncate(count);
            }
            let start = origin.unwrap_or(0);
            let mut entries = if origin.is_some() {
                self.env_vars
                    .get(&name)
                    .map(|current| indexed_array_entries(current))
                    .unwrap_or_default()
            } else {
                BTreeMap::new()
            };
            for (offset, value) in values.into_iter().enumerate() {
                let target_index = start + offset;
                if let Some(callback) = callback.as_deref() {
                    if (offset + 1) % callback_quantum == 0 {
                        if self
                            .execute_mapfile_callback(callback, target_index, &value)
                            .is_err()
                        {
                            return 1;
                        }
                    }
                }
                entries.insert(target_index, value);
            }
            self.env_vars
                .insert(name.clone(), format_indexed_array_storage(entries));
            mark_env_name(&mut self.env_vars, "__RUBASH_ARRAY_VARS", &name);
            return 0;
        }

        self.env_vars
            .insert(name.clone(), format_indexed_array_storage(BTreeMap::new()));
        mark_env_name(&mut self.env_vars, "__RUBASH_ARRAY_VARS", &name);
        0
    }
}
