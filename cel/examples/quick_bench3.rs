use cel::{Context, Program};
use cel::objects::Value;
use std::time::Instant;

fn main() {
    let ast = Program::compile("port == 80").unwrap();
    let vm = ast.compile_vm().unwrap();
    
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    
    let mut state = cel::vm::EvalState::new();
    
    // Warmup
    for _ in 0..100000 {
        cel::vm::eval(&vm, &ctx, &mut state).unwrap();
    }
    
    // Bench just reset
    let start = Instant::now();
    for _ in 0..10_000_000 {
        // We can't call reset directly (it's private), but we can time the diff between eval and eval_fast
        std::hint::black_box(cel::vm::eval_fast(&vm, &mut state).unwrap());
    }
    let fast_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
    
    let mut state2 = cel::vm::EvalState::new();
    for _ in 0..100000 { cel::vm::eval(&vm, &ctx, &mut state2).unwrap(); }
    
    let start = Instant::now();
    for _ in 0..10_000_000 {
        std::hint::black_box(cel::vm::eval(&vm, &ctx, &mut state2).unwrap());
    }
    let eval_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
    
    println!("eval={:.1} ns, eval_fast={:.1} ns, reset+check overhead={:.1} ns", eval_ns, fast_ns, eval_ns - fast_ns);
}
