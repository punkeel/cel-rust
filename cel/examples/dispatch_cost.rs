use cel::objects::Value;
use cel::vm::filter_tree::FilterNode;
use std::sync::Arc;
use std::time::Instant;

fn bench(name: &str, mut f: impl FnMut()) -> f64 {
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
    let ns = if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] };
    println!("  {:20} {:8.1} ns", name, ns);
    ns
}

fn main() {
    println!("=== Pure FilterNode dispatch cost ===\n");

    let ints = vec![80i64];
    let strings: Vec<Arc<str>> = vec![Arc::from("")];
    let f: Box<FilterNode> = Box::new(FilterNode::EqInt { idx: 0, val: 80 });
    bench("eval_fast_typed", || {
        std::hint::black_box(unsafe { f.eval_fast_typed(&ints, &strings) });
    });

    let v = vec![Value::Int(80)];
    bench("eval_fast (Value)", || {
        std::hint::black_box(unsafe { f.eval_fast(&v) });
    });

    bench("eval (safe)", || {
        std::hint::black_box(f.eval(&v));
    });
}
