use crate::common::ast::operators;
use crate::common::ast::{ComprehensionExpr, EntryExpr, Expr, LiteralValue};
use crate::objects::Value;
use crate::vm::bytecode::{Instr, Program, IDX_ACCU, IDX_ITER_ELEM};
use crate::vm::vm::{
    try_bool, vm_add, vm_div, vm_eq, vm_in, vm_index, vm_le, vm_logical_and, vm_logical_or, vm_lt,
    vm_mod, vm_mul, vm_neg, vm_sub,
};
use crate::Expression;
use std::collections::HashMap;

pub fn compile(expr: &Expression) -> Result<Program, String> {
    let mut c = Compiler::new();
    c.compile_expr(&expr.expr)?;
    c.instructions.push(Instr::Return);
    Ok(Program {
        constants: c.constants,
        var_names: c.var_names.into_iter().collect(),
        instructions: c.instructions,
    })
}

struct Compiler {
    constants: Vec<Value>,
    const_map: HashMap<String, u16>,
    var_names: Vec<String>,
    var_map: HashMap<String, u16>,
    instructions: Vec<Instr>,
    comp_scopes: Vec<HashMap<String, CompBinding>>,
}

#[derive(Clone, Copy)]
enum CompBinding {
    Iter,
    Accu,
}

