use cel::context::Context;
use cel::Program;
use criterion::{black_box, criterion_group, BenchmarkId, Criterion};
use std::collections::HashMap;

fn benchmark_map(c: &mut Criterion) {
    let mut group = c.benchmark_group("map list");
    let sizes = vec![10, 100, 1000, 10000];

    for size in sizes {
        group.bench_function(BenchmarkId::new("ast", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.map(x, x * 2)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            b.iter(|| program.execute(black_box(&ctx)).unwrap())
        });

        group.bench_function(BenchmarkId::new("vm", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.map(x, x * 2)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            let vm = program.compile_vm().unwrap();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
        });
    }
    group.finish();
}

fn benchmark_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter list");
    let sizes = vec![10, 100, 1000, 10000];

    for size in sizes {
        group.bench_function(BenchmarkId::new("ast", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.filter(x, x > 50)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            b.iter(|| program.execute(black_box(&ctx)).unwrap())
        });

        group.bench_function(BenchmarkId::new("vm", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.filter(x, x > 50)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            let vm = program.compile_vm().unwrap();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
        });
    }
    group.finish();
}

fn benchmark_all(c: &mut Criterion) {
    let mut group = c.benchmark_group("all list");
    let sizes = vec![10, 100, 1000, 10000];

    for size in sizes {
        group.bench_function(BenchmarkId::new("ast", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.all(x, x >= 0)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            b.iter(|| program.execute(black_box(&ctx)).unwrap())
        });

        group.bench_function(BenchmarkId::new("vm", size), |b| {
            let list: Vec<i64> = (0..size).collect();
            let program = Program::compile("list.all(x, x >= 0)").unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("list", list);
            let vm = program.compile_vm().unwrap();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
        });
    }
    group.finish();
}

fn benchmark_micro(c: &mut Criterion) {
    let mut group = c.benchmark_group("micro");

    group.bench_function("ast index", |b| {
        let program = Program::compile("[1, 2, 3][1]").unwrap();
        let ctx = Context::default();
        b.iter(|| program.execute(black_box(&ctx)).unwrap())
    });

    group.bench_function("vm index", |b| {
        let program = Program::compile("[1, 2, 3][1]").unwrap();
        let ctx = Context::default();
        let vm = program.compile_vm().unwrap();
        b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
    });

    group.bench_function("ast arith", |b| {
        let program = Program::compile("1 + 2 * 3 - 4 / 5").unwrap();
        let ctx = Context::default();
        b.iter(|| program.execute(black_box(&ctx)).unwrap())
    });

    group.bench_function("vm arith", |b| {
        let program = Program::compile("1 + 2 * 3 - 4 / 5").unwrap();
        let ctx = Context::default();
        let vm = program.compile_vm().unwrap();
        b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
    });

    group.bench_function("ast select", |b| {
        let program = Program::compile("foo.bar").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable_from_value("foo", HashMap::from([("bar", 42i64)]));
        b.iter(|| program.execute(black_box(&ctx)).unwrap())
    });

    group.bench_function("vm select", |b| {
        let program = Program::compile("foo.bar").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable_from_value("foo", HashMap::from([("bar", 42i64)]));
        let vm = program.compile_vm().unwrap();
        b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx)).unwrap())
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = benchmark_map, benchmark_filter, benchmark_all, benchmark_micro
}

criterion::criterion_main!(benches);
