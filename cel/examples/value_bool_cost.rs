//! Microbenchmark: measure the overhead of returning Value::Bool vs direct bool.
//!
//! Three approaches:
//!   1. Direct fn call returning bool (inlineable — theoretical limit)
//!   2. Box<dyn Fn> returning bool (closure call overhead only)
//!   3. Box<dyn Fn> returning Value::Bool (closure + Value wrapping)
//!
//! Run: cargo run --example value_bool_cost --release

use std::time::Instant;

fn median_ns<F: FnMut()>(mut f: F) -> f64 {
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 50 { f(); }
    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 1000 {
        let batch = Instant::now();
        for _ in 0..10000 { f(); }
        times.push(batch.elapsed().as_nanos() as f64 / 10000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] }
}

fn main() {
    println!("═══ Value::Bool overhead analysis ═══\n");

    let data = vec![80i64];
    let v = vec![cel::objects::Value::Int(80)];

    // 1. Direct fn returning bool (theoretical limit)
    fn eq80_direct(vals: &[i64]) -> bool { vals[0] == 80 }
    let n1 = median_ns(|| { std::hint::black_box(eq80_direct(&data)); });
    println!("{:50} {:>8.1} ns  — theoretical limit", "1. Direct fn(i64)->bool", n1);

    // 2. Direct fn on Value returning bool (old eval path)
    fn eq80_value_bool(vals: &[cel::objects::Value]) -> bool {
        matches!(&vals[0], cel::objects::Value::Int(i) if *i == 80)
    }
    let n2 = median_ns(|| { std::hint::black_box(eq80_value_bool(&v)); });
    println!("{:50} {:>8.1} ns  — old eval path (Value match -> bool)", "2. Direct fn(&[Value])->bool", n2);

    // 3. Box<dyn Fn> returning bool
    let bool_fn: Box<dyn Fn(&[i64]) -> bool> = Box::new(|vals| vals[0] == 80);
    let n3 = median_ns(|| { std::hint::black_box(bool_fn(&data)); });
    println!("{:50} {:>8.1} ns  — closure indirect call only", "3. Box<dyn Fn(&[i64])->bool", n3);

    // 4. Box<dyn Fn> on Value returning bool
    let val_bool_fn: Box<dyn Fn(&[cel::objects::Value]) -> bool> =
        Box::new(|vals| matches!(&vals[0], cel::objects::Value::Int(i) if *i == 80));
    let n4 = median_ns(|| { std::hint::black_box(val_bool_fn(&v)); });
    println!("{:50} {:>8.1} ns  — closure on Value + bool return", "4. Box<dyn Fn(&[Value])->bool", n4);

    // 5. Box<dyn Fn> returning Value::Bool (what we have now)
    let val_fn: Box<dyn Fn(&[cel::objects::Value]) -> cel::objects::Value> =
        Box::new(|vals| match &vals[0] {
            cel::objects::Value::Int(i) => cel::objects::Value::Bool(*i == 80),
            _ => cel::objects::Value::Bool(false),
        });
    let n5 = median_ns(|| { std::hint::black_box(val_fn(&v)); });
    println!("{:50} {:>8.1} ns  — current compile() path", "5. Box<dyn Fn(&[Value])->Value", n5);

    // 6. Full Filter::eval (the real thing)
    use cel::fast::{EvalContext, FieldType, Filter, Schema};
    let mut s = Schema::new();
    let fh = s.add_field("f", FieldType::Int);
    let filter = Filter::compile("f == 80", &s).unwrap();
    let mut ctx = EvalContext::new(&s);
    ctx.set_i64(fh, 80);
    let n6 = median_ns(|| { std::hint::black_box(filter.eval_bool(&ctx)); });
    println!("{:50} {:>8.1} ns  — full Filter::eval_bool (current)", "6. Full Filter::eval_bool", n6);

    println!("\n── Overhead breakdown ──\n");
    println!("  Closure call overhead (3 - 1):        {:>5.1} ns", n3 - n1);
    println!("  Value::Bool wrap cost (5 - 3):         {:>5.1} ns", n5 - n3);
    println!("  Value match vs direct (2 - 1):         {:>5.1} ns", n2 - n1);
    println!("  Total: closure + Value::Bool (6 - 1):  {:>5.1} ns", n6 - n1);
}
