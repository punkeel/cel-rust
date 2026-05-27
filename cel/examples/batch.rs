//! Comprehensive CEL fast-path benchmark suite.
//!
//! Tests function calls, OR patterns, batch evaluation,
//! and the AST fallback cost.
//!
//! Run: `cargo run --example batch --release`

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::filter_tree::FilterNode;
use cel::Program;
use std::sync::Arc;
use std::time::Instant;

fn bench_us<F: FnMut()>(name: &str, mut f: F) -> f64 {
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 100 { f(); }
    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 750 {
        let batch = Instant::now();
        if name.contains("100 rules") || name.contains("batch") {
            for _ in 0..100 { f(); }
        } else {
            for _ in 0..1000 { f(); }
        }
        times.push(batch.elapsed().as_nanos() as f64 / 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    let ns = if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] };
    println!("  {:32} {:>8.1} ns", name, ns);
    ns
}

fn main() {
    println!("=== CEL Fast-Path Deep Benchmarks ===\n");

    // ═══════════ 1. Individual patterns — fast path vs AST fallback ═══════════

    println!("--- Individual patterns ---\n");

    // Helper: schema with common fields
    fn setup_schema() -> (Schema, FieldIndexes) {
        let mut s = Schema::new();
        let port   = s.add_field("port", FieldType::Int);
        let method = s.add_field("method", FieldType::String);
        let path   = s.add_field("path", FieldType::String);
        (s, FieldIndexes { port, method, path })
    }

    struct FieldIndexes { port: cel::fast::Field, method: cel::fast::Field, path: cel::fast::Field }

    // 1a. Int OR: port == 80 || port == 443
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("port == 80 || port == 443", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(f.port, 80);
        bench_us("int OR (fast path)", || { filter.eval_bool(&ctx); });

        let mut ctx = EvalContext::new(&s);
        let p = Program::compile("port == 80 || port == 443").unwrap();
        let mut c = Context::default();
        c.add_variable("port", 80i64).unwrap();
        bench_us("int OR (AST)", || { p.execute(&c).unwrap(); });
    }

    // 1b. String OR: method == "GET" || method == "POST"
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("method == 'GET' || method == 'POST'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(f.method, "GET");
        bench_us("str OR (fast path)", || { filter.eval_bool(&ctx); });

        let p = Program::compile("method == 'GET' || method == 'POST'").unwrap();
        let mut c = Context::default();
        c.add_variable("method", "GET".to_string()).unwrap();
        bench_us("str OR (AST)", || { p.execute(&c).unwrap(); });
    }

    // 1c. String method: path.startsWith("/api")
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("path.startsWith('/api')", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(f.path, "/api/v1/users");
        bench_us("startsWith (fast path)", || { filter.eval_bool(&ctx); });
    }

    // 1d. Function call: size(path) > 5 — uses GeExpr (I64Expr path)
    // This compiles to FilterNode::GtExpr, which doesn't get a closure
    // but uses eval_fast (full Value match with get_unchecked)
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("size(path) > 5", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(f.path, "/api/v1/users");
        bench_us("size(path) > 5 (fast path fallback)", || { filter.eval(&ctx).unwrap(); });

        let p = Program::compile("size(path) > 5").unwrap();
        let mut c = Context::default();
        c.add_variable("path", "/api/v1/users".to_string()).unwrap();
        bench_us("size(path) > 5 (AST)", || { p.execute(&c).unwrap(); });
    }

    // 1e. Regex: path.matches("^/api") — needs Env for function resolution
    // Filter::compile won't work; use Program::compile_with_env for AST
    {
        let env = cel::Env::stdlib();
        let p = Program::compile_with_env(r#"path.matches("^/api")"#, &env).unwrap();
        let mut c = Context::default();
        c.add_variable("path", "/api/v1/users".to_string()).unwrap();
        bench_us("regex (AST only)", || { p.execute(&c).unwrap(); });
    }

    // 1f. Arith cmp: port + 100 >= 1024 (fused)
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("port + 100 >= 1024", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(f.port, 2000);
        bench_us("arith cmp (fast path)", || { filter.eval_bool(&ctx); });
    }

    println!();

    // ═══════════ 2. Batch evaluation patterns ═══════════

    println!("--- Batch patterns ---\n");

    // 2a. One rule, many contexts: compile once, eval with different values
    {
        let (s, f) = setup_schema();
        let filter = Filter::compile("port == 80 || port == 443", &s).unwrap();
        let ctx = EvalContext::new(&s);
        bench_us("1 rule, 1 ctx (set+i64+eval)", || {
            // Simulates reusing ctx for each request
            let mut c = EvalContext::new(&s);
            c.set_i64(f.port, 80);
            filter.eval_bool(&c);
        });

        // Reuse the same ctx (just set + eval + clear)
        let mut ctx2 = EvalContext::new(&s);
        bench_us("1 rule, same ctx (set+eval+clear)", || {
            ctx2.set_i64(f.port, 80);
            filter.eval_bool(&ctx2);
            ctx2.clear();
        });
    }

    // 2b. Many rules (100), one context
    {
        let (s, _) = setup_schema();
        // Generate 100 rules
        let mut rules: Vec<Filter> = (0..100).map(|i| {
            let p = (i * 100) % 65535;
            let rule = format!("port >= {} && port < {}", p, p + 100);
            Filter::compile(&rule, &s).unwrap()
        }).collect();
        // Filter::compile adds Expression which is not Clone, just recreate
        let rules: Vec<Filter> = (0..100).map(|i| {
            let p = (i * 100) % 65535;
            let rule = format!("port >= {} && port < {}", p, p + 100);
            Filter::compile(&rule, &s).unwrap()
        }).collect();

        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(FieldIndexes { port: s.get_field("port").unwrap(), method: s.get_field("method").unwrap(), path: s.get_field("path").unwrap() }.port, 80);

        let len = rules.len();
        bench_us("100 rules, 1 ctx (eval all)", || {
            for i in 0..len {
                std::hint::black_box(rules[i].eval_bool(&ctx));
            }
        });
    }

    // 2c. Rule compilation overhead
    {
        let (s, f) = setup_schema();
        bench_us("compile 'port == 80' once", || {
            let _ = Filter::compile("port == 80", &s).unwrap();
        });
    }

    // ═══════════ 3. Closure dispatch vs enum dispatch (same benchmark) ═══════════

    println!("--- Dispatch cost analysis ---\n");

    // Manual FilterNode::eval_fast_typed vs closure
    {
        let ints = vec![80i64];
        let strings: Vec<Arc<str>> = vec![Arc::from("")];
        let f: Box<FilterNode> = Box::new(FilterNode::EqInt { idx: 0, val: 80 });
        bench_us("FilterNode::eval_fast_typed", || {
            std::hint::black_box(unsafe { f.eval_fast_typed(&ints, &strings) });
        });

        let closure: Box<dyn Fn(&[i64], &[Arc<str>]) -> bool> =
            Box::new(|ints: &[i64], _: &[Arc<str>]| ints[0] == 80);
        bench_us("Box<dyn Fn> closure call", || {
            std::hint::black_box(closure(&ints, &strings));
        });

        // Direct inline for reference
        bench_us("direct inline i64 compare", || {
            std::hint::black_box(ints[0] == 80);
        });
    }

    println!();
}