impl Compiler {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            const_map: HashMap::new(),
            var_names: Vec::new(),
            var_map: HashMap::new(),
            instructions: Vec::new(),
            comp_scopes: Vec::new(),
        }
    }

    fn add_const(&mut self, v: Value) -> u16 {
        let key = format!("{:?}", v);
        if let Some(&idx) = self.const_map.get(&key) {
            return idx;
        }
        let idx = self.constants.len() as u16;
        self.constants.push(v.clone());
        self.const_map.insert(key, idx);
        idx
    }

    fn add_string_const(&mut self, s: &str) -> u16 {
        self.add_const(Value::String(std::sync::Arc::new(s.to_string())))
    }

    fn ensure_var(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.var_map.get(name) {
            return idx;
        }
        let idx = self.var_names.len() as u16;
        self.var_names.push(name.to_string());
        self.var_map.insert(name.to_string(), idx);
        idx
    }

    fn as_var_idx(&mut self, expr: &Expr) -> Option<u16> {
        match expr {
            Expr::Ident(name) => {
                if self.lookup_comp_var(name).is_some() {
                    None
                } else {
                    Some(self.ensure_var(name))
                }
            }
            _ => None,
        }
    }

    fn as_const_idx(&mut self, expr: &Expr) -> Option<u16> {
        match expr {
            Expr::Literal(lit) => {
                let v = lit_to_value(lit);
                Some(self.add_const(v))
            }
            _ => None,
        }
    }

    fn try_emit_var_const_cmp(
        &mut self,
        left: &Expr,
        right: &Expr,
        name: &str,
    ) -> bool {
        // left = var, right = const
        if let Some(var_idx) = self.as_var_idx(left) {
            if let Some(const_idx) = self.as_const_idx(right) {
                let instr = match name {
                    operators::EQUALS => Instr::LoadVarEqConst(var_idx, const_idx),
                    operators::NOT_EQUALS => Instr::LoadVarNeConst(var_idx, const_idx),
                    operators::LESS => Instr::LoadVarLtConst(var_idx, const_idx),
                    operators::LESS_EQUALS => Instr::LoadVarLeConst(var_idx, const_idx),
                    operators::GREATER => Instr::LoadVarGtConst(var_idx, const_idx),
                    operators::GREATER_EQUALS => Instr::LoadVarGeConst(var_idx, const_idx),
                    _ => return false,
                };
                self.emit(instr);
                return true;
            }
        }
        // left = const, right = var  (swap comparison direction)
        if let Some(var_idx) = self.as_var_idx(right) {
            if let Some(const_idx) = self.as_const_idx(left) {
                let instr = match name {
                    operators::EQUALS => Instr::LoadVarEqConst(var_idx, const_idx),
                    operators::NOT_EQUALS => Instr::LoadVarNeConst(var_idx, const_idx),
                    operators::LESS => Instr::LoadVarGtConst(var_idx, const_idx),
                    operators::LESS_EQUALS => Instr::LoadVarGeConst(var_idx, const_idx),
                    operators::GREATER => Instr::LoadVarLtConst(var_idx, const_idx),
                    operators::GREATER_EQUALS => Instr::LoadVarLeConst(var_idx, const_idx),
                    _ => return false,
                };
                self.emit(instr);
                return true;
            }
        }
        false
    }

    fn emit(&mut self, instr: Instr) -> usize {
        self.instructions.push(instr);
        self.instructions.len() - 1
    }

    fn emit_jump(&mut self, op: fn(i16) -> Instr) -> usize {
        let idx = self.emit(op(0));
        idx
    }

    fn patch_jump(&mut self, idx: usize, target: usize) {
        let offset = (target as isize) - ((idx + 1) as isize);
        let offset = offset as i16;
        self.instructions[idx] = match self.instructions[idx] {
            Instr::Jump(_) => Instr::Jump(offset),
            Instr::JumpIfFalse(_) => Instr::JumpIfFalse(offset),
            Instr::JumpIfTrue(_) => Instr::JumpIfTrue(offset),
            Instr::JumpIfFalseKeep(_) => Instr::JumpIfFalseKeep(offset),
            Instr::JumpIfTrueKeep(_) => Instr::JumpIfTrueKeep(offset),
            Instr::IterNext(_) => Instr::IterNext(offset),
            _ => panic!("patch_jump on non-jump"),
        };
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), String> {
        match expr {
            Expr::Literal(lit) => {
                let v = lit_to_value(lit);
                let idx = self.add_const(v);
                self.emit(Instr::PushConst(idx));
                Ok(())
            }
            Expr::Ident(name) => {
                if let Some(binding) = self.lookup_comp_var(name) {
                    match binding {
                        CompBinding::Iter => self.emit(Instr::LoadVar(IDX_ITER_ELEM)),
                        CompBinding::Accu => self.emit(Instr::LoadVar(IDX_ACCU)),
                    };
                } else {
                    let idx = self.ensure_var(name);
                    self.emit(Instr::LoadVar(idx));
                }
                Ok(())
            }
            Expr::Call(call) => self.compile_call(call),
            Expr::Select(sel) => self.compile_select(sel),
            Expr::Comprehension(comp) => self.compile_comprehension(comp),
            Expr::List(list) => {
                for elem in &list.elements {
                    self.compile_expr(&elem.expr)?;
                }
                self.emit(Instr::BuildList(list.elements.len() as u16));
                Ok(())
            }
            Expr::Map(map) => {
                for entry in &map.entries {
                    match &entry.expr {
                        EntryExpr::MapEntry(e) => {
                            self.compile_expr(&e.key.expr)?;
                            self.compile_expr(&e.value.expr)?;
                        }
                        _ => return Err("unsupported map entry".into()),
                    }
                }
                self.emit(Instr::BuildMap(map.entries.len() as u16));
                Ok(())
            }
            _ => Err(format!(
                "unsupported expr: {:?}",
                std::mem::discriminant(expr)
            )),
        }
    }

    fn compile_call(&mut self, call: &crate::common::ast::CallExpr) -> Result<(), String> {
        let name = call.func_name.as_str();

        // Binary ops
        if call.args.len() == 2 {
            if let (Some(a), Some(b)) = (expr_to_value(&call.args[0].expr), expr_to_value(&call.args[1].expr)) {
                if let Some(v) = fold_binary(name, a, b) {
                    let idx = self.add_const(v);
                    self.emit(Instr::PushConst(idx));
                    return Ok(());
                }
            }
            match name {
                operators::ADD => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Add);
                    return Ok(());
                }
                operators::SUBSTRACT => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Sub);
                    return Ok(());
                }
                operators::MULTIPLY => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Mul);
                    return Ok(());
                }
                operators::DIVIDE => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Div);
                    return Ok(());
                }
                operators::MODULO => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Mod);
                    return Ok(());
                }
                operators::EQUALS => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::EQUALS) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Eq);
                    return Ok(());
                }
                operators::NOT_EQUALS => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::NOT_EQUALS) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Ne);
                    return Ok(());
                }
                operators::LESS => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::LESS) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Lt);
                    return Ok(());
                }
                operators::LESS_EQUALS => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::LESS_EQUALS) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Le);
                    return Ok(());
                }
                operators::GREATER => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::GREATER) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Gt);
                    return Ok(());
                }
                operators::GREATER_EQUALS => {
                    if self.try_emit_var_const_cmp(&call.args[0].expr, &call.args[1].expr, operators::GREATER_EQUALS) {
                        return Ok(());
                    }
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Ge);
                    return Ok(());
                }
                operators::LOGICAL_AND => {
                    self.compile_expr(&call.args[0].expr)?;
                    let jump = self.emit_jump(Instr::JumpIfFalseKeep);
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::LogicalAnd);
                    self.patch_jump(jump, self.instructions.len());
                    return Ok(());
                }
                operators::LOGICAL_OR => {
                    self.compile_expr(&call.args[0].expr)?;
                    let jump = self.emit_jump(Instr::JumpIfTrueKeep);
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::LogicalOr);
                    self.patch_jump(jump, self.instructions.len());
                    return Ok(());
                }
                operators::IN => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::In);
                    return Ok(());
                }
                operators::INDEX => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.compile_expr(&call.args[1].expr)?;
                    self.emit(Instr::Index);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Unary ops
        if call.args.len() == 1 {
            if let Some(a) = expr_to_value(&call.args[0].expr) {
                if let Some(v) = fold_unary(name, a) {
                    let idx = self.add_const(v);
                    self.emit(Instr::PushConst(idx));
                    return Ok(());
                }
            }
            match name {
                operators::LOGICAL_NOT => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.emit(Instr::Not);
                    return Ok(());
                }
                operators::NEGATE => {
                    self.compile_expr(&call.args[0].expr)?;
                    self.emit(Instr::Neg);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Ternary / conditional
        if call.args.len() == 3 && name == operators::CONDITIONAL {
            self.compile_expr(&call.args[0].expr)?;
            let jump_false = self.emit_jump(Instr::JumpIfFalse);
            self.compile_expr(&call.args[1].expr)?;
            let jump_end = self.emit_jump(Instr::Jump);
            self.patch_jump(jump_false, self.instructions.len());
            self.compile_expr(&call.args[2].expr)?;
            self.patch_jump(jump_end, self.instructions.len());
            return Ok(());
        }

        // Function calls
        for arg in &call.args {
            self.compile_expr(&arg.expr)?;
        }
        let builtin_id = match name {
            "size" => 0,
            "int" => 1,
            "double" => 2,
            "uint" => 3,
            "string" => 4,
            "max" => 5,
            "@not_strictly_false" => 6,
            _ => return Err(format!("unsupported function: {}", name)),
        };
        let argc = call.args.len() as u16;
        self.emit(Instr::Call(builtin_id, argc));
        Ok(())
    }

    fn compile_select(&mut self, sel: &crate::common::ast::SelectExpr) -> Result<(), String> {
        let field_idx = self.add_string_const(&sel.field);
        match &sel.operand.expr {
            Expr::Ident(name) => {
                if let Some(binding) = self.lookup_comp_var(name) {
                    match binding {
                        CompBinding::Iter => {
                            if sel.test {
                                self.emit(Instr::LoadVarHasField(IDX_ITER_ELEM, field_idx));
                            } else {
                                self.emit(Instr::LoadVarSelect(IDX_ITER_ELEM, field_idx));
                            }
                        }
                        CompBinding::Accu => {
                            if sel.test {
                                self.emit(Instr::LoadVarHasField(IDX_ACCU, field_idx));
                            } else {
                                self.emit(Instr::LoadVarSelect(IDX_ACCU, field_idx));
                            }
                        }
                    }
                } else {
                    let var_idx = self.ensure_var(name);
                    if sel.test {
                        self.emit(Instr::LoadVarHasField(var_idx, field_idx));
                    } else {
                        self.emit(Instr::LoadVarSelect(var_idx, field_idx));
                    }
                }
            }
            _ => {
                self.compile_expr(&sel.operand.expr)?;
                if sel.test {
                    self.emit(Instr::HasField(field_idx));
                } else {
                    self.emit(Instr::Select(field_idx));
                }
            }
        }
        Ok(())
    }

    fn compile_comprehension(
        &mut self,
        comp: &ComprehensionExpr,
    ) -> Result<(), String> {
        self.compile_expr(&comp.iter_range.expr)?;
        self.emit(Instr::IterInit);

        // Accumulator
        if let Expr::Literal(lit) = &comp.accu_init.expr {
            let v = lit_to_value(lit);
            let idx = self.add_const(v);
            self.emit(Instr::AccuPush(idx));
        } else {
            self.compile_expr(&comp.accu_init.expr)?;
            self.emit(Instr::AccuPush(0xFFFF));
        }

        // Loop
        let loop_start = self.instructions.len();
        let iter_next_idx = self.emit_jump(Instr::IterNext);

        let mut scope = HashMap::new();
        scope.insert(comp.iter_var.clone(), CompBinding::Iter);
        scope.insert(comp.accu_var.clone(), CompBinding::Accu);
        self.comp_scopes.push(scope);

        // Condition
        self.compile_expr(&comp.loop_cond.expr)?;
        let cond_jump = self.emit_jump(Instr::JumpIfFalseKeep);
        self.emit(Instr::Pop);

        // Loop step
        self.compile_expr(&comp.loop_step.expr)?;
        self.emit(Instr::AccuSet);

        self.emit(Instr::Jump(0));
        let jump_back_idx = self.instructions.len() - 1;
        self.patch_jump(jump_back_idx, loop_start as usize);

        // Exit early
        self.patch_jump(cond_jump, self.instructions.len());
        self.emit(Instr::Pop);

        // Done (iterator exhausted)
        self.patch_jump(iter_next_idx, self.instructions.len());
        self.emit(Instr::IterPop);

        // Result
        self.compile_expr(&comp.result.expr)?;

        self.comp_scopes.pop();

        Ok(())
    }

    fn lookup_comp_var(&self, name: &str) -> Option<CompBinding> {
        for scope in self.comp_scopes.iter().rev() {
            if let Some(&binding) = scope.get(name) {
                return Some(binding);
            }
        }
        None
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

fn expr_to_value(expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Literal(lit) => Some(lit_to_value(lit)),
        _ => None,
    }
}

fn fold_binary(name: &str, a: Value, b: Value) -> Option<Value> {
    let res = match name {
        operators::ADD => vm_add(a, b).ok()?,
        operators::SUBSTRACT => vm_sub(a, b).ok()?,
        operators::MULTIPLY => vm_mul(a, b).ok()?,
        operators::DIVIDE => vm_div(a, b).ok()?,
        operators::MODULO => vm_mod(a, b).ok()?,
        operators::EQUALS => Value::Bool(vm_eq(&a, &b)),
        operators::NOT_EQUALS => Value::Bool(!vm_eq(&a, &b)),
        operators::LESS => Value::Bool(vm_lt(&a, &b).ok()?),
        operators::LESS_EQUALS => Value::Bool(vm_le(&a, &b).ok()?),
        operators::GREATER => Value::Bool(vm_lt(&b, &a).ok()?),
        operators::GREATER_EQUALS => Value::Bool(vm_le(&b, &a).ok()?),
        operators::LOGICAL_AND => vm_logical_and(a, b),
        operators::LOGICAL_OR => vm_logical_or(a, b),
        operators::IN => vm_in(a, b).ok()?,
        operators::INDEX => vm_index(a, b).ok()?,
        _ => return None,
    };
    Some(res)
}

fn fold_unary(name: &str, a: Value) -> Option<Value> {
    let res = match name {
        operators::LOGICAL_NOT => Value::Bool(!try_bool(a).ok()?),
        operators::NEGATE => vm_neg(a).ok()?,
        _ => return None,
    };
    Some(res)
}
