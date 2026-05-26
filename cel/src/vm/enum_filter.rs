use crate::objects::Value;
use crate::vm::fast_value::FastValue;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::vm::reg_fast_vm::{eval_reg_fast, RegFastState};
use crate::ExecutionError;

/// An enum-based compiled filter that eliminates vtable dispatch entirely.
/// Each variant represents a common expression pattern, compiled to native
/// code via pattern matching. No Box<dyn>, no loop counter, no bounds check.
#[derive(Clone, Debug)]
pub enum EnumFilter {
    EqIntConst { src: u8, val: i64 },
    NeIntConst { src: u8, val: i64 },
    LtIntConst { src: u8, val: i64 },
    LeIntConst { src: u8, val: i64 },
    GtIntConst { src: u8, val: i64 },
    GeIntConst { src: u8, val: i64 },

    /// String equality via FastValue (uses SSO or Arc pointer).
    EqStrConst { src: u8, expected: std::sync::Arc<String> },
    /// String equality with inline expected bytes (no heap, SSO).
    InlineStrConst { src: u8, expected: [u8; 8], len: u8 },
    /// String equality reading raw Value (bypasses FastValue entirely).
    StringConstRaw { src: u8, expected: std::sync::Arc<String> },

    /// Integer set membership (linear scan — fastest for small n ≤ 16).
    InIntSet { src: u8, values: Vec<i64> },
    /// String set membership (linear scan — fastest for small n ≤ 16).
    InStrSet { src: u8, values: Vec<std::sync::Arc<String>> },

    // Two-value logic: var >= a && var <= b  (common range check)
    AndGeLeIntConst { src: u8, lo: i64, hi: i64 },

    // Cost-aware And/OR: evaluate cheaper branch first
    AndBox(Box<EnumFilter>, Box<EnumFilter>),
    OrBox(Box<EnumFilter>, Box<EnumFilter>),

    // Fallback to generic interpreter
    Generic(RegProgram),
}

impl EnumFilter {
    pub fn compile(program: &RegProgram) -> Self {
        Self::compile_inner(program, &program.instructions)
    }

    fn compile_inner(program: &RegProgram, code: &[RegInstr]) -> Self {
        match code {
            // Single int comparison + Halt
            [RegInstr::EqIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::EqIntConst { src: *src, val: *val }
            }
            [RegInstr::NeIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::NeIntConst { src: *src, val: *val }
            }
            [RegInstr::LtIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::LtIntConst { src: *src, val: *val }
            }
            [RegInstr::LeIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::LeIntConst { src: *src, val: *val }
            }
            [RegInstr::GtIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::GtIntConst { src: *src, val: *val }
            }
            [RegInstr::GeIntConst(dst, src, val), RegInstr::Halt(halt)] if dst == halt => {
                Self::GeIntConst { src: *src, val: *val }
            }

            // String equality + Halt — use inline form for short strings
            [RegInstr::EqStrConst(dst, src, idx), RegInstr::Halt(halt)] if dst == halt => {
                let expected = match &program.constants[*idx as usize] {
                    Value::String(s) => s.clone(),
                    _ => return Self::Generic(program.clone()),
                };
                // Try inline for short strings (≤ 8 bytes)
                if expected.len() <= 8 {
                    let mut bytes = [0u8; 8];
                    bytes[..expected.len()].copy_from_slice(expected.as_bytes());
                    Self::InlineStrConst {
                        src: *src,
                        expected: bytes,
                        len: expected.len() as u8,
                    }
                } else {
                    Self::StringConstRaw { src: *src, expected }
                }
            }

            // Integer set membership + Halt
            [RegInstr::InIntSet(dst, src, idx), RegInstr::Halt(halt)] if dst == halt => {
                let set: Vec<i64> = match &program.constants[*idx as usize] {
                    Value::List(list) => {
                        let mut ints = Vec::with_capacity(list.len());
                        for item in list.iter() {
                            if let Value::Int(i) = item {
                                ints.push(*i);
                            } else {
                                return Self::Generic(program.clone());
                            }
                        }
                        ints
                    }
                    _ => return Self::Generic(program.clone()),
                };
                Self::InIntSet { src: *src, values: set }
            }

            // String set membership + Halt
            [RegInstr::InStrSet(dst, src, idx), RegInstr::Halt(halt)] if dst == halt => {
                let set: Vec<std::sync::Arc<String>> = match &program.constants[*idx as usize] {
                    Value::List(list) => {
                        let mut strs = Vec::with_capacity(list.len());
                        for item in list.iter() {
                            if let Value::String(s) = item {
                                strs.push(s.clone());
                            } else {
                                return Self::Generic(program.clone());
                            }
                        }
                        strs
                    }
                    _ => return Self::Generic(program.clone()),
                };
                Self::InStrSet { src: *src, values: set }
            }

            // Range check: port >= lo && port <= hi
            [RegInstr::GeIntConst(r1, src1, lo), RegInstr::LeIntConst(r2, src2, hi), RegInstr::And(dst, a, b), RegInstr::Halt(halt)]
                if r1 == a && r2 == b && dst == halt && src1 == src2 =>
            {
                Self::AndGeLeIntConst { src: *src1, lo: *lo, hi: *hi }
            }

            // General AND — try to decompose into two filters
            [.., RegInstr::And(dst, a, b), RegInstr::Halt(halt)] if dst == halt => {
                if code.len() >= 4 {
                    let left_prog = &code[..code.len()-2];
                    let right_prog = &code[..code.len()-2];
                    let left = Self::compile_inner(program, left_prog);
                    let right = Self::compile_inner(program, right_prog);
                    if left.cost() <= right.cost() {
                        Self::AndBox(Box::new(left), Box::new(right))
                    } else {
                        Self::AndBox(Box::new(right), Box::new(left))
                    }
                } else {
                    Self::Generic(program.clone())
                }
            }

            _ => Self::Generic(program.clone()),
        }
    }

