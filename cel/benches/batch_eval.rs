use cel::objects::Value;
use cel::vm::filter_tree::{AhoCorasickContains, BoolFilter, ContainsConst};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;

fn bench_aho_corasick(c: &mut Criterion) {
    let mut group = c.benchmark_group("aho_corasick");

    let haystack = Value::String(Arc::from("/api/v1/users/12345/admin/settings/healthz"));
    let vars = vec![haystack.clone()];

    // 5 individual contains filters (simulating 5 rules)
    let patterns = ["/api", "/admin", "/healthz", "/v2", "/graphql"];
    let individual_filters: Vec<_> = patterns
        .iter()
        .map(|p| ContainsConst {
            var_idx: 0,
            substring: p.to_string(),
        })
        .collect();

    group.bench_function("individual_5_contains", |b| {
        b.iter(|| {
            let mut matched = 0usize;
            for f in &individual_filters {
                if f.eval(black_box(&vars)) {
                    matched += 1;
                }
            }
            black_box(matched)
        })
    });

    // Aho-Corasick: scan once for all 5 patterns
    let ac = AhoCorasickContains {
        var_idx: 0,
        automaton: aho_corasick::AhoCorasick::new(patterns).unwrap(),
        min_matches: 1,
    };

    group.bench_function("aho_corasick_5_patterns", |b| {
        b.iter(|| black_box(ac.eval(black_box(&vars))))
    });

    // More patterns: 20
    let patterns_20 = [
        "/api", "/admin", "/healthz", "/v1", "/graphql", "/rest", "/rpc",
        "/ws", "/events", "/hooks", "/callback", "/auth", "/login", "/oauth",
        "/saml", "/mfa", "/sso", "/token", "/refresh", "/logout",
    ];
    let individual_20: Vec<_> = patterns_20
        .iter()
        .map(|p| ContainsConst {
            var_idx: 0,
            substring: p.to_string(),
        })
        .collect();

    group.bench_function("individual_20_contains", |b| {
        b.iter(|| {
            let mut matched = 0usize;
            for f in &individual_20 {
                if f.eval(black_box(&vars)) {
                    matched += 1;
                }
            }
            black_box(matched)
        })
    });

    let ac_20 = AhoCorasickContains {
        var_idx: 0,
        automaton: aho_corasick::AhoCorasick::new(patterns_20).unwrap(),
        min_matches: 1,
    };

    group.bench_function("aho_corasick_20_patterns", |b| {
        b.iter(|| black_box(ac_20.eval(black_box(&vars))))
    });

    // Worst case: haystack that matches NONE
    let no_match_haystack = Value::String(Arc::from("/home/about/contact"));
    let no_match_vars = vec![no_match_haystack];

    group.bench_function("individual_20_no_match", |b| {
        b.iter(|| {
            let mut matched = 0usize;
            for f in &individual_20 {
                if f.eval(black_box(&no_match_vars)) {
                    matched += 1;
                }
            }
            black_box(matched)
        })
    });

    group.bench_function("aho_corasick_20_no_match", |b| {
        b.iter(|| black_box(ac_20.eval(black_box(&no_match_vars))))
    });

    group.finish();
}

fn bench_ruleset_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("ruleset_batch");

    // Simulate a WAF: 50 rules, 10 of them are `path.contains(x)`
    let path = Value::String(Arc::from("/api/v1/users/12345/admin/settings/healthz"));
    let port = Value::Int(8080);
    let method = Value::String(Arc::from("GET"));
    let vars = vec![path.clone(), port, method];

    // 50 simple rules as individual FilterTree structs
    let rules: Vec<Box<dyn BoolFilter>> = vec![
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 80 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 443 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 8080 }),
        Box::new(cel::vm::filter_tree::GeIntConst { var_idx: 1, val: 1024 }),
        Box::new(cel::vm::filter_tree::EqStrConst { var_idx: 2, val: "GET".to_string() }),
        Box::new(cel::vm::filter_tree::EqStrConst { var_idx: 2, val: "POST".to_string() }),
        Box::new(cel::vm::filter_tree::InIntLinearSet { var_idx: 1, vals: vec![80, 443, 8080, 3000] }),
        Box::new(cel::vm::filter_tree::InIntLinearSet { var_idx: 1, vals: vec![22, 23, 25, 53] }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/api".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/admin".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/healthz".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/graphql".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/v1".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/v2".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/rpc".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/rest".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/ws".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/hooks".to_string() }),
        Box::new(cel::vm::filter_tree::ContainsConst { var_idx: 0, substring: "/auth".to_string() }),
        Box::new(cel::vm::filter_tree::And {
            a: cel::vm::filter_tree::GeIntConst { var_idx: 1, val: 80 },
            b: cel::vm::filter_tree::LeIntConst { var_idx: 1, val: 443 },
        }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 3306 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 5432 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 27017 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 6379 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 9200 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 9300 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 11211 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 2181 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 9092 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 9042 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 7000 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 5000 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 5001 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 5601 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 5044 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 9411 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 16686 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 14268 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 13133 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 100 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 101 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 102 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 103 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 104 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 105 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 106 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 107 }),
        Box::new(cel::vm::filter_tree::EqIntConst { var_idx: 1, val: 108 }),
    ];

    group.bench_function("linear_50_rules", |b| {
        b.iter(|| {
            let mut matches = 0usize;
            for rule in &rules {
                if rule.eval(black_box(&vars)) {
                    matches += 1;
                }
            }
            black_box(matches)
        })
    });

    // Optimized batch: separate the 10 contains patterns into one AC scan
    let ac_patterns = [
        "/api", "/admin", "/healthz", "/graphql", "/v1", "/v2", "/rpc",
        "/rest", "/ws", "/hooks", "/auth",
    ];
    let ac_filter = AhoCorasickContains {
        var_idx: 0,
        automaton: aho_corasick::AhoCorasick::new(ac_patterns).unwrap(),
        min_matches: 1,
    };
    // Non-contains rules (indices 0-7, 20-47 = 36 rules)
    let numeric_rules: Vec<&dyn BoolFilter> = vec![
        &*rules[0], &*rules[1], &*rules[2], &*rules[3], &*rules[4], &*rules[5],
        &*rules[6], &*rules[7], &*rules[20], &*rules[21], &*rules[22], &*rules[23],
        &*rules[24], &*rules[25], &*rules[26], &*rules[27], &*rules[28], &*rules[29],
        &*rules[30], &*rules[31], &*rules[32], &*rules[33], &*rules[34], &*rules[35],
        &*rules[36], &*rules[37], &*rules[38], &*rules[39], &*rules[40], &*rules[41],
        &*rules[42], &*rules[43], &*rules[44], &*rules[45], &*rules[46], &*rules[47],
    ];

    group.bench_function("batch_ac_plus_numeric", |b| {
        b.iter(|| {
            let mut matches = 0usize;
            if ac_filter.eval(black_box(&vars)) {
                matches += 1;
            }
            for rule in &numeric_rules {
                if rule.eval(black_box(&vars)) {
                    matches += 1;
                }
            }
            black_box(matches)
        })
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(30)
        .measurement_time(std::time::Duration::from_secs(2));
    targets = bench_aho_corasick, bench_ruleset_batch
}

criterion_main!(benches);
