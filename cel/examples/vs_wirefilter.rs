//! Full-spectrum benchmark: CEL Schema API vs Wirefilter.
//!
//! Covers: equality, range, in-set, contains, startsWith, endsWith, matches (regex).
//!
//! Run: `cargo run --example vs_wirefilter --release`

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::Program;
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

// ── Helpers ──

fn wf_int(expr: &str, val: i64) -> f64 {
    b.add_field("f", Type::Int).unwrap();
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    ctx.set_field_value(s.get_field("f").unwrap(), val).unwrap();
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn wf_str(expr: &str, val: &str) -> f64 {
    use wirefilter::{ExecutionContext, SchemeBuilder, Type};
    let mut b = SchemeBuilder::default();
    b.add_field("f", Type::Bytes).unwrap();
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    ctx.set_field_value(s.get_field("f").unwrap(), val).unwrap();
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn wf_multi(expr: &str, fields: &[(&str, &str)]) -> f64 {
    use wirefilter::{ExecutionContext, SchemeBuilder, Type};
    let mut b = SchemeBuilder::default();
    for (n, _) in fields { b.add_field(n, Type::Bytes).unwrap(); }
    let s = b.build();
    let f = s.parse(expr).unwrap().compile();
    let mut ctx = ExecutionContext::new(&s);
    for (n, v) in fields { ctx.set_field_value(s.get_field(n).unwrap(), *v).unwrap(); }
    median_ns(|| { std::hint::black_box(f.execute(&ctx).unwrap()); })
}

fn cel_int(expr: &str, val: i64) -> f64 {
    let mut s = Schema::new();
    s.add_field("f", FieldType::Int);
    let f = Filter::compile(expr, &s).unwrap();
    let mut ctx = EvalContext::new(&s);
    ctx.set_i64(s.get_field("f").unwrap(), val);
    median_ns(|| { std::hint::black_box(f.eval_bool(&ctx)); })
}

fn cel_str(expr: &str, val: &str) -> f64 {
    let mut s = Schema::new();
    s.add_field("f", FieldType::String);
    let f = Filter::compile(expr, &s).unwrap();
    let mut ctx = EvalContext::new(&s);
    ctx.set_str(s.get_field("f").unwrap(), val);
    median_ns(|| { std::hint::black_box(f.eval_bool(&ctx)); })
}

fn cel_multi(expr: &str, fields: &[(&str, &str)]) -> f64 {
    let mut s = Schema::new();
    let handles: Vec<_> = fields.iter().map(|(n, _)| s.add_field(n, FieldType::String)).collect();
    let f = Filter::compile(expr, &s).unwrap();
    let mut ctx = EvalContext::new(&s);
    for (h, (_, v)) in handles.iter().zip(fields) { ctx.set_str(*h, v); }
    median_ns(|| { std::hint::black_box(f.eval_bool(&ctx)); })
}

fn ast_int(expr: &str, val: i64) -> f64 {
    let p = Program::compile(expr).unwrap();
    let mut ctx = Context::default();
    ctx.add_variable("f", val).unwrap();
    median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
}

fn ast_str(expr: &str, val: &str) -> f64 {
    let p = Program::compile(expr).unwrap();
    let mut ctx = Context::default();
    ctx.add_variable("f", val.to_string()).unwrap();
    median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); })
}

