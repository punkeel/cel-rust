use cel::Program;
use std::time::Instant;

fn main() {
    let iters = 10_000_000u64;

    println!("=== CEL AST (interpreter) ===");

    // int_eq
    {
        let program = Program::compile("port == 80").unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("port", 8080i64);
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("int_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // str_eq
    {
        let program = Program::compile("method == \"GET\"").unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("method", "POST");
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("str_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // compound_and
    {
        let program = Program::compile("port >= 1024 && port < 65535").unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("port", 8080i64);
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("compound_and: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // complex_bool
    {
        let program = Program::compile("method == \"GET\" && port == 80 && path == \"/api/v1/users\"").unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("method", "GET");
        ctx.add_variable_from_value("port", 80i64);
        ctx.add_variable_from_value("path", "/api/v1/users");
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("complex_bool: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // select
    {
        let program = Program::compile("foo.bar").unwrap();
        let mut ctx = cel::context::Context::default();
        let mut map = std::collections::HashMap::new();
        map.insert(cel::objects::Key::String(std::sync::Arc::new("bar".to_string())), cel::Value::Int(42));
        let foo = cel::Value::Map(cel::objects::Map { map: std::sync::Arc::new(map) });
        ctx.add_variable_from_value("foo", foo);
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("select: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // arith
    {
        let program = Program::compile("1 + 2 * 3 - 4 / 5").unwrap();
        let ctx = cel::context::Context::default();
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("arith: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // in_set 4
    {
        let program = Program::compile("port in [80, 443, 8080, 3000]").unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("port", 443i64);
        for _ in 0..1000 { let _ = program.execute(&ctx); }
        let start = Instant::now();
        for _ in 0..iters { let _ = program.execute(&ctx); }
        println!("in_set_4: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    println!("\n=== CEL Stack VM (eval_fast, reusable state) ===");

    // int_eq
    {
        let program = Program::compile("port == 80").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        state.set_vars(vec![cel::Value::Int(8080)]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("int_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // str_eq
    {
        let program = Program::compile("method == \"GET\"").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        state.set_vars(vec![cel::Value::String(std::sync::Arc::new("POST".to_string()))]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("str_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // compound_and
    {
        let program = Program::compile("port >= 1024 && port < 65535").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        state.set_vars(vec![cel::Value::Int(8080)]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("compound_and: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // complex_bool
    {
        let program = Program::compile("method == \"GET\" && port == 80 && path == \"/api/v1/users\"").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        state.set_vars(vec![
            cel::Value::String(std::sync::Arc::new("GET".to_string())),
            cel::Value::Int(80),
            cel::Value::String(std::sync::Arc::new("/api/v1/users".to_string())),
        ]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("complex_bool: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // select
    {
        let program = Program::compile("foo.bar").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        let mut map = std::collections::HashMap::new();
        map.insert(cel::objects::Key::String(std::sync::Arc::new("bar".to_string())), cel::Value::Int(42));
        let foo = cel::Value::Map(cel::objects::Map { map: std::sync::Arc::new(map) });
        state.set_vars(vec![foo]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("select: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // arith
    {
        let program = Program::compile("1 + 2 * 3 - 4 / 5").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("arith: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // in_set 4
    {
        let program = Program::compile("port in [80, 443, 8080, 3000]").unwrap().compile_vm().unwrap();
        let mut state = cel::vm::EvalState::new();
        state.set_vars(vec![cel::Value::Int(443)]);
        for _ in 0..1000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("in_set_4: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    println!("\n=== CEL Register VM (eval_reg, reusable RegState) ===");

    // int_eq
    {
        let program = cel::vm::compile_reg(Program::compile("port == 80").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::Int(8080)];
        let mut state = cel::vm::RegState::new();
        state.set_vars(&vars);
        for _ in 0..1000 { let _ = cel::vm::eval_reg(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_reg(&program, &mut state); }
        println!("int_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // str_eq
    {
        let program = cel::vm::compile_reg(Program::compile("method == \"GET\"").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::String(std::sync::Arc::new("POST".to_string()))];
        let mut state = cel::vm::RegState::new();
        state.set_vars(&vars);
        for _ in 0..1000 { let _ = cel::vm::eval_reg(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_reg(&program, &mut state); }
        println!("str_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // compound_and
    {
        let program = cel::vm::compile_reg(Program::compile("port >= 1024 && port < 65535").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::Int(8080)];
        let mut state = cel::vm::RegState::new();
        state.set_vars(&vars);
        for _ in 0..1000 { let _ = cel::vm::eval_reg(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_reg(&program, &mut state); }
        println!("compound_and: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // complex_bool
    {
        let program = cel::vm::compile_reg(Program::compile("method == \"GET\" && port == 80 && path == \"/api/v1/users\"").unwrap().expression()).unwrap();
        let vars = vec![
            cel::Value::String(std::sync::Arc::new("GET".to_string())),
            cel::Value::Int(80),
            cel::Value::String(std::sync::Arc::new("/api/v1/users".to_string())),
        ];
        let mut state = cel::vm::RegState::new();
        state.set_vars(&vars);
        for _ in 0..1000 { let _ = cel::vm::eval_reg(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..iters { let _ = cel::vm::eval_reg(&program, &mut state); }
        println!("complex_bool: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    println!("\n=== Filter Tree (compile_filter_tree) ===");

    // int_eq
    {
        let filter = cel::vm::compile_filter_tree(Program::compile("port == 80").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::Int(8080)];
        for _ in 0..1000 { let _ = filter.eval(&vars); }
        let start = Instant::now();
        for _ in 0..iters { let _ = filter.eval(&vars); }
        println!("int_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // str_eq
    {
        let filter = cel::vm::compile_filter_tree(Program::compile("method == \"GET\"").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::String(std::sync::Arc::new("POST".to_string()))];
        for _ in 0..1000 { let _ = filter.eval(&vars); }
        let start = Instant::now();
        for _ in 0..iters { let _ = filter.eval(&vars); }
        println!("str_eq: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // compound_and (2 strings)
    {
        let filter = cel::vm::compile_filter_tree(Program::compile("method == \"GET\" && path == \"/api/v1/users\"").unwrap().expression()).unwrap();
        let vars = vec![
            cel::Value::String(std::sync::Arc::new("GET".to_string())),
            cel::Value::String(std::sync::Arc::new("/api/v1/users".to_string())),
        ];
        for _ in 0..1000 { let _ = filter.eval(&vars); }
        let start = Instant::now();
        for _ in 0..iters { let _ = filter.eval(&vars); }
        println!("compound_and (str): {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // compound_and (int range)
    {
        let filter = cel::vm::compile_filter_tree(Program::compile("port >= 1024 && port < 65535").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::Int(8080)];
        for _ in 0..1000 { let _ = filter.eval(&vars); }
        let start = Instant::now();
        for _ in 0..iters { let _ = filter.eval(&vars); }
        println!("compound_and (int): {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    // in_set_4
    {
        let filter = cel::vm::compile_filter_tree(Program::compile("port in [80, 443, 8080, 3000]").unwrap().expression()).unwrap();
        let vars = vec![cel::Value::Int(443)];
        for _ in 0..1000 { let _ = filter.eval(&vars); }
        let start = Instant::now();
        for _ in 0..iters { let _ = filter.eval(&vars); }
        println!("in_set_4: {:>6.2} ns/iter", start.elapsed().as_nanos() as f64 / iters as f64);
    }

    println!("\n=== wirefilter (reference) ===");
    println!("int_eq:       3.5 ns/iter  (port == 80)");
    println!("str_eq:       6.4 ns/iter  (method == \"GET\")");
    println!("compound_and: 8.3 ns/iter  (port >= 1024 && port < 65535)");
    println!("complex_bool: ~9 ns/iter   (3-field AND)");
    println!("in_set_4:     7.0 ns/iter  (port in {{80 443 8080 3000}})");

    println!("\n=== Comprehensions (CEL stack VM only) ===");
    {
        let list: Vec<i64> = (0..1000).collect();
        let program = Program::compile("list.map(x, x * 2)").unwrap().compile_vm().unwrap();
        let mut ctx = cel::context::Context::default();
        ctx.add_variable_from_value("list", list);
        let mut state = cel::vm::EvalState::new();
        state.bind_vars(&program, &ctx);
        for _ in 0..100 { let _ = cel::vm::eval_fast(&program, &mut state); }
        let start = Instant::now();
        for _ in 0..10000 { let _ = cel::vm::eval_fast(&program, &mut state); }
        println!("map 1000: {:>6.2} µs/iter", start.elapsed().as_micros() as f64 / 10000.0);
    }
}
