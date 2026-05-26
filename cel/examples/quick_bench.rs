use cel::{Context, Program};
use cel::objects::Value;
use std::time::Instant;

fn main() {
    for (name, expr) in [
        ("port_eq_80", "port == 80"),
        ("port_ge_1024", "port >= 1024"),
        ("arith", "port + 100 >= 1024"),
        ("logic", "port >= 80 && port <= 443"),
    ] {
        let ast = Program::compile(expr).unwrap();
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
        
        // Warmup
        for _ in 0..100000 {
            cel::vm::eval(&vm, &ctx, &mut state).unwrap();
            ef.eval(&mut reg_state2).unwrap();
        }
        
        // Bench VM
        let start = Instant::now();
        for _ in 0..10_000_000 {
            std::hint::black_box(cel::vm::eval(&vm, &ctx, &mut state).unwrap());
        }
        let vm_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
        
        // Bench EnumFilter
        let start = Instant::now();
        for _ in 0..10_000_000 {
            std::hint::black_box(ef.eval(&mut reg_state2).unwrap());
        }
        let ef_ns = start.elapsed().as_nanos() as f64 / 10_000_000.0;
        
        println!("{}: VM={:.1} ns, EnumFilter={:.1} ns, speedup={:.2}x", name, vm_ns, ef_ns, vm_ns / ef_ns);
    }
}
