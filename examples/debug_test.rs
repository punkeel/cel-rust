use cel::{Context, Program, Value};
use std::sync::Arc;

fn main() {
    let mut context = Context::default();
    let requests = vec![Value::Int(42), Value::Int(42)];
    context.add_variable("requests", Value::List(Arc::new(requests))).unwrap();
    context.add_variable("size", Value::Int(3)).unwrap();
    
    let tests = vec![
        "size(requests)",
        "size",
        "size(requests) + size",
        "size(requests) + size == 5",
    ];
    
    for expr in tests {
        let program = Program::compile(expr).unwrap();
        let result = program.execute(&context);
        println!("{} => {:?}", expr, result);
    }
}
