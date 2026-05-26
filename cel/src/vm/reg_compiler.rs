use crate::common::ast::{ComprehensionExpr, EntryExpr, Expr, LiteralValue, SelectExpr};
use crate::common::ast::operators;
use crate::objects::Value;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::Expression;
use std::collections::HashMap;

const REG_COUNT: u8 = 8;

pub fn compile_reg(expr: &Expression) -> Result<RegProgram, String> {
    let mut c = RegCompiler::new();
    let result_reg = c.compile_expr(&expr.expr)?;
    c.emit(RegInstr::Halt(result_reg));
    Ok(RegProgram {
        constants: c.constants,
        var_names: c.var_names.into_iter().collect(),
        instructions: c.instructions,
    })
}

struct RegCompiler {
    constants: Vec<Value>,
    const_map: HashMap<String, u16>,
    var_names: Vec<String>,
    var_map: HashMap<String, u16>,
    instructions: Vec<RegInstr>,
    free_regs: u8,         // bitmask of free registers
    next_spill: Vec<u8>,   // stack of spilled registers (simplified)
}

impl RegCompiler {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            const_map: HashMap::new(),
            var_names: Vec::new(),
            var_map: HashMap::new(),
            instructions: Vec::new(),
            free_regs: 0xFF, // all 8 registers free
            next_spill: Vec::new(),
        }
    }

    fn alloc_reg(&mut self) -> u8 {
        if self.free_regs == 0 {
            panic!("out of registers — need spill (not implemented for register VM)");
        }
        let reg = self.free_regs.trailing_zeros() as u8;
        self.free_regs &= !(1 << reg);
        reg
    }

    fn free_reg(&mut self, reg: u8) {
        self.free_regs |= 1 << reg;
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

    fn emit(&mut self, instr: RegInstr) -> usize {
        self.instructions.push(instr);
        self.instructions.len() - 1
    }

    fn emit_jump(&mut self, op: fn(i16) -> RegInstr) -> usize {
        let idx = self.emit(op(0));
        idx
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

    fn compile_expr(&mut self, expr: &Expr) -> Result<u8, String> {
        match expr {
            Expr::Literal(lit) => {
                let v = lit_to_value(lit);
                let idx = self.add_const(v);
                let reg = self.alloc_reg();
                self.emit(RegInstr::LoadConst(reg, idx));
                Ok(reg)
            }
            Expr::Ident(name) => {
                let idx = self.ensure_var(name);
                let reg = self.alloc_reg();
                self.emit(RegInstr::LoadVar(reg, idx));
                Ok(reg)
            }
            Expr::Call(call) => self.compile_call(call),
            Expr::Select(sel) => self.compile_select(sel),
            Expr::List(list) => {
                // Build list via stack-like approach: not optimized for reg VM
                Err("list literals not supported in reg VM".into())
            }
            Expr::Map(map) => {
                Err("map literals not supported in reg VM".into())
            }
            Expr::Comprehension(comp) => {
                Err("comprehensions not supported in reg VM".into())
            }
            _ => Err(format!("unsupported expr: {:?}", std::mem::discriminant(expr))),
        }
    }

    fn compile_call(&mut self, call: &crate::common::ast::CallExpr) -> Result<u8, String> {
        let name = call.func_name.as_str();

        // Binary ops
        if call.args.len() == 2 {
            // Constant folding
            if let (Some(a), Some(b)) = (expr_to_value(&call.args[0].expr), expr_to_value(&call.args[1].expr)) {
                if let Some(v) = fold_binary(name, a, b) {
                    let idx = self.add_const(v);
                    let reg = self.alloc_reg();
                    self.emit(RegInstr::LoadConst(reg, idx));
                    return Ok(reg);
                }
            }
            let r1 = self.compile_expr(&call.args[0].expr)?;
            let r2 = self.compile_expr(&call.args[1].expr)?;
            let dst = self.alloc_reg();
            let instr = match name {
                operators::ADD => RegInstr::Add(dst, r1, r2),
                operators::SUBSTRACT => RegInstr::Sub(dst, r1, r2),
                operators::MULTIPLY => RegInstr::Mul(dst, r1, r2),
                operators::DIVIDE => RegInstr::Div(dst, r1, r2),
                operators::MODULO => RegInstr::Mod(dst, r1, r2),
                operators::EQUALS => RegInstr::Eq(dst, r1, r2),
                operators::NOT_EQUALS => RegInstr::Ne(dst, r1, r2),
                operators::LESS => RegInstr::Lt(dst, r1, r2),
                operators::LESS_EQUALS => RegInstr::Le(dst, r1, r2),
                operators::GREATER => RegInstr::Gt(dst, r1, r2),
                operators::GREATER_EQUALS => RegInstr::Ge(dst, r1, r2),
                operators::LOGICAL_AND => RegInstr::LogicalAnd(dst, r1, r2),
                operators::LOGICAL_OR => RegInstr::LogicalOr(dst, r1, r2),
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
            // Unary folding
            if let Some(a) = expr_to_value(&call.args[0].expr) {
                if let Some(v) = fold_unary(name, a) {
                    let idx = self.add_const(v);
                    let reg = self.alloc_reg();
                    self.emit(RegInstr::LoadConst(reg, idx));
                    return Ok(reg);
                }
            }
            let r1 = self.compile_expr(&call.args[0].expr)?;
            let dst = self.alloc_reg();
            let instr = match name {
                operators::LOGICAL_NOT => RegInstr::Not(dst, r1),
                operators::NEGATE => RegInstr::Neg(dst, r1),
                "size" => RegInstr::Size(dst, r1),
                "int" | "double" | "uint" | "string" | "max" | "@not_strictly_false" => {
                    // builtins — map to Call
                    let builtin_id = match name {
                        "size" => 0u16,
                        "int" => 1,
                        "double" => 2,
                        "uint" => 3,
                        "string" => 4,
                        "max" => 5,
                        "@not_strictly_false" => 6,
                        _ => unreachable!(),
                    };
                    self.emit(RegInstr::Call(dst, builtin_id, 1));
                    // Need to indicate arg reg somehow... reg VM Call needs arg regs
                    // Simplification: for single-arg builtins, assume arg is in r1
                    // Actually we need a different encoding. For now, use stack-based call.
                    self.free_reg(dst);
                    self.free_reg(r1);
                    return Err("builtin calls in reg VM need special encoding".into());
                }
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

    fn compile_select(&mut self, sel: &SelectExpr) -> Result<u8, String> {
        let field_idx = self.add_string_const(&sel.field);
        let obj_reg = self.compile_expr(&sel.operand.expr)?;
        let dst = self.alloc_reg();
        if sel.test {
            self.emit(RegInstr::HasField(dst, obj_reg, field_idx));
        } else {
            self.emit(RegInstr::Select(dst, obj_reg, field_idx));
        }
        self.free_reg(obj_reg);
        Ok(dst)
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
    use crate::vm::vm::{vm_add, vm_div, vm_eq, vm_le, vm_lt, vm_mod, vm_mul, vm_sub};
    match name {
        operators::ADD => vm_add(a, b).ok(),
        operators::SUBSTRACT => vm_sub(a, b).ok(),
        operators::MULTIPLY => vm_mul(a, b).ok(),
        operators::DIVIDE => vm_div(a, b).ok(),
        operators::MODULO => vm_mod(a, b).ok(),
        operators::EQUALS => Some(Value::Bool(vm_eq(&a, &b))),
        operators::NOT_EQUALS => Some(Value::Bool(!vm_eq(&a, &b))),
        operators::LESS => Some(Value::Bool(vm_lt(&a, &b).ok()?)),
        operators::LESS_EQUALS => Some(Value::Bool(vm_le(&a, &b).ok()?)),
        operators::GREATER => Some(Value::Bool(vm_lt(&b, &a).ok()?)),
        operators::GREATER_EQUALS => Some(Value::Bool(vm_le(&b, &a).ok()?)),
        _ => None,
    }
}

fn fold_unary(name: &str, a: Value) -> Option<Value> {
    use crate::vm::vm::try_bool;
    match name {
        operators::LOGICAL_NOT => Some(Value::Bool(!try_bool(a).ok()?)),
        operators::NEGATE => match a {
            Value::Int(i) => Some(Value::Int(-i)),
            Value::Float(f) => Some(Value::Float(-f)),
            _ => None,
        },
        _ => None,
    }
}
