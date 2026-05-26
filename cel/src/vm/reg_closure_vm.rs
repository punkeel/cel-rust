use crate::objects::Value;
use crate::vm::fast_value::FastValue;
use crate::vm::reg_bytecode::{RegInstr, RegProgram};
use crate::vm::reg_fast_vm::{fast_eq_int_const, fast_ge_int_const, fast_gt_int_const, fast_le_int_const, fast_lt_int_const, vm_logical_and_fast};
use crate::ExecutionError;

/// A compiled closure that evaluates a specific Register VM program.
pub type RegClosure = Box<dyn Fn(&mut super::reg_fast_vm::RegFastState) -> Result<Value, ExecutionError> + Send + Sync>;

/// Compile a Register VM program into a specialized closure.
/// For common small programs this eliminates dispatch overhead entirely.
pub fn compile_reg_closure(program: &RegProgram) -> RegClosure {
    // Pattern: single comparison instruction followed by Halt
    if program.instructions.len() == 2 {
        match (&program.instructions[0], &program.instructions[1]) {
            (RegInstr::EqIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] = fast_eq_int_const(regs[src as usize], val);
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::NeIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] =
                        FastValue::from_bool(regs[src as usize].as_int().map_or(false, |i| i != val));
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::LtIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] = fast_lt_int_const(regs[src as usize], val);
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::LeIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] = fast_le_int_const(regs[src as usize], val);
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::GtIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] = fast_gt_int_const(regs[src as usize], val);
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::GeIntConst(dst, src, val), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let src = *src;
                let val = *val;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[src as usize] = fast_ge_int_const(regs[src as usize], val);
                    Ok(regs[src as usize].to_value())
                });
            }
            (RegInstr::AddInt(dst, a, b), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let a = *a;
                let b = *b;
                let halt_reg = *halt_reg;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    if let (Some(av), Some(bv)) = (regs[a as usize].as_int(), regs[b as usize].as_int()) {
                        regs[usize::from(halt_reg)] = FastValue::from_int(av.wrapping_add(bv));
                        Ok(regs[usize::from(halt_reg)].to_value())
                    } else {
                        Err(ExecutionError::InternalError("non-int AddInt".into()))
                    }
                });
            }
            (RegInstr::And(dst, a, b), RegInstr::Halt(halt_reg))
                if *dst == *halt_reg =>
            {
                let a = *a;
                let b = *b;
                let halt_reg = *halt_reg;
                return Box::new(move |state| {
                    let regs = &mut state.regs;
                    regs[usize::from(halt_reg)] =
                        vm_logical_and_fast(regs[a as usize], regs[b as usize]);
                    Ok(regs[usize::from(halt_reg)].to_value())
                });
            }
            _ => {}
        }
    }

    // Pattern: single Halt (trivial program)
    if program.instructions.len() == 1 {
        if let RegInstr::Halt(r) = &program.instructions[0] {
            let r = *r;
            return Box::new(move |state| Ok(state.regs[r as usize].to_value()));
        }
    }

    // Fallback: generic interpreter
    let program = program.clone();
    Box::new(move |state| super::reg_fast_vm::eval_reg_fast(&program, state))
}
