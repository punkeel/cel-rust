// Quick test to see if reset matters
use cel::{Context, Program};

fn main() {
    let ast = Program::compile("port == 80").unwrap();
    let vm = ast.compile_vm().unwrap();
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    
    let mut state = cel::vm::EvalState::new();
    cel::vm::eval(&vm, &ctx, &mut state).unwrap();
    
    let start = std::time::Instant::now();
    for _ in 0..10_000_000 {
        state.reset(&vm);
    }
    let dur = start.elapsed();
    println!("reset only: {:?} per iter", dur / 10_000_000);
    
    let start = std::time::Instant::now();
    for _ in 0..10_000_000 {
        cel::vm::eval_fast(&vm, &mut state).unwrap();
    }
    let dur = start.elapsed();
    println!("eval_fast: {:?} per iter", dur / 10_000_000);
}
