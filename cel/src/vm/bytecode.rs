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
    Select(u16),   // field name index in const pool
    HasField(u16), // field name index in const pool

    // Calls
    Call(u16, u16), // (builtin_id, argc)
    Size,

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
#[derive(Clone, Debug)]
pub struct Program {
    pub constants: Vec<Value>,
    pub var_names: Vec<String>,
    pub instructions: Vec<Instr>,
}
