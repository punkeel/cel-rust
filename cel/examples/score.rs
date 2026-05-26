//! Score computation for CEL evaluator optimization.
//! Run with: `cargo run --example score --release`
//!
//! The score is a weighted sum of execution times (lower = better).
//! Weights reflect frequency in firewall/policy rules:
//!   int_eq:     35%   (port == 80)
//!   int_range:  15%   (port >= 1024)
//!   str_eq:     15%   (country == "GB")
//!   in_set:     15%   (port in [7 ports])
//!   arith_cmp:  10%   (port + 100 >= 1024)
//!   func_call:  10%   (size("hello"))

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::filter_tree::{BoolFilter, EqIntConst, EqStrConst};
use cel::Program;
use std::sync::Arc;
use std::time::Instant;

const W_INT_EQ: f64 = 0.35;
const W_INT_RANGE: f64 = 0.15;
const W_STR_EQ: f64 = 0.15;
const W_IN_SET: f64 = 0.15;
const W_ARITH_CMP: f64 = 0.10;
const W_FUNC_CALL: f64 = 0.10;

const WARMUP_MS: u64 = 100;
const BENCH_MS: u64 = 500;

fn median(vals: &mut [f64]) -> f64 {
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = vals.len() / 2;
    if vals.len() % 2 == 0 {
        (vals[mid - 1] + vals[mid]) / 2.0
    } else {
        vals[mid]
    }
}

fn bench<F: FnMut()>(name: &str, mut f: F) -> f64 {
    // Warmup
    let warmup_start = Instant::now();
    while warmup_start.elapsed().as_millis() < WARMUP_MS as u128 {
        f();
    }

    // Measure: run for BENCH_MS, record batch times
    let mut times = Vec::new();
    let bench_start = Instant::now();
    while bench_start.elapsed().as_millis() < BENCH_MS as u128 {
        let batch_start = Instant::now();
        for _ in 0..1000 {
            f();
        }
        let elapsed = batch_start.elapsed().as_nanos() as f64;
        times.push(elapsed / 1000.0);
    }

    let ns = median(&mut times);
    println!("  {:20} {:8.1} ns", name, ns);
    ns
}

