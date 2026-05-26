use cel::{Context, Program};
use cel::objects::Value;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_components(c: &mut Criterion) {
    let mut group = c.benchmark_group("port_eq_80_components");

    // 1. Raw i64 comparison
    let val_a = 80i64;
    let val_b = 8081i64;
    group.bench_function("raw_i64_eq", |bencher| {
        bencher.iter(|| black_box(val_a) == black_box(val_b))
    });

    // 2. Filter tree
    let ast = Program::compile("port == 80").unwrap();
    let tree = cel::vm::compile_filter_tree(ast.expression()).unwrap();
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    let vars = tree.bind_vars(&ctx);
    group.bench_function("filter_tree", |bencher| {
        bencher.iter(|| tree.filter.eval(black_box(&vars)))
    });

    // 3. VM with fresh state each iteration (includes bind_vars)
    let vm = ast.compile_vm().unwrap();
    group.bench_function("vm_fresh_state", |bencher| {
        bencher.iter(|| {
            let mut state = cel::vm::EvalState::new();
            cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap()
        })
    });

    // 4. VM with reused state (bind_vars only on first call)
    let mut state = cel::vm::EvalState::new();
    group.bench_function("vm_reused_state", |bencher| {
        bencher.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
    });

    // 5. Register VM
    let reg_prog = cel::vm::compile_reg(ast.expression()).unwrap();
    let mut reg_state = cel::vm::RegState::new();
    let vars = vec![Value::Int(8081)];
    reg_state.set_vars(&vars);
    group.bench_function("reg_vm", |bencher| {
        bencher.iter(|| cel::vm::eval_reg(black_box(&reg_prog), black_box(&mut reg_state)).unwrap())
    });

    // 6. FastValue Register VM
    let mut reg_fast_state = cel::vm::RegFastState::new();
    reg_fast_state.set_vars(&vars);
    group.bench_function("reg_fast_vm", |bencher| {
        bencher.iter(|| cel::vm::eval_reg_fast(black_box(&reg_prog), black_box(&mut reg_fast_state)).unwrap())
    });

    // 7. Closure Register VM (direct threading / partial eval)
    let closure = cel::vm::compile_reg_closure(&reg_prog);
    let mut reg_fast_state2 = cel::vm::RegFastState::new();
    reg_fast_state2.set_vars(&vars);
    group.bench_function("reg_closure_vm", |bencher| {
        bencher.iter(|| closure(black_box(&mut reg_fast_state2)).unwrap())
    });

    // 8. Enum Filter (vtable-free dispatch)
    let ef = cel::vm::EnumFilter::compile(&reg_prog);
    let mut reg_fast_state3 = cel::vm::RegFastState::new();
    reg_fast_state3.set_vars(&vars);
    group.bench_function("enum_filter", |bencher| {
        bencher.iter(|| ef.eval(black_box(&mut reg_fast_state3)).unwrap())
    });

    group.finish();
}

fn bench_inline_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("inline_vs_call");

    let ast = Program::compile("port == 80").unwrap();
    let vm = ast.compile_vm().unwrap();
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    let mut state = cel::vm::EvalState::new();
    cel::vm::eval(&vm, &ctx, &mut state).unwrap(); // warm up

    // Direct inline access (no function call)
    group.bench_function("direct_inline", |bencher| {
        let vars = &state.vars;
        let constants = &vm.constants;
        bencher.iter(|| {
            let a = &vars[0];
            let b = &constants[0];
            let result = match (a, b) {
                (cel::objects::Value::Int(x), cel::objects::Value::Int(y)) => x == y,
                _ => false,
            };
            black_box(result)
        })
    });

    group.finish();
}

