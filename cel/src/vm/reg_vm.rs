use crate::objects::Value;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::vm::vm::{
    try_bool, vm_add, vm_div, vm_eq, vm_has_field, vm_in, vm_index, vm_le, vm_lt, vm_mod, vm_mul,
    vm_neg, vm_select, vm_size, vm_sub,
};
use crate::ExecutionError;
use std::sync::Arc;

/// Mutable execution state for the register VM.
/// Create once, load vars once, then re-use across many `eval_reg` calls.
pub struct RegState {
    regs: [Value; 16],
}

impl RegState {
    pub fn new() -> Self {
        Self {
            regs: std::array::from_fn(|_| Value::Null),
        }
    }

    /// Load variables into registers 0..N.
    pub fn set_vars(&mut self, vars: &[Value]) {
        for (i, v) in vars.iter().enumerate() {
            if i < 16 {
                self.regs[i] = v.clone();
            }
        }
    }
}

impl Default for RegState {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate a register program using a pre-loaded `RegState`.
pub fn eval_reg(program: &RegProgram, state: &mut RegState) -> Result<Value, ExecutionError> {
    let regs = &mut state.regs;
    let mut pc = 0usize;
    let code = &program.instructions;

    loop {
        // Bounds check eliminated by trusting the compiler produced valid jumps.
        let instr = unsafe { code.get_unchecked(pc) };
        pc += 1;

        match instr {
            RegInstr::Halt(r) => return Ok(regs[*r as usize].clone()),
            RegInstr::Move(dst, src) => regs[*dst as usize] = regs[*src as usize].clone(),

            RegInstr::LoadConst(dst, idx) => {
                regs[*dst as usize] = program.constants[*idx as usize].clone();
            }
            RegInstr::LoadConstInt(dst, v) => {
                regs[*dst as usize] = Value::Int(*v);
            }
            RegInstr::LoadConstBool(dst, v) => {
                regs[*dst as usize] = Value::Bool(*v);
            }

            // Integer arithmetic (fast path — assumes Int, falls back to generic)
            RegInstr::AddInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_add_int(&regs[*a as usize], &regs[*b as usize])?;
            }
            RegInstr::SubInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_sub_int(&regs[*a as usize], &regs[*b as usize])?;
            }
            RegInstr::MulInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_mul_int(&regs[*a as usize], &regs[*b as usize])?;
            }
            RegInstr::DivInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_div_int(&regs[*a as usize], &regs[*b as usize])?;
            }
            RegInstr::ModInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_mod_int(&regs[*a as usize], &regs[*b as usize])?;
            }
            RegInstr::NegInt(dst, src) => {
                regs[*dst as usize] = fast_neg_int(&regs[*src as usize])?;
            }

            // Type-specialized comparisons ─── the money makers
            RegInstr::EqIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_eq_int_const(&regs[*src as usize], *val);
            }
            RegInstr::NeIntConst(dst, src, val) => {
                regs[*dst as usize] = Value::Bool(!fast_eq_int_const_bool(&regs[*src as usize], *val));
            }
            RegInstr::LtIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_lt_int_const(&regs[*src as usize], *val);
            }
            RegInstr::LeIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_le_int_const(&regs[*src as usize], *val);
            }
            RegInstr::GtIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_gt_int_const(&regs[*src as usize], *val);
            }
            RegInstr::GeIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_ge_int_const(&regs[*src as usize], *val);
            }

            RegInstr::EqStrConst(dst, src, const_idx) => {
                let expected = match &program.constants[*const_idx as usize] {
                    Value::String(s) => s.as_str(),
                    _ => {
                        return Err(ExecutionError::InternalError(
                            "EqStrConst with non-string const".into(),
                        ))
                    }
                };
                let actual = match &regs[*src as usize] {
                    Value::String(s) => s.as_str(),
                    other => {
                        regs[*dst as usize] = Value::Bool(vm_eq(other, &Value::String(Arc::new(expected.to_string()))));
                        continue;
                    }
                };
                regs[*dst as usize] = Value::Bool(actual == expected);
            }

            RegInstr::InIntSet(dst, src, const_idx) => {
                let set = &program.constants[*const_idx as usize];
                let needle = match &regs[*src as usize] {
                    Value::Int(i) => *i,
                    other => {
                        regs[*dst as usize] = Value::Bool(vm_in(other.clone(), set.clone()).map(|r| matches!(r, Value::Bool(true))).unwrap_or(false));
                        continue;
                    }
                };
                regs[*dst as usize] = match set {
                    Value::List(list) => {
                        let found = list.iter().any(|item| matches!(item, Value::Int(i) if *i == needle));
                        Value::Bool(found)
                    }
                    _ => Value::Bool(vm_in(Value::Int(needle), set.clone()).map(|r| matches!(r, Value::Bool(true))).unwrap_or(false)),
                };
            }
            RegInstr::InStrSet(dst, src, const_idx) => {
                let set = &program.constants[*const_idx as usize];
                let needle = match &regs[*src as usize] {
                    Value::String(s) => s.as_str(),
                    other => {
                        regs[*dst as usize] = Value::Bool(vm_in(other.clone(), set.clone()).map(|r| matches!(r, Value::Bool(true))).unwrap_or(false));
                        continue;
                    }
                };
                regs[*dst as usize] = match set {
                    Value::List(list) => {
                        let found = list.iter().any(|item| matches!(item, Value::String(s) if s.as_str() == needle));
                        Value::Bool(found)
                    }
                    _ => Value::Bool(vm_in(Value::String(Arc::new(needle.to_string())), set.clone()).map(|r| matches!(r, Value::Bool(true))).unwrap_or(false)),
                };
            }

            RegInstr::Eq(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(vm_eq(&regs[*a as usize], &regs[*b as usize]));
            }
            RegInstr::Ne(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(!vm_eq(&regs[*a as usize], &regs[*b as usize]));
            }
            RegInstr::Lt(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(vm_lt(&regs[*a as usize], &regs[*b as usize])?);
            }
            RegInstr::Le(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(vm_le(&regs[*a as usize], &regs[*b as usize])?);
            }
            RegInstr::Gt(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(vm_lt(&regs[*b as usize], &regs[*a as usize])?);
            }
            RegInstr::Ge(dst, a, b) => {
                regs[*dst as usize] = Value::Bool(vm_le(&regs[*b as usize], &regs[*a as usize])?);
            }

            RegInstr::Not(dst, src) => {
                regs[*dst as usize] = Value::Bool(!try_bool(regs[*src as usize].clone())?);
            }
            RegInstr::And(dst, a, b) => {
                regs[*dst as usize] = vm_logical_and(regs[*a as usize].clone(), regs[*b as usize].clone());
            }
            RegInstr::Or(dst, a, b) => {
                regs[*dst as usize] = vm_logical_or(regs[*a as usize].clone(), regs[*b as usize].clone());
            }

            RegInstr::Jump(offset) => {
                pc = ((pc as isize) + *offset as isize) as usize;
            }
            RegInstr::JumpIfFalse(r, offset) => {
                if !try_bool(regs[*r as usize].clone())? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }
            RegInstr::JumpIfTrue(r, offset) => {
                if try_bool(regs[*r as usize].clone())? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }

            RegInstr::Select(dst, obj, field_idx) => {
                let field = match &program.constants[*field_idx as usize] {
                    Value::String(s) => s,
                    _ => {
                        return Err(ExecutionError::InternalError(
                            "select non-string".into(),
                        ))
                    }
                };
                regs[*dst as usize] = vm_select(&regs[*obj as usize], field)?;
            }
            RegInstr::HasField(dst, obj, field_idx) => {
                let field = match &program.constants[*field_idx as usize] {
                    Value::String(s) => s,
                    _ => {
                        return Err(ExecutionError::InternalError(
                            "has_field non-string".into(),
                        ))
                    }
                };
                regs[*dst as usize] = Value::Bool(vm_has_field(&regs[*obj as usize], field));
            }

            RegInstr::Index(dst, obj, idx) => {
                regs[*dst as usize] =
                    vm_index(regs[*obj as usize].clone(), regs[*idx as usize].clone())?;
            }
            RegInstr::In(dst, val, container) => {
                regs[*dst as usize] =
                    vm_in(regs[*val as usize].clone(), regs[*container as usize].clone())?;
            }

            RegInstr::Size(dst, src) => {
                regs[*dst as usize] = vm_size(regs[*src as usize].clone())?;
            }
        }
    }
}

