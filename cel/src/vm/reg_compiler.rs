use crate::common::ast::operators;
use crate::common::ast::{Expr, LiteralValue, SelectExpr};
use crate::objects::Value;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::Expression;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAX_REGS: u8 = 16;

pub fn compile_reg(expr: &Expression) -> Result<RegProgram, String> {
    // Phase 1: collect variables and assign fixed registers 0..N
    let mut collector = VarCollector::new();
    collector.visit_expr(&expr.expr);
    let var_names: Vec<String> = collector.vars.into_iter().collect();

    let mut c = RegCompiler::new(var_names);
    let result_reg = c.compile_expr(&expr.expr)?;
    c.emit(RegInstr::Halt(result_reg));

    Ok(RegProgram {
        var_names: c.var_names,
        constants: c.constants,
        instructions: c.instructions,
        num_regs: c.next_reg,
    })
}

/* ---------- Variable collector ---------- */

struct VarCollector {
    vars: Vec<String>,
    seen: HashSet<String>,
}

impl VarCollector {
    fn new() -> Self {
        Self {
            vars: Vec::new(),
            seen: HashSet::new(),
        }
    }
    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(name) => {
                if !self.seen.contains(name) {
                    self.seen.insert(name.clone());
                    self.vars.push(name.clone());
                }
            }
            Expr::Call(call) => {
                for arg in &call.args {
                    self.visit_expr(&arg.expr);
                }
            }
            Expr::Select(sel) => {
                self.visit_expr(&sel.operand.expr);
            }
            Expr::List(list) => {
                for el in &list.elements {
                    self.visit_expr(&el.expr);
                }
            }
            Expr::Map(map) => {
                for entry in &map.entries {
                    match &entry.expr {
                        crate::common::ast::EntryExpr::MapEntry(me) => {
                            self.visit_expr(&me.key.expr);
                            self.visit_expr(&me.value.expr);
                        }
                        crate::common::ast::EntryExpr::StructField(_) => {}
                    }
                }
            }
            Expr::Comprehension(comp) => {
                self.visit_expr(&comp.iter_range.expr);
                self.visit_expr(&comp.loop_cond.expr);
                self.visit_expr(&comp.accu_init.expr);
                self.visit_expr(&comp.loop_step.expr);
                self.visit_expr(&comp.result.expr);
            }
            _ => {}
        }
    }
}

/* ---------- Register compiler ---------- */

struct RegCompiler {
    var_names: Vec<String>,
    var_reg: HashMap<String, u8>,
    constants: Vec<Value>,
    const_map: HashMap<String, u16>,
    instructions: Vec<RegInstr>,
    next_reg: u8, // next temp register (vars occupy 0..num_vars)
    free_temps: Vec<u8>,
}

impl RegCompiler {
    fn new(var_names: Vec<String>) -> Self {
        let num_vars = var_names.len() as u8;
        let mut var_reg = HashMap::new();
        for (i, name) in var_names.iter().enumerate() {
            var_reg.insert(name.clone(), i as u8);
        }
        Self {
            var_names,
            var_reg,
            constants: Vec::new(),
            const_map: HashMap::new(),
            instructions: Vec::new(),
            next_reg: num_vars,
            free_temps: Vec::new(),
        }
    }

    fn alloc_reg(&mut self) -> u8 {
        if let Some(r) = self.free_temps.pop() {
            return r;
        }
        let r = self.next_reg;
        if r >= MAX_REGS {
            panic!("out of registers (max {})", MAX_REGS);
        }
        self.next_reg += 1;
        r
    }

    fn free_reg(&mut self, r: u8) {
        if r >= self.var_names.len() as u8 {
            self.free_temps.push(r);
        }
    }

    fn emit(&mut self, instr: RegInstr) -> usize {
        self.instructions.push(instr);
        self.instructions.len() - 1
    }

