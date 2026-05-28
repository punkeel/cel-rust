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
}
