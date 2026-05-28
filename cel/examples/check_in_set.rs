
use cel::fast::{EvalContext, FieldType, Filter, Schema};
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

fn main() {
    // Generate 436 unique string IPs
    let mut ips = Vec::new();
    for i in 0..436 {
        ips.push(format!("10.0.{}.{}", i / 256, i % 256));
    }
    let list_expr = ips.iter().map(|s| format!("'{}'", s)).collect::<Vec<_>>().join(", ");
    let expr = format!("ip in [{}]", list_expr);

    let mut schema = Schema::new();
    let ip_f = schema.add_field("ip", FieldType::String);
    let filter = Filter::compile(&expr, &schema).unwrap();

    // Show what FilterNode was produced
    println!("Variables: {:?}", filter.variables());
    println!("Expr length: {} chars, {} elements", expr.len(), ips.len());

    // Positive: matching IP
    let mut ctx = EvalContext::new(&schema);
    ctx.set_str(ip_f, "10.0.100.50");
    let pos = median_ns(|| { filter.eval_bool(&ctx); });
    println!("Match (hash):  {:.1} ns", pos);

    // Negative: non-matching IP
    ctx.set_str(ip_f, "192.168.1.1");
    let neg = median_ns(|| { filter.eval_bool(&ctx); });
    println!("No match (hash): {:.1} ns", neg);

    // Compare: what if we force linear scan? (small set of 5)
    let small_expr = "ip in ['10.0.0.1', '10.0.0.2', '10.0.0.3', '10.0.0.4', '10.0.0.5']";
    let small_filter = Filter::compile(small_expr, &schema).unwrap();
    ctx.set_str(ip_f, "10.0.0.3");
    let small_pos = median_ns(|| { small_filter.eval_bool(&ctx); });
    ctx.set_str(ip_f, "10.0.0.99");
    let small_neg = median_ns(|| { small_filter.eval_bool(&ctx); });
    println!("Match (linear): {:.1} ns", small_pos);
    println!("No match (linear): {:.1} ns", small_neg);
}

