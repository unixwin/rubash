use super::expression::{
    assignment_op_start_in, parse_comma_with, parse_conditional_tail_with, parse_conditional_with,
    parse_logical_and_with,
};
use super::{ArithBackend, ArithBinOp, ArithLValue, ArithUnaryOp, ConditionalArithParser};
use crate::executor::arithmetic::{assignment_operator_at, bash_arith, checked_arithmetic_pow};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const ARITH_COMPILE_CACHE_MAX_ENTRIES: usize = 256;
const MAX_COMPILE_DEPTH: usize = 256;

thread_local! {
    /// Bounded LRU compiled-form cache for general arithmetic expressions
    /// (rubash#2826). Keyed by the preprocessed expression text
    /// (`normalized`), which is a pure function of the compiled AST.
    /// Evaluation always runs fresh against current variable bindings.
    static ARITH_COMPILE_CACHE: RefCell<ArithCompileCache> =
        RefCell::new(ArithCompileCache::default());
}

#[derive(Default)]
struct ArithCompileCache {
    entries: HashMap<String, (Rc<CompiledArithExpr>, u64)>,
    clock: u64,
}

impl ArithCompileCache {
    fn get(&mut self, key: &str) -> Option<Rc<CompiledArithExpr>> {
        let (compiled, last_used) = self.entries.get_mut(key)?;
        *last_used = self.clock;
        self.clock = self.clock.wrapping_add(1);
        Some(Rc::clone(compiled))
    }

    fn insert(&mut self, key: String, compiled: Rc<CompiledArithExpr>) {
        if !self.entries.contains_key(&key) && self.entries.len() >= ARITH_COMPILE_CACHE_MAX_ENTRIES
        {
            if let Some(evict_key) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, ts))| *ts)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&evict_key);
            }
        }
        self.entries.insert(key, (compiled, self.clock));
        self.clock = self.clock.wrapping_add(1);
    }
}

#[derive(Clone, Debug)]
enum ArithNode {
    Lit(i128),
    Var(u32),
    Unary {
        op: ArithUnaryOp,
        operand: u32,
    },
    PreInc(u32),
    PreDec(u32),
    PostInc(u32),
    PostDec(u32),
    Binary {
        op: ArithBinOp,
        lhs: u32,
        rhs: u32,
    },
    Div {
        lhs: u32,
        rhs: u32,
        rhs_start: usize,
    },
    Mod {
        lhs: u32,
        rhs: u32,
        rhs_start: usize,
    },
    Pow {
        lhs: u32,
        rhs: u32,
        exp_tok: usize,
    },
    LogicalOr {
        lhs: u32,
        rhs: u32,
    },
    LogicalAnd {
        lhs: u32,
        rhs: u32,
    },
    Cond {
        cond: u32,
        then_br: u32,
        else_br: u32,
    },
    Assign {
        var_idx: u32,
        op: &'static str,
        rhs: u32,
        rhs_start: usize,
    },
    Comma {
        lhs: u32,
        rhs: u32,
    },
}

#[derive(Debug)]
pub(in crate::executor::arithmetic) struct CompiledArithExpr {
    nodes: Vec<ArithNode>,
    names: Vec<String>,
    root: u32,
}

#[inline]
fn is_shell_name_start_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphabetic()
}

#[inline]
fn is_shell_name_char_byte(b: u8) -> bool {
    b == b'_' || b.is_ascii_alphanumeric()
}

fn expr_admissible_for_compile(input: &[u8]) -> bool {
    !input.is_empty()
        && input
            .iter()
            .all(|&b| b.is_ascii() && !matches!(b, b'[' | b']' | b'$' | b'\'' | b'"' | b'\\'))
}

pub(in crate::executor::arithmetic) fn lookup_or_compile(
    normalized: &str,
) -> Option<Rc<CompiledArithExpr>> {
    if let Some(hit) = ARITH_COMPILE_CACHE.with(|cache| cache.borrow_mut().get(normalized)) {
        return Some(hit);
    }
    let compiled = Rc::new(compile_arith_expr(normalized)?);
    ARITH_COMPILE_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(normalized.to_string(), Rc::clone(&compiled));
    });
    Some(compiled)
}

