use super::*;

impl Executor {
    pub(in crate::executor) fn expand_braced_indexed_parameter(
        &self,
        name: &str,
    ) -> Option<String> {
        if name == "DIRSTACK[@]" || name == "DIRSTACK[*]" {
            return Some(crate::builtins::pushd::stack_words(&self.env_vars));
        }
        if let Some(index) = name
            .strip_prefix("DIRSTACK[")
            .and_then(|rest| rest.strip_suffix(']'))
            .and_then(|index| self.dirstack_subscript(index))
        {
            return Some(
                crate::builtins::pushd::stack_value(&self.env_vars, index).unwrap_or_default(),
            );
        }
        if let Some(array_name) = name.strip_prefix('#').and_then(|name| {
            name.strip_suffix("[@]")
                .or_else(|| name.strip_suffix("[*]"))
        }) {
            if array_name == "GROUPS" {
                return Some(self.groups_words().len().to_string());
            }
            return Some(
                self.parameter_array_storage(array_name)
                    .map(|value| {
                        if is_marked_array_var(&self.env_vars, array_name)
                            || is_array_storage(&value)
                        {
                            self.array_length(array_name)
                        } else {
                            1
                        }
                    })
                    .unwrap_or(0)
                    .to_string(),
            );
        }
        // `${#}` is the braced spelling of `$#`; `${##pat}` and `${#%pat}`
        // are prefix/suffix removal on `$#`, not length expansions. The
        // bare `${##}` however is the LENGTH of `$#` (exp.tests:376 prints
        // 1 with $#=5), like `${#!}` is the length of `$!`.
        if name == "#" {
            return Some(self.expand_parameter_named_value("#"));
        }
        if (name.starts_with("##") && name.len() > 2) || name.starts_with("#%") {
            return None;
        }
        if let Some(var_name) = name.strip_prefix('#') {
            return Some(self.expand_braced_length_parameter(var_name));
        }
        if let Some((var_name, offset, length)) = self.parse_parameter_substring(name) {
            return Some(self.expand_braced_substring_parameter(var_name, offset, length));
        }
        // GNU valid_array_reference only treats NAME[...] as an array
        // subscript when NAME is a valid identifier. A pattern-replacement
        // word like `z//[^;]` extracts array name "z//" here; without the
        // identifier check the `;` key failed arithmetic evaluation and
        // printed a spurious "z//: bad array subscript" (new-exp8.sub).
        if let Some((array_name, _)) = parse_array_subscript(name) {
            if is_shell_name(array_name) {
                return self.array_element_parameter_value(name);
            }
        }
        None
    }

    fn expand_braced_length_parameter(&self, var_name: &str) -> String {
        if matches!(var_name, "@" | "*") {
            return self.positional_params.len().to_string();
        }
        if is_special_parameter_name(var_name) || var_name.parse::<usize>().is_ok() {
            return self
                .expand_parameter_named_value(var_name)
                .chars()
                .count()
                .to_string();
        }
        if let Some((array_name, index)) = parse_array_integer_subscript(var_name) {
            return self
                .env_vars
                .get(array_name)
                .and_then(|value| {
                    resolve_indexed_array_subscript(value, index)
                        .and_then(|index| array_value_at(value, index))
                })
                .map(|value| parameter_char_length(&value).to_string())
                .unwrap_or_else(|| "0".to_string());
        }
        if let Some((array_name, index)) = parse_array_numeric_subscript(var_name) {
            return self
                .env_vars
                .get(array_name)
                .and_then(|value| array_value_at(value, index))
                .map(|value| parameter_char_length(&value).to_string())
                .unwrap_or_else(|| "0".to_string());
        }
        if let Some((array_name, key)) = parse_array_subscript(var_name) {
            if self.is_assoc_parameter_array(array_name) {
                let key = self.assoc_subscript_key(key);
                return self
                    .parameter_array_storage(array_name)
                    .and_then(|value| assoc_value_at(&value, &key))
                    .map(|value| parameter_char_length(&value).to_string())
                    .unwrap_or_else(|| "0".to_string());
            }
        }
        if let Some(value) = self.dynamic_parameter_value(var_name) {
            return parameter_char_length(&value).to_string();
        }
        // GNU subst.c ${#name} over a nameref: the length of the value the
        // reference resolves to. A cell naming a variable resolves through
        // it (an array contributes element 0); a cell naming an array
        // element contributes that element; an invalid cell falls back to
        // the length of the cell string itself
        // (nameref24.sub: name4 -> 'aa&bb' prints 5, name2 -> unset prints 0).
        if is_marked_var(&self.env_vars, NAMEREF_VARS, var_name) {
            let cell = self.env_vars.get(var_name).cloned().unwrap_or_default();
            if is_shell_name(&cell) {
                if let Some(target_value) = self.env_vars.get(&cell) {
                    if is_array_storage(target_value) {
                        let element_zero = if self.is_assoc_parameter_array(&cell) {
                            assoc_value_at(target_value, "0")
                        } else {
                            array_value_at(target_value, 0)
                        };
                        return element_zero
                            .map(|value| parameter_char_length(&value).to_string())
                            .unwrap_or_else(|| "0".to_string());
                    }
                    return parameter_char_length(&target_value).to_string();
                }
                if let Some(crate::shell::Variable {
                    value: crate::shell::ShellValue::Scalar(scalar),
                    ..
                }) = self.shell_state.variables.get(&cell)
                {
                    return parameter_char_length(&scalar).to_string();
                }
                return "0".to_string();
            }
            if let Some((array_name, key)) = parse_array_subscript(&cell) {
                if self.is_assoc_parameter_array(array_name) {
                    let key = self.assoc_subscript_key(key);
                    return self
                        .parameter_array_storage(array_name)
                        .and_then(|value| assoc_value_at(&value, &key))
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                if let Some(index) = key.parse::<usize>().ok() {
                    return self
                        .env_vars
                        .get(array_name)
                        .and_then(|value| array_value_at(value, index))
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string());
                }
                return "0".to_string();
            }
            return parameter_char_length(&cell).to_string();
        }
        self.env_vars
            .get(var_name)
            .map(|value| {
                if is_array_storage(value) {
                    let element_zero = if self.is_assoc_parameter_array(var_name) {
                        assoc_value_at(value, "0")
                    } else {
                        array_value_at(value, 0)
                    };
                    element_zero
                        .map(|value| parameter_char_length(&value).to_string())
                        .unwrap_or_else(|| "0".to_string())
                } else {
                    parameter_char_length(&value).to_string()
                }
            })
            .unwrap_or_else(|| "0".to_string())
    }

