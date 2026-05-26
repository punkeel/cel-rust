use crate::context::Context;
use crate::objects::{AsKeyRef, Key, KeyRef, Map, Value};
use crate::vm::bytecode::{Instr, Program, IDX_ACCU, IDX_ITER_ELEM};
use crate::ExecutionError;
use std::sync::Arc;

pub struct IterFrame {
    items: Arc<Vec<Value>>,
    idx: usize,
}

pub struct EvalState {
    pub stack: Vec<Value>,
    pub iter_stack: Vec<IterFrame>,
    pub vars: Vec<Value>,
    needs_bind: bool,
}

impl EvalState {
    pub fn new() -> Self {
        Self {
            stack: Vec::with_capacity(64),
            iter_stack: Vec::new(),
            vars: Vec::with_capacity(16),
            needs_bind: true,
        }
    }

    fn reset(&mut self, program: &Program) {
        self.stack.clear();
        self.iter_stack.clear();
        let need = program.var_names.len();
        if self.vars.len() != need {
            self.vars.resize_with(need, || Value::Null);
            self.needs_bind = true;
        }
    }

    fn bind_vars(&mut self, program: &Program, ctx: &Context) {
        if !self.needs_bind {
            return;
        }
        for (i, name) in program.var_names.iter().enumerate() {
            let v = ctx
                .get_variable(name)
                .and_then(|cow| Value::try_from(cow.as_ref()).ok())
                .unwrap_or(Value::Null);
            self.vars[i] = v;
        }
        self.needs_bind = false;
    }
}

impl Default for EvalState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn eval(program: &Program, ctx: &Context, state: &mut EvalState) -> Result<Value, ExecutionError> {
    state.reset(program);
    state.bind_vars(program, ctx);
    let stack = &mut state.stack;
    let vars = &mut state.vars;
    let iter_stack = &mut state.iter_stack;
    let mut pc = 0usize;
    let mut accu = Value::Null;
    let mut iter_elem = Value::Null;

