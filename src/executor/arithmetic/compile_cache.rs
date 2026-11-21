use super::factor::strlong_value;
use super::{ArithLValue, ConditionalArithParser};
use crate::executor::arithmetic::{
    assignment_operator_at, bash_arith, checked_arithmetic_pow, skip_arith_ws,
};
use crate::executor::{is_shell_name_char, is_shell_name_start};
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

#[derive(Clone, Copy, Debug)]
enum ArithBinOp {
    Add,
    Sub,
    Mul,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Eq,
    Ne,
    Ge,
    Le,
    Gt,
    Lt,
}

#[derive(Clone, Debug)]
enum ArithNode {
    Lit(i128),
    Var(u32),
    UnaryPlus(u32),
    UnaryNeg(u32),
    LogicalNot(u32),
    BitNot(u32),
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

fn expr_admissible_for_compile(input: &[u8]) -> bool {
    !input.is_empty()
        && input
            .iter()
            .all(|&b| b.is_ascii() && !matches!(b, b'[' | b']' | b'$' | b'\'' | b'"' | b'\\'))
}

fn compile_arith_expr(expression: &str) -> Option<CompiledArithExpr> {
    let input = expression.as_bytes();
    if !expr_admissible_for_compile(input) {
        return None;
    }
    let mut compiler = ArithCompiler {
        input,
        pos: 0,
        nodes: Vec::new(),
        names: Vec::new(),
        depth: 0,
    };
    let root = compiler.compile_comma()?;
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

struct ArithCompiler<'a> {
    input: &'a [u8],
    pos: usize,
    nodes: Vec<ArithNode>,
    names: Vec<String>,
    depth: usize,
}

impl ArithCompiler<'_> {
    fn push_node(&mut self, node: ArithNode) -> Option<u32> {
        let idx = u32::try_from(self.nodes.len()).ok()?;
        self.nodes.push(node);
        Some(idx)
    }

    fn intern_name(&mut self, name: &str) -> Option<u32> {
        if let Some(pos) = self.names.iter().position(|existing| existing == name) {
            return u32::try_from(pos).ok();
        }
        let idx = u32::try_from(self.names.len()).ok()?;
        self.names.push(name.to_string());
        Some(idx)
    }

