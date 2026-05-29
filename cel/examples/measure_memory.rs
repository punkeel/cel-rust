//! Measure memory footprint of key CEL types.
use cel::fast::{EvalContext, FieldType, Filter, Schema};
use cel::objects::Value;
use cel::vm::filter_tree::FilterNode;
use std::mem;
use std::sync::Arc;

fn main() {
    println!("═══ Type sizes ═══");
    println!("FilterNode:           {} bytes", mem::size_of::<FilterNode>());
    println!("Box<FilterNode>:      {} bytes", mem::size_of::<Box<FilterNode>>());
    println!("Option<Box<FilterNode>>: {} bytes", mem::size_of::<Option<Box<FilterNode>>>());
    println!("Value:                {} bytes", mem::size_of::<Value>());
    println!("Option<Value>:        {} bytes", mem::size_of::<Option<Value>>());
    println!("Arc<str>:             {} bytes", mem::size_of::<Arc<str>>());
    println!("String:               {} bytes", mem::size_of::<String>());
    println!("Vec<i64>:             {} bytes", mem::size_of::<Vec<i64>>());
    println!("HashMap<i64,()>:      {} bytes", mem::size_of::<std::collections::HashMap<i64,()>>());
    println!("HashSet<i64>:         {} bytes", mem::size_of::<std::collections::HashSet<i64>>());
    println!("Schema:               {} bytes", mem::size_of::<Schema>());
    println!("Filter:               {} bytes", mem::size_of::<Filter>());
    println!("EvalContext:          {} bytes", mem::size_of::<EvalContext>());

    // Measure actual allocated size of a compiled filter tree
    println!("\n═══ Filter tree allocations ═══");
    let mut s = Schema::new();
    s.add_field("port", FieldType::Int);
    s.add_field("method", FieldType::String);
    s.add_field("flag", FieldType::Bool);
    s.add_field("headers", FieldType::Any);

    let cases = vec![
        "port == 80",
        "method == 'GET'",
        "flag && method == 'POST'",
        r#"headers["via"].exists(it, it == "X-MSIP-Via")"#,
        "port in [80, 443, 8080, 8443]",
        "port >= 1024 && port < 65535",
    ];

    for expr in &cases {
        match Filter::compile(expr, &s) {
            Ok(f) => {
                let size = mem::size_of_val(&*f.variables());
                println!("  {:40} {} vars, FilterNode heap depth ~{}", expr, f.variables().len(), "...");
            }
            Err(e) => {
                println!("  {:40} FAIL: {}", expr, e);
            }
        }
    }

    // Measure what the string cache does
    println!("\n═══ EvalContext overhead ═══");
    let mut s2 = Schema::new();
    let p = s2.add_field("port", FieldType::Int);
    let m = s2.add_field("method", FieldType::String);
    let h = s2.add_field("headers", FieldType::Any);
    let ctx = EvalContext::new(&s2);
    println!("  Empty EvalContext: {} bytes ({} fields)", 
        mem::size_of_val(ctx.as_slice()),
        ctx.len());
}
