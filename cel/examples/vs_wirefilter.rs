//! Benchmark: CEL AST vs Tree vs cel::fast vs Wirefilter.
//!
//! Shows two cel::fast columns:
//!   - "eval": set values once, eval many (same as Wirefilter/Tree benchmarks)
//!   - "all":  set + eval each iteration (real per-request usage)
//!
//! Run: `cargo run --example vs_wirefilter --release`

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::filter_tree::{BoolFilter, EqIntConst, EqStrConst, GeIntConst, InIntLinearSet, LeIntConst};
use cel::Program;
use std::sync::Arc;
use std::time::Instant;

fn median_ns<F: FnMut()>(name: &str, label: &str, mut f: F) -> f64 {
    // Warmup
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 50 { f(); }

    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 500 {
        let batch = Instant::now();
        for _ in 0..1000 { f(); }
        times.push(batch.elapsed().as_nanos() as f64 / 1000.0);
    }

    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    let ns = if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] };
    // comment out individual lines to keep output clean
    // println!("  {:28} {:8.1} ns", format!("{} ({})", name, label), ns);
    ns
}

fn main() {
    println!("=== CEL vs Wirefilter (fair comparison) ===\n");

    // ── Helper: wirefilter int benchmark ──
    fn wf_int(expr: &str, field: &str, val: i64) -> f64 {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field(field, Type::Int).unwrap();
        let s = b.build();
        let f = s.parse(expr).unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field(field).unwrap(), val).unwrap();
        median_ns(expr, "Wirefilter", || { std::hint::black_box(f.execute(&ctx).unwrap()); })
    }
    fn wf_bytes(expr: &str, field: &str, val: &str) -> f64 {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field(field, Type::Bytes).unwrap();
        let s = b.build();
        let f = s.parse(expr).unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field(field).unwrap(), val).unwrap();
        median_ns(expr, "Wirefilter", || { std::hint::black_box(f.execute(&ctx).unwrap()); })
    }

    // ── Helper: cel::fast benchmark in two modes ──
    struct FastBench {
        t_eval: f64,  // set once, eval many (like wirefilter)
        t_all: f64,   // set + eval each iteration
    }

    fn bench_fast(setup: impl Fn(&mut Schema, &mut EvalContext, &mut Filter), label: &str) -> FastBench {
        let mut s = Schema::new();
        let mut ctx = EvalContext::new(&s);
        let f = Filter::compile("port == 0", &s).unwrap(); // placeholder
        // The setup closure receives mutable refs and sets up schema/ctx/filter
        // We can't pass filter by ref easily, so we do it inline at each callsite.
        todo!()
    }

    // ═══════════ 1. port == 80 ═══════════════════════════════════════════

    let t1_tree = {
        let v = vec![Value::Int(80)];
        let f: Box<dyn BoolFilter> = Box::new(EqIntConst { var_idx: 0, val: 80 });
        median_ns("port == 80", "Tree", || { std::hint::black_box(f.eval(&v)); })
    };

    // cel::fast — eval only (set once, like wirefilter)
    let t1_fast_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port == 80", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 80);
        median_ns("port == 80", "cel::fast (eval)", || {
            std::hint::black_box(filter.eval_bool(&ctx));
        })
    };

    // cel::fast — set + eval each iteration (real per-request)
    let t1_fast_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port == 80", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns("port == 80", "cel::fast (all)", || {
            ctx.set_i64(port, 80);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let t1_ast = {
        let p = Program::compile("port == 80").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns("port == 80", "AST", || {
            std::hint::black_box(p.execute(&ctx).unwrap());
        })
    };

    let t1_wf = wf_int("port == 80", "port", 80);

    // ═══════════ 2. method == "GET" ═══════════════════════════════════════

    let t2_tree = {
        let v = vec![Value::String(Arc::from("GET"))];
        let f: Box<dyn BoolFilter> = Box::new(EqStrConst { var_idx: 0, val: "GET".to_string() });
        median_ns("method == GET", "Tree", || { std::hint::black_box(f.eval(&v)); })
    };

    let t2_fast_eval = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let filter = Filter::compile("method == 'GET'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(method, "GET");
        median_ns("method == GET", "cel::fast (eval)", || {
            std::hint::black_box(filter.eval_bool(&ctx));
        })
    };

    let t2_fast_all = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let filter = Filter::compile("method == 'GET'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns("method == GET", "cel::fast (all)", || {
            ctx.set_str(method, "GET");
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let t2_ast = {
        let p = Program::compile("method == 'GET'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        median_ns("method == GET", "AST", || {
            std::hint::black_box(p.execute(&ctx).unwrap());
        })
    };

    let t2_wf = wf_bytes(r#"method == "GET""#, "method", "GET");

    // ═══════════ 3. port range ════════════════════════════════════════════

    let t3_tree = {
        let v = vec![Value::Int(2000)];
        let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::And {
            a: GeIntConst { var_idx: 0, val: 1024 },
            b: LeIntConst { var_idx: 0, val: 65535 },
        });
        median_ns("port range", "Tree", || { std::hint::black_box(f.eval(&v)); })
    };

    let t3_fast_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port >= 1024 && port < 65535", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 2000);
        median_ns("port range", "cel::fast (eval)", || {
            std::hint::black_box(filter.eval_bool(&ctx));
        })
    };

    let t3_fast_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port >= 1024 && port < 65535", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns("port range", "cel::fast (all)", || {
            ctx.set_i64(port, 2000);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let t3_ast = {
        let p = Program::compile("port >= 1024 && port < 65535").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 2000i64).unwrap();
        median_ns("port range", "AST", || {
            std::hint::black_box(p.execute(&ctx).unwrap());
        })
    };

    let t3_wf = wf_int("port >= 1024 && port < 65535", "port", 2000);

    // ═══════════ 4. port IN set ═══════════════════════════════════════════

    let t4_tree = {
        let v = vec![Value::Int(80)];
        let f: Box<dyn BoolFilter> = Box::new(InIntLinearSet { var_idx: 0, vals: vec![80, 443, 8080, 3000] });
        median_ns("port IN set", "Tree", || { std::hint::black_box(f.eval(&v)); })
    };

    let t4_fast_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port in [80, 443, 8080, 3000]", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 80);
        median_ns("port IN set", "cel::fast (eval)", || {
            std::hint::black_box(filter.eval_bool(&ctx));
        })
    };

    let t4_fast_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port in [80, 443, 8080, 3000]", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns("port IN set", "cel::fast (all)", || {
            ctx.set_i64(port, 80);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let t4_ast = {
        let p = Program::compile("port in [80, 443, 8080, 3000]").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns("port IN set", "AST", || {
            std::hint::black_box(p.execute(&ctx).unwrap());
        })
    };

    let t4_wf = wf_int("port in { 80 443 8080 3000 }", "port", 80);

    // ═══════════ 5. multi-field ═══════════════════════════════════════════

    let t5_tree = {
        let v = vec![
            Value::String(Arc::from("GET")),
            Value::String(Arc::from("/api")),
        ];
        let a: Box<dyn BoolFilter> = Box::new(EqStrConst { var_idx: 0, val: "GET".to_string() });
        let b: Box<dyn BoolFilter> = Box::new(EqStrConst { var_idx: 1, val: "/api".to_string() });
        let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::And { a, b });
        median_ns("multi-field", "Tree", || { std::hint::black_box(f.eval(&v)); })
    };

    let t5_fast_eval = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let path = s.add_field("path", FieldType::String);
        let filter = Filter::compile("method == 'GET' && path == '/api'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(method, "GET");
        ctx.set_str(path, "/api");
        median_ns("multi-field", "cel::fast (eval)", || {
            std::hint::black_box(filter.eval_bool(&ctx));
        })
    };

    let t5_fast_all = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let path = s.add_field("path", FieldType::String);
        let filter = Filter::compile("method == 'GET' && path == '/api'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns("multi-field", "cel::fast (all)", || {
            ctx.set_str(method, "GET");
            ctx.set_str(path, "/api");
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let t5_ast = {
        let p = Program::compile("method == 'GET' && path == '/api'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        ctx.add_variable("path", "/api".to_string()).unwrap();
        median_ns("multi-field", "AST", || {
            std::hint::black_box(p.execute(&ctx).unwrap());
        })
    };

    let t5_wf = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("method", Type::Bytes).unwrap();
        b.add_field("path", Type::Bytes).unwrap();
        let s = b.build();
        let f = s.parse(r#"method == "GET" && path == "/api""#).unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("method").unwrap(), "GET").unwrap();
        ctx.set_field_value(s.get_field("path").unwrap(), "/api").unwrap();
        median_ns("multi-field", "Wirefilter", || {
            std::hint::black_box(f.execute(&ctx).unwrap());
        })
    };

    // ═══════════ Summary ═════════════════════════════════════════════════

    println!("{:24} {:>10} {:>10} {:>10} {:>10} {:>10}", "", "Tree", "cel::fast", "cel::fast", "AST", "Wirefilter");
    println!("{:24} {:>10} {:>10} {:>10} {:>10} {:>10}", "", "", "(eval)", "(set+eval)", "", "");
    println!("{}", "-".repeat(74));
    println!("{:24} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}", "port == 80", t1_tree, t1_fast_eval, t1_fast_all, t1_ast, t1_wf);
    println!("{:24} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}", "method == GET", t2_tree, t2_fast_eval, t2_fast_all, t2_ast, t2_wf);
    println!("{:24} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}", "port range", t3_tree, t3_fast_eval, t3_fast_all, t3_ast, t3_wf);
    println!("{:24} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}", "port IN set", t4_tree, t4_fast_eval, t4_fast_all, t4_ast, t4_wf);
    println!("{:24} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}", "multi-field", t5_tree, t5_fast_eval, t5_fast_all, t5_ast, t5_wf);
    println!("{}", "=".repeat(74));

    let w_int_eq = 0.35;
    let w_str_eq = 0.15;
    let w_in_set = 0.15;
    let w_range  = 0.25; // range + arith_cmp proxy
    let w_multi  = 0.10;

    let tree_score   = t1_tree*w_int_eq + t2_tree*w_str_eq + t4_tree*w_in_set + t3_tree*w_range + t5_tree*w_multi;
    let fast_eval    = t1_fast_eval*w_int_eq + t2_fast_eval*w_str_eq + t4_fast_eval*w_in_set + t3_fast_eval*w_range + t5_fast_eval*w_multi;
    let fast_all     = t1_fast_all*w_int_eq + t2_fast_all*w_str_eq + t4_fast_all*w_in_set + t3_fast_all*w_range + t5_fast_all*w_multi;
    let ast_score    = t1_ast*w_int_eq + t2_ast*w_str_eq + t4_ast*w_in_set + t3_ast*w_range + t5_ast*w_multi;
    let wf_score     = t1_wf*w_int_eq + t2_wf*w_str_eq + t4_wf*w_in_set + t3_wf*w_range + t5_wf*w_multi;

    println!("\nWeighted score:\n");
    println!("{:24} {:>10.1} ns", "Tree (eval only)", tree_score);
    println!("{:24} {:>10.1} ns", "cel::fast (eval only)", fast_eval);
    println!("{:24} {:>10.1} ns", "cel::fast (set + eval)", fast_all);
    println!("{:24} {:>10.1} ns", "AST", ast_score);
    println!("{:24} {:>10.1} ns", "Wirefilter", wf_score);
    println!();
    println!("{:24} {:>8.1}x", "Tree vs Wirefilter", wf_score / tree_score);
    println!("{:24} {:>8.1}x", "cel::fast (eval) vs Wirefilter", wf_score / fast_eval);
    println!("{:24} {:>8.1}x", "cel::fast (all) vs Wirefilter", wf_score / fast_all);
}