    loop {
        let instr = unsafe { program.instructions.get_unchecked(pc) };
        pc += 1;
        match instr {
            Instr::Halt | Instr::Return => {
                return stack.pop().ok_or(ExecutionError::InternalError(
                    "empty stack at return".into(),
                ));
            }
            Instr::PushConst(idx) => stack.push(program.constants[*idx as usize].clone()),
            Instr::LoadVar(idx) => match *idx {
                IDX_ITER_ELEM => stack.push(iter_elem.clone()),
                IDX_ACCU => stack.push(accu.clone()),
                n => {
                    let slot = n as usize;
                    stack.push(vars[slot].clone());
                }
            },
            Instr::Pop => {
                stack.pop();
            }
            Instr::Jump(offset) => {
                pc = ((pc as isize) + *offset as isize) as usize;
            }
            Instr::JumpIfFalse(offset) => {
                if !try_bool(stack.pop().unwrap())? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }
            Instr::JumpIfTrue(offset) => {
                if try_bool(stack.pop().unwrap())? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }
            Instr::JumpIfFalseKeep(offset) => {
                let v = stack.last().unwrap();
                if !try_bool_ref(v)? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }
            Instr::JumpIfTrueKeep(offset) => {
                let v = stack.last().unwrap();
                if try_bool_ref(v)? {
                    pc = ((pc as isize) + *offset as isize) as usize;
                }
            }

            // Arithmetic
            Instr::Add => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(vm_add(a, b)?);
            }
            Instr::Sub => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(vm_sub(a, b)?);
            }
            Instr::Mul => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(vm_mul(a, b)?);
            }
            Instr::Div => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(vm_div(a, b)?);
            }
            Instr::Mod => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(vm_mod(a, b)?);
            }
            Instr::Neg => {
                let a = stack.pop().unwrap();
                stack.push(vm_neg(a)?);
            }

            // Comparison / logic
            Instr::Eq => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(vm_eq(&a, &b)));
            }
            Instr::Ne => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(!vm_eq(&a, &b)));
            }
            Instr::Lt => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(vm_lt(&a, &b)?));
            }
            Instr::Le => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(vm_le(&a, &b)?));
            }
            Instr::Gt => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(vm_lt(&b, &a)?));
            }
            Instr::Ge => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(vm_le(&b, &a)?));
            }
            Instr::LoadVarEqConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(vm_eq(a, b)));
            }
            Instr::LoadVarNeConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(!vm_eq(a, b)));
            }
            Instr::LoadVarLtConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(vm_lt(a, b)?));
            }
            Instr::LoadVarLeConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(vm_le(a, b)?));
            }
            Instr::LoadVarGtConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(vm_lt(b, a)?));
            }
            Instr::LoadVarGeConst(var_idx, const_idx) => {
                let a = &vars[*var_idx as usize];
                let b = &program.constants[*const_idx as usize];
                stack.push(Value::Bool(vm_le(b, a)?));
            }
            Instr::Not => {
                let a = stack.pop().unwrap();
                stack.push(Value::Bool(!try_bool(a)?));
            }
            Instr::In => {
                let container = stack.pop().unwrap();
                let val = stack.pop().unwrap();
                stack.push(vm_in(val, container)?);
            }
            Instr::LogicalAnd => {
                let rhs = stack.pop().unwrap();
                let lhs = stack.pop().unwrap();
                stack.push(vm_logical_and(lhs, rhs));
            }
            Instr::LogicalOr => {
                let rhs = stack.pop().unwrap();
                let lhs = stack.pop().unwrap();
                stack.push(vm_logical_or(lhs, rhs));
            }

            // Collections
            Instr::Index => {
                let idx = stack.pop().unwrap();
                let obj = stack.pop().unwrap();
                stack.push(vm_index(obj, idx)?);
            }
            Instr::BuildList(n) => {
                let n = *n as usize;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(stack.pop().unwrap());
                }
                list.reverse();
                stack.push(Value::List(Arc::new(list)));
            }
            Instr::BuildMap(n) => {
                let n = *n as usize;
                let mut map = std::collections::HashMap::with_capacity(n);
                for _ in 0..n {
                    let v = stack.pop().unwrap();
                    let k = stack.pop().unwrap();
                    let key = value_to_key(k)?;
                    map.insert(key, v);
                }
                stack.push(Value::Map(Map { map: Arc::new(map) }));
            }

            // Fields
            Instr::LoadVarSelect(var_idx, field_idx) => {
                let obj = match *var_idx {
                    IDX_ITER_ELEM => iter_elem.clone(),
                    IDX_ACCU => accu.clone(),
                    n => vars[n as usize].clone(),
                };
                let field = match &program.constants[*field_idx as usize] {
                    Value::String(s) => s,
                    _ => return Err(ExecutionError::InternalError("select non-string".into())),
                };
                stack.push(vm_select(&obj, field)?);
            }
            Instr::LoadVarHasField(var_idx, field_idx) => {
                let obj = match *var_idx {
                    IDX_ITER_ELEM => &iter_elem,
                    IDX_ACCU => &accu,
                    n => &vars[n as usize],
                };
                let field = match &program.constants[*field_idx as usize] {
                    Value::String(s) => s,
                    _ => return Err(ExecutionError::InternalError("has_field non-string".into())),
                };
                stack.push(Value::Bool(vm_has_field(obj, field)));
            }
            Instr::Select(idx) => {
                let obj = stack.pop().unwrap();
                let field = match &program.constants[*idx as usize] {
                    Value::String(s) => s,
                    _ => return Err(ExecutionError::InternalError("select non-string".into())),
                };
                stack.push(vm_select(&obj, field)?);
            }
            Instr::HasField(idx) => {
                let obj = stack.pop().unwrap();
                let field = match &program.constants[*idx as usize] {
                    Value::String(s) => s,
                    _ => return Err(ExecutionError::InternalError("has_field non-string".into())),
                };
                stack.push(Value::Bool(vm_has_field(&obj, field)));
            }

            // Calls
            Instr::Call(builtin_id, argc) => {
                let argc = *argc as usize;
                let args_start = stack.len() - argc;
                let args = &stack[args_start..];
                let result = vm_call(*builtin_id, args, ctx)?;
                stack.truncate(args_start);
                stack.push(result);
            }
            Instr::Size => {
                let a = stack.pop().unwrap();
                stack.push(vm_size(a)?);
            }

            // Comprehensions
            Instr::IterInit => {
                let val = stack.pop().unwrap();
                let items = match val {
                    Value::List(l) => l,
                    Value::Map(m) => {
                        let keys: Vec<Value> = m.map.keys().cloned().map(key_to_value).collect();
                        Arc::new(keys)
                    }
                    _ => return Err(ExecutionError::NoSuchOverload),
                };
                iter_stack.push(IterFrame { items, idx: 0 });
            }
            Instr::IterNext(offset) => {
                let frame = iter_stack.last_mut().unwrap();
                if frame.idx >= frame.items.len() {
                    pc = ((pc as isize) + *offset as isize) as usize;
                } else {
                    iter_elem = frame.items[frame.idx].clone();
                    frame.idx += 1;
                }
            }
            Instr::IterPop => {
                iter_stack.pop();
            }
            Instr::AccuPush(idx) => {
                if *idx == 0xFFFF {
                    accu = stack.pop().unwrap();
                } else {
                    accu = program.constants[*idx as usize].clone();
                }
            }
            Instr::AccuSet => {
                accu = stack.pop().unwrap();
            }
        }
    }
}

