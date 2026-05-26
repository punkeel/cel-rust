use cel::{Context, Program};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use wirefilter::{ExecutionContext, SchemeBuilder, Type};

macro_rules! define_bench {
    (
        $name:ident,
        wirefilter_expr: $wf_expr:literal,
        cel_expr: $cel_expr:literal,
        scheme: [$($wf_field:ident : $wf_ty:expr),*],
        cel_vars: [$($cel_var:ident = $cel_val:expr),*],
        wirefilter_values: [$($wf_set_field:ident = $wf_set_val:expr),*],
        expect_true: $expect:expr
    ) => {
        fn $name(c: &mut Criterion) {
            let mut group = c.benchmark_group(stringify!($name));

            // -- wirefilter --
            {
                let mut builder = SchemeBuilder::default();
                $(builder.add_field(stringify!($wf_field), $wf_ty).unwrap();)*
                let scheme = builder.build();
                let ast = scheme.parse($wf_expr).unwrap();
                let filter = ast.compile();
                let mut wf_ctx = ExecutionContext::new(&scheme);
                $(wf_ctx.set_field_value(scheme.get_field(stringify!($wf_set_field)).unwrap(), $wf_set_val).unwrap();)*
                let result = filter.execute(&wf_ctx).unwrap();
                assert_eq!(result, $expect, "wirefilter result mismatch");

                group.bench_function("wirefilter", |b| {
                    b.iter(|| filter.execute(black_box(&wf_ctx)).unwrap())
                });
            }

            // -- CEL AST --
            {
                let program = Program::compile($cel_expr).unwrap();
                let mut cel_ctx = Context::default();
                $(cel_ctx.add_variable_from_value(stringify!($cel_var), $cel_val);)*
                let result = program.execute(&cel_ctx).unwrap();
                let result_bool = match result {
                    cel::Value::Bool(b) => b,
                    _ => panic!("CEL AST expected bool, got {:?}", result),
                };
                assert_eq!(result_bool, $expect, "CEL AST result mismatch");

                group.bench_function("cel_ast", |b| {
                    b.iter(|| program.execute(black_box(&cel_ctx)).unwrap())
                });
            }

            // -- CEL VM --
            {
                let program = Program::compile($cel_expr).unwrap();
                let vm = program.compile_vm().unwrap();
                let mut cel_ctx = Context::default();
                $(cel_ctx.add_variable_from_value(stringify!($cel_var), $cel_val);)*
                let mut state = cel::vm::EvalState::new();
                let result = cel::vm::eval(&vm, &cel_ctx, &mut state).unwrap();
                let result_bool = match result {
                    cel::Value::Bool(b) => b,
                    _ => panic!("CEL VM expected bool, got {:?}", result),
                };
                assert_eq!(result_bool, $expect, "CEL VM result mismatch");

                group.bench_function("cel_vm", |b| {
                    let mut state = cel::vm::EvalState::new();
                    b.iter(|| cel::vm::eval(black_box(&vm), black_box(&cel_ctx), &mut state).unwrap())
                });
            }

            group.finish();
        }
    };
}

define_bench!(
    int_eq,
    wirefilter_expr: "port == 80",
    cel_expr: "port == 80",
    scheme: [port: Type::Int],
    cel_vars: [port = 80i64],
    wirefilter_values: [port = 80i64],
    expect_true: true
);

define_bench!(
    int_range,
    wirefilter_expr: "port >= 1024 && port < 65535",
    cel_expr: "port >= 1024 && port < 65535",
    scheme: [port: Type::Int],
    cel_vars: [port = 8080i64],
    wirefilter_values: [port = 8080i64],
    expect_true: true
);

define_bench!(
    str_eq,
    wirefilter_expr: "method == \"GET\"",
    cel_expr: "method == \"GET\"",
    scheme: [method: Type::Bytes],
    cel_vars: [method = "GET"],
    wirefilter_values: [method = "GET"],
    expect_true: true
);

define_bench!(
    compound_and,
    wirefilter_expr: "method == \"GET\" && path == \"/api/v1/users\"",
    cel_expr: "method == \"GET\" && path ==\"/api/v1/users\"",
    scheme: [method: Type::Bytes, path: Type::Bytes],
    cel_vars: [method = "GET", path = "/api/v1/users"],
    wirefilter_values: [method = "GET", path = "/api/v1/users"],
    expect_true: true
);

define_bench!(
    in_set,
    wirefilter_expr: "port in { 80 443 8080 3000 }",
    cel_expr: "port in [80, 443, 8080, 3000]",
    scheme: [port: Type::Int],
    cel_vars: [port = 443i64],
    wirefilter_values: [port = 443i64],
    expect_true: true
);

// NOTE: wirefilter has a native `contains` operator for substring search.
// CEL supports this via `method.contains("GET")` but our VM does not yet
// compile string method calls, so this benchmark is omitted for fairness.

define_bench!(
    complex_bool,
    wirefilter_expr: "method == \"GET\" && port == 80 && path == \"/api/v1/users\"",
    cel_expr: "method == \"GET\" && port == 80 && path ==\"/api/v1/users\"",
    scheme: [method: Type::Bytes, port: Type::Int, path: Type::Bytes],
    cel_vars: [method = "GET", port = 80i64, path = "/api/v1/users"],
    wirefilter_values: [method = "GET", port = 80i64, path = "/api/v1/users"],
    expect_true: true
);

// Additional: larger set membership (wirefilter's strong suit)
define_bench!(
    large_in_set,
    wirefilter_expr: "port in { 80 443 8080 3000 5000 6000 7000 8000 9000 10000 11000 12000 13000 14000 15000 16000 17000 18000 19000 20000 }",
    cel_expr: "port in [80, 443, 8080, 3000, 5000, 6000, 7000, 8000, 9000, 10000, 11000, 12000, 13000, 14000, 15000, 16000, 17000, 18000, 19000, 20000]",
    scheme: [port: Type::Int],
    cel_vars: [port = 15000i64],
    wirefilter_values: [port = 15000i64],
    expect_true: true
);

// NOT + OR expressions
define_bench!(
    not_eq,
    wirefilter_expr: "not method == \"POST\"",
    cel_expr: "!(method == \"POST\")",
    scheme: [method: Type::Bytes],
    cel_vars: [method = "GET"],
    wirefilter_values: [method = "GET"],
    expect_true: true
);

define_bench!(
    or_eq,
    wirefilter_expr: "method == \"GET\" || method == \"POST\"",
    cel_expr: "method == \"GET\" || method == \"POST\"",
    scheme: [method: Type::Bytes],
    cel_vars: [method = "GET"],
    wirefilter_values: [method = "GET"],
    expect_true: true
);

criterion_group!(
    benches,
    int_eq,
    int_range,
    str_eq,
    compound_and,
    in_set,
    complex_bool,
    large_in_set,
    not_eq,
    or_eq
);

criterion_main!(benches);