fn compile_arith_expr(expression: &str) -> Option<CompiledArithExpr> {
    let input = expression.as_bytes();
    if !expr_admissible_for_compile(input) {
        return None;
    }
    let mut compiler = AstCompiler {
        input,
        pos: 0,
        last_tok_start: 0,
        last_tok_operand: false,
        depth: 0,
        nodes: Vec::with_capacity(16),
        names: Vec::with_capacity(4),
    };
    let root = parse_comma_with(&mut compiler)?;
    compiler.skip_ws();
    if compiler.pos != compiler.input.len() {
        return None;
    }
    Some(CompiledArithExpr {
        nodes: compiler.nodes,
        names: compiler.names,
        root,
    })
}

struct AstCompiler<'a> {
    input: &'a [u8],
    pos: usize,
    last_tok_start: usize,
    last_tok_operand: bool,
    depth: usize,
    nodes: Vec<ArithNode>,
    names: Vec<String>,
}

impl AstCompiler<'_> {
    fn push(&mut self, node: ArithNode) -> Option<u32> {
        let idx = u32::try_from(self.nodes.len()).ok()?;
        self.nodes.push(node);
        Some(idx)
    }

    fn intern_name(&mut self, name: &str) -> Option<u32> {
        if let Some(pos) = self.names.iter().position(|existing| existing == name) {
            return u32::try_from(pos).ok();
        }
        let idx = u32::try_from(self.names.len()).ok()?;
        self.names.push(name.to_owned());
        Some(idx)
    }

    fn parse_scalar_name(&mut self) -> Option<u32> {
        self.skip_ws();
        let start = self.pos;
        let first = self.peek()?;
        if !is_shell_name_start_byte(first) {
            return None;
        }
        self.pos += 1;
        while self.peek().is_some_and(is_shell_name_char_byte) {
            self.pos += 1;
        }
        // Admissible expressions contain no `[` / `]`; if one ever appeared,
        // decline compilation cleanly.
        if self.peek() == Some(b'[') {
            return None;
        }
        let name = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
        self.intern_name(name)
    }
}

