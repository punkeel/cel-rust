//! Benchmark: exists() patterns.
//!
//! Measures all exists() variants: ExistsInIntSet, ExistsEqInt, ExistsClosure, generic Exists.
//!
//! Run: `cargo run --example bench_exists --release`

use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn median_ns<F: FnMut()>(mut f: F) -> f64 {
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
    if times.len() % 2 == 0 { (times[mid-1] + times[mid]) / 2.0 } else { times[mid] }
}

fn main() {
    println!("═══ exists() Benchmark ═══\n");
    println!("{:50} {:>12} {:>14} {:>12}", "Pattern", "CEL fast", "Result", "Notes");
    println!("{}", "-".repeat(90));

    // Helper: bench with int list data
    fn bench_int(expr: &str, field: &str, vals: Vec<i64>) {
        let mut s = Schema::new();
        s.add_field(field, FieldType::Any);
        let filter = Filter::compile(expr, &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        let fh = s.get_field(field).unwrap();
        ctx.set(fh, Value::List(Arc::new(vals.iter().map(|v| Value::Int(*v)).collect())));
        let result = filter.eval(&ctx).unwrap();
        let note = if result { "hit" } else { "miss" };
        let ns = median_ns(|| { std::hint::black_box(filter.eval(&ctx).unwrap()); });
        println!("{:50} {:>12.1} {:>14} {:>12}", expr, ns, result, note);
    }

    // Helper: bench with string list data
    fn bench_str(expr: &str, field: &str, vals: Vec<&str>) {
        let mut s = Schema::new();
        s.add_field(field, FieldType::Any);
        let filter = Filter::compile(expr, &s).unwrap();
        let mut ctx = EvalContext::new(&s);
        let fh = s.get_field(field).unwrap();
        ctx.set(fh, Value::List(Arc::new(vals.iter().map(|v| Value::String(Arc::from(*v))).collect())));
        let result = filter.eval(&ctx).unwrap();
        let note = if result { "hit" } else { "miss" };
        let ns = median_ns(|| { std::hint::black_box(filter.eval(&ctx).unwrap()); });
        println!("{:50} {:>12.1} {:>14} {:>12}", expr, ns, result, note);
    }

    // ═══════ INT EXISTS PATTERNS ═══════

    // ExistsInIntSet: it in [int_set]
    bench_int("xx.exists(it, it in [767, 12345])", "xx", vec![1, 3, 767, 99]);
    bench_int("xx.exists(it, it in [767, 12345])", "xx", vec![1, 3, 5, 99]);  // miss

    // ExistsEqInt: it == int
    bench_int("xx.exists(it, it == 42)", "xx", vec![1, 42, 3]);
    bench_int("xx.exists(it, it == 42)", "xx", vec![1, 2, 3]);  // miss

    // ExistsClosure: it == int (compiled to closure via try_compile_item_predicate)
    // Note: this goes through the closure path since we try closure after EqInt
    bench_int("xx.exists(it, it != 42)", "xx", vec![1, 42, 3]);  // it != 42 → closure

    // Generic Exists: it > 5 (no pattern match → scratch vector)
    bench_int("xx.exists(it, it > 5)", "xx", vec![1, 3, 7, 2]);

    // ExistsClosure: AND composition
    bench_int("xx.exists(it, it > 5 && it < 100)", "xx", vec![1, 42, 200]);

    // ExistsClosure: OR composition
    bench_int("xx.exists(it, it < 5 || it > 100)", "xx", vec![1, 42, 200]);

    // ExistsClosure: NOT(==)
    bench_int("!xx.exists(it, it == 0)", "xx", vec![1, 2, 3]);

    // ═══════ STRING EXISTS PATTERNS ═══════

    // ExistsClosure: string ==
    bench_str(r#"names.exists(n, n == "bob")"#, "names", vec!["alice", "bob", "charlie"]);

    // ExistsClosure: string !=
    bench_str(r#"names.exists(n, n != "bob")"#, "names", vec!["alice", "bob"]);

    // ExistsClosure: startsWith
    bench_str(r#"host.exists(h, h.startsWith("192."))"#, "host", vec!["10.0.0.1", "192.168.1.1", "8.8.8.8"]);

    // ExistsClosure: endsWith
    bench_str(r#"file.exists(f, f.endsWith(".html"))"#, "file", vec!["index.html", "style.css"]);

    // ExistsClosure: contains
    bench_str(r#"header.exists(h, h.contains("api"))"#, "header", vec!["/users", "/api/v2", "/health"]);

    // ExistsClosure: matches (regex)
    bench_str(r#"path.exists(p, p.matches("^/[a-z]+"))"#, "path", vec!["/users", "/42"]);

    // ═══════ MAP EXISTS PATTERNS ═══════

    // MapKeyContains: map["key"].exists(it, it == "val")
    {
        use cel::objects::{Key, Map};
        let mut s = Schema::new();
        s.add_field("hdrs", FieldType::Any);
        let filter = Filter::compile(
            r#"hdrs["via"].exists(it, it == "X-MSIP-Via")"#, &s,
        ).unwrap();
        let mut ctx = EvalContext::new(&s);
        let fh = s.get_field("hdrs").unwrap();
        let mut map = HashMap::new();
        map.insert(Key::String(Arc::from("via")),
            Value::String(Arc::from("X-MSIP-Via")));
        ctx.set(fh, Value::Map(Map { map: Arc::new(map) }));
        let result = filter.eval(&ctx).unwrap();
        let ns = median_ns(|| { std::hint::black_box(filter.eval(&ctx).unwrap()); });
        println!("{:50} {:>12.1} {:>14} {:>12}", r#"hdrs["via"].exists(it,it=="X-MSIP-Via")"#, ns, result, "MapKeyContains");
    }

    println!("\n── Summary ──\n");
    println!("Path                   Patterns                                                          Allocation per eval");
    println!("──────────────────────────────────────────────────────────────────────────────────────────────────────────────────");
    println!("ExistsInIntSet        it in [int_set]                                                      None");
    println!("ExistsEqInt           it == int_val                                                        None");
    println!("ExistsClosure         it ==/!= str, startsWith, endsWith, contains, matches, &&, ||, !    None");
    println!("MapKeyContains        map[\"key\"].exists(it, it == \"val\")                                   None");
    println!("Generic Exists        it > 5, it >= 0, etc.                                                 vars.to_vec() + item.clone()");
}
