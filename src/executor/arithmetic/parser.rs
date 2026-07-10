#[path = "cursor.rs"]
mod cursor;
#[path = "expression.rs"]
mod expression;
#[path = "factor.rs"]
mod factor;
#[path = "lvalue.rs"]
mod lvalue;
#[path = "value.rs"]
mod value;

use std::cell::Cell;
use std::collections::HashMap;

pub(super) struct ConditionalArithParser<'a> {
    pub(super) input: &'a [u8],
    pub(super) pos: usize,
    pub(super) env_vars: &'a mut HashMap<String, String>,
    pub(super) resolving: Vec<String>,
    pub(super) random_state: Option<&'a Cell<u32>>,
}

#[derive(Clone)]
pub(super) enum ArithLValue {
    Scalar(String),
    Indexed { name: String, index: i128 },
    Assoc { name: String, key: String },
}