impl ArithBackend for AstCompiler<'_> {
    type Val = u32;

    fn input(&self) -> &[u8] {
        self.input
    }

    fn pos(&self) -> usize {
        self.pos
    }

    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    fn last_tok_start(&self) -> usize {
        self.last_tok_start
    }

    fn last_tok_operand(&self) -> bool {
        self.last_tok_operand
    }

    fn note_op(&mut self, start: usize) {
        self.last_tok_start = start;
        self.last_tok_operand = false;
    }

    fn note_operand(&mut self, start: usize) {
        self.last_tok_start = start;
        self.last_tok_operand = true;
    }

    fn fail(&mut self, _msg: &str, _tok_start: usize) -> Option<u32> {
        None
    }

    fn fail_display(&mut self, _msg: &str, _tok_start: usize, _display_end: usize) -> Option<u32> {
        None
    }

    fn fail_operand_expected(&mut self) -> Option<u32> {
        None
    }

    fn fail_invalid_operator(&mut self) -> Option<u32> {
        None
    }

    fn enter_depth(&mut self) -> bool {
        if self.depth >= MAX_COMPILE_DEPTH {
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave_depth(&mut self) {
        self.depth -= 1;
    }

    fn lit(&mut self, value: i128) -> Option<u32> {
        self.push(ArithNode::Lit(value))
    }

    fn unary(&mut self, op: ArithUnaryOp, operand: u32) -> Option<u32> {
        self.push(ArithNode::Unary { op, operand })
    }

    fn binary(&mut self, op: ArithBinOp, lhs: u32, rhs: u32) -> Option<u32> {
        self.push(ArithNode::Binary { op, lhs, rhs })
    }

    fn div(&mut self, lhs: u32, rhs: u32, rhs_start: usize) -> Option<u32> {
        self.push(ArithNode::Div {
            lhs,
            rhs,
            rhs_start,
        })
    }

    fn rem(&mut self, lhs: u32, rhs: u32, rhs_start: usize) -> Option<u32> {
        self.push(ArithNode::Mod {
            lhs,
            rhs,
            rhs_start,
        })
    }

    fn pow(&mut self, lhs: u32, rhs: u32, exp_tok: usize) -> Option<u32> {
        self.push(ArithNode::Pow { lhs, rhs, exp_tok })
    }

    fn comma(&mut self, lhs: u32, rhs: u32) -> Option<u32> {
        self.push(ArithNode::Comma { lhs, rhs })
    }

    fn eval_logical_or(&mut self, lhs: u32) -> Option<u32> {
        let rhs = parse_logical_and_with(self)?;
        self.push(ArithNode::LogicalOr { lhs, rhs })
    }

    fn eval_logical_and(&mut self, lhs: u32) -> Option<u32> {
        let rhs = parse_conditional_tail_with(self)?;
        self.push(ArithNode::LogicalAnd { lhs, rhs })
    }

    fn eval_conditional(&mut self, cond: u32) -> Option<u32> {
        let then_br = parse_comma_with(self)?;
        self.skip_ws();
        if self.peek() != Some(b':') {
            return None;
        }
        self.note_op(self.pos);
        self.pos += 1;
        self.skip_ws();
        if self.pos >= self.input.len() {
            return None;
        }
        if assignment_op_start_in(self.input, self.pos).is_some() {
            return None;
        }
        let else_br = parse_conditional_with(self)?;
        self.push(ArithNode::Cond {
            cond,
            then_br,
            else_br,
        })
    }

    fn try_parse_assignment(&mut self) -> Option<Option<u32>> {
        if assignment_op_start_in(self.input, self.pos).is_none() {
            return Some(None);
        }
        let var_idx = self.parse_scalar_name()?;
        self.skip_ws();
        let op_start = self.pos;
        let Some(op) = assignment_operator_at(self.input, self.pos) else {
            return Some(None);
        };
        self.pos += op.len();
        self.note_op(op_start);
        self.skip_ws();
        let rhs_start = self.pos;
        let rhs = super::expression::parse_assignment_with(self)?;
        self.push(ArithNode::Assign {
            var_idx,
            op,
            rhs,
            rhs_start,
        })
        .map(Some)
    }

    fn try_parse_pre_incdec(&mut self, delta: i128) -> Option<Option<u32>> {
        let op_start = self.pos;
        self.pos += 2;
        if let Some(var_idx) = self.parse_scalar_name() {
            self.note_op(op_start);
            let node = if delta > 0 {
                ArithNode::PreInc(var_idx)
            } else {
                ArithNode::PreDec(var_idx)
            };
            let id = self.push(node)?;
            if !super::factor::post_incdec_after_preincdec_with(self) {
                return None;
            }
            return Some(Some(id));
        }
        self.pos = op_start;
        Some(None)
    }

    fn parse_variable_factor(&mut self) -> Option<u32> {
        let tok_start = self.pos;
        let var_idx = self.parse_scalar_name()?;
        self.skip_ws();
        if self.input[self.pos..].starts_with(b"++") {
            self.pos += 2;
            self.note_operand(tok_start);
            return self.push(ArithNode::PostInc(var_idx));
        }
        if self.input[self.pos..].starts_with(b"--") {
            self.pos += 2;
            self.note_operand(tok_start);
            return self.push(ArithNode::PostDec(var_idx));
        }
        self.note_operand(tok_start);
        self.push(ArithNode::Var(var_idx))
    }
}

impl ConditionalArithParser<'_> {
    pub(in crate::executor::arithmetic) fn eval_compiled(
        &mut self,
        compiled: &CompiledArithExpr,
    ) -> Option<i128> {
        self.eval_compiled_node(compiled, compiled.root)
    }

    fn scalar_lvalue(&self, compiled: &CompiledArithExpr, var_idx: u32) -> ArithLValue {
        let name = &compiled.names[var_idx as usize];
        let resolved = self.resolved_lvalue_name(name);
        ArithLValue::Scalar {
            name: resolved,
            written: name.clone(),
        }
    }

    fn eval_compiled_node(&mut self, compiled: &CompiledArithExpr, idx: u32) -> Option<i128> {
        match &compiled.nodes[idx as usize] {
            ArithNode::Lit(value) => Some(*value),
            ArithNode::Var(var_idx) => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                let value = self.lvalue_value(&lvalue);
                if value.is_none() && self.error.is_none() {
                    self.fail_operand_expected();
                }
                value
            }
            ArithNode::Unary { op, operand } => {
                let value = self.eval_compiled_node(compiled, *operand)?;
                Some(match op {
                    ArithUnaryOp::Plus => value,
                    ArithUnaryOp::Neg => bash_arith(-value),
                    ArithUnaryOp::LogicalNot => i128::from(value == 0),
                    ArithUnaryOp::BitNot => bash_arith(!value),
                })
            }
            ArithNode::PreInc(var_idx) => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                self.update_lvalue(&lvalue, 1, true)
            }
            ArithNode::PreDec(var_idx) => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                self.update_lvalue(&lvalue, -1, true)
            }
            ArithNode::PostInc(var_idx) => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                self.update_lvalue(&lvalue, 1, false)
            }
            ArithNode::PostDec(var_idx) => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                self.update_lvalue(&lvalue, -1, false)
            }
            ArithNode::Binary { op, lhs, rhs } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                let r = self.eval_compiled_node(compiled, *rhs)?;
                Some(match op {
                    ArithBinOp::Add => bash_arith(l + r),
                    ArithBinOp::Sub => bash_arith(l - r),
                    ArithBinOp::Mul => bash_arith(l * r),
                    ArithBinOp::BitOr => bash_arith(l | r),
                    ArithBinOp::BitXor => bash_arith(l ^ r),
                    ArithBinOp::BitAnd => bash_arith(l & r),
                    ArithBinOp::Shl => bash_arith((l as i64).wrapping_shl(r as u32) as i128),
                    ArithBinOp::Shr => bash_arith((l as i64).wrapping_shr(r as u32) as i128),
                    ArithBinOp::Eq => i128::from(l == r),
                    ArithBinOp::Ne => i128::from(l != r),
                    ArithBinOp::Ge => i128::from(l >= r),
                    ArithBinOp::Le => i128::from(l <= r),
                    ArithBinOp::Gt => i128::from(l > r),
                    ArithBinOp::Lt => i128::from(l < r),
                })
            }
            ArithNode::Div {
                lhs,
                rhs,
                rhs_start,
            } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                let mut r = self.eval_compiled_node(compiled, *rhs)?;
                if r == 0 {
                    if self.noeval == 0 {
                        return self.fail("division by 0", *rhs_start);
                    }
                    r = 1;
                }
                Some(bash_arith((l as i64).wrapping_div(r as i64) as i128))
            }
            ArithNode::Mod {
                lhs,
                rhs,
                rhs_start,
            } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                let mut r = self.eval_compiled_node(compiled, *rhs)?;
                if r == 0 {
                    if self.noeval == 0 {
                        return self.fail("division by 0", *rhs_start);
                    }
                    r = 1;
                }
                if l == i128::from(i64::MIN) && r == -1 {
                    Some(0)
                } else {
                    Some(l % r)
                }
            }
            ArithNode::Pow { lhs, rhs, exp_tok } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                let r = self.eval_compiled_node(compiled, *rhs)?;
                if r < 0 {
                    if self.noeval == 0 {
                        return self.fail("exponent less than 0", *exp_tok);
                    }
                    return Some(1);
                }
                checked_arithmetic_pow(l, r)
            }
            ArithNode::LogicalOr { lhs, rhs } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                if l != 0 {
                    return Some(1);
                }
                let r = self.eval_compiled_node(compiled, *rhs)?;
                Some(i128::from(r != 0))
            }
            ArithNode::LogicalAnd { lhs, rhs } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                if l == 0 {
                    return Some(0);
                }
                let r = self.eval_compiled_node(compiled, *rhs)?;
                Some(i128::from(r != 0))
            }
            ArithNode::Cond {
                cond,
                then_br,
                else_br,
            } => {
                let c = self.eval_compiled_node(compiled, *cond)?;
                if c != 0 {
                    self.eval_compiled_node(compiled, *then_br)
                } else {
                    self.eval_compiled_node(compiled, *else_br)
                }
            }
            ArithNode::Assign {
                var_idx,
                op,
                rhs,
                rhs_start,
            } => {
                let lvalue = self.scalar_lvalue(compiled, *var_idx);
                let precaptured = if *op != "=" {
                    Some(self.lvalue_value(&lvalue)?)
                } else {
                    None
                };
                let r = self.eval_compiled_node(compiled, *rhs)?;
                if matches!(*op, "/=" | "%=") && r == 0 && self.noeval == 0 {
                    return self.fail("division by 0", *rhs_start);
                }
                self.assign_lvalue_precaptured(&lvalue, op, r, precaptured)
            }
            ArithNode::Comma { lhs, rhs } => {
                let _ = self.eval_compiled_node(compiled, *lhs)?;
                self.eval_compiled_node(compiled, *rhs)
            }
        }
    }
}
