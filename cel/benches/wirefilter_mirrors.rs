use cel::{Context, Program};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::net::IpAddr;
use wirefilter::{ExecutionContext, SchemeBuilder, Type};

// Helper: run a single wirefilter-like benchmark for a field type
fn wirefilter_bench<T: Copy + std::fmt::Debug>(
    c: &mut Criterion,
    field: &str,
    ty: Type,
    filters: &[&str],
    values: &[T],
) where
    T: Into<wirefilter::LhsValue<'static>> + Clone,
{
    for &filter in filters {
        let name = if let Some(pos) = filter.find(" in {") {
            format!("{} in ...", &filter[..pos])
        } else {
            filter.to_string()
        };

        // --- Parsing (wirefilter) ---
        {
            let mut group = c.benchmark_group(&format!("{}/parse", field));
            let mut builder = SchemeBuilder::default();
            builder.add_field(field, ty).unwrap();
            let scheme = builder.build();
            group.bench_function(&name, |b| b.iter(|| scheme.parse(filter).unwrap()));
            group.finish();
        }

        // --- Execution (wirefilter) ---
        {
            let mut group = c.benchmark_group(&format!("{}/exec", field));
            let mut builder = SchemeBuilder::default();
            builder.add_field(field, ty).unwrap();
            let scheme = builder.build();
            let ast = scheme.parse(filter).unwrap();
            let filter_compiled = ast.compile();

            for value in values {
                let label = format!("{:?}", value);
                let mut exec_ctx = ExecutionContext::new(&scheme);
                exec_ctx
                    .set_field_value(scheme.get_field(field).unwrap(), *value)
                    .unwrap();

                group.bench_function(&format!("{}/{}_{}", name, label, "wirefilter"), |b| {
                    b.iter(|| filter_compiled.execute(black_box(&exec_ctx)).unwrap())
                });
            }
            group.finish();
        }
    }
}

// --- 1. IP comparisons ---
fn bench_ip(c: &mut Criterion) {
    wirefilter_bench(
        c,
        "ip.addr",
        Type::Ip,
        &[
            "ip.addr == 173.245.48.1",
            "ip.addr == 2606:4700:4700::1111",
            "ip.addr >= 173.245.48.0 && ip.addr < 173.245.49.0",
            "ip.addr >= 2606:4700:: && ip.addr < 2606:4701::",
            "ip.addr in { 103.21.244.0/22 2405:8100::/32 104.16.0.0/12 2803:f800::/32 131.0.72.0/22 173.245.48.0/20 2405:b500::/32 172.64.0.0/13 190.93.240.0/20 103.22.200.0/22 2606:4700::/32 198.41.128.0/17 197.234.240.0/22 162.158.0.0/15 108.162.192.0/18 2c0f:f248::/32 2400:cb00::/32 103.31.4.0/22 2a06:98c0::/29 141.101.64.0/18 188.114.96.0/20 }"
        ],
        &[
            IpAddr::from([127, 0, 0, 1]),
            IpAddr::from([0x2001, 0x0db8, 0x85a3, 0x0000, 0x0000, 0x8a2e, 0x0370, 0x7334]),
            IpAddr::from([173, 245, 48, 1]),
            IpAddr::from([0x2606, 0x4700, 0x4700, 0x0000, 0x0000, 0x0000, 0x0000, 0x1111]),
        ],
    );
}