fn bench_reg_vs_stack(c: &mut Criterion) {
    let mut group = c.benchmark_group("reg_vs_stack");

    for (name, expr) in [
        ("port_eq_80", "port == 80"),
        ("port_ge_1024", "port >= 1024"),
        ("arith", "port + 100 >= 1024"),
        ("logic", "port >= 80 && port <= 443"),
    ] {
        let ast = Program::compile(expr).unwrap();
        let vm = ast.compile_vm().unwrap();
        let reg = cel::vm::compile_reg(ast.expression()).unwrap();

        let mut ctx = Context::default();
        ctx.add_variable_from_value("port", 8081i64);

        let mut state = cel::vm::EvalState::new();
        let mut reg_state = cel::vm::RegState::new();
        let vars = vec![Value::Int(8081)];
        reg_state.set_vars(&vars);

        group.bench_function(BenchmarkId::new("stack", name), |b| {
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
        group.bench_function(BenchmarkId::new("reg", name), |b| {
            b.iter(|| cel::vm::eval_reg(black_box(&reg), black_box(&mut reg_state)).unwrap())
        });

        let mut reg_fast_state = cel::vm::RegFastState::new();
        reg_fast_state.set_vars(&vars);
        group.bench_function(BenchmarkId::new("reg_fast", name), |b| {
            b.iter(|| cel::vm::eval_reg_fast(black_box(&reg), black_box(&mut reg_fast_state)).unwrap())
        });

        let closure = cel::vm::compile_reg_closure(&reg);
        let mut reg_fast_state2 = cel::vm::RegFastState::new();
        reg_fast_state2.set_vars(&vars);
        group.bench_function(BenchmarkId::new("reg_closure", name), |b| {
            b.iter(|| closure(black_box(&mut reg_fast_state2)).unwrap())
        });

        let ef = cel::vm::EnumFilter::compile(&reg);
        let mut reg_fast_state3 = cel::vm::RegFastState::new();
        reg_fast_state3.set_vars(&vars);
        group.bench_function(BenchmarkId::new("enum_filter", name), |b| {
            b.iter(|| ef.eval(black_box(&mut reg_fast_state3)).unwrap())
        });
    }

    group.finish();
}

fn bench_string_eq(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_eq");
    let ast = Program::compile(r#"country == "GB""#).unwrap();
    let vm = ast.compile_vm().unwrap();
    let tree = cel::vm::compile_filter_tree(ast.expression()).unwrap();
    let mut ctx = Context::default();
    ctx.add_variable_from_value("country", "GB");
    let vars = tree.bind_vars(&ctx);
    let var_values: Vec<Value> = vec![Value::String(std::sync::Arc::new("GB".to_string()))];

    group.bench_function("ast", |b| {
        b.iter(|| ast.execute(black_box(&ctx)).unwrap())
    });
    group.bench_function("tree", |b| {
        b.iter(|| tree.filter.eval(black_box(&vars)))
    });
    group.bench_function("vm_reused", |b| {
        let mut state = cel::vm::EvalState::new();
        b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
    });

    let reg = cel::vm::compile_reg(ast.expression()).unwrap();
    let mut reg_state = cel::vm::RegFastState::new();
    reg_state.set_vars(&var_values);
    group.bench_function("reg_fast", |b| {
        b.iter(|| cel::vm::eval_reg_fast(black_box(&reg), black_box(&mut reg_state)).unwrap())
    });

    let ef = cel::vm::EnumFilter::compile(&reg);
    let mut reg_state2 = cel::vm::RegFastState::new();
    reg_state2.set_vars(&var_values);
    group.bench_function("enum_filter", |b| {
        b.iter(|| ef.eval(black_box(&mut reg_state2)).unwrap())
    });
    group.finish();
}

fn bench_in_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("in_set");
    let ast = Program::compile("port in [80, 443, 8080, 3000]").unwrap();
    let vm = ast.compile_vm().unwrap();
    let tree = cel::vm::compile_filter_tree(ast.expression()).unwrap();
    let mut ctx = Context::default();
    ctx.add_variable_from_value("port", 8081i64);
    let vars = tree.bind_vars(&ctx);
    let var_values: Vec<Value> = vec![Value::Int(8081i64)];

    group.bench_function("ast", |b| {
        b.iter(|| ast.execute(black_box(&ctx)).unwrap())
    });
    group.bench_function("tree", |b| {
        b.iter(|| tree.filter.eval(black_box(&vars)))
    });
    group.bench_function("vm_reused", |b| {
        let mut state = cel::vm::EvalState::new();
        b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
    });

    let reg = cel::vm::compile_reg(ast.expression()).unwrap();
    let mut reg_state = cel::vm::RegFastState::new();
    reg_state.set_vars(&var_values);
    group.bench_function("reg_fast", |b| {
        b.iter(|| cel::vm::eval_reg_fast(black_box(&reg), black_box(&mut reg_state)).unwrap())
    });

    let ef = cel::vm::EnumFilter::compile(&reg);
    let mut reg_state2 = cel::vm::RegFastState::new();
    reg_state2.set_vars(&var_values);
    group.bench_function("enum_filter", |b| {
        b.iter(|| ef.eval(black_box(&mut reg_state2)).unwrap())
    });
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(30)
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_components, bench_inline_comparison, bench_reg_vs_stack, bench_string_eq, bench_in_set
}

criterion_main!(benches);
