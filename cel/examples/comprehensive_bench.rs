//! Benchmark: Schema API vs AST vs Wirefilter.
//!
//! Measures three approaches on realistic CEL filter expressions:
//!   1. AST        — Original cel-rust AST interpreter (Program::execute)
//!   2. Schema     — Schema + EvalContext + closure dispatch (new API)
//!   3. Wirefilter — External filter engine
//!
//! Run: cargo run --release --example comprehensive_bench

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::Program;
use std::sync::Arc;
use std::time::Instant;

fn median_ns<F: FnMut()>(mut f: F) -> f64 {
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 100 { f(); }
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

fn wf_int(expr: &str, field: &str, val: i64) -> f64 {
    use wirefilter::{ExecutionContext, SchemeBuilder, Type};
    let mut b = SchemeBuilder::default();
    b.add_field(field, Type::Int).unwrap();
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    ctx.set_field_value(s.get_field(field).unwrap(), val).unwrap();
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn wf_bytes(expr: &str, field: &str, val: &str) -> f64 {
    use wirefilter::{ExecutionContext, SchemeBuilder, Type};
    let mut b = SchemeBuilder::default();
    b.add_field(field, Type::Bytes).unwrap();
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    ctx.set_field_value(s.get_field(field).unwrap(), val).unwrap();
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn wf_multi(expr: &str, str_fields: &[(&str, &str)]) -> f64 {
    use wirefilter::{ExecutionContext, SchemeBuilder, Type};
    let mut b = SchemeBuilder::default();
    for (name, _) in str_fields { b.add_field(name, Type::Bytes).unwrap(); }
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    for (name, val) in str_fields { ctx.set_field_value(s.get_field(name).unwrap(), *val).unwrap(); }
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn main() {
    println!("═══════════════════════════════════════════════════════");
    println!("  CEL Schema API — Full Comparison");
    println!("═══════════════════════════════════════════════════════\n");

    // Test patterns
    let patterns = vec![
        ("port == 80", Pattern::new(
            |ctx| { ctx.add_variable("port", 80i64).unwrap(); },
            |s| { let p = s.add_field("port", FieldType::Int); (p, vec![], |c| { c.set_i64(p, 80); }) },
            |s| wf_int("port == 80", "port", 80),
        )),
        ("method == 'GET'", Pattern::new(
            |ctx| { ctx.add_variable("method", "GET".to_string()).unwrap(); },
            |s| { let m = s.add_field("method", FieldType::String); (m, vec![], |c| { c.set_str(m, "GET"); }) },
            |_| wf_bytes(r#"method == "GET""#, "method", "GET"),
        )),
    ];

    for (name, pat) in &patterns {
        let ast = median_ns(|| {
            let p = Program::compile(name).unwrap();
            let mut ctx = Context::default();
            (pat.setup_ctx)(&mut ctx);
            std::hint::black_box(p.execute(&ctx).unwrap());
        });
        let schema = median_ns(|| {
            let mut s = Schema::new();
            let (field, _, setter) = (pat.setup_schema)(&mut s);
            let filter = Filter::compile(name, &s).unwrap();
            let mut ctx = EvalContext::new(&s);
            setter(&mut ctx);
            std::hint::black_box(filter.eval_bool(&ctx));
        });
        let wf = (pat.wirefilter)(name);

        println!("{:24} {:>8.1} ns {:>8.1} ns {:>10.1} ns",
            name, ast, schema, wf);
    }
}

struct Pattern {
    setup_ctx: Box<dyn Fn(&mut Context)>,
    setup_schema: Box<dyn Fn(&mut Schema) -> ((), Vec<()>, Box<dyn Fn(&mut EvalContext)>)>,
    wirefilter: Box<dyn Fn(&str) -> f64>,
}

impl Pattern {
    fn new(
        setup_ctx: impl Fn(&mut Context) + 'static,
        setup_schema: impl Fn(&mut Schema) -> ((), Vec<()>, Box<dyn Fn(&mut EvalContext)>) + 'static,
        wirefilter: impl Fn(&str) -> f64 + 'static,
    ) -> Self {
        Self {
            setup_ctx: Box::new(setup_ctx),
            setup_schema: Box::new(setup_schema),
            wirefilter: Box::new(wirefilter),
        }
    }
}