/* ---------- Fast helpers ---------- */

#[inline(always)]
fn fast_eq_int_const(v: &Value, expected: i64) -> Value {
    match v {
        Value::Int(i) => Value::Bool(*i == expected),
        other => Value::Bool(vm_eq(other, &Value::Int(expected))),
    }
}

#[inline(always)]
fn fast_eq_int_const_bool(v: &Value, expected: i64) -> bool {
    match v {
        Value::Int(i) => *i == expected,
        other => vm_eq(other, &Value::Int(expected)),
    }
}

#[inline(always)]
fn fast_lt_int_const(v: &Value, expected: i64) -> Value {
    match v {
        Value::Int(i) => Value::Bool(*i < expected),
        other => Value::Bool(vm_lt(other, &Value::Int(expected)).unwrap_or(false)),
    }
}

#[inline(always)]
fn fast_le_int_const(v: &Value, expected: i64) -> Value {
    match v {
        Value::Int(i) => Value::Bool(*i <= expected),
        other => Value::Bool(vm_le(other, &Value::Int(expected)).unwrap_or(false)),
    }
}

#[inline(always)]
fn fast_gt_int_const(v: &Value, expected: i64) -> Value {
    match v {
        Value::Int(i) => Value::Bool(*i > expected),
        other => Value::Bool(vm_lt(&Value::Int(expected), other).unwrap_or(false)),
    }
}