// ---- Runtime helpers --------------------------------------------------------
// ---- Runtime helpers --------------------------------------------------------

#[inline(always)]
pub(super) fn try_bool(v: Value) -> Result<bool, ExecutionError> {
    match v {
        Value::Bool(b) => Ok(b),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn try_bool_ref(v: &Value) -> Result<bool, ExecutionError> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_add(a: Value, b: Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(b))),
        (Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_add(b))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::String(a), Value::String(b)) => {
            Ok(Value::String(Arc::new(format!("{}{}", a, b))))
        }
        (Value::List(a), Value::List(b)) => {
            let mut v = Vec::with_capacity(a.len() + b.len());
            v.extend(a.iter().cloned());
            v.extend(b.iter().cloned());
            Ok(Value::List(Arc::new(v)))
        }
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_sub(a: Value, b: Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(b))),
        (Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_sub(b))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_mul(a: Value, b: Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(b))),
        (Value::UInt(a), Value::UInt(b)) => Ok(Value::UInt(a.wrapping_mul(b))),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_div(a: Value, b: Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => {
            if b == 0 {
                return Err(ExecutionError::DivisionByZero(Value::Int(a)));
            }
            Ok(Value::Int(a / b))
        }
        (Value::UInt(a), Value::UInt(b)) => {
            if b == 0 {
                return Err(ExecutionError::DivisionByZero(Value::UInt(a)));
            }
            Ok(Value::UInt(a / b))
        }
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_mod(a: Value, b: Value) -> Result<Value, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => {
            if b == 0 {
                return Err(ExecutionError::RemainderByZero(Value::Int(a)));
            }
            Ok(Value::Int(a % b))
        }
        (Value::UInt(a), Value::UInt(b)) => {
            if b == 0 {
                return Err(ExecutionError::RemainderByZero(Value::UInt(a)));
            }
            Ok(Value::UInt(a % b))
        }
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_neg(a: Value) -> Result<Value, ExecutionError> {
    match a {
        Value::Int(a) => Ok(Value::Int(-a)),
        Value::Float(a) => Ok(Value::Float(-a)),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::UInt(a), Value::UInt(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bytes(a), Value::Bytes(b)) => a == b,
        (Value::List(a), Value::List(b)) => a == b,
        (Value::Map(a), Value::Map(b)) => a.map == b.map,
        (Value::Int(a), Value::Float(b)) => *a as f64 == *b,
        (Value::Float(a), Value::Int(b)) => *a == *b as f64,
        (Value::Int(a), Value::UInt(b)) => *a >= 0 && (*a as u64) == *b,
        (Value::UInt(a), Value::Int(b)) => *b >= 0 && *a == (*b as u64),
        _ => false,
    }
}

#[inline(always)]
pub(super) fn vm_lt(a: &Value, b: &Value) -> Result<bool, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(a < b),
        (Value::UInt(a), Value::UInt(b)) => Ok(a < b),
        (Value::Float(a), Value::Float(b)) => Ok(a < b),
        (Value::String(a), Value::String(b)) => Ok(a < b),
        (Value::Bool(a), Value::Bool(b)) => {
            Ok((if *a { 1 } else { 0 }) < (if *b { 1 } else { 0 }))
        }
        (Value::Int(a), Value::Float(b)) => Ok((*a as f64) < *b),
        (Value::Float(a), Value::Int(b)) => Ok(*a < *b as f64),
        _ => Err(ExecutionError::ValuesNotComparable(a.clone(), b.clone())),
    }
}