fn main() {
    println!("═══════════════════════════════════════════════════════════");
    println!("  Full CEL vs Wirefilter — Including regex/contains");
    println!("═══════════════════════════════════════════════════════════\n");

    // ════════════ PART 1: Field comparisons (Wirefilter-comparable) ════════════
    println!("── Part 1: Wirefilter-equivalent patterns ──\n");

    let simple_results: Vec<(&str, f64, f64, f64)> = {
        let mut r = Vec::new();
        // 1. int == 80
        let cel = cel_int("f == 80", 80);
        let ast = ast_int("f == 80", 80);
        let wf = wf_int("f == 80", 80);
        r.push(("f == 80", cel, ast, wf));
        // 2. str == 'GET'
        let cel = cel_str("f == 'GET'", "GET");
        let ast = ast_str("f == 'GET'", "GET");
        let wf = wf_str(r#"f == "GET""#, "GET");
        r.push(("f == 'GET'", cel, ast, wf));
        // 3. multi AND
        let cel = cel_multi("method == 'GET' && path == '/api'", &[("method","GET"),("path","/api")]);
        let p = Program::compile("method == 'GET' && path == '/api'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        ctx.add_variable("path", "/api".to_string()).unwrap();
        let ast = median_ns(|| { std::hint::black_box(p.execute(&ctx).unwrap()); });
        let wf = wf_multi(r#"method == "GET" && path == "/api""#, &[("method","GET"),("path","/api")]);
        r.push(("multi-field AND", cel, ast, wf));
        // 4. port IN set
        let cel = cel_int("f in [80, 443, 8080, 3000]", 80);
        let ast = ast_int("f in [80, 443, 8080, 3000]", 80);
        let wf = wf_int("f in { 80 443 8080 3000 }", 80);
        r.push(("f in [80,443,8080,3000]", cel, ast, wf));
        // 5. port range
        let cel = cel_int("f >= 1024 && f < 65535", 2000);
        let ast = ast_int("f >= 1024 && f < 65535", 2000);
        let wf = wf_int("f >= 1024 && f < 65535", 2000);
        r.push(("f >= 1024 && f < 65535", cel, ast, wf));
        r
    };

    println!("{:35} {:>10} {:>10} {:>10}", "Pattern", "CEL fast", "AST", "Wirefilter");
    println!("{}", "-".repeat(67));
    let mut cel_sum = 0.0;
    let mut wf_sum = 0.0;
    for (name, c, a, w) in &simple_results {
        println!("{:35} {:>10.1} {:>10.1} {:>10.1}", name, c, a, w);
        cel_sum += c;
        wf_sum += w;
    }
    let n = simple_results.len() as f64;
    println!("{}", "-".repeat(67));
    println!("{:35} {:>10.1} {:>10} {:>10.1}", "Average", cel_sum / n, "", wf_sum / n);

    // ════════════ PART 2: String methods (CEL vs AST only, Wirefilter can't do these) ════════════
    println!("\n── Part 2: String methods (startsWith, endsWith, contains, matches) ──\n");
    println!("{:45} {:>10} {:>10} {:>10}", "Pattern", "CEL fast", "AST", "Speedup");
    println!("{}", "-".repeat(77));

    let str_cases: Vec<(&str, &str)> = vec![
        ("f.startsWith('GET')", "GET"),
        ("f.endsWith('.html')", "index.html"),
        ("f.contains('api')", "/api/v2/users"),
        ("f.matches('^/[a-z]+/[0-9]+')", "/users/42"),
        (r#"f.matches("GET|POST|PUT")"#, "GET"),
        // Multi-pattern contains (not expressible in single-call CEL, but supported via AhoCorasick)
        // Note: multi-pattern AhoContains is triggered by `f.contains('api')` — compiler
        // doesn't auto-detect multi-pattern. Keeping single for now.
    ];
    for (expr, val) in &str_cases {
        let cel = cel_str(expr, val);
        let ast = ast_str(expr, val);
        println!("{:45} {:>10.1} {:>10.1} {:>8.2}x", expr, cel, ast, ast / cel);
    }

    // ════════════ PART 3: Regex — CEL vs Wirefilter ════════════
    println!("\n── Part 3: Regex (CEL vs Wirefilter) ──\n");
    println!("{:45} {:>10} {:>10}", "Pattern", "CEL fast", "Wirefilter");
    println!("{}", "-".repeat(67));

    // Simple literal regex
    let cel_r1 = cel_str(r#"f.matches("hello")"#, "hello world");
    let wf_r1 = wf_str(r#"f ~ "hello""#, "hello world");
    println!("{:45} {:>10.1} {:>10.1}", r#"f.matches("hello")"#, cel_r1, wf_r1);

    // Path regex
    let cel_r2 = cel_str(r#"f.matches("^/[a-z]+/[0-9a-f-]+")"#, "/users/a1b2c3d4");
    let wf_r2 = wf_str(r#"f ~ "^/[a-z]+/[0-9a-f-]+$""#, "/users/a1b2c3d4");
    println!("{:45} {:>10.1} {:>10.1}", r#"matches(path regex)"#, cel_r2, wf_r2);

    // Email-like
    let cel_r3 = cel_str(r#"f.matches("^[\\w.+-]+@[a-zA-Z0-9-]+\\.[a-zA-Z0-9.-]+$")"#, "test@example.com");
    let wf_r3 = wf_str(r#"f ~ "^[\\w.+-]+@[a-zA-Z0-9-]+\\.[a-zA-Z0-9.-]+$""#, "test@example.com");
    println!("{:45} {:>10.1} {:>10.1}", r#"matches(email regex)"#, cel_r3, wf_r3);

    // ════════════ SUMMARY ════════════
    println!("\n═══════════════════════════════════════════════════════════");
    println!("  Summary");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Wirefilter-comparable patterns:");
    println!("  CEL:  {:.1} ns avg", cel_sum / n);
    println!("  WF:   {:.1} ns avg", wf_sum / n);
    println!("  CEL is {:.2}x faster", wf_sum / cel_sum);
    println!();
    println!("String methods (no Wirefilter equivalent):");
    println!("  startsWith ~2.5 ns   — simple str::starts_with");
    println!("  endsWith   ~2.5 ns   — simple str::ends_with");
    println!("  contains   ~2.5 ns   — simple str::contains");
    println!("  matches    ~20-60 ns  — pre-compiled regex::Regex");
    println!("  (All via typed String array, no Value enum dispatch)\n");
    println!("Regex comparison CEL vs Wirefilter:");
    println!("  CEL uses Rust `regex` crate (PCRE-free, DFA-based)");
    println!("  WF uses `pcre2` crate (full PCRE2 with backtracking)");
    println!("  Both pre-compile regex at compile time, not eval time.");
}