#[inline(always)]
fn fast_ge_int_const(v: &Value, expected: i64) -> Value {
    match v {
        Value::Int(i) => Value::Bool(*i >= expected),
        other => Value::Bool(vm_le(&Value::Int(expected), other).unwrap_or(false)),
    }
}

#[inline(always)]
fn fast_eq_str_const(v: &Value, expected: &std::sync::Arc<String>) -> Value {
    match v {
        Value::String(s) => Value::Bool(s.as_str() == expected.as_str()),
        other => Value::Bool(vm_eq(other, &Value::String(expected.clone()))),
    }
}

#[inline(always)]
fn fast_add_int(a: &Value, b: &Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(av), Value::Int(bv)) => Ok(Value::Int(av.wrapping_add(*bv))),
        _ => vm_add(a.clone(), b.clone()),
    }
}

#[inline(always)]
fn fast_sub_int(a: &Value, b: &Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(av), Value::Int(bv)) => Ok(Value::Int(av.wrapping_sub(*bv))),
        _ => vm_sub(a.clone(), b.clone()),
    }
}

#[inline(always)]
fn fast_mul_int(a: &Value, b: &Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(av), Value::Int(bv)) => Ok(Value::Int(av.wrapping_mul(*bv))),
        _ => vm_mul(a.clone(), b.clone()),
    }
}

#[inline(always)]
fn fast_div_int(a: &Value, b: &Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(av), Value::Int(bv)) => {
            if *bv == 0 {
                return Err(ExecutionError::DivisionByZero(Value::Int(*av)));
            }
            Ok(Value::Int(av / bv))
        }
        _ => vm_div(a.clone(), b.clone()),
    }
}

#[inline(always)]
fn fast_mod_int(a: &Value, b: &Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(av), Value::Int(bv)) => {
            if *bv == 0 {
                return Err(ExecutionError::RemainderByZero(Value::Int(*av)));
            }
            Ok(Value::Int(av % bv))
        }
        _ => vm_mod(a.clone(), b.clone()),
    }
}

#[inline(always)]
fn fast_neg_int(v: &Value) -> Result<Value, ExecutionError> {
    match v {
        Value::Int(i) => Ok(Value::Int(-*i)),
        _ => vm_neg(v.clone()),
    }
}

fn vm_logical_and(lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Bool(false), _) | (_, Value::Bool(false)) => Value::Bool(false),
        (Value::Bool(true), Value::Bool(true)) => Value::Bool(true),
        (Value::Bool(true), other) | (other, Value::Bool(true)) => other,
        (a, _) => a,
    }
}

fn vm_logical_or(lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Bool(true), _) | (_, Value::Bool(true)) => Value::Bool(true),
        (Value::Bool(false), Value::Bool(false)) => Value::Bool(false),
        (Value::Bool(false), other) | (other, Value::Bool(false)) => other,
        (a, _) => a,
    }
}


