use crate::objects::Value;

/// A bytecode instruction.
/// Operands are stored inline; multi-operand instructions use wide encoding.
#[derive(Clone, Debug)]
pub enum Instr {
    Halt,
    PushConst(u16),
    LoadVar(u16),
    Pop,
    Return,
    Jump(i16),
    JumpIfFalse(i16),
    JumpIfTrue(i16),
    JumpIfFalseKeep(i16),
    JumpIfTrueKeep(i16),

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,

    // Comparison / logic
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    In,
    LogicalAnd,
    LogicalOr,

    // Collections
    Index,
    BuildList(u16),
    BuildMap(u16),

    // Fields
    Select(u16),       // field name index in const pool
    HasField(u16),     // field name index in const pool
    LoadVarSelect(u16, u16), // (var_idx, field_idx)  fused LoadVar+Select
    LoadVarHasField(u16, u16), // (var_idx, field_idx) fused LoadVar+HasField

    // Fused var+const comparisons
    LoadVarEqConst(u16, u16), // (var_idx, const_idx)
    LoadVarNeConst(u16, u16),
    LoadVarLtConst(u16, u16),
    LoadVarLeConst(u16, u16),
    LoadVarGtConst(u16, u16),
    LoadVarGeConst(u16, u16),

    // Peephole-optimized: return var op const directly (no stack)
    ReturnEqConst(u16, u16),
    ReturnNeConst(u16, u16),
    ReturnLtConst(u16, u16),
    ReturnLeConst(u16, u16),
    ReturnGtConst(u16, u16),
    ReturnGeConst(u16, u16),

    // Calls
    Call(u16, u16), // (builtin_id, argc)
    Size,
    MatchesCompiled(u16), // (regex_pool_idx) — target is on stack

    // Comprehensions
    IterInit,
    IterNext(i16), // jump offset if exhausted
    IterPop,
    AccuPush(u16), // const index for initial value, 0xFFFF => use TOS
    AccuSet,
}

/// Special variable indices used during comprehension execution.
pub const IDX_ITER_ELEM: u16 = 0xFFFE;
pub const IDX_ACCU: u16 = 0xFFFF;

/// A compiled program for the VM.
pub struct Program {
    pub constants: Vec<Value>,
    pub var_names: Vec<String>,
    pub instructions: Vec<Instr>,
    #[cfg(feature = "regex")]
    pub regex_pool: Vec<regex::Regex>,
    #[cfg(not(feature = "regex"))]
    pub regex_pool: Vec<()>,
}

impl Program {
    pub fn var_names(&self) -> &[String] {
        &self.var_names
    }
}

impl Clone for Program {
    fn clone(&self) -> Self {
        Self {
            constants: self.constants.clone(),
            var_names: self.var_names.clone(),
            instructions: self.instructions.clone(),
            regex_pool: self.regex_pool.clone(),
        }
    }
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("instructions", &self.instructions)
            .field("constants", &self.constants)
            .field("var_names", &self.var_names)
            .field("regex_pool", &self.regex_pool.len())
            .finish()
    }
}