    fn skip_ws(&mut self) {
        skip_arith_ws(self.input, &mut self.pos);
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn starts_with(&self, value: &str) -> bool {
        self.input[self.pos..].starts_with(value.as_bytes())
    }

    fn at_assignment_op(&self) -> bool {
        assignment_operator_at(self.input, self.pos).is_some()
    }

    fn enter(&mut self) -> Option<()> {
        if self.depth >= MAX_COMPILE_DEPTH {
            return None;
        }
        self.depth += 1;
        Some(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn scan_name_at(&self, mut pos: usize) -> Option<(usize, usize)> {
        skip_arith_ws(self.input, &mut pos);
        let start = pos;
        let first = *self.input.get(pos)? as char;
        if !is_shell_name_start(first) {
            return None;
        }
        pos += 1;
        while self
            .input
            .get(pos)
            .is_some_and(|ch| is_shell_name_char(*ch as char))
        {
            pos += 1;
        }
        Some((start, pos))
    }

    fn assignment_op_start(&self) -> Option<usize> {
        let (_, mut pos) = self.scan_name_at(self.pos)?;
        skip_arith_ws(self.input, &mut pos);
        assignment_operator_at(self.input, pos).map(|_| pos)
    }

    fn compile_comma(&mut self) -> Option<u32> {
        self.enter()?;
        let res = (|| {
            let mut node = self.compile_assignment()?;
            loop {
                self.skip_ws();
                if self.peek() != Some(b',') {
                    return Some(node);
                }
                self.pos += 1;
                let rhs = self.compile_assignment()?;
                node = self.push_node(ArithNode::Comma { lhs: node, rhs })?;
            }
        })();
        self.leave();
        res
    }

    fn compile_assignment(&mut self) -> Option<u32> {
        self.enter()?;
        let res = (|| {
            self.skip_ws();
            let start = self.pos;
            if let Some(op_pos) = self.assignment_op_start() {
                let (name_start, name_end) = self.scan_name_at(start)?;
                let name = std::str::from_utf8(&self.input[name_start..name_end]).ok()?;
                let var_idx = self.intern_name(name)?;
                self.pos = op_pos;
                let op = assignment_operator_at(self.input, self.pos)?;
                self.pos += op.len();
                self.skip_ws();
                let rhs_start = self.pos;
                let rhs = self.compile_assignment()?;
                return self.push_node(ArithNode::Assign {
                    var_idx,
                    op,
                    rhs,
                    rhs_start,
                });
            }
            self.pos = start;
            let value = self.compile_conditional()?;
            self.skip_ws();
            if self.at_assignment_op() {
                return None;
            }
            Some(value)
        })();
        self.leave();
        res
    }

    fn compile_conditional(&mut self) -> Option<u32> {
        self.enter()?;
        let res = (|| {
            let cond = self.compile_logical_or()?;
            self.skip_ws();
            if self.peek() != Some(b'?') {
                return Some(cond);
            }
            self.pos += 1;
            self.skip_ws();
            if self.pos >= self.input.len() || self.peek() == Some(b':') {
                return None;
            }
            let then_br = self.compile_comma()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return None;
            }
            self.pos += 1;
            self.skip_ws();
            if self.pos >= self.input.len() || self.assignment_op_start().is_some() {
                return None;
            }
            let else_br = self.compile_conditional()?;
            self.push_node(ArithNode::Cond {
                cond,
                then_br,
                else_br,
            })
        })();
        self.leave();
        res
    }

    fn compile_logical_or(&mut self) -> Option<u32> {
        let mut left = self.compile_logical_and()?;
        loop {
            self.skip_ws();
            if !self.starts_with("||") {
                return Some(left);
            }
            self.pos += 2;
            self.skip_ws();
            if self.assignment_op_start().is_some() {
                return None;
            }
            let right = self.compile_logical_and()?;
            left = self.push_node(ArithNode::LogicalOr {
                lhs: left,
                rhs: right,
            })?;
        }
    }

    fn compile_logical_and(&mut self) -> Option<u32> {
        let mut left = self.compile_bitwise_or()?;
        loop {
            self.skip_ws();
            if !self.starts_with("&&") {
                return Some(left);
            }
            self.pos += 2;
            self.skip_ws();
            if self.assignment_op_start().is_some() {
                return None;
            }
            let right = self.compile_bitwise_or()?;
            left = self.push_node(ArithNode::LogicalAnd {
                lhs: left,
                rhs: right,
            })?;
        }
    }

    fn compile_bitwise_or(&mut self) -> Option<u32> {
        let mut left = self.compile_bitwise_xor()?;
        loop {
            self.skip_ws();
            if self.starts_with("||") || self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'|') {
                self.pos += 1;
                let right = self.compile_bitwise_xor()?;
                left = self.push_node(ArithNode::Binary {
                    op: ArithBinOp::BitOr,
                    lhs: left,
                    rhs: right,
                })?;
            } else {
                return Some(left);
            }
        }
    }

    fn compile_bitwise_xor(&mut self) -> Option<u32> {
        let mut left = self.compile_bitwise_and()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'^') {
                self.pos += 1;
                let right = self.compile_bitwise_and()?;
                left = self.push_node(ArithNode::Binary {
                    op: ArithBinOp::BitXor,
                    lhs: left,
                    rhs: right,
                })?;
            } else {
                return Some(left);
            }
        }
    }

    fn compile_bitwise_and(&mut self) -> Option<u32> {
        let mut left = self.compile_comparison()?;
        loop {
            self.skip_ws();
            if self.starts_with("&&") || self.at_assignment_op() {
                return Some(left);
            }
            if self.peek() == Some(b'&') {
                self.pos += 1;
                let right = self.compile_comparison()?;
                left = self.push_node(ArithNode::Binary {
                    op: ArithBinOp::BitAnd,
                    lhs: left,
                    rhs: right,
                })?;
            } else {
                return Some(left);
            }
        }
    }

    fn compile_comparison(&mut self) -> Option<u32> {
        let mut left = self.compile_shift()?;
        loop {
            self.skip_ws();
            let op = if self.starts_with("==") {
                self.pos += 2;
                ArithBinOp::Eq
            } else if self.starts_with("!=") {
                self.pos += 2;
                ArithBinOp::Ne
            } else if self.starts_with(">=") {
                self.pos += 2;
                ArithBinOp::Ge
            } else if self.starts_with("<=") {
                self.pos += 2;
                ArithBinOp::Le
            } else if self.peek() == Some(b'>') {
                self.pos += 1;
                ArithBinOp::Gt
            } else if self.peek() == Some(b'<') {
                self.pos += 1;
                ArithBinOp::Lt
            } else {
                return Some(left);
            };
            let right = self.compile_shift()?;
            left = self.push_node(ArithNode::Binary {
                op,
                lhs: left,
                rhs: right,
            })?;
        }
    }

    fn compile_shift(&mut self) -> Option<u32> {
        let mut left = self.compile_expr()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(left);
            }
            let op = if self.starts_with("<<") {
                self.pos += 2;
                ArithBinOp::Shl
            } else if self.starts_with(">>") {
                self.pos += 2;
                ArithBinOp::Shr
            } else {
                return Some(left);
            };
            let right = self.compile_expr()?;
            left = self.push_node(ArithNode::Binary {
                op,
                lhs: left,
                rhs: right,
            })?;
        }
    }

    fn compile_expr(&mut self) -> Option<u32> {
        let mut left = self.compile_term()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(left);
            }
            let op = match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    ArithBinOp::Add
                }
                Some(b'-') => {
                    self.pos += 1;
                    ArithBinOp::Sub
                }
                _ => return Some(left),
            };
            let right = self.compile_term()?;
            left = self.push_node(ArithNode::Binary {
                op,
                lhs: left,
                rhs: right,
            })?;
        }
    }

    fn compile_term(&mut self) -> Option<u32> {
        let mut left = self.compile_power()?;
        loop {
            self.skip_ws();
            if self.at_assignment_op() {
                return Some(left);
            }
            match self.peek() {
                Some(b'*') => {
                    if self.starts_with("**") {
                        return Some(left);
                    }
                    self.pos += 1;
                    let right = self.compile_power()?;
                    left = self.push_node(ArithNode::Binary {
                        op: ArithBinOp::Mul,
                        lhs: left,
                        rhs: right,
                    })?;
                }
                Some(b'/') => {
                    self.pos += 1;
                    self.skip_ws();
                    let rhs_start = self.pos;
                    let right = self.compile_power()?;
                    left = self.push_node(ArithNode::Div {
                        lhs: left,
                        rhs: right,
                        rhs_start,
                    })?;
                }
                Some(b'%') => {
                    self.pos += 1;
                    self.skip_ws();
                    let rhs_start = self.pos;
                    let right = self.compile_power()?;
                    left = self.push_node(ArithNode::Mod {
                        lhs: left,
                        rhs: right,
                        rhs_start,
                    })?;
                }
                _ => return Some(left),
            }
        }
    }

    fn compile_power(&mut self) -> Option<u32> {
        self.enter()?;
        let res = (|| {
            let left = self.compile_factor()?;
            self.skip_ws();
            if self.starts_with("**") && !self.at_assignment_op() {
                self.pos += 2;
                self.skip_ws();
                let mut tok = self.pos;
                if matches!(self.input.get(tok), Some(b'-') | Some(b'+')) {
                    tok += 1;
                    skip_arith_ws(self.input, &mut tok);
                }
                let right = self.compile_power()?;
                self.push_node(ArithNode::Pow {
                    lhs: left,
                    rhs: right,
                    exp_tok: tok,
                })
            } else {
                Some(left)
            }
        })();
        self.leave();
        res
    }

    fn compile_factor(&mut self) -> Option<u32> {
        self.enter()?;
        let res = (|| {
            self.skip_ws();
            if self.starts_with("++") {
                let op_start = self.pos;
                self.pos += 2;
                if let Some((start, end)) = self.scan_name_at(self.pos) {
                    self.pos = end;
                    self.skip_ws();
                    if self.starts_with("++") || self.starts_with("--") {
                        return None;
                    }
                    let name = std::str::from_utf8(&self.input[start..end]).ok()?;
                    let var_idx = self.intern_name(name)?;
                    return self.push_node(ArithNode::PreInc(var_idx));
                }
                self.pos = op_start;
            }
            if self.starts_with("--") {
                let op_start = self.pos;
                self.pos += 2;
                if let Some((start, end)) = self.scan_name_at(self.pos) {
                    self.pos = end;
                    self.skip_ws();
                    if self.starts_with("++") || self.starts_with("--") {
                        return None;
                    }
                    let name = std::str::from_utf8(&self.input[start..end]).ok()?;
                    let var_idx = self.intern_name(name)?;
                    return self.push_node(ArithNode::PreDec(var_idx));
                }
                self.pos = op_start;
            }
            match self.peek() {
                Some(b'+') | Some(b'-') | Some(b'!')
                    if self.input.get(self.pos + 1) == Some(&b'=') =>
                {
                    None
                }
                Some(b'+') => {
                    self.pos += 1;
                    let inner = self.compile_factor()?;
                    self.push_node(ArithNode::UnaryPlus(inner))
                }
                Some(b'-') => {
                    self.pos += 1;
                    let inner = self.compile_factor()?;
                    self.push_node(ArithNode::UnaryNeg(inner))
                }
                Some(b'!') => {
                    self.pos += 1;
                    let inner = self.compile_factor()?;
                    self.push_node(ArithNode::LogicalNot(inner))
                }
                Some(b'~') => {
                    self.pos += 1;
                    let inner = self.compile_factor()?;
                    self.push_node(ArithNode::BitNot(inner))
                }
                Some(b'(') => {
                    self.pos += 1;
                    let inner = self.compile_comma()?;
                    self.skip_ws();
                    if self.peek() != Some(b')') {
                        return None;
                    }
                    self.pos += 1;
                    Some(inner)
                }
                Some(ch) if ch.is_ascii_digit() => {
                    let start = self.pos;
                    while self.peek().is_some_and(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'#' | b'@' | b'_')
                    }) {
                        self.pos += 1;
                    }
                    let value = strlong_value(&self.input[start..self.pos]).ok()?;
                    self.push_node(ArithNode::Lit(value))
                }
                Some(ch) if is_shell_name_start(ch as char) => {
                    let start = self.pos;
                    self.pos += 1;
                    while self.peek().is_some_and(|b| is_shell_name_char(b as char)) {
                        self.pos += 1;
                    }
                    let name = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
                    let var_idx = self.intern_name(name)?;
                    self.skip_ws();
                    if self.starts_with("++") {
                        self.pos += 2;
                        self.push_node(ArithNode::PostInc(var_idx))
                    } else if self.starts_with("--") {
                        self.pos += 2;
                        self.push_node(ArithNode::PostDec(var_idx))
                    } else {
                        self.push_node(ArithNode::Var(var_idx))
                    }
                }
                _ => None,
            }
        })();
        self.leave();
        res
    }
}

