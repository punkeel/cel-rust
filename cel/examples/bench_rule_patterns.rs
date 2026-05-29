//! Benchmark the specific rule patterns from the firewall workload.
//! Measures per-expression eval time for both Program::execute and Filter::eval_bool.

use cel::context::Context;
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::compiler::FnTable;
use cel::Program;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

fn median_ns<F: FnMut()>(mut f: F) -> f64 {
    let warmup = Instant::now();
    while warmup.elapsed().as_millis() < 50 { f(); }

    let mut times = Vec::new();
    let start = Instant::now();
    while start.elapsed().as_millis() < 300 {
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
    println!("  FILTER RULE PATTERN BENCHMARK — per-expression eval (ns)");
    println!("═══════════════════════════════════════════════════════════════\n");

    // ── 1. Simple int comparison (baseline) ──
    println!("── 1. Int comparison ──");
    {
        // Program path
        let p = Program::compile("port == 80").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();
        let t = median_ns(|| { p.execute(&ctx).unwrap(); });
        println!("  Program::execute:   {:>8.2} ns", t);

        // Filter path
        let mut s = Schema::new();
        s.add_field("port", FieldType::Int);
        let f = Filter::compile("port == 80", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_i64(s.get_field("port").unwrap(), 80);
        let t2 = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool:  {:>8.2} ns", t2);
    }

    // ── 2. String comparison ──
    println!("\n── 2. String comparison ──");
    {
        let p = Program::compile("method == 'GET'").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("method", "GET".to_string()).unwrap();
        let t = median_ns(|| { p.execute(&ctx).unwrap(); });
        println!("  Program::execute:   {:>8.2} ns", t);

        let mut s = Schema::new();
        s.add_field("method", FieldType::String);
        let f = Filter::compile("method == 'GET'", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_str(s.get_field("method").unwrap(), "GET");
        let t2 = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool:  {:>8.2} ns", t2);
    }

    // ── 3. Map-indexed exists (rule_000 pattern) ──
    println!("\n── 3. Map-indexed exists: xxx_map[\"abs\"].exists(it, it == \"def\") ──");
    {
        // Program path
        let p = Program::compile(r#"xxx_map["abs"].exists(it, it == "def")"#).unwrap();
        let mut ctx = Context::default();
        let mut map = HashMap::new();
        map.insert(cel::objects::Key::from("abs"), Value::String(Arc::from("def")));
        ctx.add_variable_from_value("xxx_map", map);
        let t = median_ns(|| { p.execute(&ctx).unwrap(); });
        println!("  Program::execute:   {:>8.2} ns", t);

        // Filter path
        let mut s = Schema::new();
        s.add_field("xxx_map", FieldType::Any);
        let f = Filter::compile(r#"xxx_map["abs"].exists(it, it == "def")"#, &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        let mut map2 = HashMap::new();
        map2.insert(cel::objects::Key::from("abs"), Value::String(Arc::from("def")));
        ctx2.set(s.get_field("xxx_map").unwrap(), Value::Map(cel::objects::Map { map: Arc::new(map2) }));
        let t2 = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool:  {:>8.2} ns", t2);

        println!("  filter tree: {} (needs_values={})", f.variables().join(", "), true);
    }

    // ── 4. Function call (rule_008 pattern) ──
    println!("\n── 4. Function call: hamming_distance(xx, \"fdsdf\") > 10 ──");
    {
        // Program path
        let mut ft = FnTable::new();
        ft.insert("hamming_distance".into(), Arc::new(|args: &[Value]| {
            Ok(Value::Int(42))
        }));
        let ft = Arc::new(ft);
        let p = Program::compile_with_fns(r#"hamming_distance(xx, "fdsdf") > 10"#, Arc::clone(&ft)).unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("xx", "hello".to_string()).unwrap();
        let t = median_ns(|| { p.execute(&ctx).unwrap(); });
        println!("  Program::execute:   {:>8.2} ns", t);

        // Filter path — note: Filter::compile doesn't support FnTable, so this goes through closure
        // But we can test via Filter path with needs_values
        let mut s = Schema::new();
        s.add_field("xx", FieldType::String);
        let f = Filter::compile("xx == \"hello\"", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_str(s.get_field("xx").unwrap(), "hello");
        let t2 = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool (simple str cmp): {:>8.2} ns (reference)", t2);
    }

    // ── 5. AND with mixed simple + exists (BoolVar && Exists) ──
    println!("\n── 5. Mixed tree: flag_enabled && xxx_map[\"abs\"].exists(it, it == \"def\") ──");
    {
        let mut s = Schema::new();
        s.add_field("flag_enabled", FieldType::Bool);
        s.add_field("xxx_map", FieldType::Any);
        let f = Filter::compile(
            r#"flag_enabled && xxx_map["abs"].exists(it, it == "def")"#,
            &s,
        ).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_bool(s.get_field("flag_enabled").unwrap(), true);
        let mut map = HashMap::new();
        map.insert(cel::objects::Key::from("abs"), Value::String(Arc::from("def")));
        ctx2.set(s.get_field("xxx_map").unwrap(), Value::Map(cel::objects::Map { map: Arc::new(map) }));
        let t = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool:  {:>8.2} ns", t);

        // Hit + miss timing to test short-circuit
        ctx2.set_bool(s.get_field("flag_enabled").unwrap(), false);
        let t_false = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool (flag=false, short-circ): {:>8.2} ns", t_false);
    }

    // ── 6. In-set membership ──
    println!("\n── 6. In-set membership: port in [80, 443, 8080] ──");
    {
        let mut s = Schema::new();
        s.add_field("port", FieldType::Int);
        let f = Filter::compile("port in [80, 443, 8080]", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_i64(s.get_field("port").unwrap(), 80);
        let t = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool (hit):  {:>8.2} ns", t);

        ctx2.set_i64(s.get_field("port").unwrap(), 999);
        let t2 = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool (miss): {:>8.2} ns", t2);
    }

    // ── 7. Overhead analysis: eval_mixed vs eval_fast for pure trees ──
    println!("\n── 7. Overhead: eval_mixed vs eval_fast vs closure for pure int tree ──");
    {
        let mut s = Schema::new();
        s.add_field("port", FieldType::Int);
        let f = Filter::compile("port >= 1024", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_i64(s.get_field("port").unwrap(), 2000);
        let t = median_ns(|| { f.eval_bool(&ctx2); });
        println!("  Filter::eval_bool:  {:>8.2} ns (pure int, closure path)", t);

        // Compare with program path
        let p = Program::compile("port >= 1024").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 2000i64).unwrap();
        let t2 = median_ns(|| { p.execute(&ctx).unwrap(); });
        println!("  Program::execute:   {:>8.2} ns (HashMap bind)", t2);
    }

    // ── 8. Filter path with FnCall (compile via Program + Filter should show diff) ──
    println!("\n── 8. Direct comparison: Program vs Filter for same expression ──");
    {
        let p = Program::compile("port == 80").unwrap();
        let mut ctx = Context::default();
        ctx.add_variable("port", 80i64).unwrap();

        let mut s = Schema::new();
        s.add_field("port", FieldType::Int);
        let f = Filter::compile("port == 80", &s).unwrap();
        let mut ctx2 = EvalContext::new(&s);
        ctx2.set_i64(s.get_field("port").unwrap(), 80);

        // Measure just the eval, excluding setup
        let t_prog = median_ns(|| { p.execute(&ctx).unwrap(); });
        let t_filt = median_ns(|| { f.eval_bool(&ctx2); });

        println!("  Program::execute:   {:>8.2} ns  (bind_vars from HashMap)", t_prog);
        println!("  Filter::eval_bool:  {:>8.2} ns  (direct array access)", t_filt);
        println!("  Speedup: {:.1}×", t_prog / t_filt);
    }

    println!("\n═══════════════════════════════════════════════════════════════");
}