#[inline(always)]
pub(super) fn vm_le(a: &Value, b: &Value) -> Result<bool, ExecutionError> {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => Ok(a <= b),
        (Value::UInt(a), Value::UInt(b)) => Ok(a <= b),
        (Value::Float(a), Value::Float(b)) => Ok(a <= b),
        (Value::String(a), Value::String(b)) => Ok(a <= b),
        (Value::Bool(a), Value::Bool(b)) => {
            Ok((if *a { 1 } else { 0 }) <= (if *b { 1 } else { 0 }))
        }
        (Value::Int(a), Value::Float(b)) => Ok((*a as f64) <= *b),
        (Value::Float(a), Value::Int(b)) => Ok(*a <= *b as f64),
        _ => Err(ExecutionError::ValuesNotComparable(a.clone(), b.clone())),
    }
}

#[inline(always)]
pub(super) fn vm_in(val: Value, container: Value) -> Result<Value, ExecutionError> {
    match container {
        Value::List(list) => {
            for item in list.iter() {
                if vm_eq(item, &val) {
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        Value::Map(map) => {
            let key = value_to_key(val)?;
            Ok(Value::Bool(map.map.contains_key(&key)))
        }
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_logical_and(lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Bool(false), _) | (_, Value::Bool(false)) => Value::Bool(false),
        (Value::Bool(true), Value::Bool(true)) => Value::Bool(true),
        (Value::Bool(true), other) | (other, Value::Bool(true)) => other,
        (a, _) => a,
    }
}

#[inline(always)]
pub(super) fn vm_logical_or(lhs: Value, rhs: Value) -> Value {
    match (lhs, rhs) {
        (Value::Bool(true), _) | (_, Value::Bool(true)) => Value::Bool(true),
        (Value::Bool(false), Value::Bool(false)) => Value::Bool(false),
        (Value::Bool(false), other) | (other, Value::Bool(false)) => other,
        (a, _) => a,
    }
}

#[inline(always)]
pub(super) fn vm_index(obj: Value, idx: Value) -> Result<Value, ExecutionError> {
    match obj {
        Value::List(list) => {
            let i = match idx {
                Value::Int(i) => i as usize,
                Value::UInt(u) => u as usize,
                _ => return Err(ExecutionError::UnsupportedListIndex(idx)),
            };
            list.get(i)
                .cloned()
                .ok_or(ExecutionError::IndexOutOfBounds(Value::Int(i as i64)))
        }
        Value::Map(map) => {
            let key = value_to_key(idx)?;
            map.get(&key)
                .cloned()
                .ok_or_else(|| ExecutionError::no_such_key("?"))
        }
        Value::String(s) => {
            let i = match idx {
                Value::Int(i) => i as usize,
                Value::UInt(u) => u as usize,
                _ => return Err(ExecutionError::UnsupportedListIndex(idx)),
            };
            s.chars()
                .nth(i)
                .map(|c| Value::String(Arc::new(c.to_string())))
                .ok_or(ExecutionError::IndexOutOfBounds(Value::Int(i as i64)))
        }
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn vm_select(obj: &Value, field: &Arc<String>) -> Result<Value, ExecutionError> {
    match obj {
        Value::Map(map) => {
            let f = field.as_str();
            if map.map.len() <= 8 {
                for (k, v) in map.map.iter() {
                    if k.as_keyref() == KeyRef::String(f) {
                        return Ok(v.clone());
                    }
                }
                return Err(ExecutionError::no_such_key(f));
            }
            map.get(&KeyRef::String(f))
                .cloned()
                .ok_or_else(|| ExecutionError::no_such_key(f))
        }
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

pub(super) fn vm_has_field(obj: &Value, field: &Arc<String>) -> bool {
    match obj {
        Value::Map(map) => {
            let f = field.as_str();
            if map.map.len() <= 8 {
                for (k, _) in map.map.iter() {
                    if k.as_keyref() == KeyRef::String(f) {
                        return true;
                    }
                }
                return false;
            }
            map.get(&KeyRef::String(f)).is_some()
        }
        _ => false,
    }
}

#[inline(always)]
pub(super) fn vm_size(a: Value) -> Result<Value, ExecutionError> {
    match a {
        Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
        Value::List(l) => Ok(Value::Int(l.len() as i64)),
        Value::Map(m) => Ok(Value::Int(m.map.len() as i64)),
        Value::Bytes(b) => Ok(Value::Int(b.len() as i64)),
        _ => Err(ExecutionError::NoSuchOverload),
    }
}

#[inline(always)]
pub(super) fn value_to_key(v: Value) -> Result<Key, ExecutionError> {
    match v {
        Value::Int(i) => Ok(Key::Int(i)),
        Value::UInt(u) => Ok(Key::Uint(u)),
        Value::Bool(b) => Ok(Key::Bool(b)),
        Value::String(s) => Ok(Key::String(s)),
        _ => Err(ExecutionError::unsupported_key_type(v)),
    }
}

#[inline(always)]
pub(super) fn key_to_value(k: Key) -> Value {
    match k {
        Key::Int(i) => Value::Int(i),
        Key::Uint(u) => Value::UInt(u),
        Key::Bool(b) => Value::Bool(b),
        Key::String(s) => Value::String(s),
    }
}

fn vm_call(builtin_id: u16, args: &[Value], _ctx: &Context) -> Result<Value, ExecutionError> {
    match builtin_id {
        0 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            vm_size(args[0].clone())
        }
        1 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            match &args[0] {
                Value::Int(i) => Ok(Value::Int(*i)),
                Value::UInt(u) => Ok(Value::Int(*u as i64)),
                Value::Float(f) => Ok(Value::Int(*f as i64)),
                Value::String(s) => s
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|e| ExecutionError::function_error("int", e.to_string())),
                _ => Err(ExecutionError::NoSuchOverload),
            }
        }
        2 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            match &args[0] {
                Value::Int(i) => Ok(Value::Float(*i as f64)),
                Value::UInt(u) => Ok(Value::Float(*u as f64)),
                Value::Float(f) => Ok(Value::Float(*f)),
                Value::String(s) => s
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|e| ExecutionError::function_error("double", e.to_string())),
                _ => Err(ExecutionError::NoSuchOverload),
            }
        }
        3 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            match &args[0] {
                Value::Int(i) => Ok(Value::UInt(*i as u64)),
                Value::UInt(u) => Ok(Value::UInt(*u)),
                Value::Float(f) => Ok(Value::UInt(*f as u64)),
                Value::String(s) => s
                    .parse::<u64>()
                    .map(Value::UInt)
                    .map_err(|e| ExecutionError::function_error("uint", e.to_string())),
                _ => Err(ExecutionError::NoSuchOverload),
            }
        }
        4 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            Ok(Value::String(Arc::new(format!("{:?}", args[0]))))
        }
        5 => {
            if args.is_empty() {
                return Err(ExecutionError::invalid_argument_count(1, 0));
            }
            let mut max = args[0].clone();
            for arg in &args[1..] {
                if vm_lt(&max, arg)? {
                    max = arg.clone();
                }
            }
            Ok(max)
        }
        6 => {
            if args.len() != 1 {
                return Err(ExecutionError::invalid_argument_count(1, args.len()));
            }
            Ok(Value::Bool(try_bool(args[0].clone()).unwrap_or(true)))
        }
        _ => Err(ExecutionError::undeclared_reference("unknown builtin")),
    }
}
