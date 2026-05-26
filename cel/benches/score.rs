//! Combined score benchmark for CEL evaluator optimization.
//!
//! The score is a weighted sum of execution times (lower = better).
//! Weights reflect frequency in real-world firewall/policy rules.

use cel::context::Context;
use cel::objects::Value;
use cel::vm::filter_tree::{BoolFilter, EqIntConst, EqStrConst};
use cel::Program;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;
use std::time::Instant;

const W_INT_EQ: f64 = 0.35;
const W_INT_RANGE: f64 = 0.15;
const W_STR_EQ: f64 = 0.15;
const W_IN_SET: f64 = 0.15;
const W_ARITH_CMP: f64 = 0.10;
const W_FUNC_CALL: f64 = 0.10;

/// Run a micro-benchmark and return average ns per iteration.
fn micro_bench<F: FnMut()>(mut f: F, iters: u64) -> f64 {
    let start = Instant::now();
    for _ in 0..iters {
        black_box(f());
    }
    let elapsed = start.elapsed().as_nanos() as f64;
    elapsed / iters as f64
}

fn bench_score(c: &mut Criterion) {
    // We run a single criterion benchmark that internally measures all components
    c.bench_function("combined_score", |b| {
        b.iter_custom(|iters| {
            // --- AST benchmarks ---
            let ast_int_eq = {
                let p = Program::compile("port == 80").unwrap();
                let mut ctx = Context::default();
                ctx.add_variable("port", 80i64);
                micro_bench(|| { p.execute(&ctx).unwrap(); }, iters)
            };

            let ast_str_eq = {
                let p = Program::compile("country == 'GB'").unwrap();
                let mut ctx = Context::default();
                ctx.add_variable("country", "GB".to_string());
                micro_bench(|| { p.execute(&ctx).unwrap(); }, iters)
            };

            let ast_in_set = {
                let p = Program::compile("port in [80, 443, 8080, 8443, 21, 22, 23]").unwrap();
                let mut ctx = Context::default();
                ctx.add_variable("port", 80i64);
                micro_bench(|| { p.execute(&ctx).unwrap(); }, iters)
            };

            let ast_arith_cmp = {
                let p = Program::compile("port + 100 >= 1024").unwrap();
                let mut ctx = Context::default();
                ctx.add_variable("port", 924i64);
                micro_bench(|| { p.execute(&ctx).unwrap(); }, iters)
            };

            let ast_func_call = {
                let env = cel::Env::stdlib();
                let p = Program::compile_with_env("'hello'.size() == 5", &env).unwrap();
                let ctx = Context::default();
                micro_bench(|| { p.execute(&ctx).unwrap(); }, iters)
            };

            // --- VM benchmarks ---
            let vm_int_eq = {
                let p = Program::compile("port == 80").unwrap();
                let mut ctx = Context::default();
                ctx.add_variable("port", 80i64);
                micro_bench(|| { p.execute_vm(&ctx).unwrap(); }, iters)
            };

            // --- Filter Tree benchmarks ---
            let tree_int_eq = {
                let vars = vec![Value::Int(80)];
                let f: Box<dyn BoolFilter> = Box::new(EqIntConst { var_idx: 0, val: 80 });
                micro_bench(|| { f.eval(&vars); }, iters)
            };

            let tree_str_eq = {
                let vars = vec![Value::String(Arc::new("GB".to_string()))];
                let f: Box<dyn BoolFilter> = Box::new(EqStrConst {
                    var_idx: 0,
                    val: "GB".to_string(),
                });
                micro_bench(|| { f.eval(&vars); }, iters)
            };

            let tree_in_set = {
                let vars = vec![Value::Int(80)];
                let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::InIntLinearSet {
                    var_idx: 0,
                    vals: vec![80, 443, 8080, 8443, 21, 22, 23],
                });
                micro_bench(|| { f.eval(&vars); }, iters)
            };

            let tree_arith_cmp = {
                let vars = vec![Value::Int(924)];
                let f: Box<dyn BoolFilter> = Box::new(cel::vm::filter_tree::AddConstGe {
                    var_idx: 0,
                    arith: 100,
                    cmp: 1024,
                });
                micro_bench(|| { f.eval(&vars); }, iters)
            };

            // --- Compute weighted scores ---
            let ast_score = ast_int_eq * W_INT_EQ
                + ast_int_eq * W_INT_RANGE
                + ast_str_eq * W_STR_EQ
                + ast_in_set * W_IN_SET
                + ast_arith_cmp * W_ARITH_CMP
                + ast_func_call * W_FUNC_CALL;

            let vm_score = vm_int_eq * W_INT_EQ
                + vm_int_eq * W_INT_RANGE
                + vm_int_eq * W_STR_EQ
                + vm_int_eq * W_IN_SET
                + vm_int_eq * W_ARITH_CMP
                + vm_int_eq * W_FUNC_CALL;

            let tree_score = tree_int_eq * W_INT_EQ
                + tree_int_eq * W_INT_RANGE
                + tree_str_eq * W_STR_EQ
                + tree_in_set * W_IN_SET
                + tree_arith_cmp * W_ARITH_CMP
                + tree_int_eq * W_FUNC_CALL;

            println!("\n=== CEL SCORECARD (iters={}) ===", iters);
            println!("AST score:   {:.1} ns", ast_score);
            println!("VM score:    {:.1} ns", vm_score);
            println!("Tree score:  {:.1} ns", tree_score);
            println!("Tree vs AST: {:.1}x faster", ast_score / tree_score);
            println!("Tree vs VM:  {:.1}x faster", vm_score / tree_score);
            println!("=================================\n");

            // Return a dummy duration for criterion
            std::time::Duration::from_nanos(ast_score as u64)
        })
    });
}

criterion_group!(benches, bench_score);
criterion_main!(benches);
