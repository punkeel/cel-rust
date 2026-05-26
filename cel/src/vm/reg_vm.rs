use crate::objects::Value;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::vm::vm::{
    try_bool, vm_add, vm_div, vm_eq, vm_has_field, vm_in, vm_index, vm_le, vm_lt, vm_mod, vm_mul,
    vm_neg, vm_select, vm_size, vm_sub,
};
use crate::ExecutionError;
use std::sync::Arc;

pub struct RegState {
    pub regs: [Value; 8],
    pub vars: Vec<Value>,
}

impl RegState {
    pub fn new() -> Self {
        Self {
            regs: std::array::from_fn(|_| Value::Null),
            vars: Vec::with_capacity(16),
        }
    }

    pub fn bind_vars(&mut self, program: &RegProgram, ctx: &crate::context::Context) {
        let need = program.var_names.len();
        if self.vars.len() != need {
            self.vars.resize_with(need, || Value::Null);
        }
        for (i, name) in program.var_names.iter().enumerate() {
            let v = ctx
                .get_variable(name)
                .and_then(|cow| Value::try_from(cow.as_ref()).ok())
                .unwrap_or(Value::Null);
            self.vars[i] = v;
        }
    }

    pub fn set_vars(&mut self, vars: Vec<Value>) {
        self.vars = vars;
    }
}

impl Default for RegState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn eval_reg(program: &RegProgram, state: &mut RegState) -> Result<Value, ExecutionError> {
    let regs = &mut state.regs;
    let vars = &state.vars;
    let mut pc = 0usize;

    loop {
        let instr = unsafe { program.instructions.get_unchecked(pc) };
        pc += 1;
        match instr {
            RegInstr::Halt(r) => return Ok(regs[*r as usize].clone()),
            RegInstr::LoadVar(dst, idx) => regs[*dst as usize] = vars[*idx as usize].clone(),
            RegInstr::LoadConst(dst, idx) => regs[*dst as usize] = program.constants[*idx as usize].clone(),
            RegInstr::Move(dst, src) => regs[*dst as usize] = regs[*src as usize].clone(),

            RegInstr::Add(dst, a, b) => {
                let r = vm_add(regs[*a as usize].clone(), regs[*b as usize].clone())?;
                regs[*dst as usize] = r;
            }
            RegInstr::Sub(dst, a, b) => {
                let r = vm_sub(regs[*a as usize].clone(), regs[*b as usize].clone())?;
                regs[*dst as usize] = r;
            }
            RegInstr::Mul(dst, a, b) => {
                let r = vm_mul(regs[*a as usize].clone(), regs[*b as usize].clone())?;
                regs[*dst as usize] = r;
            }
            RegInstr::Div(dst, a, b) => {
                let r = vm_div(regs[*a as usize].clone(), regs[*b as usize].clone())?;
                regs[*dst as usize] = r;
            }
            RegInstr::Mod(dst, a, b) => {
                let r = vm_mod(regs[*a as usize].clone(), regs[*b as usize].clone())?;
                regs[*dst as usize] = r;
            }
            RegInstr::Neg(dst, src) => {
                regs[*dst as usize] = vm_neg(regs[*src as usize].clone())?;
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
            RegInstr::LogicalAnd(dst, a, b) => {
                regs[*dst as usize] = vm_logical_and(regs[*a as usize].clone(), regs[*b as usize].clone());
            }
            RegInstr::LogicalOr(dst, a, b) => {
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
                    _ => return Err(ExecutionError::InternalError("select non-string".into())),
                };
                regs[*dst as usize] = vm_select(&regs[*obj as usize], field)?;
            }
            RegInstr::HasField(dst, obj, field_idx) => {
                let field = match &program.constants[*field_idx as usize] {
                    Value::String(s) => s,
                    _ => return Err(ExecutionError::InternalError("has_field non-string".into())),
                };
                regs[*dst as usize] = Value::Bool(vm_has_field(&regs[*obj as usize], field));
            }

            RegInstr::Index(dst, obj, idx) => {
                regs[*dst as usize] = vm_index(regs[*obj as usize].clone(), regs[*idx as usize].clone())?;
            }
            RegInstr::In(dst, val, container) => {
                regs[*dst as usize] = vm_in(regs[*val as usize].clone(), regs[*container as usize].clone())?;
            }

            RegInstr::Call(dst, builtin_id, argc) => {
                // Register VM call: arguments are in consecutive registers starting at dst+1
                // Actually this encoding is problematic. For now, use a simple approach:
                // single-arg builtins only.
                let argc = *argc as usize;
                let args_start = *dst as usize + 1;
                let args = &regs[args_start..args_start + argc];
                let result = vm_call_reg(*builtin_id, args)?;
                regs[*dst as usize] = result;
            }
            RegInstr::Size(dst, src) => {
                regs[*dst as usize] = vm_size(regs[*src as usize].clone())?;
            }
        }
    }
}

fn vm_call_reg(builtin_id: u16, args: &[Value]) -> Result<Value, ExecutionError> {
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
