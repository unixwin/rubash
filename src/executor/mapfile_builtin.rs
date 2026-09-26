use super::*;

impl Executor {
    pub(in crate::executor) fn execute_mapfile(&mut self, cmd: &CommandNode) -> i32 {
        // TODO(builtins/mapfile.def/subst.c/redir.c): Implement the full option
        // set, callbacks, origin/count handling, and newline-preserving storage.
        let command_name = cmd.words.first().map(String::as_str).unwrap_or("mapfile");
        let mut trim_newline = false;
        let mut count = None;
        let mut delimiter: Option<u8> = None;
        let mut origin = None;
        let mut skip = 0;
        let mut callback = None;
        let mut callback_quantum = 5000usize;
        let mut read_fd = None;
        let mut array_name = None;
        let mut index = 1;
        let mut stderr = Vec::new();
        let mut end_options = false;
        while index < cmd.words.len() {
            if end_options {
                if array_name.is_none() {
                    if is_shell_name(&cmd.words[index]) {
                        array_name = Some(cmd.words[index].clone());
                    } else {
                        return self.mapfile_invalid_identifier(
                            cmd,
                            command_name,
                            &cmd.words[index],
                            &mut stderr,
                        );
                    }
                }
                index += 1;
                continue;
            }

            match cmd.words[index].as_str() {
                "--" => {
                    end_options = true;
                    index += 1;
                }
                "-t" => {
                    trim_newline = true;
                    index += 1;
                }
                "-tn" => {
                    trim_newline = true;
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
                word if word.starts_with("-tn") && word.len() > 3 => {
                    trim_newline = true;
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[3..],
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => count = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                "-td" => {
                    trim_newline = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "d",
                            &mut stderr,
                        );
                    };
                    delimiter = Some(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(word)
                            .first()
                            .copied()
                            .unwrap_or(0),
                    );
                    index += 2;
                }
                word if word.starts_with("-td") && word.len() > 3 => {
                    trim_newline = true;
                    delimiter = Some(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&word[3..])
                            .first()
                            .copied()
                            .unwrap_or(0),
                    );
                    index += 1;
                }
                "-tO" => {
                    trim_newline = true;
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
                word if word.starts_with("-tO") && word.len() > 3 => {
                    trim_newline = true;
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[3..],
                        "invalid array origin",
                        &mut stderr,
                    ) {
                        Ok(value) => origin = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                "-ts" => {
                    trim_newline = true;
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
                word if word.starts_with("-ts") && word.len() > 3 => {
                    trim_newline = true;
                    match self.parse_mapfile_usize(
                        command_name,
                        &word[3..],
                        "invalid line count",
                        &mut stderr,
                    ) {
                        Ok(value) => skip = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                "-tC" => {
                    trim_newline = true;
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
                word if word.starts_with("-tC") && word.len() > 3 => {
                    trim_newline = true;
                    callback = Some(word[3..].to_string());
                    index += 1;
                }
                "-tu" => {
                    trim_newline = true;
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "u",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_fd(command_name, word, &mut stderr) {
                        Ok(value) => read_fd = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 2;
                }
                word if word.starts_with("-tu") && word.len() > 3 => {
                    trim_newline = true;
                    match self.parse_mapfile_fd(command_name, &word[3..], &mut stderr) {
                        Ok(value) => read_fd = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
                    index += 1;
                }
                "-tc" => {
                    trim_newline = true;
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
                word if word.starts_with("-tc") && word.len() > 3 => {
                    trim_newline = true;
                    match self.parse_mapfile_callback_quantum(command_name, &word[3..], &mut stderr)
                    {
                        Ok(value) => callback_quantum = value,
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
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
                    delimiter = Some(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(word)
                            .first()
                            .copied()
                            .unwrap_or(0),
                    );
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
                "-u" => {
                    let Some(word) = cmd.words.get(index + 1) else {
                        return self.mapfile_missing_option_argument(
                            cmd,
                            command_name,
                            "u",
                            &mut stderr,
                        );
                    };
                    match self.parse_mapfile_fd(command_name, word, &mut stderr) {
                        Ok(value) => read_fd = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
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
                    delimiter = Some(
                        crate::executor::substitution_metadata::shell_text_to_raw_bytes(&word[2..])
                            .first()
                            .copied()
                            .unwrap_or(0),
                    );
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
                word if word.starts_with("-u") && word.len() > 2 => {
                    match self.parse_mapfile_fd(command_name, &word[2..], &mut stderr) {
                        Ok(value) => read_fd = Some(value),
                        Err(status) => return self.finish_mapfile_error(cmd, &stderr, status),
                    }
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
                word if word.is_empty() => {
                    if array_name.is_none() {
                        // GNU builtins/mapfile.def:330: empty array name
                        // reports "empty array variable name" (EX_USAGE),
                        // distinct from sh_invalidid for non-identifier names.
                        return self.mapfile_empty_array_name(cmd, command_name, &mut stderr);
                    }
                    index += 1;
                }
                word if is_shell_name(word) => {
                    if array_name.is_none() {
                        array_name = Some(word.to_string());
                    }
                    index += 1;
                }
                word => {
                    if array_name.is_none() {
                        return self.mapfile_invalid_identifier(
                            cmd,
                            command_name,
                            word,
                            &mut stderr,
                        );
                    }
                    index += 1;
                }
            }
        }

        let name = array_name.unwrap_or_else(|| "MAPFILE".to_string());
        // GNU mapfile.def -> builtin_find_indexed_array ->
        // find_or_make_array_variable (arrayfunc.c:454-497): the array name
        // is resolved through namerefs. An existing resolved variable is
        // converted in place (`name: readonly variable` on refusal); an
        // unset-target nameref creates the array on the CELL name after
        // valid_nameref_value(cell, 2) — which rejects array references
        // like `XXX[0]' as not-a-valid-identifier; an invisible
        // (empty-cell) nameref drops the attribute with a "removing
        // nameref attribute" warning and binds on the name itself.
        let name = match self.nameref_resolution(&name) {
            NamerefResolution::Target(target) => {
                let base = target.split('[').next().unwrap_or(target.as_str());
                let target_exists = if target.contains('[') {
                    self.array_element_parameter_value(&target).is_some()
                } else {
                    self.shell_state.env_vars.contains_key(base)
                        || self.shell_state.variables.get(base).is_some()
                };
                if target_exists {
                    if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, base) {
                        let _ = writeln!(
                            stderr,
                            "{}{name}: readonly variable",
                            self.diagnostic_prefix()
                        );
                        return self.finish_mapfile_error(cmd, &stderr, 1);
                    }
                    base.to_string()
                } else if !is_shell_name(&target) {
                    let _ = writeln!(
                        stderr,
                        "{}{command_name}: `{target}': not a valid identifier",
                        self.diagnostic_prefix()
                    );
                    return self.finish_mapfile_error(cmd, &stderr, 1);
                } else {
                    target
                }
            }
            NamerefResolution::Unresolved => {
                let _ = writeln!(
                    stderr,
                    "{}warning: {name}: removing nameref attribute",
                    self.diagnostic_prefix()
                );
                unmark_env_name(&mut self.shell_state.env_vars, NAMEREF_VARS, &name);
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name) {
                    let _ = writeln!(
                        stderr,
                        "{}{name}: readonly variable",
                        self.diagnostic_prefix()
                    );
                    return self.finish_mapfile_error(cmd, &stderr, 1);
                }
                name
            }
            _ => {
                if is_marked_var(&self.shell_state.env_vars, READONLY_VARS, &name) {
                    let _ = writeln!(
                        stderr,
                        "{}{name}: readonly variable",
                        self.diagnostic_prefix()
                    );
                    return self.finish_mapfile_error(cmd, &stderr, 1);
                }
                name
            }
        };
        if let Some(fd) = read_fd {
            if !self.mapfile_fd_is_available(cmd, fd) {
                return self.mapfile_bad_file_descriptor(cmd, command_name, fd, &mut stderr);
            }
        }

        if let Some(input) = self.mapfile_input_for_command(cmd, read_fd) {
            let mut values = split_mapfile_input(&input, delimiter, trim_newline)
                .into_iter()
                .skip(skip)
                .collect::<Vec<_>>();
            if let Some(count) = count.filter(|count| *count > 0) {
                values.truncate(count);
            }
            let start = origin.unwrap_or(0);
            let mut entries = if origin.is_some() {
                self.shell_state
                    .env_vars
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
            self.shell_state
                .env_vars
                .insert(name.clone(), format_indexed_array_storage(entries));
            mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
            // Diagnostics already buffered (e.g. the nameref-attribute
            // warning) must still reach stderr on success.
            let _ = self.write_buffered_builtin_output(cmd, &[], &stderr);
            return 0;
        }

        self.shell_state
            .env_vars
            .insert(name.clone(), format_indexed_array_storage(BTreeMap::new()));
        mark_env_name(&mut self.shell_state.env_vars, "__RUBASH_ARRAY_VARS", &name);
        let _ = self.write_buffered_builtin_output(cmd, &[], &stderr);
        0
    }
}
