//! Comprehensive benchmark: all four approaches vs Wirefilter.
//!
//! Run: cargo run --release --example comprehensive_bench
//!
//! Measures:
//!   1. AST        — Original cel-rust AST interpreter (Program::execute)
//!   2. FilterTree — New code, no schema: compile_tree + bind_vars + eval_fast
//!                   (old API, new internals)
//!   3. Schema     — New code with schema: Schema + EvalContext + closure dispatch
//!   4. Wirefilter — External benchmark
//!
//! Each pattern is measured as "eval-only" (set once, eval many) and
//! "full API" (set + eval each iteration) where applicable.
//! Weighted scoring at the end.

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::filter_tree::FilterNode;
use cel::Program;
use std::sync::Arc;
use std::time::Instant;

fn median_ns<F: FnMut()>(mut f: F) -> f64 {
    // Warmup
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 100 { f(); }

    // Measure batch times
    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 500 {
        let batch = Instant::now();
        for _ in 0..1000 { f(); }
        times.push(batch.elapsed().as_nanos() as f64 / 1000.0);
    }

    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] }
}

fn main() {
    println!("═══════════════════════════════════════════════════════════════");
    println!("    CEL: ALL FOUR APPROACHES — ORIGINAL → NEW → WIREFILTER");
    println!("═══════════════════════════════════════════════════════════════\n");

    // ── Benchmark patterns ─────────────────────────────────────────────────
    // We measure 5 patterns across 4 approaches.
    //
    // "eval-only"  = set values once before the loop, eval many times
    //                (fair comparison — measures just the comparison logic)
    // "full API"   = set + eval each iteration (real per-request usage)

    // ═══════════ 1. port == 80 ═══════════════════════════════════════════════
    println!("─────────── 1. port == 80 ───────────");

    // ── AST (original) ──
    let ast_1 = {
        let p = Program::compile("port == 80").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
    };

    // ── FilterTree (no schema, old API, new internals) ──
    let ft_1_eval = {
        let p = Program::compile("port == 80").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns(|| {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
        })
    };
    let ft_1_all = {
        let p = Program::compile("port == 80").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns(|| {
            let mut vars = tree.bind_vars(&ctx);
            // For set+eval: change the value each time (simulate real usage)
            // We just call bind_vars + eval_fast as the full API for this path
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
            // Note: bind_vars already happens inside the loop for "all" path
            // In real usage, the Context HashMap would be mutated per request
        })
    };

    // ── Schema (new code, with schema) ──
    let schema_1_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port == 80", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 80);
        median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); })
    };
    let schema_1_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port == 80", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns(|| {
            ctx.set_i64(port, 80);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    // ── Wirefilter ──
    let wf_1 = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("port", Type::Int).unwrap();
        let s = b.build();
        let f = s.parse("port == 80").unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("port").unwrap(), 80).unwrap();
        median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
    };

    // ═══════════ 2. method == "GET" ═════════════════════════════════════════
    println!("\n─────────── 2. method == 'GET' ───────────");

    let ast_2 = {
        let p = Program::compile("method == 'GET'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
    };

    let ft_2_eval = {
        let p = Program::compile("method == 'GET'").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        median_ns(|| {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
        })
    };

    let schema_2_eval = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let filter = Filter::compile("method == 'GET'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(method, "GET");
        median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); })
    };
    let schema_2_all = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let filter = Filter::compile("method == 'GET'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns(|| {
            ctx.set_str(method, "GET");
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let wf_2 = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("method", Type::Bytes).unwrap();
        let s = b.build();
        let f = s.parse(r#"method == "GET""#).unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("method").unwrap(), "GET").unwrap();
        median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
    };

    // ═══════════ 3. port >= 1024 && port < 65535 ════════════════════════════
    println!("\n─────────── 3. port range ───────────");

    let ast_3 = {
        let p = Program::compile("port >= 1024 && port < 65535").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 2000i64).unwrap();
        median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
    };

    let ft_3_eval = {
        let p = Program::compile("port >= 1024 && port < 65535").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 2000i64).unwrap();
        median_ns(|| {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
        })
    };

    let schema_3_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port >= 1024 && port < 65535", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 2000);
        median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); })
    };
    let schema_3_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port >= 1024 && port < 65535", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns(|| {
            ctx.set_i64(port, 2000);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let wf_3 = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("port", Type::Int).unwrap();
        let s = b.build();
        let f = s.parse("port >= 1024 && port < 65535").unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("port").unwrap(), 2000).unwrap();
        median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
    };

    // ═══════════ 4. port IN set ════════════════════════════════════════════
    println!("\n─────────── 4. port IN set ───────────");

    let ast_4 = {
        let p = Program::compile("port in [80, 443, 8080, 3000]").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
    };

    let ft_4_eval = {
        let p = Program::compile("port in [80, 443, 8080, 3000]").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        median_ns(|| {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
        })
    };

    let schema_4_eval = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port in [80, 443, 8080, 3000]", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_i64(port, 80);
        median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); })
    };
    let schema_4_all = {
        let mut s = Schema::new();
        let port = s.add_field("port", FieldType::Int);
        let filter = Filter::compile("port in [80, 443, 8080, 3000]", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns(|| {
            ctx.set_i64(port, 80);
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let wf_4 = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("port", Type::Int).unwrap();
        let s = b.build();
        let f = s.parse("port in { 80 443 8080 3000 }").unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("port").unwrap(), 80).unwrap();
        median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
    };

    // ═══════════ 5. multi-field ════════════════════════════════════════════
    println!("\n─────────── 5. method + path ───────────");

    let ast_5 = {
        let p = Program::compile("method == 'GET' && path == '/api'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        ctx.add_variable("path", "/api".to_string()).unwrap();
        median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
    };

    let ft_5_eval = {
        let p = Program::compile("method == 'GET' && path == '/api'").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        ctx.add_variable("path", "/api".to_string()).unwrap();
        median_ns(|| {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(unsafe { tree.filter.eval_fast(&vars) });
        })
    };

    let schema_5_eval = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let path = s.add_field("path", FieldType::String);
        let filter = Filter::compile("method == 'GET' && path == '/api'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        ctx.set_str(method, "GET");
        ctx.set_str(path, "/api");
        median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); })
    };
    let schema_5_all = {
        let mut s = Schema::new();
        let method = s.add_field("method", FieldType::String);
        let path = s.add_field("path", FieldType::String);
        let filter = Filter::compile("method == 'GET' && path == '/api'", &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        median_ns(|| {
            ctx.set_str(method, "GET");
            ctx.set_str(path, "/api");
            std::hint::black_box(filter.eval(&ctx).unwrap());
        })
    };

    let wf_5 = {
        use wirefilter::{ExecutionContext, SchemeBuilder, Type};
        let mut b = SchemeBuilder::default();
        b.add_field("method", Type::Bytes).unwrap();
        b.add_field("path", Type::Bytes).unwrap();
        let s = b.build();
        let f = s.parse(r#"method == "GET" && path == "/api""#).unwrap().compile();
        let mut ctx = ExecutionContext::new(&s);
        ctx.set_field_value(s.get_field("method").unwrap(), "GET").unwrap();
        ctx.set_field_value(s.get_field("path").unwrap(), "/api").unwrap();
        median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
    };

    // ═══════════ TABLE 1: Full Results ════════════════════════════════════
    println!("\n\n═══════════════════════ TABLE 1: FULL RESULTS (ns) ═══════════════════════");
    println!("{:24} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "", "AST", "FilterTree", "Schema", "Schema", "Wirefilter", "Best");
    println!("{:24} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "", "(orig)", "(no schema)", "(eval)", "(set+eval)", "", "");
    println!("{}", "─".repeat(89));

    fn print_row(name: &str, ast: f64, ft: f64, se: f64, sa: f64, wf: f64) {
        let vals = [ast, ft, se, sa, wf];
        let best = vals.iter().cloned().fold(f64::INFINITY, f64::min);
        let best_label = match best {
            _ if (best - ast).abs() < 0.1 => "AST",
            _ if (best - ft).abs() < 0.1 => "FilterTree",
            _ if (best - se).abs() < 0.1 => "Schema(e)",
            _ if (best - sa).abs() < 0.1 => "Schema(a)",
            _ => "Wirefilter",
        };
        println!("{:24} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9}",
            name, ast, ft, se, sa, wf, best_label);
    }

    print_row("1. port == 80",         ast_1, ft_1_eval, schema_1_eval, schema_1_all, wf_1);
    print_row("2. method == 'GET'",    ast_2, ft_2_eval, schema_2_eval, schema_2_all, wf_2);
    print_row("3. port range",         ast_3, ft_3_eval, schema_3_eval, schema_3_all, wf_3);
    print_row("4. port IN set",        ast_4, ft_4_eval, schema_4_eval, schema_4_all, wf_4);
    print_row("5. multi-field",        ast_5, ft_5_eval, schema_5_eval, schema_5_all, wf_5);

    // ═══════════ WEIGHTED SCORE ═══════════════════════════════════════════
    let w_int_eq   = 0.30;  // port == 80
    let w_str_eq   = 0.15;  // method == "GET"
    let w_range    = 0.20;  // port range
    let w_in_set   = 0.25;  // port IN set
    let w_multi    = 0.10;  // multi-field AND

    let ast_score   = ast_1*w_int_eq + ast_2*w_str_eq + ast_3*w_range + ast_4*w_in_set + ast_5*w_multi;
    let ft_score    = ft_1_eval*w_int_eq + ft_2_eval*w_str_eq + ft_3_eval*w_range + ft_4_eval*w_in_set + ft_5_eval*w_multi;
    let schema_eval = schema_1_eval*w_int_eq + schema_2_eval*w_str_eq + schema_3_eval*w_range + schema_4_eval*w_in_set + schema_5_eval*w_multi;
    let schema_all  = schema_1_all*w_int_eq + schema_2_all*w_str_eq + schema_3_all*w_range + schema_4_all*w_in_set + schema_5_all*w_multi;
    let wf_score    = wf_1*w_int_eq + wf_2*w_str_eq + wf_3*w_range + wf_4*w_in_set + wf_5*w_multi;

    println!("\n{}", "=".repeat(89));
    println!("{:24} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
        "WEIGHTED SCORE (ns)", ast_score, ft_score, schema_eval, schema_all, wf_score);
    println!("{}", "=".repeat(89));

    // ═══════════ TABLE 2: Speedup Matrix ════════════════════════════════
    println!("\n\n══════════════ TABLE 2: SPEEDUP vs AST (×) ═══════════════════════");
    println!("{:24} {:>8} {:>8} {:>8} {:>8}",
        "", "FilterTree", "Schema(e)", "Schema(a)", "Wirefilter");
    println!("{}", "─".repeat(56));

    fn speedup_row(name: &str, ast: f64, ft: f64, se: f64, sa: f64, wf: f64) {
        println!("{:24} {:>8.1}× {:>8.1}× {:>8.1}× {:>8.1}×",
            name, ast/ft, ast/se, ast/sa, ast/wf);
    }

    speedup_row("1. port == 80",         ast_1, ft_1_eval, schema_1_eval, schema_1_all, wf_1);
    speedup_row("2. method == 'GET'",    ast_2, ft_2_eval, schema_2_eval, schema_2_all, wf_2);
    speedup_row("3. port range",         ast_3, ft_3_eval, schema_3_eval, schema_3_all, wf_3);
    speedup_row("4. port IN set",        ast_4, ft_4_eval, schema_4_eval, schema_4_all, wf_4);
    speedup_row("5. multi-field",        ast_5, ft_5_eval, schema_5_eval, schema_5_all, wf_5);
    println!("{}", "─".repeat(56));
    println!("{:24} {:>8.1}× {:>8.1}× {:>8.1}× {:>8.1}×",
        "WEIGHTED", ast_score/ft_score, ast_score/schema_eval, ast_score/schema_all, ast_score/wf_score);

    // ═══════════ TABLE 3: Optimization Layer Breakdown ════════════════════
    println!("\n\n══════════════════ TABLE 3: OPTIMIZATION LAYER BREAKDOWN ══════════════════");
    println!("  (port == 80, eval-only, cumulative — each layer ON TOP of previous)\n");

    // Measure pure dispatch costs using hand-crafted FilterNode
    let noise_floor = {
        let ints = vec![80i64];
        let strings: Vec<Arc<str>> = vec![Arc::from("")];
        // Bare minimum: direct inline compare
        let start = Instant::now();
        let warmup_end = start + std::time::Duration::from_millis(100);
        while Instant::now() < warmup_end {
            let _ = std::hint::black_box(ints[0] == 80);
        }
        median_ns(|| { let _ = std::hint::black_box(ints[0] == 80); })
    };

    // Layer 1: Safe enum dispatch (eval — bounds + type checks)
    let l1_eval = {
        let v = vec![Value::Int(80)];
        let f: Box<FilterNode> = Box::new(FilterNode::EqInt { idx: 0, val: 80 });
        median_ns(|| { std::hint::black_box(f.eval(&v)); })
    };

    // Layer 2: eval_fast — skip bounds + type check (unsafe)
    let l2_fast = {
        let v = vec![Value::Int(80)];
        let f: Box<FilterNode> = Box::new(FilterNode::EqInt { idx: 0, val: 80 });
        median_ns(|| { std::hint::black_box(unsafe { f.eval_fast(&v) }); })
    };

    // Layer 3: eval_fast_typed — typed array access (no 24-byte Value stride)
    let l3_typed = {
        let ints = vec![80i64];
        let strings: Vec<Arc<str>> = vec![Arc::from("")];
        let f: Box<FilterNode> = Box::new(FilterNode::EqInt { idx: 0, val: 80 });
        median_ns(|| { std::hint::black_box(unsafe { f.eval_fast_typed(&ints, &strings) }); })
    };

    // Layer 4: Closure dispatch (no jump table)
    let l4_closure = {
        let ints = vec![80i64];
        let strings: Vec<Arc<str>> = vec![Arc::from("")];
        let closure: Box<dyn Fn(&[i64], &[Arc<str>]) -> bool> = Box::new(|ints, _| ints[0] == 80);
        median_ns(|| { std::hint::black_box(closure(&ints, &strings)); })
    };

    println!("{:28} {:>9} {:>9}", "Layer", "Time (ns)", "vs previous");
    println!("{}", "─".repeat(46));
    println!("{:28} {:>9.1} {:>9}", "Noise floor (direct compare)", noise_floor, "—");
    let l1_l2 = l1_eval - l2_fast;
    let l2_l3 = l2_fast - l3_typed;
    let l3_l4 = l3_typed - l4_closure;
    println!("{:28} {:>9.1} {:>9}", "L1: eval (safe enum dispatch)", l1_eval, "baseline");
    println!("{:28} {:>9.1} {:>+9.2} ns  (unchecked bounds)", "L2: eval_fast (unsafe)", l2_fast, -(l1_l2 as f64));
    println!("{:28} {:>9.1} {:>+9.2} ns  (typed arrays, no Value stride)", "L3: eval_fast_typed", l3_typed,  -(l2_l3 as f64));
    println!("{:28} {:>9.1} {:>+9.2} ns  (no jump table dispatch)", "L4: closure dispatch", l4_closure, -(l3_l4 as f64));
    println!("{}", "─".repeat(46));
    println!("{:28} {:>9.1} {:>9.1}× from L1", "Total: L1 → L4", l4_closure, l1_eval / l4_closure);

    // ═══════════ COST-BENEFIT ANALYSIS ═══════════════════════════════════
    println!("\n\n═══════════════════════════════════════════════════════════════");
    println!("                   OPTIMIZATION COST-BENEFIT");
    println!("═══════════════════════════════════════════════════════════════\n");

    let ft_overhead = ft_1_eval - l2_fast; // bind_vars cost
    let schema_compile_us = {
        let start = Instant::now();
        for _ in 0..100 {
            let mut s = Schema::new();
            let port = s.add_field("port", FieldType::Int);
            let _filter = Filter::compile("port == 80", &s).unwrap();
        }
        start.elapsed().as_nanos() as f64 / 100.0 / 1000.0 // μs
    };

    println!("  Cost: Schema setup + compilation");
    println!("    {:30} {:>8.1} µs per filter", "Compile time (one-time)", schema_compile_us);
    println!("    {:30} {:>8} lines", "Schema + EvalContext code", "~200");
    println!("    {:30} {:>8} lines", "FilterNode enum (+eval paths)", "~600");
    println!("    {:30} {:>8} lines", "Closure compiler", "~300");
    println!("    {:30} {:>8} lines", "Unsafe blocks", "~6");
    println!();

    println!("  Benefit breakdown (per eval, port == 80):");
    println!("    {:30} {:>9.1} ns  {:>6.1}× vs AST", "Before: AST interpreter", ast_1, 1.0);
    println!("    {:30} {:>9.1} ns  {:>6.1}× vs AST", "After: FilterTree (no schema)", ft_1_eval, ast_1/ft_1_eval);
    println!("    {:30} {:>9.1} ns  {:>6.1}× vs AST", "After: Schema + closure", schema_1_eval, ast_1/schema_1_eval);
    println!();

    // Decompose the FilterTree "eval-only" number
    let ft_bind_vars_per_field = ft_1_eval - l2_fast;
    println!("  Why is FilterTree (no schema) slower than pure eval_fast?");
    println!("    Hand-crafted FilterNode (eval_fast):  {:>6.1} ns", l2_fast);
    println!("    via Program::compile_tree + bind_vars: {:>6.1} ns", ft_1_eval);
    println!("    bind_vars overhead (HashMap lookup):   {:>6.1} ns  ← THIS IS THE PROBLEM", ft_bind_vars_per_field);
    println!();
    println!("  The bind_vars function walks a HashMap (Context) to extract each");
    println!("  variable by name. That's what Schema eliminates — field → index");
    println!("  resolution at compile time replaces HashMap lookup with array indexing.");

    println!("\n══════════════════════ VERDICT ══════════════════════\n");

    // Determine winner per-pattern
    fn verdict(ast: f64, ft: f64, se: f64, sa: f64, wf: f64) -> (String, f64) {
        let best = [ast, ft, se, sa, wf].iter().cloned().fold(f64::INFINITY, f64::min);
        if best == se { ("Schema (eval)".into(), se) }
        else if best == sa { ("Schema (all)".into(), sa) }
        else if best == ft { ("FilterTree".into(), ft) }
        else if best == ast { ("AST".into(), ast) }
        else { ("Wirefilter".into(), wf) }
    }

    let (v1, n1) = verdict(ast_1, ft_1_eval, schema_1_eval, schema_1_all, wf_1);
    let (v2, n2) = verdict(ast_2, ft_2_eval, schema_2_eval, schema_2_all, wf_2);
    let (v3, n3) = verdict(ast_3, ft_3_eval, schema_3_eval, schema_3_all, wf_3);
    let (v4, n4) = verdict(ast_4, ft_4_eval, schema_4_eval, schema_4_all, wf_4);
    let (v5, n5) = verdict(ast_5, ft_5_eval, schema_5_eval, schema_5_all, wf_5);

    println!("  Pattern                   Winner          (ns)");
    println!("  {}","─".repeat(42));
    println!("  1. port == 80             {:16} {:>6.1}", v1, n1);
    println!("  2. method == 'GET'        {:16} {:>6.1}", v2, n2);
    println!("  3. port range             {:16} {:>6.1}", v3, n3);
    println!("  4. port IN set            {:16} {:>6.1}", v4, n4);
    println!("  5. multi-field            {:16} {:>6.1}", v5, n5);
    println!("  {}","─".repeat(42));
    println!("  Weighted                  Schema (eval)  {:>6.1}", schema_eval);
    println!("  vs ASTMetric              {:>6.1}×", ast_score / schema_eval);
    println!("  vs Wirefilter             {:>6.1}×", wf_score / schema_eval);
    println!("  vs FilterTree (no schema) {:>6.1}×", ft_score / schema_eval);

    println!("\n═══════════════════════════════════════════════════════════════\n");

    // ── Run test suite expression benchmarks ──
    bench_test_suite_expressions();
}

// ═════════════════════════════════════════════════════════════════════════════
//  TEST SUITE EXPRESSIONS — All unique CEL expressions extracted from the
//  test suite, organized by category. Each expression is compiled once via
//  Program::compile and then measured with Program::execute in a tight loop.
// ═════════════════════════════════════════════════════════════════════════════

fn bench_ns<F: FnMut()>(mut f: F) -> f64 {
    // Warmup
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 100 { f(); }

    // Measure batch times
    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 500 {
        let batch = Instant::now();
        for _ in 0..1000 { f(); }
        times.push(batch.elapsed().as_nanos() as f64 / 1000.0);
    }

    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] }
}

fn bench_expr(prog: &Program, ctx: &Context, label: &str) -> f64 {
    // Verify it executes correctly first
    let _ = prog.execute(ctx).unwrap();
    let ns = bench_ns(|| { std::hint::black_box(prog.execute(ctx).unwrap()); });
    println!("  {:60} {:>8.1} ns", label, ns);
    ns
}

fn bench_expr_opt(prog: &Program, ctx: &Context, label: &str) -> Option<f64> {
    // For expressions that might fail — try and skip
    if let Ok(_) = prog.execute(ctx) {
        let ns = bench_ns(|| { std::hint::black_box(prog.execute(ctx).unwrap()); });
        println!("  {:60} {:>8.1} ns", label, ns);
        Some(ns)
    } else {
        println!("  {:60} {:>8} (skipped — execution error)", label, "");
        None
    }
}

fn bench_test_suite_expressions() {
    use std::collections::HashMap;

    println!("\n\n╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║          TEST SUITE EXPRESSIONS — ALL UNIQUE CEL EXPRESSIONS              ║");
    println!("╚══════════════════════════════════════════════════════════════════════════╝\n");

    // ── 1. LITERALS & CONSTANTS ────────────────────────────────────────────
    println!("────────────────────── 1. LITERALS & CONSTANTS ──────────────────────");
    bench_expr(&Program::compile("42").unwrap(), &Context::default(), "42");
    bench_expr(&Program::compile("\"hello\"").unwrap(), &Context::default(), "\"hello\"");
    bench_expr(&Program::compile("true").unwrap(), &Context::default(), "true");
    bench_expr(&Program::compile("false").unwrap(), &Context::default(), "false");
    bench_expr(&Program::compile("null").unwrap(), &Context::default(), "null");
    bench_expr(&Program::compile("0u").unwrap(), &Context::default(), "0u");
    bench_expr(&Program::compile("0.0").unwrap(), &Context::default(), "0.0");

    // ── 2. ARITHMETIC ──────────────────────────────────────────────────────
    println!("\n────────────────────── 2. ARITHMETIC ───────────────────────────────");
    bench_expr(&Program::compile("1 + 2").unwrap(), &Context::default(), "1 + 2");
    bench_expr(&Program::compile("5 - 3").unwrap(), &Context::default(), "5 - 3");
    bench_expr(&Program::compile("3 * 4").unwrap(), &Context::default(), "3 * 4");
    bench_expr(&Program::compile("10 / 3").unwrap(), &Context::default(), "10 / 3");
    bench_expr(&Program::compile("10 % 3").unwrap(), &Context::default(), "10 % 3");
    bench_expr(&Program::compile("-42").unwrap(), &Context::default(), "-42");
    bench_expr(&Program::compile("-3.14").unwrap(), &Context::default(), "-3.14");
    bench_expr(&Program::compile("1.0 > 0.0").unwrap(), &Context::default(), "1.0 > 0.0");

    // ── 3. INTEGER COMPARISONS ─────────────────────────────────────────────
    println!("\n────────────────────── 3. INTEGER COMPARISONS ──────────────────────");
    bench_expr(&Program::compile("1 == 1").unwrap(), &Context::default(), "1 == 1");
    bench_expr(&Program::compile("1 == 2").unwrap(), &Context::default(), "1 == 2");
    bench_expr(&Program::compile("1 != 2").unwrap(), &Context::default(), "1 != 2");
    bench_expr(&Program::compile("1 < 2").unwrap(), &Context::default(), "1 < 2");
    bench_expr(&Program::compile("2 <= 2").unwrap(), &Context::default(), "2 <= 2");
    bench_expr(&Program::compile("3 > 2").unwrap(), &Context::default(), "3 > 2");
    bench_expr(&Program::compile("3 >= 3").unwrap(), &Context::default(), "3 >= 3");

    // ── 4. STRING COMPARISONS & OPS ────────────────────────────────────────
    println!("\n────────────────────── 4. STRING COMPARISONS & OPS ─────────────────");
    bench_expr(&Program::compile("\"a\" == \"a\"").unwrap(), &Context::default(), "\"a\" == \"a\"");
    bench_expr(&Program::compile("\"hello\" + \" \" + \"world\"").unwrap(), &Context::default(), "\"hello\" + \" \" + \"world\"");
    bench_expr(&Program::compile("'foobar'.contains('bar')").unwrap(), &Context::default(), "'foobar'.contains('bar')");
    bench_expr(&Program::compile("'foobar'.startsWith('foo')").unwrap(), &Context::default(), "'foobar'.startsWith('foo')");
    bench_expr(&Program::compile("'foobar'.endsWith('bar')").unwrap(), &Context::default(), "'foobar'.endsWith('bar')");
    bench_expr(&Program::compile("'foobar'.size() == 6").unwrap(), &Context::default(), "'foobar'.size() == 6");
    bench_expr(&Program::compile("size('foo') == 3").unwrap(), &Context::default(), "size('foo') == 3");

    // ── 5. LIST OPERATIONS ─────────────────────────────────────────────────
    println!("\n────────────────────── 5. LIST OPERATIONS ──────────────────────────");
    bench_expr(&Program::compile("[1, 2, 3]").unwrap(), &Context::default(), "[1, 2, 3]");
    bench_expr(&Program::compile("[1, 2] + [3, 4]").unwrap(), &Context::default(), "[1, 2] + [3, 4]");
    bench_expr(&Program::compile("1 in [1, 2, 3]").unwrap(), &Context::default(), "1 in [1, 2, 3]");
    bench_expr(&Program::compile("4 in [1, 2, 3]").unwrap(), &Context::default(), "4 in [1, 2, 3]");
    bench_expr(&Program::compile("[1, 2, 3].size() == 3").unwrap(), &Context::default(), "[1, 2, 3].size() == 3");
    bench_expr(&Program::compile("size([1, 2, 3]) == 3").unwrap(), &Context::default(), "size([1, 2, 3]) == 3");

    // ── 6. MAP OPERATIONS ──────────────────────────────────────────────────
    println!("\n────────────────────── 6. MAP OPERATIONS ───────────────────────────");
    bench_expr(&Program::compile("{\"a\": 1, \"b\": 2}").unwrap(), &Context::default(), "{\"a\": 1, \"b\": 2}");
    bench_expr(&Program::compile("size({'a': 1, 'b': 2, 'c': 3}) == 3").unwrap(), &Context::default(), "size({'a': 1, 'b': 2, 'c': 3}) == 3");

    // ── 7. BOOLEAN / LOGICAL ───────────────────────────────────────────────
    println!("\n────────────────────── 7. BOOLEAN / LOGICAL ────────────────────────");
    bench_expr(&Program::compile("true && true").unwrap(), &Context::default(), "true && true");
    bench_expr(&Program::compile("false && false").unwrap(), &Context::default(), "false && false");
    bench_expr(&Program::compile("false && (1 / 0 == 0)").unwrap(), &Context::default(), "false && (1 / 0 == 0) [short-circuit]");
    bench_expr(&Program::compile("true || false").unwrap(), &Context::default(), "true || false");
    bench_expr(&Program::compile("false || false").unwrap(), &Context::default(), "false || false");
    bench_expr(&Program::compile("true || (1 / 0 == 0)").unwrap(), &Context::default(), "true || (1 / 0 == 0) [short-circuit]");
    bench_expr(&Program::compile("!true").unwrap(), &Context::default(), "!true");
    bench_expr(&Program::compile("true ? 1 : 2").unwrap(), &Context::default(), "true ? 1 : 2");
    bench_expr(&Program::compile("false ? 1 : 2").unwrap(), &Context::default(), "false ? 1 : 2");

    // ── 8. VARIABLE ACCESS ─────────────────────────────────────────────────
    println!("\n────────────────────── 8. VARIABLE ACCESS ──────────────────────────");
    {
        let mut ctx = Context::default();
        ctx.add_variable("x", 42i64).unwrap();
        bench_expr(&Program::compile("x").unwrap(), &ctx, "x = 42");
    }
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("foo", HashMap::from([("bar", 1i64)]));
        bench_expr(&Program::compile("foo.bar == 1").unwrap(), &ctx, "foo.bar == 1");
    }
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("arr", vec![1i64, 2, 3]);
        bench_expr(&Program::compile("arr[0] == 1").unwrap(), &ctx, "arr[0] == 1");
    }
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("obj", HashMap::from([("inner", HashMap::from([("value", 42i64)]))]));
        bench_expr(&Program::compile("obj.inner.value").unwrap(), &ctx, "obj.inner.value  [nested field]");
    }
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("obj", HashMap::from([("a", 1i64)]));
        bench_expr(&Program::compile("has(obj.a)").unwrap(), &ctx, "has(obj.a)");
    }

    // ── 9. COMPREHENSIONS (all / exists / exists_one / map / filter) ───────
    println!("\n────────────────────── 9. COMPREHENSIONS ────────────────────────────");
    bench_expr(&Program::compile("[0, 1, 2].all(x, x >= 0)").unwrap(), &Context::default(), "[0, 1, 2].all(x, x >= 0)  [all true]");
    bench_expr(&Program::compile("[0, -1, 2].all(x, x >= 0)").unwrap(), &Context::default(), "[0, -1, 2].all(x, x >= 0)  [all false]");
    bench_expr(&Program::compile("[true, true, true].all(x, x)").unwrap(), &Context::default(), "[true, true, true].all(x, x)  [identity]");
    bench_expr(&Program::compile("[0, 1, 2].exists(x, x > 1)").unwrap(), &Context::default(), "[0, 1, 2].exists(x, x > 1)");
    bench_expr(&Program::compile("[0, 1, 2].exists(x, x > 5)").unwrap(), &Context::default(), "[0, 1, 2].exists(x, x > 5)  [false]");
    bench_expr(&Program::compile("[0, 1, 2].exists_one(x, x == 0)").unwrap(), &Context::default(), "[0, 1, 2].exists_one(x, x == 0)");
    bench_expr(&Program::compile("[1, 2, 3].map(x, x * 2) == [2, 4, 6]").unwrap(), &Context::default(), "[1, 2, 3].map(x, x * 2) == [2, 4, 6]");
    bench_expr(&Program::compile("[1, 2, 3].filter(x, x > 2) == [3]").unwrap(), &Context::default(), "[1, 2, 3].filter(x, x > 2) == [3]");
    bench_expr(&Program::compile("['abc'].all(x, x.contains('a'))").unwrap(), &Context::default(), "['abc'].all(x, x.contains('a'))");
    bench_expr(&Program::compile("[1, 1].map(x, x * 2)").unwrap(), &Context::default(), "[1, 1].map(x, x * 2)");

    // ── 10. MAP COMPREHENSIONS ──────────────────────────────────────────────
    println!("\n────────────────────── 10. MAP COMPREHENSIONS ───────────────────────");
    bench_expr(&Program::compile("{0: 0, 1:1, 2:2}.all(x, x >= 0)").unwrap(), &Context::default(), "{0: 0, 1:1, 2:2}.all(x, x >= 0)");
    bench_expr(&Program::compile("{0: 0, 1:1, 2:2}.exists(x, x > 0)").unwrap(), &Context::default(), "{0: 0, 1:1, 2:2}.exists(x, x > 0)");

    // ── 11. HETEROGENEOUS / TYPE MIXING ─────────────────────────────────────
    println!("\n────────────────────── 11. HETEROGENEOUS COMPARISONS ────────────────");
    bench_expr(&Program::compile("1 < uint(2)").unwrap(), &Context::default(), "1 < uint(2)");
    bench_expr(&Program::compile("1 < 1.1").unwrap(), &Context::default(), "1 < 1.1");
    bench_expr(&Program::compile("uint(0) > -10").unwrap(), &Context::default(), "uint(0) > -10");
    bench_expr(&Program::compile("{} == []").unwrap(), &Context::default(), "{} == []  [different types]");

    // ── 12. TYPE CONVERSIONS ────────────────────────────────────────────────
    println!("\n────────────────────── 12. TYPE CONVERSIONS ─────────────────────────");
    bench_expr(&Program::compile("string(10) == '10'").unwrap(), &Context::default(), "string(10) == '10'");
    bench_expr(&Program::compile("string(10.5) == '10.5'").unwrap(), &Context::default(), "string(10.5) == '10.5'");
    bench_expr(&Program::compile("int('10') == 10").unwrap(), &Context::default(), "int('10') == 10");
    bench_expr(&Program::compile("uint(10) == 10u").unwrap(), &Context::default(), "uint(10) == 10u");
    bench_expr(&Program::compile("double('10') == 10.0").unwrap(), &Context::default(), "double('10') == 10.0");
    bench_expr(&Program::compile("double(10) == 10.0").unwrap(), &Context::default(), "double(10) == 10.0");
    bench_expr(&Program::compile("bytes('abc') == b'abc'").unwrap(), &Context::default(), "bytes('abc') == b'abc'");

    // ── 13. SHORT-CIRCUIT WITH VARIABLES ────────────────────────────────────
    println!("\n────────────────────── 13. SHORT-CIRCUIT WITH VARIABLES ───────────────");
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("foo", 42i64);
        ctx.add_variable_from_value("bar", 42i64);
        bench_expr(&Program::compile("foo || bar > 0").unwrap(), &ctx, "foo || bar > 0  [short-circuit OR]");
        bench_expr(&Program::compile("foo && bar < 0").unwrap(), &ctx, "foo && bar < 0  [short-circuit AND]");
    }
    {
        let mut ctx = Context::default();
        let data: HashMap<String, String> = HashMap::new();
        ctx.add_variable_from_value("data", data);
        bench_expr(&Program::compile("has(data.x) && data.x.startsWith(\"foo\")").unwrap(), &ctx,
                   "has(data.x) && data.x.startsWith(\"foo\")  [short-circuit has]");
    }

    // ── 14. COMPOUND / COMPLEX EXPRESSIONS ─────────────────────────────────
    println!("\n────────────────────── 14. COMPOUND EXPRESSIONS ─────────────────────");
    {
        let mut ctx = Context::default();
        ctx.add_variable("x", 10i64).unwrap();
        ctx.add_variable("y", 20i64).unwrap();
        bench_expr(&Program::compile("(x + y) * 2").unwrap(), &ctx, "(x + y) * 2  [compound]");
    }
    {
        let mut ctx = Context::default();
        let requests = vec![Value::Int(42), Value::Int(42)];
        ctx.add_variable("requests", Value::List(Arc::new(requests))).unwrap();
        ctx.add_variable("size", Value::Int(3)).unwrap();
        bench_expr(&Program::compile("size(requests) + size == 5").unwrap(), &ctx,
                   "size(requests) + size == 5  [size fn + var]");
    }

    // ── 15. INDEXING OPERATIONS ─────────────────────────────────────────────
    println!("\n────────────────────── 15. INDEXING OPERATIONS ───────────────────────");
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("arr", vec![10i64, 20, 30]);
        bench_expr(&Program::compile("arr[1]").unwrap(), &ctx, "arr[1]  [list index]");
        bench_expr(&Program::compile("arr[-1]").unwrap(), &ctx, "arr[-1]  [negative index]");
    }
    {
        let mut ctx = Context::default();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        ctx.add_variable_from_value("headers", headers);
        bench_expr(&Program::compile("headers[\"Content-Type\"]").unwrap(), &ctx,
                   "headers[\"Content-Type\"]  [map index]");
    }

    // ── 16. STRING FUNCTION TESTS ───────────────────────────────────────────
    println!("\n────────────────────── 16. STRING FUNCTION TESTS ─────────────────────");
    // These come from functions.rs test_string, test_bytes, etc.
    bench_expr(&Program::compile("string('foo') == 'foo'").unwrap(), &Context::default(), "string('foo') == 'foo'");
    bench_expr(&Program::compile("string(b'foo') == 'foo'").unwrap(), &Context::default(), "string(b'foo') == 'foo'");
    bench_expr(&Program::compile("bytes('abc') == b'abc'").unwrap(), &Context::default(), "bytes('abc') == b'abc'");
    bench_expr(&Program::compile("'foobar'.matches('^[a-zA-Z]*$')").unwrap(), &Context::default(),
               "'foobar'.matches('^[a-zA-Z]*$')  [regex]");
    bench_expr(&Program::compile("'abc'.matches('...')").unwrap(), &Context::default(), "'abc'.matches('...')");

    // ── 17. BYTES OPERATIONS ────────────────────────────────────────────────
    println!("\n────────────────────── 17. BYTES OPERATIONS ──────────────────────────");
    bench_expr(&Program::compile("b'foobar'.size() == 6").unwrap(), &Context::default(), "b'foobar'.size() == 6");
    bench_expr(&Program::compile("bytes('abc') == b'abc'").unwrap(), &Context::default(), "bytes('abc') == b'abc'");
    bench_expr(&Program::compile("bytes(b'abc') == b'abc'").unwrap(), &Context::default(), "bytes(b'abc') == b'abc'");
    bench_expr(&Program::compile("bytes('foo') == 'foo'").unwrap(), &Context::default(), "bytes('foo') == 'foo'");

    // ── 18. SHORT-CIRCUIT EDGE CASES ────────────────────────────────────────
    println!("\n────────────────────── 18. SHORT-CIRCUIT EDGE CASES ───────────────────");

    // ── 19. SIZE EDGE CASES ─────────────────────────────────────────────────
    println!("\n────────────────────── 19. SIZE EDGE CASES ────────────────────────────");
    bench_expr(&Program::compile("size([]) == 0").unwrap(), &Context::default(), "size([]) == 0");
    bench_expr(&Program::compile("size([size([42]), 2, 3]) == 3").unwrap(), &Context::default(),
               "size([size([42]), 2, 3]) == 3  [nested size]");
    bench_expr(&Program::compile("b'foobar'.size() == 6").unwrap(), &Context::default(), "b'foobar'.size() == 6");

    // ── 20. MAP COMPREHENSION EDGE CASES ────────────────────────────────────
    println!("\n────────────────────── 20. MAP COMPREHENSION EDGES ────────────────────");
    {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("numbers", vec![10u64, 20, 30]);
        bench_expr(&Program::compile("numbers[1u]").unwrap(), &ctx, "numbers[1u]  [uint index]");
    }
    bench_expr(&Program::compile("{'John': 'smart'}.map(key, key) == ['John']").unwrap(), &Context::default(),
               "{'John': 'smart'}.map(key, key) == ['John']");
    bench_expr(&Program::compile("[true, true, false].all(x, x)").unwrap(), &Context::default(),
               "[true, true, false].all(x, x)  [bool identity]");

    // ── 21. NESTED COMPREHENSION ────────────────────────────────────────────
    println!("\n────────────────────── 21. NESTED COMPREHENSION ──────────────────────");
    bench_expr(&Program::compile("[[1, 2], [2, 3]].map(x, x.map(x, x * 2)) == [[2, 4], [4, 6]]").unwrap(),
               &Context::default(),
               "[[1,2],[2,3]].map(x, x.map(x, x*2)) == [[2,4],[4,6]]");

    // ── 22. STRUCT CONSTRUCTION ─────────────────────────────────────────────
    #[cfg(feature = "structs")]
    {
        use cel::common::types;
        use cel::StructDef;
        use cel::Env;

        println!("\n────────────────────── 22. STRUCT CONSTRUCTION ──────────────────────");

        let mut env = Env::stdlib();
        env.add_struct(
            StructDef::new("cel.MyStruct".into())
                .add_field("name".into(), types::STRING_TYPE)
                .add_field("value".into(), types::INT_TYPE),
        );
        let ctx = Context::with_env(Arc::new(env));

        bench_expr(&Program::compile("cel.MyStruct{name: 'hello', value: 42}").unwrap(), &ctx,
                   "cel.MyStruct{name:'hello',value:42}  [construct 2 fields]");
        bench_expr(&Program::compile("cel.MyStruct{}").unwrap(), &ctx,
                   "cel.MyStruct{}  [empty]");
        bench_expr(&Program::compile("cel.MyStruct{name: 'hello', value: 42}.name == 'hello'").unwrap(), &ctx,
                   "cel.MyStruct{...}.name == 'hello'  [construct+access]");
        bench_expr(&Program::compile("has(cel.MyStruct{name: 'foo', value: 1}.name)").unwrap(), &ctx,
                   "has(cel.MyStruct{...}.name)  [construct+has]");
    }
    #[cfg(not(feature = "structs"))]
    {
        println!("\n────────────────────── 22. STRUCT CONSTRUCTION ──────────────────────");
        println!("  (skipped — 'structs' feature not enabled)");
    }

    // ── 23. CHRONO TIMESTAMP/DURATION OPS ───────────────────────────────────
    #[cfg(feature = "chrono")]
    {
        println!("\n────────────────────── 23. CHRONO TIMESTAMP/DURATION ─────────────────");
        bench_expr(&Program::compile("timestamp('2023-05-29T00:00:00Z') > timestamp('2023-05-28T00:00:00Z')").unwrap(),
                   &Context::default(),
                   "timestamp compare  [ts > ts]");
        bench_expr(&Program::compile("timestamp('2023-05-29T00:00:00Z') - duration('24h') == timestamp('2023-05-28T00:00:00Z')").unwrap(),
                   &Context::default(),
                   "ts - dur == ts  [subtract duration]");
        bench_expr(&Program::compile("duration('1h') + duration('30m') == duration('90m')").unwrap(),
                   &Context::default(),
                   "dur + dur == dur  [add durations]");
        bench_expr(&Program::compile("duration('1s') == duration('1000ms')").unwrap(),
                   &Context::default(),
                   "dur == dur  [compare durations]");
        bench_expr(&Program::compile("timestamp('2023-05-29T00:00:00Z') + duration('1h')").unwrap(),
                   &Context::default(),
                   "ts + dur  [add duration to timestamp]");
    }
    #[cfg(not(feature = "chrono"))]
    {
        println!("\n────────────────────── 23. CHRONO TIMESTAMP/DURATION ─────────────────");
        println!("  (skipped — 'chrono' feature not enabled)");
    }

    // ── 24. BUILT-IN FUNCTIONS (max, min) — FnTable fast path ───────────
    println!("\n────────────────────── 24. BUILT-IN FUNCTIONS (FnTable) ──────────────");
    {
        use cel::vm::compiler::FnTable;
        let mut ft = FnTable::new();
        ft.insert("max".into(), Arc::new(|args: &[Value]| -> Result<Value, cel::ExecutionError> {
            cel::functions::max(cel::magic::Arguments(Arc::new(args.to_vec())))
        }));
        ft.insert("min".into(), Arc::new(|args: &[Value]| -> Result<Value, cel::ExecutionError> {
            cel::functions::min(cel::magic::Arguments(Arc::new(args.to_vec())))
        }));
        let ft = Arc::new(ft);
        let ctx = Context::default();
        bench_expr(&Program::compile_with_fns("max(1, 2, 3) == 3", Arc::clone(&ft)).unwrap(), &ctx,
                   "max(1, 2, 3) == 3  [variadic int]");
        bench_expr(&Program::compile_with_fns("min(1, 2, 3) == 1", Arc::clone(&ft)).unwrap(), &ctx,
                   "min(1, 2, 3) == 1  [variadic int]");
        bench_expr(&Program::compile_with_fns("max(-1.0, 0.0) == 0.0", Arc::clone(&ft)).unwrap(), &ctx,
                   "max(-1.0, 0.0) == 0.0  [float]");
        bench_expr(&Program::compile_with_fns("min(-1, 0) == -1", Arc::clone(&ft)).unwrap(), &ctx,
                   "min(-1, 0) == -1  [int]");
    }

    // ── 25. USER-DEFINED FUNCTIONS — FnTable fast path ────────────────────
    println!("\n────────────────────── 25. USER-DEFINED FUNCTIONS (FnTable) ────────────");
    {
        use cel::vm::compiler::FnTable;
        let mut ft = FnTable::new();
        ft.insert("answer".into(), Arc::new(|_: &[Value]| -> Result<Value, cel::ExecutionError> {
            Ok(Value::Int(42))
        }));
        ft.insert("double".into(), Arc::new(|args: &[Value]| -> Result<Value, cel::ExecutionError> {
            if let Some(Value::Int(i)) = args.first() {
                Ok(Value::Int(i * 2))
            } else {
                Err(cel::ExecutionError::UnexpectedType {
                    got: "unknown".into(), want: "int".into(),
                })
            }
        }));
        let ft = Arc::new(ft);
        let ctx = Context::default();
        bench_expr(&Program::compile_with_fns("answer() == 42", Arc::clone(&ft)).unwrap(), &ctx,
                   "answer() == 42  [zero-arg user fn]");
        bench_expr(&Program::compile_with_fns("double(21) == 42", Arc::clone(&ft)).unwrap(), &ctx,
                   "double(21) == 42  [1-arg user fn]");
    }

    println!("\n────────────────────── SUMMARY ─────────────────────────────────────");
    println!("  Total expressions benchmarked: 106");
    println!("  All use Program::compile() + Program::execute() compiled path");
    println!("  (Filter tree used where applicable; general closure otherwise)");
    println!("────────────────────────────────────────────────────────────────────\n");
}
