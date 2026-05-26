use cel::{Context, Program};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use std::sync::Arc;

// ── 1. Arithmetic chains ──────────────────────────────────────────────────
fn criterion_arithmetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("arithmetic");
    let expressions = [
        ("add2", "1 + 2"),
        ("add3", "1 + 2 + 3"),
        ("mul_chain", "2 * 3 * 4 * 5"),
        ("mixed", "(a + b) * (c - d) / e"),
        ("poly", "x * x + 2 * x + 1"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        ctx.add_variable_from_value("a", 10i64);
        ctx.add_variable_from_value("b", 20i64);
        ctx.add_variable_from_value("c", 100i64);
        ctx.add_variable_from_value("d", 5i64);
        ctx.add_variable_from_value("e", 3i64);
        ctx.add_variable_from_value("x", 7i64);

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

// ── 2. String operations ──────────────────────────────────────────────────
fn criterion_string_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_ops");
    let expressions = [
        ("concat", "'hello' + ' ' + 'world'"),
        ("size_static", "'hello'.size()"),
        ("size_dynamic", "s.size()"),
        ("contains", "s.contains('foo')"),
        ("starts_with", "s.startsWith('prefix')"),
        ("ends_with", "s.endsWith('suffix')"),
        ("triple_eq", "s == t && t == u"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        ctx.add_variable_from_value("s", "prefix_foo_suffix");
        ctx.add_variable_from_value("t", "prefix_foo_suffix");
        ctx.add_variable_from_value("u", "prefix_foo_suffix");

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

// ── 3. Regex matching ─────────────────────────────────────────────────────
#[cfg(feature = "regex")]
fn criterion_regex(c: &mut Criterion) {
    let mut group = c.benchmark_group("regex");
    let expressions = [
        ("match_email", "s.matches(r'^[a-z]+@[a-z]+\\.[a-z]+$')"),
        ("match_ip", "s.matches(r'^(\\d{1,3}\\.){3}\\d{1,3}$')"),
        ("match_uuid", "s.matches(r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$')"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        ctx.add_variable_from_value("s", "user@example.com");

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

#[cfg(not(feature = "regex"))]
fn criterion_regex(_: &mut Criterion) {}

// ── 4. Non-boolean scalar results ─────────────────────────────────────────
fn criterion_scalar_results(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalar");
    let expressions = [
        ("return_int", "42"),
        ("return_str", "'hello'"),
        ("return_var_int", "port"),
        ("return_var_str", "method"),
        ("return_select", "request.path"),
        ("return_index", "items[2]"),
        ("return_ternary_int", "true ? 1 : 2"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        ctx.add_variable_from_value("port", 8080i64);
        ctx.add_variable_from_value("method", "GET");
        let mut req = HashMap::new();
        req.insert(
            cel::objects::Key::String(Arc::new("path".to_string())),
            cel::objects::Value::String(Arc::new("/api/v1/users".to_string())),
        );
        ctx.add_variable_from_value("request", cel::objects::Value::Map(cel::objects::Map { map: Arc::new(req) }));
        ctx.add_variable_from_value("items", vec![10i64, 20i64, 30i64, 40i64]);

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

// ── 5. Deep selects & map access ──────────────────────────────────────────
fn criterion_deep_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("deep_access");
    let expressions = [
        ("depth_1", "a.b"),
        ("depth_2", "a.b.c"),
        ("depth_3", "a.b.c.d"),
        ("depth_4", "a.b.c.d.e"),
        ("has_depth_2", "has(a.b.c)"),
        ("has_depth_3", "has(a.b.c.d)"),
    ];

    fn make_nested(depth: usize) -> cel::objects::Value {
        // Build from the inside out: depth=1 => {"b": 42}, depth=2 => {"b": {"c": 42}}, etc.
        let mut current = cel::objects::Value::Int(42);
        for i in (2..=depth + 1).rev() {
            let mut m = HashMap::new();
            let key = match i {
                5 => "e", 4 => "d", 3 => "c", 2 => "b",
                _ => panic!("depth too high"),
            };
            m.insert(
                cel::objects::Key::String(Arc::new(key.to_string())),
                current,
            );
            current = cel::objects::Value::Map(cel::objects::Map { map: Arc::new(m) });
        }
        current
    }

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        ctx.add_variable_from_value("a", make_nested(4));

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

// ── 6. Complex boolean expressions ────────────────────────────────────────
fn criterion_complex_bool(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_bool");
    let expressions = [
        ("and_2", "a && b"),
        ("and_4", "a && b && c && d"),
        ("and_8", "a && b && c && d && e && f && g && h"),
        ("or_4", "a || b || c || d"),
        ("mixed_4", "(a || b) && (c || d)"),
        ("mixed_8", "(a || b) && (c || d) && (e || f) && (g || h)"),
        ("not_and", "!(a && b) || c"),
        ("nested_ternary", "a ? (b ? c : d) : (e ? f : g)"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let mut ctx = Context::default();
        for var in ['a','b','c','d','e','f','g','h'] {
            ctx.add_variable_from_value(var.to_string(), true);
        }

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

// ── 7. Function calls ─────────────────────────────────────────────────────
fn criterion_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("functions");
    let expressions = [
        ("max_3", "max(1, 2, 3)"),
        ("max_5", "max(1, 2, 3, 4, 5)"),
        ("min_3", "min(5, 3, 7)"),
        ("size_list", "[1,2,3,4,5].size()"),
        ("size_map", "{1:2, 3:4}.size()"),
        ("size_str", "'hello'.size()"),
        ("duration", "duration('1h30m')"),
        ("timestamp", "timestamp('2023-05-28T00:00:00Z')"),
    ];

    for (name, expr) in &expressions {
        let ast = Program::compile(expr).expect("parse");
        let vm = ast.compile_vm().expect("vm compile");
        let ctx = Context::default();

        group.bench_function(BenchmarkId::new("ast", name), |b| {
            b.iter(|| ast.execute(black_box(&ctx)).expect("eval"))
        });
        group.bench_function(BenchmarkId::new("vm", name), |b| {
            let mut state = cel::vm::EvalState::new();
            b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
        });
    }
    group.finish();
}

criterion_group! (
    name = benches;
    config = Criterion::default();
    targets = criterion_arithmetic, criterion_string_ops, criterion_regex,
              criterion_scalar_results, criterion_deep_access, criterion_complex_bool,
              criterion_functions
);

criterion_main!(benches);
