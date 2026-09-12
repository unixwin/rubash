use super::{ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{bash_arith, checked_arithmetic_pow};
use crate::executor::{
    array_value_at, assoc_entries, assoc_value_at, current_epoch_seconds,
    env_derived_dynamic_parameter_value, format_assoc_storage, format_indexed_array_storage,
    indexed_array_entries, is_marked_var, is_noassign_bash_array, mark_env_name,
    next_random_from_state, next_srandom_from_state, resolve_indexed_array_subscript,
    set_process_env, ARRAY_VARS, ASSOC_VARS, READONLY_VARS, SECONDS_OFFSET, SHELL_START_EPOCH,
};

impl ConditionalArithParser<'_> {
    pub(super) fn lvalue_value(&mut self, lvalue: &ArithLValue) -> Option<i128> {
        match lvalue {
            ArithLValue::Scalar(name) => self.variable_value(name),
            ArithLValue::Indexed { name, index } => {
                let value = self.env_vars.get(name).and_then(|value| {
                    resolve_indexed_array_subscript(value, *index)
                        .and_then(|index| array_value_at(value, index))
                });
                let value = value.unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{index}]"), &value)
            }
            ArithLValue::Assoc { name, key } => {
                let value = self
                    .env_vars
                    .get(name)
                    .and_then(|value| assoc_value_at(value, key))
                    .unwrap_or_default();
                self.evaluate_variable_text(&format!("{name}[{key}]"), &value)
            }
        }
    }

    pub(super) fn variable_value(&mut self, name: &str) -> Option<i128> {
        if self.resolving.iter().any(|resolving| resolving == name) {
            return None;
        }
        if name == "RANDOM" {
            return self
                .random_state
                .map(|state| i128::from(next_random_from_state(state)));
        }
        if name == "SRANDOM" {
            return self
                .random_state
                .map(|state| i128::from(next_srandom_from_state(state)));
        }
        if name == "LINENO" {
            return self
                .env_vars
                .get("__RUBASH_CURRENT_LINE")
                .and_then(|line| line.parse::<i128>().ok())
                .or(Some(1));
        }
        // Dynamic parameters ($SECONDS, $EPOCHSECONDS, ...) never have a
        // stored env_vars entry, so the fallback below would read them as 0.
        // Resolve them through the same path parameter expansion uses.
        if let Some(value) = env_derived_dynamic_parameter_value(self.env_vars, name) {
            if let Ok(number) = value.parse::<i128>() {
                return Some(bash_arith(number));
            }
        }

        // GNU expr.c treats a bare indexed-array operand as element zero.
        // The typed store serializes the whole array, which is not itself an
        // arithmetic expression, so resolve the scalar view before parsing.
        if is_marked_var(self.env_vars, ARRAY_VARS, name) {
            if let Some(value) = self
                .env_vars
                .get(name)
                .and_then(|value| array_value_at(value, 0))
            {
                return self.evaluate_variable_text(name, &value);
            }
        }

        let value = self
            .env_vars
            .get(name)
            .cloned()
            .or_else(|| std::env::var(name).ok())
            .unwrap_or_default();
        self.evaluate_variable_text(name, &value)
    }

    pub(super) fn evaluate_variable_text(
        &mut self,
        resolving_name: &str,
        value: &str,
    ) -> Option<i128> {
        if self
            .resolving
            .iter()
            .any(|resolving| resolving == resolving_name)
        {
            return None;
        }

        let value = value.trim();
        if value.is_empty() {
            return Some(0);
        }
        if let Ok(number) = value.parse::<i128>() {
            return Some(bash_arith(number));
        }

        let mut resolving = self.resolving.clone();
        resolving.push(resolving_name.to_string());
        let mut parser = ConditionalArithParser {
            input: value.as_bytes(),
            pos: 0,
            env_vars: self.env_vars,
            resolving,
            random_state: self.random_state,
            error_category: None,
        };
        let value = parser.parse_comma()?;
        parser.skip_ws();
        let category = parser.error_category;
        if category.is_some() {
            self.error_category = category;
        }
        (parser.pos == parser.input.len()).then_some(value)
    }

    pub(super) fn update_lvalue(
        &mut self,
        lvalue: &ArithLValue,
        delta: i128,
        prefix: bool,
    ) -> Option<i128> {
        if !self.lvalue_is_writable(lvalue) {
            return None;
        }
        let current = self.lvalue_value(lvalue)?;
        let updated = bash_arith(current + delta);
        self.set_lvalue(lvalue, updated);
        Some(if prefix { updated } else { current })
    }

    pub(super) fn assign_lvalue(
        &mut self,
        lvalue: &ArithLValue,
        op: &str,
        rhs: i128,
    ) -> Option<i128> {
        if !self.lvalue_is_writable(lvalue) {
            return None;
        }
        if op == "=" {
            self.set_lvalue(lvalue, rhs);
            return Some(rhs);
        }
        let current = self.lvalue_value(lvalue)?;
        let value = match op {
            "+=" => bash_arith(current + rhs),
            "-=" => bash_arith(current - rhs),
            "*=" => bash_arith(current * rhs),
            "**=" => checked_arithmetic_pow(current, rhs)?,
            "<<=" => bash_arith((current as i64).wrapping_shl(u32::try_from(rhs).ok()?) as i128),
            ">>=" => bash_arith((current as i64).wrapping_shr(u32::try_from(rhs).ok()?) as i128),
            "&=" => bash_arith(current & rhs),
            "^=" => bash_arith(current ^ rhs),
            "|=" => bash_arith(current | rhs),
            "/=" if rhs != 0 => bash_arith((current as i64).wrapping_div(rhs as i64) as i128),
            "%=" if rhs != 0 => {
                if current == i128::from(i64::MIN) && rhs == -1 {
                    return None;
                }
                current % rhs
            }
            "/=" | "%=" => return None,
            _ => return None,
        };
        self.set_lvalue(lvalue, value);
        Some(value)
    }

    fn lvalue_is_writable(&mut self, lvalue: &ArithLValue) -> bool {
        let name = match lvalue {
            ArithLValue::Scalar(name)
            | ArithLValue::Indexed { name, .. }
            | ArithLValue::Assoc { name, .. } => name,
        };
        if is_marked_var(self.env_vars, READONLY_VARS, name) {
            self.env_vars
                .insert("__RUBASH_ARITH_READONLY_ERROR".to_string(), name.clone());
            return false;
        }
        true
    }

    pub(super) fn set_lvalue(&mut self, lvalue: &ArithLValue, value: i128) {
        match lvalue {
            ArithLValue::Scalar(name) => self.set_variable(name, value),
            ArithLValue::Indexed { name, index } => self.set_array_element(name, *index, value),
            ArithLValue::Assoc { name, key } => self.set_assoc_element(name, key, value),
        }
    }

    pub(super) fn set_variable(&mut self, name: &str, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let value = bash_arith(value).to_string();
        if name == "SECONDS" {
            // Assignment resets the reference point so the dynamic value
            // becomes the assigned number and grows from there, matching
            // the parameter-assignment path in temporary_assignments.rs.
            let assigned = value.parse::<i64>().unwrap_or(0);
            let start = self
                .env_vars
                .get(SHELL_START_EPOCH)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or_else(current_epoch_seconds);
            let elapsed = current_epoch_seconds() - start;
            self.env_vars
                .insert(SECONDS_OFFSET.to_string(), (assigned - elapsed).to_string());
            set_process_env(name, value);
            return;
        }
        if name == "RANDOM" {
            if let Some(state) = self.random_state {
                state.set(value.parse::<u32>().unwrap_or(0));
            }
        }
        if name == "SRANDOM" {
            return;
        }
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars.insert(name.to_string(), value.clone());
        super::super::record_arith_write(name, old_value);
        set_process_env(name, value);
    }

    pub(super) fn set_array_element(&mut self, name: &str, index: i128, value: i128) {
        if is_noassign_bash_array(name) {
            return;
        }
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| indexed_array_entries(value))
            .unwrap_or_default();
        let index = if index < 0 {
            let storage = format_indexed_array_storage(entries.clone());
            let Some(index) = resolve_indexed_array_subscript(&storage, index) else {
                return;
            };
            index
        } else {
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            index
        };
        entries.insert(index, value.to_string());
        let value = format_indexed_array_storage(entries);
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars.insert(name.to_string(), value);
        super::super::record_arith_write(name, old_value);
        mark_env_name(self.env_vars, ARRAY_VARS, name);
    }

    pub(super) fn set_assoc_element(&mut self, name: &str, key: &str, value: i128) {
        let mut entries = self
            .env_vars
            .get(name)
            .map(|value| assoc_entries(value))
            .unwrap_or_default();
        let value = value.to_string();
        if let Some((_, existing)) = entries.iter_mut().find(|(entry_key, _)| entry_key == key) {
            *existing = value;
        } else {
            entries.push((key.to_string(), value));
        }
        let old_value = self.env_vars.get(name).cloned();
        self.env_vars
            .insert(name.to_string(), format_assoc_storage(entries));
        super::super::record_arith_write(name, old_value);
        mark_env_name(self.env_vars, ASSOC_VARS, name);
    }
}
