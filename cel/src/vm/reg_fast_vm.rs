use crate::objects::Value;
use crate::vm::fast_value::FastValue;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::vm::vm::{vm_eq, vm_has_field, vm_in, vm_index, vm_le, vm_lt, vm_select, vm_size};
use crate::ExecutionError;

/// Mutable execution state for the fast-value register VM.
/// Create once, load vars once, then re-use across many `eval_reg_fast` calls.
pub struct RegFastState {
    pub(super) regs: [FastValue; 16],
}

impl RegFastState {
    pub fn new() -> Self {
        Self {
            regs: [FastValue::null(); 16],
        }
    }

    /// Load variables into registers 0..N as FastValues.
    pub fn set_vars(&mut self, vars: &[Value]) {
        for (i, v) in vars.iter().enumerate() {
            if i < 16 {
                self.regs[i] = FastValue::from_value(v.clone());
            }
        }
    }
}

impl Default for RegFastState {
    fn default() -> Self {
        Self::new()
    }
}

/// Evaluate a register program using FastValues.
pub fn eval_reg_fast(program: &RegProgram, state: &mut RegFastState) -> Result<Value, ExecutionError> {
    eval_reg_fast_loop(program, state)
}

fn eval_reg_fast_loop(program: &RegProgram, state: &mut RegFastState) -> Result<Value, ExecutionError> {
    let regs = &mut state.regs;
    let mut pc = 0usize;
    let code = &program.instructions;

    loop {
        let instr = unsafe { code.get_unchecked(pc) };
        pc += 1;

        match instr {
            RegInstr::Halt(r) => return Ok(regs[*r as usize].to_value()),
            RegInstr::Move(dst, src) => regs[*dst as usize] = regs[*src as usize],

            RegInstr::LoadConst(dst, idx) => {
                regs[*dst as usize] = FastValue::from_value(program.constants[*idx as usize].clone());
            }
            RegInstr::LoadConstInt(dst, v) => {
                regs[*dst as usize] = FastValue::from_int(*v);
            }
            RegInstr::LoadConstBool(dst, v) => {
                regs[*dst as usize] = FastValue::from_bool(*v);
            }

            RegInstr::AddInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_add_int(regs[*a as usize], regs[*b as usize])?;
            }
            RegInstr::SubInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_sub_int(regs[*a as usize], regs[*b as usize])?;
            }
            RegInstr::MulInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_mul_int(regs[*a as usize], regs[*b as usize])?;
            }
            RegInstr::DivInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_div_int(regs[*a as usize], regs[*b as usize])?;
            }
            RegInstr::ModInt(dst, a, b) => {
                regs[*dst as usize] =
                    fast_mod_int(regs[*a as usize], regs[*b as usize])?;
            }
            RegInstr::NegInt(dst, src) => {
                regs[*dst as usize] = fast_neg_int(regs[*src as usize])?;
            }

            // Type-specialized comparisons ─── the money makers
            RegInstr::EqIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_eq_int_const(regs[*src as usize], *val);
            }
            RegInstr::NeIntConst(dst, src, val) => {
                regs[*dst as usize] = FastValue::from_bool(!fast_eq_int_const_bool(regs[*src as usize], *val));
            }
            RegInstr::LtIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_lt_int_const(regs[*src as usize], *val);
            }
            RegInstr::LeIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_le_int_const(regs[*src as usize], *val);
            }
            RegInstr::GtIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_gt_int_const(regs[*src as usize], *val);
            }
            RegInstr::GeIntConst(dst, src, val) => {
                regs[*dst as usize] = fast_ge_int_const(regs[*src as usize], *val);
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
                regs[*dst as usize] = fast_eq_str_const(regs[*src as usize], expected);
            }

            RegInstr::InIntSet(dst, src, const_idx) => {
                let set = &program.constants[*const_idx as usize];
                regs[*dst as usize] = fast_in_int_set(regs[*src as usize], set);
            }
            RegInstr::InStrSet(dst, src, const_idx) => {
                let set = &program.constants[*const_idx as usize];
                regs[*dst as usize] = fast_in_str_set(regs[*src as usize], set);
            }

            RegInstr::Eq(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(vm_eq(
                    &regs[*a as usize].to_value(),
                    &regs[*b as usize].to_value(),
                ));
            }
            RegInstr::Ne(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(!vm_eq(
                    &regs[*a as usize].to_value(),
                    &regs[*b as usize].to_value(),
                ));
            }
            RegInstr::Lt(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(vm_lt(
                    &regs[*a as usize].to_value(),
                    &regs[*b as usize].to_value(),
                )?);
            }
            RegInstr::Le(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(vm_le(
                    &regs[*a as usize].to_value(),
                    &regs[*b as usize].to_value(),
                )?);
            }
            RegInstr::Gt(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(vm_lt(
                    &regs[*b as usize].to_value(),
                    &regs[*a as usize].to_value(),
                )?);
            }
            RegInstr::Ge(dst, a, b) => {
                regs[*dst as usize] = FastValue::from_bool(vm_le(
                    &regs[*b as usize].to_value(),
                    &regs[*a as usize].to_value(),
                )?);
            }

            RegInstr::Not(dst, src) => {
                let b = try_bool_fast(regs[*src as usize])?;
                regs[*dst as usize] = FastValue::from_bool(!b);
            }
            RegInstr::And(dst, a, b) => {
                regs[*dst as usize] = vm_logical_and_fast(regs[*a as usize], regs[*b as usize]);
            }
            RegInstr::Or(dst, a, b) => {
                regs[*dst as usize] = vm_logical_or_fast(regs[*a as usize], regs[*b as usize]);
            }

            RegInstr::Jump(offset) => {
                pc = ((pc as isize) + *offset as isize) as usize;
            }
            RegInstr::JumpIfFalse(r, offset) => {
                if !try_bool_fast(regs[*r as usize])? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }
            RegInstr::JumpIfTrue(r, offset) => {
                if try_bool_fast(regs[*r as usize])? {
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
                regs[*dst as usize] =
                    FastValue::from_value(vm_select(&regs[*obj as usize].to_value(), field)?);
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
                regs[*dst as usize] =
                    FastValue::from_bool(vm_has_field(&regs[*obj as usize].to_value(), field));
            }

            RegInstr::Index(dst, obj, idx) => {
                regs[*dst as usize] = FastValue::from_value(vm_index(
                    regs[*obj as usize].to_value(),
                    regs[*idx as usize].to_value(),
                )?);
            }
            RegInstr::In(dst, val, container) => {
                regs[*dst as usize] = FastValue::from_value(vm_in(
                    regs[*val as usize].to_value(),
                    regs[*container as usize].to_value(),
                )?);
            }

            RegInstr::Size(dst, src) => {
                regs[*dst as usize] =
                    FastValue::from_value(vm_size(regs[*src as usize].to_value())?);
            }
        }
    }
}

