use crate::objects::Value;

/// Register-machine instruction for a fast scalar expression evaluator.
/// Variables are assigned to fixed registers 0..N at compile time;
/// no LoadVar instruction exists.  Vars are pre-loaded before eval.
#[derive(Clone, Debug)]
pub enum RegInstr {
    Halt(u8),
    Move(u8, u8), // dst, src

    // Generic constant (for types we can't inline)
    LoadConst(u8, u16), // dst, const_idx

    // Inline scalar constants (no const-pool indirection)
    LoadConstInt(u8, i64),
    LoadConstBool(u8, bool),

    // Integer arithmetic
    AddInt(u8, u8, u8),
    SubInt(u8, u8, u8),
    MulInt(u8, u8, u8),
    DivInt(u8, u8, u8),
    ModInt(u8, u8, u8),
    NegInt(u8, u8),

    // Type-specialized comparisons  ── the big win for filters
    EqIntConst(u8, u8, i64), // dst, src_reg, inline_int
    NeIntConst(u8, u8, i64),
    LtIntConst(u8, u8, i64),
    LeIntConst(u8, u8, i64),
    GtIntConst(u8, u8, i64),
    GeIntConst(u8, u8, i64),

    EqStrConst(u8, u8, u16), // dst, src_reg, const_idx (string)

    // Generic comparisons (fallback)
    Eq(u8, u8, u8),
    Ne(u8, u8, u8),
    Lt(u8, u8, u8),
    Le(u8, u8, u8),
    Gt(u8, u8, u8),
    Ge(u8, u8, u8),

    // Logic
    Not(u8, u8),
    // Eager AND (both sides already computed)
    And(u8, u8, u8),
    // Eager OR
    Or(u8, u8, u8),

    // Branching (used to implement short-circuit && / ||)
    Jump(i16),                // relative offset
    JumpIfFalse(u8, i16),     // if reg is false-ish, jump
    JumpIfTrue(u8, i16),      // if reg is true-ish, jump

    // Fields
    Select(u8, u8, u16), // dst, obj_reg, field_const_idx
    HasField(u8, u8, u16),

    // Collections
    Index(u8, u8, u8),
    In(u8, u8, u8),

    // Calls / builtins
    Size(u8, u8),
}

pub struct RegProgram {
    pub var_names: Vec<String>,
    pub constants: Vec<Value>,
    pub instructions: Vec<RegInstr>,
    pub num_regs: u8,
}

impl Clone for RegProgram {
    fn clone(&self) -> Self {
        Self {
            var_names: self.var_names.clone(),
            constants: self.constants.clone(),
            instructions: self.instructions.clone(),
            num_regs: self.num_regs,
        }
    }
}

impl std::fmt::Debug for RegProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegProgram")
            .field("instructions", &self.instructions)
            .field("num_regs", &self.num_regs)
            .field("var_names", &self.var_names)
            .finish()
    }
}