    pub(in crate::executor) fn expand_braced_substring_parameter(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> String {
        if let Some(value) = self.indirect_substring_parameter(var_name, offset, length) {
            return value;
        }
        if matches!(var_name, "@" | "*") {
            if offset == 0 {
                let mut params = Vec::with_capacity(self.positional_params.len() + 1);
                params.push(self.script_name_value());
                params.extend(self.positional_params.iter().cloned());
                return positional_parameter_substring(&params, 1, length).join(" ");
            }
            return positional_parameter_substring(&self.positional_params, offset, length)
                .join(" ");
        }
        if let Some(array_name) = var_name
            .strip_suffix("[@]")
            .or_else(|| var_name.strip_suffix("[*]"))
        {
            return self
                .parameter_array_storage(array_name)
                .map(|value| {
                    let values = array_parameter_slice(
                        &value,
                        offset,
                        length.and_then(|length| usize::try_from(length).ok()),
                    )
                    .into_iter()
                    .map(normalize_array_expanded_value)
                    .collect::<Vec<_>>();
                    self.join_expanded_array_values(values, var_name)
                })
                .unwrap_or_default();
        }
        if let Some(value) = self.array_element_parameter_value(var_name) {
            return parameter_substring(&value, offset, length);
        }
        if is_shell_name(var_name) {
            // GNU variables.c get_string_value returns element [0] when a
            // bare array name is expanded without a subscript, so the slice
            // must operate on element 0 rather than the raw typed array
            // storage (probe: av=(abcd efgh); ${av:1:2} -> "bc").
            return self
                .parameter_pattern_scalar_value(var_name)
                .map(|value| parameter_substring(&value, offset, length))
                .unwrap_or_default();
        }
        String::new()
    }

    fn indirect_substring_parameter(
        &self,
        var_name: &str,
        offset: isize,
        length: Option<isize>,
    ) -> Option<String> {
        let indirect_name = var_name.strip_prefix('!')?;
        if let Some(target_name) = self.nameref_target_name(indirect_name) {
            return Some(parameter_substring(&target_name, offset, length));
        }

        let target_expr = self.env_vars.get(indirect_name)?;
        if target_expr.ends_with("[@]") || target_expr.ends_with("[*]") {
            let values = slice_array_values(
                self.indirect_target_values(target_expr),
                offset,
                length.and_then(|length| usize::try_from(length).ok()),
            );
            return Some(self.join_expanded_array_values(values, target_expr));
        }

        let value = self
            .indirect_target_values(target_expr)
            .into_iter()
            .next()?;
        Some(parameter_substring(&value, offset, length))
    }
}

/// GNU subst.c ${#name} length: MB_STRLEN over the value (subst.c:8308) with
/// the UTF-8 locale byte walk of lib/sh/utf8.c:167-184 utf8_mbstrlen -- one
/// character per valid multibyte sequence, one per byte of an invalid
/// sequence (MB_INVALIDCH -> clen = 1). Raw bytes >= 0x80 travel inside
/// rubash words as U+E000 marker pairs (substitution_metadata), so the value
/// must be decoded to its byte view before counting
/// (intl.tests: a=$'\303\251' is 1, not the 4 chars of two marker pairs).
fn parameter_char_length(value: &str) -> usize {
    let sentinel =
        char::from_u32(crate::executor::substitution_metadata::RAW_BYTE_MARKER_ESCAPE)
            .expect("raw-byte sentinel is a valid char");
    if !value.contains(sentinel) {
        return value.chars().count();
    }
    let bytes =
        crate::executor::substitution_metadata::decode_raw_byte_markers(value.as_bytes());
    let mut count = 0usize;
    let mut rest: &[u8] = &bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(text) => {
                count += text.chars().count();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if let Ok(text) = std::str::from_utf8(&rest[..valid]) {
                    count += text.chars().count();
                }
                count += 1; // utf8.c:177-178: an invalid byte counts as one
                rest = &rest[valid + 1..];
            }
        }
    }
    count
}