/* ---------- Fast helpers on FastValue ---------- */

#[inline(always)]
pub(super) fn fast_eq_int_const(v: FastValue, expected: i64) -> FastValue {
    if v.is_int() {
        FastValue::from_bool(v.as_int().unwrap() == expected)
    } else {
        FastValue::from_bool(vm_eq(&v.to_value(), &Value::Int(expected)))
    }
}

#[inline(always)]
fn fast_eq_int_const_bool(v: FastValue, expected: i64) -> bool {
    if v.is_int() {
        v.as_int().unwrap() == expected
    } else {
        vm_eq(&v.to_value(), &Value::Int(expected))
    }
}

#[inline(always)]
pub(super) fn fast_lt_int_const(v: FastValue, expected: i64) -> FastValue {
    if v.is_int() {
        FastValue::from_bool(v.as_int().unwrap() < expected)
    } else {
        FastValue::from_bool(vm_lt(&v.to_value(), &Value::Int(expected)).unwrap_or(false))
    }
}

#[inline(always)]
pub(super) fn fast_le_int_const(v: FastValue, expected: i64) -> FastValue {
    if v.is_int() {
        FastValue::from_bool(v.as_int().unwrap() <= expected)
    } else {
        FastValue::from_bool(vm_le(&v.to_value(), &Value::Int(expected)).unwrap_or(false))
    }
}

#[inline(always)]
pub(super) fn fast_gt_int_const(v: FastValue, expected: i64) -> FastValue {
    if v.is_int() {
        FastValue::from_bool(v.as_int().unwrap() > expected)
    } else {
        FastValue::from_bool(vm_lt(&Value::Int(expected), &v.to_value()).unwrap_or(false))
    }
}

#[inline(always)]
pub(super) fn fast_ge_int_const(v: FastValue, expected: i64) -> FastValue {
    if v.is_int() {
        FastValue::from_bool(v.as_int().unwrap() >= expected)
    } else {
        FastValue::from_bool(vm_le(&Value::Int(expected), &v.to_value()).unwrap_or(false))
    }
}

#[inline(always)]
fn fast_eq_str_const(v: FastValue, expected: &str) -> FastValue {
    FastValue::from_bool(v.eq_str(expected))
}

#[inline(always)]
fn fast_add_int(a: FastValue, b: FastValue) -> Result<FastValue, ExecutionError> {
    if a.is_int() && b.is_int() {
        Ok(FastValue::from_int(a.as_int().unwrap().wrapping_add(b.as_int().unwrap())))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_add(
            a.to_value(),
            b.to_value(),
        )?))
    }
}

#[inline(always)]
fn fast_sub_int(a: FastValue, b: FastValue) -> Result<FastValue, ExecutionError> {
    if a.is_int() && b.is_int() {
        Ok(FastValue::from_int(a.as_int().unwrap().wrapping_sub(b.as_int().unwrap())))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_sub(
            a.to_value(),
            b.to_value(),
        )?))
    }
}

#[inline(always)]
fn fast_mul_int(a: FastValue, b: FastValue) -> Result<FastValue, ExecutionError> {
    if a.is_int() && b.is_int() {
        Ok(FastValue::from_int(a.as_int().unwrap().wrapping_mul(b.as_int().unwrap())))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_mul(
            a.to_value(),
            b.to_value(),
        )?))
    }
}