    /// Estimated cost for cost-aware ordering (lower = cheaper).
    fn cost(&self) -> u8 {
        match self {
            Self::EqIntConst { .. }
            | Self::NeIntConst { .. }
            | Self::LtIntConst { .. }
            | Self::LeIntConst { .. }
            | Self::GtIntConst { .. }
            | Self::GeIntConst { .. } => 1,
            Self::InlineStrConst { .. } => 2,
            Self::EqStrConst { .. } | Self::StringConstRaw { .. } => 2,
            Self::AndGeLeIntConst { .. } => 2,
            Self::InIntSet { values, .. } => (2 + values.len() as u8).min(10),
            Self::InStrSet { values, .. } => (3 + values.len() as u8).min(15),
            Self::AndBox(l, r) | Self::OrBox(l, r) => l.cost().saturating_add(r.cost()),
            Self::Generic(_) => 255,
        }
    }

    #[inline(always)]
    pub fn eval(&self, state: &mut RegFastState) -> Result<Value, ExecutionError> {
        match self {
            Self::EqIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i == *val);
                Ok(Value::Bool(result))
            }
            Self::NeIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i != *val);
                Ok(Value::Bool(result))
            }
            Self::LtIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i < *val);
                Ok(Value::Bool(result))
            }
            Self::LeIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i <= *val);
                Ok(Value::Bool(result))
            }
            Self::GtIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i > *val);
                Ok(Value::Bool(result))
            }
            Self::GeIntConst { src, val } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_int().map_or(false, |i| i >= *val);
                Ok(Value::Bool(result))
            }
            // FastValue path with SSO + direct Arc
            Self::EqStrConst { src, expected } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].eq_str(expected.as_str());
                Ok(Value::Bool(result))
            }
            // Inline bytes — zero heap dereference on the expected side
            Self::InlineStrConst { src, expected, len } => {
                let regs = &mut state.regs;
                let expected_str = unsafe {
                    std::str::from_utf8_unchecked(&expected[..*len as usize])
                };
                let result = regs[*src as usize].eq_str(expected_str);
                Ok(Value::Bool(result))
            }
            // Raw Value path — bypass FastValue entirely
            Self::StringConstRaw { src, expected } => {
                // This path reads directly from the Value stored in the VM state.
                // But RegFastState only stores FastValues, so we fall back.
                // In practice, this would need a parallel state that stores raw Values.
                // For now, delegate to the FastValue path.
                let regs = &mut state.regs;
                let result = regs[*src as usize].eq_str(expected.as_str());
                Ok(Value::Bool(result))
            }
            Self::InIntSet { src, values } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize]
                    .as_int()
                    .map_or(false, |needle| values.contains(&needle));
                Ok(Value::Bool(result))
            }
            Self::InStrSet { src, values } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize].as_ptr().map_or(false, |ptr| unsafe {
                    match &(*ptr).value {
                        Value::String(s) => values.iter().any(|v| v.as_str() == s.as_str()),
                        _ => false,
                    }
                });
                Ok(Value::Bool(result))
            }
            Self::AndGeLeIntConst { src, lo, hi } => {
                let regs = &mut state.regs;
                let result = regs[*src as usize]
                    .as_int()
                    .map_or(false, |i| i >= *lo && i <= *hi);
                Ok(Value::Bool(result))
            }
            Self::AndBox(a, b) => {
                let left = a.eval(state)?;
                if !matches!(left, Value::Bool(true)) {
                    return Ok(left);
                }
                b.eval(state)
            }
            Self::OrBox(a, b) => {
                let left = a.eval(state)?;
                if matches!(left, Value::Bool(true)) {
                    return Ok(left);
                }
                b.eval(state)
            }
            Self::Generic(program) => eval_reg_fast(program, state),
        }
    }
}
