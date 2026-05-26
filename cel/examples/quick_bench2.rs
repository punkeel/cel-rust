use cel::{Context, Program};
use cel::objects::Value;
use std::time::Instant;

fn main() {
    let ast = Program::compile("port == 80").unwrap();
    let vm = ast.compile_vm().unwrap();
    let reg = cel::vm::compile_reg(ast.expression()).unwrap();
    let ef = cel::vm::EnumFilter::compile(&reg);
    
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    
    let mut state = cel::vm::EvalState::new();
    let mut reg_state = cel::vm::RegFastState::new();
    reg_state.set_vars(&[Value::Int(8081)]);
    let mut reg_state2 = cel::vm::RegFastState::new();
    reg_state2.set_vars(&[Value::Int(8081)]);
    
    // Warmup with full eval (bind_vars)
    for _ in 0..100000 {
        cel::vm::eval(&vm, &ctx, &mut state).unwrap();
        cel::vm::eval_fast(&vm, &mut state).unwrap();
        ef.eval(&mut reg_state2).unwrap();
    }
    
    // Bench eval (with reset + bind_vars check)
    let start = Instant::now();
    for _ in 0..10_000_000 {
        std::hint::black_box(cel::vm::eval(&vm, &ctx, &mut state).unwrap());
    }
    let eval_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
    
    // Bench eval_fast (skip bind_vars)
    let start = Instant::now();
    for _ in 0..10_000_000 {
        std::hint::black_box(cel::vm::eval_fast(&vm, &mut state).unwrap());
    }
    let fast_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
    
    // Bench EnumFilter
    let start = Instant::now();
    for _ in 0..10_000_000 {
        std::hint::black_box(ef.eval(&mut reg_state2).unwrap());
    }
    let ef_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
    
    println!("eval (full)={:.1} ns, eval_fast={:.1} ns, EnumFilter={:.1} ns", eval_ns, fast_ns, ef_ns);
    println!("speedup eval→fast: {:.2}x, eval→enum: {:.2}x", eval_ns / fast_ns, eval_ns / ef_ns);
}