#[inline(always)]
fn fast_div_int(a: FastValue, b: FastValue) -> Result<FastValue, ExecutionError> {
    if a.is_int() && b.is_int() {
        let bv = b.as_int().unwrap();
        if bv == 0 {
            return Err(ExecutionError::DivisionByZero(a.to_value()));
        }
        Ok(FastValue::from_int(a.as_int().unwrap() / bv))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_div(
            a.to_value(),
            b.to_value(),
        )?))
    }
}

#[inline(always)]
fn fast_mod_int(a: FastValue, b: FastValue) -> Result<FastValue, ExecutionError> {
    if a.is_int() && b.is_int() {
        let bv = b.as_int().unwrap();
        if bv == 0 {
            return Err(ExecutionError::RemainderByZero(a.to_value()));
        }
        Ok(FastValue::from_int(a.as_int().unwrap() % bv))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_mod(
            a.to_value(),
            b.to_value(),
        )?))
    }
}

#[inline(always)]
fn fast_neg_int(v: FastValue) -> Result<FastValue, ExecutionError> {
    if v.is_int() {
        Ok(FastValue::from_int(-v.as_int().unwrap()))
    } else {
        Ok(FastValue::from_value(crate::vm::vm::vm_neg(v.to_value())?))
    }
}

#[inline(always)]
fn try_bool_fast(v: FastValue) -> Result<bool, ExecutionError> {
    if v.is_bool() {
        Ok(v.as_bool().unwrap())
    } else {
        crate::vm::vm::try_bool(v.to_value())
    }
}

#[inline(always)]
pub(super) fn vm_logical_and_fast(lhs: FastValue, rhs: FastValue) -> FastValue {
    match (lhs.is_bool(), rhs.is_bool()) {
        (true, true) => FastValue::from_bool(lhs.as_bool().unwrap() && rhs.as_bool().unwrap()),
        _ => {
            let l = lhs.to_value();
            let r = rhs.to_value();
            match (&l, &r) {
                (Value::Bool(false), _) | (_, Value::Bool(false)) => FastValue::from_bool(false),
                (Value::Bool(true), Value::Bool(true)) => FastValue::from_bool(true),
                (Value::Bool(true), other) | (other, Value::Bool(true)) => FastValue::from_value(other.clone()),
                (a, _) => FastValue::from_value(a.clone()),
            }
        }
    }
}

#[inline(always)]
fn vm_logical_or_fast(lhs: FastValue, rhs: FastValue) -> FastValue {
    match (lhs.is_bool(), rhs.is_bool()) {
        (true, true) => FastValue::from_bool(lhs.as_bool().unwrap() || rhs.as_bool().unwrap()),
        _ => {
            let l = lhs.to_value();
            let r = rhs.to_value();
            match (&l, &r) {
                (Value::Bool(true), _) | (_, Value::Bool(true)) => FastValue::from_bool(true),
                (Value::Bool(false), Value::Bool(false)) => FastValue::from_bool(false),
                (Value::Bool(false), other) | (other, Value::Bool(false)) => FastValue::from_value(other.clone()),
                (a, _) => FastValue::from_value(a.clone()),
            }
        }
    }
}

#[inline(always)]
fn fast_in_int_set(v: FastValue, set: &Value) -> FastValue {
    let needle = match v.as_int() {
        Some(i) => i,
        None => {
            return FastValue::from_bool(
                vm_in(v.to_value(), set.clone())
                    .map(|r| matches!(r, Value::Bool(true)))
                    .unwrap_or(false),
            )
        }
    };
    match set {
        Value::List(list) => {
            for item in list.iter() {
                if let Value::Int(i) = item {
                    if *i == needle {
                        return FastValue::from_bool(true);
                    }
                }
            }
            FastValue::from_bool(false)
        }
        _ => FastValue::from_bool(
            vm_in(v.to_value(), set.clone())
                .map(|r| matches!(r, Value::Bool(true)))
                .unwrap_or(false),
        ),
    }
}

#[inline(always)]
fn fast_in_str_set(v: FastValue, set: &Value) -> FastValue {
    let needle = match v.as_ptr() {
        Some(ptr) => unsafe {
            match &(*ptr).value {
                Value::String(s) => s.as_str(),
                _ => {
                    return FastValue::from_bool(
                        vm_in(v.to_value(), set.clone())
                            .map(|r| matches!(r, Value::Bool(true)))
                            .unwrap_or(false),
                    )
                }
            }
        },
        None => {
            return FastValue::from_bool(
                vm_in(v.to_value(), set.clone())
                    .map(|r| matches!(r, Value::Bool(true)))
                    .unwrap_or(false),
            )
        }
    };
    match set {
        Value::List(list) => {
            for item in list.iter() {
                if let Value::String(s) = item {
                    if s.as_str() == needle {
                        return FastValue::from_bool(true);
                    }
                }
            }
            FastValue::from_bool(false)
        }
        _ => FastValue::from_bool(
            vm_in(v.to_value(), set.clone())
                .map(|r| matches!(r, Value::Bool(true)))
                .unwrap_or(false),
        ),
    }
}