    fn add_const(&mut self, v: Value) -> u16 {
        let key = format!("{:?}", v);
        if let Some(&idx) = self.const_map.get(&key) {
            return idx;
        }
        let idx = self.constants.len() as u16;
        self.constants.push(v);
        self.const_map.insert(key, idx);
        idx
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<u8, String> {
        match expr {
            Expr::Literal(lit) => {
                let dst = self.alloc_reg();
                match lit {
                    LiteralValue::Int(i) => {
                        self.emit(RegInstr::LoadConstInt(dst, *i.inner()));
                    }
                    LiteralValue::Boolean(b) => {
                        self.emit(RegInstr::LoadConstBool(dst, *b.inner()));
                    }
                    other => {
                        let v = lit_to_value(other);
                        let idx = self.add_const(v);
                        self.emit(RegInstr::LoadConst(dst, idx));
                    }
                }
                Ok(dst)
            }
            Expr::Ident(name) => {
                if let Some(&reg) = self.var_reg.get(name) {
                    Ok(reg)
                } else {
                    Err(format!("undeclared variable: {}", name))
                }
            }
            Expr::Call(call) => {
                let name = call.func_name.as_str();

                // Short-circuit logical ops
                if name == operators::LOGICAL_AND && call.args.len() == 2 {
                    return self.compile_short_circuit_and(&call.args[0].expr, &call.args[1].expr);
                }
                if name == operators::LOGICAL_OR && call.args.len() == 2 {
                    return self.compile_short_circuit_or(&call.args[0].expr, &call.args[1].expr);
                }

                if call.args.len() == 2 {
                    // Try type-specialized binary ops
                    if let Some(dst) =
                        self.try_compile_specialized_binary(name, &call.args[0].expr, &call.args[1].expr)
                    {
                        return Ok(dst);
                    }

                    // Generic binary
                    let r1 = self.compile_expr(&call.args[0].expr)?;
                    let r2 = self.compile_expr(&call.args[1].expr)?;
                    let dst = self.alloc_reg();
                    let instr = match name {
                        operators::ADD => RegInstr::AddInt(dst, r1, r2),
                        operators::SUBSTRACT => RegInstr::SubInt(dst, r1, r2),
                        operators::MULTIPLY => RegInstr::MulInt(dst, r1, r2),
                        operators::DIVIDE => RegInstr::DivInt(dst, r1, r2),
                        operators::MODULO => RegInstr::ModInt(dst, r1, r2),
                        operators::EQUALS => RegInstr::Eq(dst, r1, r2),
                        operators::NOT_EQUALS => RegInstr::Ne(dst, r1, r2),
                        operators::LESS => RegInstr::Lt(dst, r1, r2),
                        operators::LESS_EQUALS => RegInstr::Le(dst, r1, r2),
                        operators::GREATER => RegInstr::Gt(dst, r1, r2),
                        operators::GREATER_EQUALS => RegInstr::Ge(dst, r1, r2),
                        operators::IN => RegInstr::In(dst, r1, r2),
                        operators::INDEX => RegInstr::Index(dst, r1, r2),
                        _ => {
                            self.free_reg(dst);
                            self.free_reg(r2);
                            self.free_reg(r1);
                            return Err(format!("unsupported binary op: {}", name));
                        }
                    };
                    self.emit(instr);
                    self.free_reg(r2);
                    self.free_reg(r1);
                    Ok(dst)
                } else if call.args.len() == 1 {
                    let r1 = self.compile_expr(&call.args[0].expr)?;
                    let dst = self.alloc_reg();
                    let instr = match name {
                        operators::LOGICAL_NOT => RegInstr::Not(dst, r1),
                        operators::NEGATE => RegInstr::NegInt(dst, r1),
                        "size" => RegInstr::Size(dst, r1),
                        _ => {
                            self.free_reg(dst);
                            self.free_reg(r1);
                            return Err(format!("unsupported unary op: {}", name));
                        }
                    };
                    self.emit(instr);
                    self.free_reg(r1);
                    Ok(dst)
                } else {
                    Err(format!("unsupported call arity: {}", call.args.len()))
                }
            }
            Expr::Select(sel) => {
                let obj = self.compile_expr(&sel.operand.expr)?;
                let dst = self.alloc_reg();
                let field_idx = self.add_const(Value::String(std::sync::Arc::new(
                    sel.field.clone(),
                )));
                if sel.test {
                    self.emit(RegInstr::HasField(dst, obj, field_idx));
                } else {
                    self.emit(RegInstr::Select(dst, obj, field_idx));
                }
                self.free_reg(obj);
                Ok(dst)
            }
            Expr::List(list) => {
                // Build list — not optimized for reg VM, use generic path
                if list.elements.is_empty() {
                    let dst = self.alloc_reg();
                    let idx = self.add_const(Value::List(std::sync::Arc::new(vec![])));
                    self.emit(RegInstr::LoadConst(dst, idx));
                    return Ok(dst);
                }
                Err("non-empty list literals not supported in reg VM".into())
            }
            Expr::Map(_) => Err("map literals not supported in reg VM".into()),
            Expr::Comprehension(_) => Err("comprehensions not supported in reg VM".into()),
            _ => Err(format!("unsupported expr in reg VM: {:?}", std::mem::discriminant(expr))),
        }
    }

    /// Try to emit a type-specialized instruction for a binary op involving a literal.
    fn try_compile_specialized_binary(
        &mut self,
        op: &str,
        left: &Expr,
        right: &Expr,
    ) -> Option<u8> {
        // Pattern:  var  op  literal_int   OR   literal_int  op  var
        if let Some((var_reg, val)) = self.as_var_vs_int_literal(left, right) {
            let dst = self.alloc_reg();
            let instr = match op {
                operators::EQUALS => RegInstr::EqIntConst(dst, var_reg, val),
                operators::NOT_EQUALS => RegInstr::NeIntConst(dst, var_reg, val),
                operators::LESS => RegInstr::LtIntConst(dst, var_reg, val),
                operators::LESS_EQUALS => RegInstr::LeIntConst(dst, var_reg, val),
                operators::GREATER => RegInstr::GtIntConst(dst, var_reg, val),
                operators::GREATER_EQUALS => RegInstr::GeIntConst(dst, var_reg, val),
                _ => {
                    self.free_reg(dst);
                    return None;
                }
            };
            self.emit(instr);
            return Some(dst);
        }

    // Pattern: var op literal_string
    if let Some((var_reg, const_idx)) = self.as_var_vs_str_literal(left, right) {
        if op == operators::EQUALS {
            let dst = self.alloc_reg();
            self.emit(RegInstr::EqStrConst(dst, var_reg, const_idx));
            return Some(dst);
        }
    }

    // Pattern: var in [int1, int2, ...]  or  var in [str1, str2, ...]
    if op == operators::IN {
        if let Expr::Ident(name) = left {
            if let Some(&reg) = self.var_reg.get(name) {
                if let Expr::List(list) = right {
                    if !list.elements.is_empty() && list.elements.len() <= 16 {
                        if let Some(set_idx) = self.compile_int_set(list) {
                            let dst = self.alloc_reg();
                            self.emit(RegInstr::InIntSet(dst, reg, set_idx));
                            return Some(dst);
                        }
                        if let Some(set_idx) = self.compile_str_set(list) {
                            let dst = self.alloc_reg();
                            self.emit(RegInstr::InStrSet(dst, reg, set_idx));
                            return Some(dst);
                        }
                    }
                }
            }
        }
    }

    None
}

    fn compile_int_set(&mut self, list: &crate::common::ast::ListExpr) -> Option<u16> {
        let mut ints = Vec::with_capacity(list.elements.len());
        for el in &list.elements {
            if let Expr::Literal(LiteralValue::Int(i)) = &el.expr {
                ints.push(Value::Int(*i.inner()));
            } else {
                return None;
            }
        }
        let idx = self.add_const(Value::List(Arc::new(ints)));
        Some(idx)
    }

    fn compile_str_set(&mut self, list: &crate::common::ast::ListExpr) -> Option<u16> {
        let mut strs = Vec::with_capacity(list.elements.len());
        for el in &list.elements {
            if let Expr::Literal(LiteralValue::String(s)) = &el.expr {
                strs.push(Value::String(Arc::new(s.inner().to_string())));
            } else {
                return None;
            }
        }
        let idx = self.add_const(Value::List(Arc::new(strs)));
        Some(idx)
    }

    fn as_var_vs_int_literal(&self, left: &Expr, right: &Expr) -> Option<(u8, i64)> {
        if let Expr::Ident(name) = left {
            if let Some(&reg) = self.var_reg.get(name) {
                if let Expr::Literal(LiteralValue::Int(i)) = right {
                    return Some((reg, *i.inner()));
                }
            }
        }
        // Symmetric: literal on left
        if let Expr::Literal(LiteralValue::Int(i)) = left {
            if let Expr::Ident(name) = right {
                if let Some(&reg) = self.var_reg.get(name) {
                    return Some((reg, *i.inner()));
                }
            }
        }
        None
    }

    fn as_var_vs_str_literal(&mut self, left: &Expr, right: &Expr) -> Option<(u8, u16)> {
        if let Expr::Ident(name) = left {
            if let Some(&reg) = self.var_reg.get(name) {
                if let Expr::Literal(LiteralValue::String(s)) = right {
                    let idx = self.add_const(Value::String(std::sync::Arc::new(
                        s.inner().to_string(),
                    )));
                    return Some((reg, idx));
                }
            }
        }
        if let Expr::Literal(LiteralValue::String(s)) = left {
            let idx = self.add_const(Value::String(std::sync::Arc::new(s.inner().to_string())));
            if let Expr::Ident(name) = right {
                if let Some(&reg) = self.var_reg.get(name) {
                    return Some((reg, idx));
                }
            }
        }
        None
    }

    fn compile_short_circuit_and(&mut self, lhs: &Expr, rhs: &Expr) -> Result<u8, String> {
        let r1 = self.compile_expr(lhs)?;
        let dst = r1; // reuse the left result register

        // If left is false, jump over the RHS evaluation
        let jump_idx = self.emit(RegInstr::JumpIfFalse(dst, 0)); // patch later

        let r2 = self.compile_expr(rhs)?;
        // dst = dst && r2
        self.emit(RegInstr::And(dst, dst, r2));
        self.free_reg(r2);

        let after_rhs = self.instructions.len();
        self.patch_jump(jump_idx, after_rhs);

        Ok(dst)
    }

    fn compile_short_circuit_or(&mut self, lhs: &Expr, rhs: &Expr) -> Result<u8, String> {
        let r1 = self.compile_expr(lhs)?;
        let dst = r1;

        // If left is true, jump over the RHS evaluation
        let jump_idx = self.emit(RegInstr::JumpIfTrue(dst, 0));

        let r2 = self.compile_expr(rhs)?;
        self.emit(RegInstr::Or(dst, dst, r2));
        self.free_reg(r2);

        let after_rhs = self.instructions.len();
        self.patch_jump(jump_idx, after_rhs);

        Ok(dst)
    }

    fn patch_jump(&mut self, idx: usize, target: usize) {
        let offset = (target as isize) - ((idx + 1) as isize);
        let offset = offset as i16;
        self.instructions[idx] = match self.instructions[idx] {
            RegInstr::Jump(_) => RegInstr::Jump(offset),
            RegInstr::JumpIfFalse(r, _) => RegInstr::JumpIfFalse(r, offset),
            RegInstr::JumpIfTrue(r, _) => RegInstr::JumpIfTrue(r, offset),
            _ => panic!("patch_jump on non-jump"),
        };
    }
}

fn lit_to_value(lit: &LiteralValue) -> Value {
    match lit {
        LiteralValue::Boolean(b) => Value::Bool(*b.inner()),
        LiteralValue::Int(i) => Value::Int(*i.inner()),
        LiteralValue::UInt(u) => Value::UInt(*u.inner()),
        LiteralValue::Double(f) => Value::Float(*f.inner()),
        LiteralValue::String(s) => Value::String(std::sync::Arc::new(s.inner().to_string())),
        LiteralValue::Bytes(b) => Value::Bytes(std::sync::Arc::new(b.inner().to_vec())),
        LiteralValue::Null => Value::Null,
    }
}