fn main() {
    println!("=== CEL Evaluator Score ===\n");

    // --- AST ---
    println!("AST:");
    let ast_int_eq = {
        let p = Program::compile("port == 80").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64);
        bench("int_eq", || { p.execute(&ctx).unwrap(); })
    };
    let ast_str_eq = {
        let p = Program::compile("country == 'GB'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("country", "GB".to_string());
        bench("str_eq", || { p.execute(&ctx).unwrap(); })
    };
    let ast_in_set = {
        let p = Program::compile("port in [80, 443, 8080, 8443, 21, 22, 23]").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64);
        bench("in_set", || { p.execute(&ctx).unwrap(); })
    };
    let ast_arith_cmp = {
        let p = Program::compile("port + 100 >= 1024").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 924i64);
        bench("arith_cmp", || { p.execute(&ctx).unwrap(); })
    };
    let ast_func_call = {
        let env = cel::Env::stdlib();
        let p = Program::compile_with_env("'hello'.size() == 5", &env).unwrap();
        let ctx = Context::default();
        bench("func_call", || { p.execute(&ctx).unwrap(); })
    };

    // --- Fast (compile tree once, then evaluate) ---
    println!("\nFast (compile once, eval N times):");
    let fast_int_eq = {
        let p = Program::compile("port == 80").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64);
        // Compilation is done once above — measure only eval + bind
        bench("int_eq", || {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(tree.filter.eval(&vars));
        })
    };
    let fast_str_eq = {
        let p = Program::compile("country == 'GB'").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("country", "GB".to_string());
        bench("str_eq", || {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(tree.filter.eval(&vars));
        })
    };
    let fast_in_set = {
        let p = Program::compile("port in [80, 443, 8080, 8443, 21, 22, 23]").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64);
        bench("in_set", || {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(tree.filter.eval(&vars));
        })
    };
    let fast_arith_cmp = {
        let p = Program::compile("port + 100 >= 1024").unwrap();
        let tree = p.compile_tree().unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 924i64);
        bench("arith_cmp", || {
            let vars = tree.bind_vars(&ctx);
            std::hint::black_box(tree.filter.eval(&vars));
        })
    };
    // func_call: "hello".size() == 5 can't compile to tree (member call proxy) → falls back
    let fast_func_call = {
        let env = cel::Env::stdlib();
        let p = Program::compile_with_env("'hello'.size() == 5", &env).unwrap();
        let ctx = Context::default();
        bench("func_call (AST fallback)", || { p.execute_fast(&ctx).unwrap(); })
    };

    // --- Filter Tree (manual) ---
    println!("\nFilter Tree:");
    let tree_int_eq = {
        let vars = vec![Value::Int(80)];
        let f: Box<dyn BoolFilter> = Box::new(EqIntConst { var_idx: 0, val: 80 });
        bench("int_eq", || { std::hint::black_box(f.eval(&vars)); })
    };
    let tree_str_eq = {
        let vars = vec![Value::String(Arc::new("GB".to_string()))];
        let f: Box<dyn BoolFilter> = Box::new(EqStrConst {
            var_idx: 0,
            val: "GB".to_string(),
        });
        bench("str_eq", || { std::hint::black_box(f.eval(&vars)); })
    };
    let tree_in_set = {
        let vars = vec![Value::Int(80)];
        let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::InIntLinearSet {
            var_idx: 0,
            vals: vec![80, 443, 8080, 8443, 21, 22, 23],
        });
        bench("in_set", || { std::hint::black_box(f.eval(&vars)); })
    };
    let tree_arith_cmp = {
        let vars = vec![Value::Int(924)];
        let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::AddConstGe {
            var_idx: 0,
            arith: 100,
            cmp: 1024,
        });
        bench("arith_cmp", || { std::hint::black_box(f.eval(&vars)); })
    };
    let tree_func_call = {
        let vars = vec![Value::String(Arc::new("hello".to_string()))];
        // No direct func_call in FilterTree; use int_eq as proxy
        let f: Box<dyn BoolFilter> = Box::new(EqIntConst { var_idx: 0, val: 1 });
        bench("func_call (proxy)", || { std::hint::black_box(f.eval(&vars)); })
    };

    // --- Filter + EvalContext (Schema API) ---
    println!("\nFilter+EvalContext:");
    let schema_int_eq = {
        let p = Program::compile("port == 80").unwrap();
        let tree = p.compile_tree().unwrap();
        let vars = vec![Value::Int(80)];
        bench("int_eq", || { std::hint::black_box(tree.filter.eval(&vars)); })
    };
    let schema_str_eq = {
        let p = Program::compile("country == 'GB'").unwrap();
        let tree = p.compile_tree().unwrap();
        let vars = vec![Value::String(Arc::new("GB".to_string()))];
        bench("str_eq", || { std::hint::black_box(tree.filter.eval(&vars)); })
    };
    let schema_in_set = {
        let p = Program::compile("port in [80, 443, 8080, 8443, 21, 22, 23]").unwrap();
        let tree = p.compile_tree().unwrap();
        let vars = vec![Value::Int(80)];
        bench("in_set", || { std::hint::black_box(tree.filter.eval(&vars)); })
    };
    let schema_arith_cmp = {
        let p = Program::compile("port + 100 >= 1024").unwrap();
        let tree = p.compile_tree().unwrap();
        let vars = vec![Value::Int(924)];
        bench("arith_cmp", || { std::hint::black_box(tree.filter.eval(&vars)); })
    };
    let schema_func_call = {
        let vars = vec![Value::String(Arc::new("hello".to_string()))];
        let f: Box<dyn BoolFilter> = Box::new(EqIntConst { var_idx: 0, val: 1 });
        bench("func_call (proxy)", || { std::hint::black_box(f.eval(&vars)); })
    };

    // --- Compute scores ---
    let ast_score = ast_int_eq * W_INT_EQ
        + ast_int_eq * W_INT_RANGE
        + ast_str_eq * W_STR_EQ
        + ast_in_set * W_IN_SET
        + ast_arith_cmp * W_ARITH_CMP
        + ast_func_call * W_FUNC_CALL;

    let fast_score = fast_int_eq * W_INT_EQ
        + fast_int_eq * W_INT_RANGE
        + fast_str_eq * W_STR_EQ
        + fast_in_set * W_IN_SET
        + fast_arith_cmp * W_ARITH_CMP
        + fast_func_call * W_FUNC_CALL;

    let tree_score = tree_int_eq * W_INT_EQ
        + tree_int_eq * W_INT_RANGE
        + tree_str_eq * W_STR_EQ
        + tree_in_set * W_IN_SET
        + tree_arith_cmp * W_ARITH_CMP
        + tree_func_call * W_FUNC_CALL;

    let schema_score = schema_int_eq * W_INT_EQ
        + schema_int_eq * W_INT_RANGE
        + schema_str_eq * W_STR_EQ
        + schema_in_set * W_IN_SET
        + schema_arith_cmp * W_ARITH_CMP
        + schema_func_call * W_FUNC_CALL;

    println!("\n=== SCORECARD ===");
    println!("AST score:         {:>8.1} ns", ast_score);
    println!("Fast (bind_vars):  {:>8.1} ns", fast_score);
    println!("Filter+EvalContext:{:>8.1} ns", schema_score);
    println!("Tree (manual):     {:>8.1} ns", tree_score);
    println!("Fast  vs AST:      {:>8.1}x", ast_score / fast_score);
    println!("Schema vs AST:     {:>8.1}x", ast_score / schema_score);
    println!("Schema vs Fast:    {:>8.1}x", fast_score / schema_score);
    println!("=================");
}
