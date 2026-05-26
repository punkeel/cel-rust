use cel::context::Context;
use cel::objects::{ResolveResult, Value};
use cel::Program;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_function_dispatch(c: &mut Criterion) {
    let mut group = c.benchmark_group("function_dispatch");

    // Benchmark: `size("hello")` — a simple stdlib function call
    // WITHOUT compile-time resolution
    group.bench_function("size_unresolved", |b| {
        let program = Program::compile("'hello'.size()").unwrap();
        let ctx = Context::default();
        b.iter(|| {
            let _ = black_box(program.execute(black_box(&ctx)));
        })
    });

    // WITH compile-time resolution
    group.bench_function("size_resolved", |b| {
        let env = cel::Env::stdlib();
        let program = Program::compile_with_env("'hello'.size()", &env).unwrap();
        let ctx = Context::default();
        b.iter(|| {
            let _ = black_box(program.execute(black_box(&ctx)));
        })
    });

    // Benchmark: `abs(-5)` — another stdlib call
    group.bench_function("abs_unresolved", |b| {
        let program = Program::compile("abs(-5)").unwrap();
        let mut ctx = Context::default();
        ctx.add_function("abs", |v: i64| v.abs());
        b.iter(|| {
            let _ = black_box(program.execute(black_box(&ctx)));
        })
    });

    group.bench_function("abs_resolved", |b| {
        let env = cel::Env::stdlib();
        let program = Program::compile_with_env("abs(-5)", &env).unwrap();
        let mut ctx = Context::default();
        ctx.add_function("abs", |v: i64| v.abs());
        b.iter(|| {
            let _ = black_box(program.execute(black_box(&ctx)));
        })
    });

    group.finish();
}

criterion_group!(benches, bench_function_dispatch);
criterion_main!(benches);
