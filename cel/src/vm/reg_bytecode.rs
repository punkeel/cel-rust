use crate::objects::Value;

/// Register-machine instruction.
/// All instructions are three-address or two-address to avoid stack traffic.
#[derive(Clone, Debug)]
pub enum RegInstr {
    Halt(u8),                       // return register
    LoadVar(u8, u16),               // dst, var_idx
    LoadConst(u8, u16),             // dst, const_idx
    Move(u8, u8),                   // dst, src

    // Arithmetic
    Add(u8, u8, u8),                // dst, src1, src2
    Sub(u8, u8, u8),
    Mul(u8, u8, u8),
    Div(u8, u8, u8),
    Mod(u8, u8, u8),
    Neg(u8, u8),                    // dst, src

    // Comparison
    Eq(u8, u8, u8),
    Ne(u8, u8, u8),
    Lt(u8, u8, u8),
    Le(u8, u8, u8),
    Gt(u8, u8, u8),
    Ge(u8, u8, u8),

    // Logic
    Not(u8, u8),                    // dst, src
    LogicalAnd(u8, u8, u8),
    LogicalOr(u8, u8, u8),

    // Branching
    Jump(i16),
    JumpIfFalse(u8, i16),           // if reg is false, jump
    JumpIfTrue(u8, i16),            // if reg is true, jump

    // Fields
    Select(u8, u8, u16),            // dst, obj_reg, field_const_idx
    HasField(u8, u8, u16),          // dst, obj_reg, field_const_idx

    // Collections
    Index(u8, u8, u8),              // dst, obj_reg, idx_reg
    In(u8, u8, u8),                 // dst, val_reg, container_reg

    // Calls
    Call(u8, u16, u16),             // dst_reg, builtin_id, argc, then arg regs follow implicitly
    Size(u8, u8),                   // dst, src
}

pub struct RegProgram {
    pub constants: Vec<Value>,
    pub var_names: Vec<String>,
    pub instructions: Vec<RegInstr>,
}

impl RegProgram {
    pub fn var_names(&self) -> &[String] {
        &self.var_names
    }
}

impl Clone for RegProgram {
    fn clone(&self) -> Self {
        Self {
            constants: self.constants.clone(),
            var_names: self.var_names.clone(),
            instructions: self.instructions.clone(),
        }
    }
}

impl std::fmt::Debug for RegProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegProgram")
            .field("instructions", &self.instructions)
            .field("constants", &self.constants)
            .field("var_names", &self.var_names)
            .finish()
    }
}