impl ConditionalArithParser<'_> {
    fn scalar_lvalue(&self, name: &str) -> ArithLValue {
        ArithLValue::Scalar {
            name: self.resolved_lvalue_name(name),
            written: name.to_string(),
        }
    }

    pub(in crate::executor::arithmetic) fn eval_compiled(
        &mut self,
        compiled: &CompiledArithExpr,
    ) -> Option<i128> {
        self.eval_compiled_node(compiled, compiled.root)
    }

    fn eval_compiled_node(&mut self, compiled: &CompiledArithExpr, idx: u32) -> Option<i128> {
        match &compiled.nodes[idx as usize] {
            ArithNode::Lit(v) => Some(*v),
            ArithNode::Var(var_idx) => {
                if self.noeval > 0 {
                    return Some(0);
                }
                let name = &compiled.names[*var_idx as usize];
                let resolved = self.resolved_lvalue_name(name);
                self.variable_value(&resolved)
            }
            ArithNode::UnaryPlus(inner) => self.eval_compiled_node(compiled, *inner),
            ArithNode::UnaryNeg(inner) => self
                .eval_compiled_node(compiled, *inner)
                .map(|v| bash_arith(-v)),
            ArithNode::LogicalNot(inner) => self
                .eval_compiled_node(compiled, *inner)
                .map(|v| i128::from(v == 0)),
            ArithNode::BitNot(inner) => self
                .eval_compiled_node(compiled, *inner)
                .map(|v| bash_arith(!v)),
            ArithNode::PreInc(var_idx) => {
                let lvalue = self.scalar_lvalue(&compiled.names[*var_idx as usize]);
                self.update_lvalue(&lvalue, 1, true)
            }
            ArithNode::PreDec(var_idx) => {
                let lvalue = self.scalar_lvalue(&compiled.names[*var_idx as usize]);
                self.update_lvalue(&lvalue, -1, true)
            }
            ArithNode::PostInc(var_idx) => {
                let lvalue = self.scalar_lvalue(&compiled.names[*var_idx as usize]);
                self.update_lvalue(&lvalue, 1, false)
            }
            ArithNode::PostDec(var_idx) => {
                let lvalue = self.scalar_lvalue(&compiled.names[*var_idx as usize]);
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
                    self.noeval += 1;
                    let r = self.eval_compiled_node(compiled, *rhs);
                    self.noeval -= 1;
                    r?;
                    Some(1)
                } else {
                    let r = self.eval_compiled_node(compiled, *rhs)?;
                    Some(i128::from(r != 0))
                }
            }
            ArithNode::LogicalAnd { lhs, rhs } => {
                let l = self.eval_compiled_node(compiled, *lhs)?;
                if l == 0 {
                    self.noeval += 1;
                    let r = self.eval_compiled_node(compiled, *rhs);
                    self.noeval -= 1;
                    r?;
                    Some(0)
                } else {
                    let r = self.eval_compiled_node(compiled, *rhs)?;
                    Some(i128::from(r != 0))
                }
            }
            ArithNode::Cond {
                cond,
                then_br,
                else_br,
            } => {
                let c = self.eval_compiled_node(compiled, *cond)?;
                if c == 0 {
                    self.noeval += 1;
                    let b = self.eval_compiled_node(compiled, *then_br);
                    self.noeval -= 1;
                    b?;
                    self.eval_compiled_node(compiled, *else_br)
                } else {
                    let v = self.eval_compiled_node(compiled, *then_br)?;
                    self.noeval += 1;
                    let b = self.eval_compiled_node(compiled, *else_br);
                    self.noeval -= 1;
                    b?;
                    Some(v)
                }
            }
            ArithNode::Assign {
                var_idx,
                op,
                rhs,
                rhs_start,
            } => {
                let lvalue = self.scalar_lvalue(&compiled.names[*var_idx as usize]);
                let precaptured = if *op != "=" {
                    Some(self.lvalue_value(&lvalue)?)
                } else {
                    None
                };
                let rhs_val = self.eval_compiled_node(compiled, *rhs)?;
                if matches!(*op, "/=" | "%=") && rhs_val == 0 && self.noeval == 0 {
                    return self.fail("division by 0", *rhs_start);
                }
                self.assign_lvalue_precaptured(&lvalue, op, rhs_val, precaptured)
            }
            ArithNode::Comma { lhs, rhs } => {
                self.eval_compiled_node(compiled, *lhs)?;
                self.eval_compiled_node(compiled, *rhs)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compile_arith_expr, lookup_or_compile, ARITH_COMPILE_CACHE, ARITH_COMPILE_CACHE_MAX_ENTRIES,
    };

    #[test]
    fn compile_cache_is_bounded_and_retains_hot_entries() {
        ARITH_COMPILE_CACHE.with(|cache| cache.borrow_mut().entries.clear());

        let hot = "sum += i * 2 + 1";
        assert!(lookup_or_compile(hot).is_some());

        for idx in 0..(ARITH_COMPILE_CACHE_MAX_ENTRIES * 4) {
            if idx % 50 == 0 {
                assert!(lookup_or_compile(hot).is_some());
            }
            let one_off = format!("x + {idx} * 3");
            assert!(lookup_or_compile(&one_off).is_some());
            let len = ARITH_COMPILE_CACHE.with(|cache| cache.borrow().entries.len());
            assert!(
                len <= ARITH_COMPILE_CACHE_MAX_ENTRIES,
                "cache exceeded bound: {len} > {ARITH_COMPILE_CACHE_MAX_ENTRIES}"
            );
        }

        let contains_hot =
            ARITH_COMPILE_CACHE.with(|cache| cache.borrow().entries.contains_key(hot));
        assert!(
            contains_hot,
            "hot expression should remain cached under LRU"
        );
    }

    #[test]
    fn non_admissible_or_syntax_error_expressions_fall_through() {
        assert!(compile_arith_expr("a[0] + 1").is_none());
        assert!(compile_arith_expr("1 + ").is_none());
        assert!(compile_arith_expr("7 = 43").is_none());
        assert!(compile_arith_expr("1 ? 2 : x += 3").is_none());
        assert!(compile_arith_expr("1 || x = 2").is_none());
        assert!(compile_arith_expr("08").is_none());
        assert!(compile_arith_expr("++x++").is_none());
    }
}
