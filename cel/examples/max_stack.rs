use cel::Program;

fn max_stack_depth(instructions: &[cel::vm::bytecode::Instr]) -> usize {
    let mut max_depth = 0usize;
    let mut depth = 0i32;
    for instr in instructions {
        match instr {
            cel::vm::bytecode::Instr::PushConst(_) | cel::vm::bytecode::Instr::LoadVar(_) 
            | cel::vm::bytecode::Instr::LoadVarSelect(_, _) | cel::vm::bytecode::Instr::LoadVarHasField(_, _) => {
                depth += 1;
            }
            cel::vm::bytecode::Instr::Pop | cel::vm::bytecode::Instr::IterPop => {
                depth -= 1;
            }
            cel::vm::bytecode::Instr::Add | cel::vm::bytecode::Instr::Sub | cel::vm::bytecode::Instr::Mul 
            | cel::vm::bytecode::Instr::Div | cel::vm::bytecode::Instr::Mod
            | cel::vm::bytecode::Instr::Eq | cel::vm::bytecode::Instr::Ne | cel::vm::bytecode::Instr::Lt
            | cel::vm::bytecode::Instr::Le | cel::vm::bytecode::Instr::Gt | cel::vm::bytecode::Instr::Ge
            | cel::vm::bytecode::Instr::LogicalAnd | cel::vm::bytecode::Instr::LogicalOr | cel::vm::bytecode::Instr::In
            | cel::vm::bytecode::Instr::Index => {
                depth -= 1; // binary: pop2 push1 = -1
            }
            cel::vm::bytecode::Instr::Call(_, argc) => {
                depth -= *argc as i32 - 1;
            }
            cel::vm::bytecode::Instr::Neg | cel::vm::bytecode::Instr::Not | cel::vm::bytecode::Instr::Size 
            | cel::vm::bytecode::Instr::MatchesCompiled(_) => {
                // unary: net 0
            }
            cel::vm::bytecode::Instr::BuildList(n) | cel::vm::bytecode::Instr::BuildMap(n) => {
                depth -= *n as i32 - 1;
            }
            _ => {}
        }
        max_depth = max_depth.max(depth as usize);
    }
    max_depth
}

fn main() {
    for expr in &[
        "port == 80",
        "port >= 1024", 
        "port in [80, 8080, 8880, 2052, 2082, 2086, 2095]",
        "a + b * c",
        "a == 1 && b == 2 && c == 3",
        "[1,2,3,4,5,6,7,8,9,10].exists(x, x > 5)",
    ] {
        let prog = Program::compile(expr).unwrap();
        let vm = prog.compile_vm().unwrap();
        let depth = max_stack_depth(&vm.instructions);
        println!("{:50} => max stack depth: {}", expr, depth);
    }
}