// --- 2. Int comparisons ---
fn bench_int(c: &mut Criterion) {
    wirefilter_bench(
        c,
        "tcp.port",
        Type::Int,
        &[
            "tcp.port == 80",
            "tcp.port >= 1024",
            "tcp.port in { 80 8080 8880 2052 2082 2086 2095 }",
        ],
        &[80i64, 8081i64],
    );

    // CEL comparison for tcp.port
    #[cfg(feature = "regex")]
    {
        let mut group = c.benchmark_group("tcp.port/cel");
        let expressions = [
            ("eq_80", "port == 80"),
            ("ge_1024", "port >= 1024"),
            ("in_set", "port in [80, 8080, 8880, 2052, 2082, 2086, 2095]"),
        ];
        for (name, expr) in &expressions {
            let ast = Program::compile(expr).unwrap();
            let vm = ast.compile_vm().unwrap();
            let tree = cel::vm::compile_filter_tree(ast.expression()).unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("port", 8081i64);

            group.bench_function(BenchmarkId::new("ast", name), |b| {
                b.iter(|| ast.execute(black_box(&ctx)).unwrap())
            });
            group.bench_function(BenchmarkId::new("vm", name), |b| {
                let mut state = cel::vm::EvalState::new();
                b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
            });
            group.bench_function(BenchmarkId::new("tree", name), |b| {
                let vars = tree.bind_vars(&ctx);
                b.iter(|| tree.filter.eval(black_box(&vars)))
            });
        }
        group.finish();
    }
}

// --- 3. String comparisons ---
fn bench_string(c: &mut Criterion) {
    wirefilter_bench(
        c,
        "ip.geoip.country",
        Type::Bytes,
        &[
            r#"ip.geoip.country == "GB""#,
            r#"ip.geoip.country in { "AT" "BE" "BG" "HR" "CY" "CZ" "DK" "EE" "FI" "FR" "DE" "GR" "HU" "IE" "IT" "LV" "LT" "LU" "MT" "NL" "PL" "PT" "RO" "SK" "SI" "ES" "SE" "GB" "GF" "GP" "MQ" "ME" "YT" "RE" "MF" "GI" "AX" "PM" "GL" "BL" "SX" "AW" "CW" "WF" "PF" "NC" "TF" "AI" "BM" "IO" "VG" "KY" "FK" "MS" "PN" "SH" "GS" "TC" "AD" "LI" "MC" "SM" "VA" "JE" "GG" "GI" "CH" }"#,
        ],
        &["GB", "T1"],
    );
}

// --- 4. String matches (regex + contains) ---
fn bench_string_matches(c: &mut Criterion) {
    wirefilter_bench(
        c,
        "http.user_agent",
        Type::Bytes,
        &[
            r#"http.user_agent ~ "(?i)googlebot/\d+\.\d+""#,
            r#"http.user_agent ~ "Googlebot""#,
            r#"http.user_agent contains "Googlebot""#,
        ],
        &[
            "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Safari/537.36",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.103 Safari/537.36"
        ],
    );

    // CEL regex comparison
    #[cfg(feature = "regex")]
    {
        let mut group = c.benchmark_group("http.user_agent/cel");
        let expressions = [
            ("regex_complex", r#"ua.matches(r'(?i)googlebot/\d+\.\d+')"#),
            ("regex_simple", r#"ua.matches(r'Googlebot')"#),
            ("contains", r#"ua.contains('Googlebot')"#),
        ];
        let ua_google = "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Safari/537.36";

        for (name, expr) in &expressions {
            let ast = Program::compile(expr).unwrap();
            let vm = ast.compile_vm().unwrap();
            let mut ctx = Context::default();
            ctx.add_variable_from_value("ua", ua_google);

            group.bench_function(BenchmarkId::new("ast", name), |b| {
                b.iter(|| ast.execute(black_box(&ctx)).unwrap())
            });
            group.bench_function(BenchmarkId::new("vm", name), |b| {
                let mut state = cel::vm::EvalState::new();
                b.iter(|| cel::vm::eval(black_box(&vm), black_box(&ctx), &mut state).unwrap())
            });
            // Filter tree only works for bool-returning expressions; these are all bool
            if let Ok(tree) = cel::vm::compile_filter_tree(ast.expression()) {
                group.bench_function(BenchmarkId::new("tree", name), |b| {
                    let vars = tree.bind_vars(&ctx);
                    b.iter(|| tree.filter.eval(black_box(&vars)))
                });
            }
        }
        group.finish();
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(30)
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_ip, bench_int, bench_string, bench_string_matches
}

criterion_main!(benches);
