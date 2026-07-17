use super::{ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{bash_arith, checked_arithmetic_pow};
use crate::executor::{
    array_value_at, assoc_entries, assoc_value_at, format_assoc_storage,
    format_indexed_array_storage, indexed_array_entries, is_noassign_bash_array, mark_env_name,
    next_random_from_state, next_srandom_from_state, resolve_indexed_array_subscript,
    set_process_env, ARRAY_VARS, ASSOC_VARS,
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
        };
        let value = parser.parse_comma()?;
        parser.skip_ws();
        (parser.pos == parser.input.len()).then_some(value)
    }

    pub(super) fn update_lvalue(
        &mut self,
        lvalue: &ArithLValue,
        delta: i128,
        prefix: bool,
    ) -> Option<i128> {
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
        if name == "RANDOM" {
            if let Some(state) = self.random_state {
                state.set(value.parse::<u32>().unwrap_or(0));
            }
        }
        if name == "SRANDOM" {
            return;
        }
        self.env_vars.insert(name.to_string(), value.clone());
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
        self.env_vars.insert(name.to_string(), value);
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
        self.env_vars
            .insert(name.to_string(), format_assoc_storage(entries));
        mark_env_name(self.env_vars, ASSOC_VARS, name);
    }
}
