use crate::common::ast::operators;
use crate::common::ast::{Expr, LiteralValue};
use crate::vm::filter_tree::{
    AhoCorasickContains, And, BoolFilter, ContainsConst, EndsWithConst, EqI64Expr, EqIntConst,
    EqStrConst, GeI64Expr, GeIntConst, GtI64Expr, GtIntConst, I64Expr, InIntHashSet,
    InIntLinearSet, InStrHashSet, InStrLinearSet, LeI64Expr, LeIntConst, ListExpr, LtI64Expr,
    LtIntConst, NeI64Expr, NeIntConst, Not, Or, StartsWithConst, StrExpr,
};
use crate::Expression;
use std::collections::HashMap;

pub struct CompiledFilterTree {
    pub filter: Box<dyn BoolFilter>,
    pub var_names: Vec<String>,
}

impl CompiledFilterTree {
    pub fn eval(&self, vars: &[crate::objects::Value]) -> bool {
        self.filter.eval(vars)
    }

    pub fn bind_vars(&self, ctx: &crate::Context) -> Vec<crate::objects::Value> {
        self.var_names
            .iter()
            .map(|name| {
                ctx.get_variable(name)
                    .and_then(|cow| crate::objects::Value::try_from(cow.as_ref()).ok())
                    .unwrap_or(crate::objects::Value::Null)
            })
            .collect()
    }
}

pub fn compile_filter_tree(expr: &Expression) -> Result<CompiledFilterTree, String> {
    let mut ctx = FilterCtx::new();
    let filter = compile_expr(&mut ctx, &expr.expr)?;
    Ok(CompiledFilterTree {
        filter,
        var_names: ctx.var_names,
    })
}

struct FilterCtx {
    var_map: HashMap<String, usize>,
    var_names: Vec<String>,
}

impl FilterCtx {
    fn new() -> Self {
        Self {
            var_map: HashMap::new(),
            var_names: Vec::new(),
        }
    }

    fn var_idx(&mut self, name: &str) -> usize {
        if let Some(&idx) = self.var_map.get(name) {
            return idx;
        }
        let idx = self.var_names.len();
        self.var_names.push(name.to_string());
        self.var_map.insert(name.to_string(), idx);
        idx
    }
}

fn compile_expr(ctx: &mut FilterCtx, expr: &Expr) -> Result<Box<dyn BoolFilter>, String> {
    match expr {
        Expr::Call(call) => {
            let name = call.func_name.as_str();

            // --- AC merging: try to merge OR/AND of contains before normal compilation ---
            if name == operators::LOGICAL_OR && call.args.len() == 2 {
                if let Some(f) = try_compile_ac_or(ctx, expr) {
                    return Ok(f);
                }
            }
            if name == operators::LOGICAL_AND && call.args.len() == 2 {
                if let Some(f) = try_compile_ac_and(ctx, expr) {
                    return Ok(f);
                }
            }

            if name == operators::LOGICAL_AND && call.args.len() == 2 {
                let a = compile_expr(ctx, &call.args[0].expr)?;
                let b = compile_expr(ctx, &call.args[1].expr)?;
                return Ok(Box::new(And { a, b }));
            }
            if name == operators::LOGICAL_OR && call.args.len() == 2 {
                let a = compile_expr(ctx, &call.args[0].expr)?;
                let b = compile_expr(ctx, &call.args[1].expr)?;
                return Ok(Box::new(Or { a, b }));
            }
            if name == operators::LOGICAL_NOT && call.args.len() == 1 {
                let inner = compile_expr(ctx, &call.args[0].expr)?;
                return Ok(Box::new(Not { inner }));
            }

            // String method calls: startsWith, endsWith, contains (function-style: contains(path, "/api"))
            if call.args.len() == 2 {
                if let Some(f) =
                    try_compile_str_bool(ctx, name, &call.args[0].expr, &call.args[1].expr)
                {
                    return Ok(f);
                }
            }

            // String method calls: target-based (path.contains("/api"))
            if let Some(f) = try_compile_target_str_bool(ctx, call) {
                return Ok(f);
            }

            if call.args.len() == 2 {
                if let Some(f) =
                    try_compile_int_cmp(ctx, name, &call.args[0].expr, &call.args[1].expr)
                {
                    return Ok(f);
                }
                if let Some(f) =
                    try_compile_str_cmp(ctx, name, &call.args[0].expr, &call.args[1].expr)
                {
                    return Ok(f);
                }
                if name == operators::IN {
                    if let Some(f) = try_compile_in_set(ctx, &call.args[0].expr, &call.args[1].expr)
                    {
                        return Ok(f);
                    }
                }
            }

            Err(format!("unsupported filter expr: {}", name))
        }
        _ => Err("unsupported expr kind in filter tree".into()),
    }
}

/// Try to compile a pure OR-tree of `.contains(literal)` into a single AC scan.
fn try_compile_ac_or(ctx: &mut FilterCtx, expr: &Expr) -> Option<Box<dyn BoolFilter>> {
    let mut patterns = Vec::new();
    let mut var_name: Option<String> = None;
    collect_contains_or(expr, &mut var_name, &mut patterns)?;
    if patterns.len() < 2 {
        return None; // Not worth AC for a single pattern
    }
    let var_name = var_name?;
    let idx = ctx.var_idx(&var_name);
    let automaton = aho_corasick::AhoCorasick::new(&patterns).ok()?;
    Some(Box::new(AhoCorasickContains {
        var_idx: idx,
        automaton,
        min_matches: 1,
    }))
}

/// Recursively collect all `.contains(literal)` from an OR tree.
/// Returns None if any non-contains leaf is found (not a pure OR-of-contains).
fn collect_contains_or(
    expr: &Expr,
    var_name: &mut Option<String>,
    patterns: &mut Vec<String>,
) -> Option<()> {
    match expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR && call.args.len() == 2 => {
            collect_contains_or(&call.args[0].expr, var_name, patterns)?;
            collect_contains_or(&call.args[1].expr, var_name, patterns)?;
            Some(())
        }
        _ => {
            let (vname, pat) = extract_contains(expr)?;
            match var_name {
                Some(existing) if existing != &vname => None,
                Some(_) => {
                    patterns.push(pat);
                    Some(())
                }
                None => {
                    *var_name = Some(vname);
                    patterns.push(pat);
                    Some(())
                }
            }
        }
    }
}

/// Try to compile a pure AND-tree of `.contains(literal)` into a single AC scan.
fn try_compile_ac_and(ctx: &mut FilterCtx, expr: &Expr) -> Option<Box<dyn BoolFilter>> {
    let mut patterns = Vec::new();
    let mut var_name: Option<String> = None;
    collect_contains_and(expr, &mut var_name, &mut patterns)?;
    if patterns.len() < 2 {
        return None;
    }
    let var_name = var_name?;
    let idx = ctx.var_idx(&var_name);
    let automaton = aho_corasick::AhoCorasick::new(&patterns).ok()?;
    Some(Box::new(AhoCorasickContains {
        var_idx: idx,
        automaton,
        min_matches: patterns.len(),
    }))
}

/// Recursively collect all `.contains(literal)` from an AND tree.
fn collect_contains_and(
    expr: &Expr,
    var_name: &mut Option<String>,
    patterns: &mut Vec<String>,
) -> Option<()> {
    match expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_AND && call.args.len() == 2 => {
            collect_contains_and(&call.args[0].expr, var_name, patterns)?;
            collect_contains_and(&call.args[1].expr, var_name, patterns)?;
            Some(())
        }
        _ => {
            let (vname, pat) = extract_contains(expr)?;
            match var_name {
                Some(existing) if existing != &vname => None,
                Some(_) => {
                    patterns.push(pat);
                    Some(())
                }
                None => {
                    *var_name = Some(vname);
                    patterns.push(pat);
                    Some(())
                }
            }
        }
    }
}

/// Extract `var.contains("literal")` → (var_name, literal).
fn extract_contains(expr: &Expr) -> Option<(String, String)> {
    let call = match expr {
        Expr::Call(call) => call,
        _ => return None,
    };
    if call.func_name.as_str() != "contains" {
        return None;
    }
    // Method call: receiver is in `target`, arg is in `args[0]`
    let var_name = match &call.target {
        Some(t) => match &t.expr {
            Expr::Ident(name) => name.clone(),
            _ => return None,
        },
        None => return None,
    };
    let literal = call.args.first().and_then(|a| match &a.expr {
        Expr::Literal(LiteralValue::String(s)) => Some(s.inner().to_string()),
        _ => None,
    })?;
    Some((var_name, literal))
}

fn try_compile_int_cmp(
    ctx: &mut FilterCtx,
    op: &str,
    left: &Expr,
    right: &Expr,
) -> Option<Box<dyn BoolFilter>> {
    let left_expr = try_compile_i64_expr(ctx, left)?;
    let right_expr = try_compile_i64_expr(ctx, right)?;
    let f: Box<dyn BoolFilter> = match op {
        operators::EQUALS => Box::new(EqI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        operators::NOT_EQUALS => Box::new(NeI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        operators::LESS => Box::new(LtI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        operators::LESS_EQUALS => Box::new(LeI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        operators::GREATER => Box::new(GtI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        operators::GREATER_EQUALS => Box::new(GeI64Expr {
            left: left_expr,
            right: right_expr,
        }),
        _ => return None,
    };
    Some(f)
}

fn try_compile_i64_expr(ctx: &mut FilterCtx, expr: &Expr) -> Option<I64Expr> {
    match expr {
        Expr::Literal(LiteralValue::Int(i)) => Some(I64Expr::Literal(*i.inner())),
        Expr::Ident(name) => Some(I64Expr::Var(ctx.var_idx(name))),
        Expr::Call(call) if call.args.len() == 2 => {
            let name = call.func_name.as_str();
            let a = try_compile_i64_expr(ctx, &call.args[0].expr)?;
            let b = try_compile_i64_expr(ctx, &call.args[1].expr)?;
            match name {
                operators::ADD => Some(I64Expr::Add(Box::new(a), Box::new(b))),
                operators::SUBSTRACT => Some(I64Expr::Sub(Box::new(a), Box::new(b))),
                operators::MULTIPLY => Some(I64Expr::Mul(Box::new(a), Box::new(b))),
                operators::DIVIDE => Some(I64Expr::Div(Box::new(a), Box::new(b))),
                operators::MODULO => Some(I64Expr::Mod(Box::new(a), Box::new(b))),
                _ => None,
            }
        }
        Expr::Call(call) if call.args.len() == 1 => {
            let name = call.func_name.as_str();
            if name == operators::NEGATE {
                let a = try_compile_i64_expr(ctx, &call.args[0].expr)?;
                Some(I64Expr::Neg(Box::new(a)))
            } else if name == "size" {
                if let Some(str_expr) = try_compile_str_expr(ctx, &call.args[0].expr) {
                    Some(I64Expr::StrLen(Box::new(str_expr)))
                } else if let Some(list_expr) = try_compile_list_expr(ctx, &call.args[0].expr) {
                    Some(I64Expr::ListLen(Box::new(list_expr)))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn try_compile_str_expr(ctx: &mut FilterCtx, expr: &Expr) -> Option<StrExpr> {
    match expr {
        Expr::Literal(LiteralValue::String(s)) => Some(StrExpr::Literal(s.inner().to_string())),
        Expr::Ident(name) => Some(StrExpr::Var(ctx.var_idx(name))),
        Expr::Call(call) if call.args.len() == 2 => {
            let name = call.func_name.as_str();
            if name == operators::ADD {
                let a = try_compile_str_expr(ctx, &call.args[0].expr)?;
                let b = try_compile_str_expr(ctx, &call.args[1].expr)?;
                Some(StrExpr::Concat(Box::new(a), Box::new(b)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn try_compile_list_expr(ctx: &mut FilterCtx, expr: &Expr) -> Option<ListExpr> {
    match expr {
        Expr::Ident(name) => Some(ListExpr::Var(ctx.var_idx(name))),
        _ => None,
    }
}

fn try_compile_str_cmp(
    ctx: &mut FilterCtx,
    op: &str,
    left: &Expr,
    right: &Expr,
) -> Option<Box<dyn BoolFilter>> {
    let (var_name, val) = match (left, right) {
        (Expr::Ident(name), Expr::Literal(LiteralValue::String(s))) => {
            (name, s.inner().to_string())
        }
        (Expr::Literal(LiteralValue::String(s)), Expr::Ident(name)) => {
            (name, s.inner().to_string())
        }
        _ => return None,
    };
    if op != operators::EQUALS {
        return None;
    }
    let idx = ctx.var_idx(var_name);
    Some(Box::new(EqStrConst { var_idx: idx, val }))
}

fn try_compile_str_bool(
    ctx: &mut FilterCtx,
    func: &str,
    receiver: &Expr,
    arg: &Expr,
) -> Option<Box<dyn BoolFilter>> {
    let var_name = match receiver {
        Expr::Ident(name) => name,
        _ => return None,
    };
    let val = match arg {
        Expr::Literal(LiteralValue::String(s)) => s.inner().to_string(),
        _ => return None,
    };
    let idx = ctx.var_idx(var_name);
    match func {
        "startsWith" => Some(Box::new(StartsWithConst { var_idx: idx, prefix: val })),
        "endsWith" => Some(Box::new(EndsWithConst { var_idx: idx, suffix: val })),
        "contains" => Some(Box::new(ContainsConst { var_idx: idx, substring: val })),
        _ => None,
    }
}

// Also handle target-based method calls (e.g. path.contains("/api"))
fn try_compile_target_str_bool(
    ctx: &mut FilterCtx,
    call: &crate::common::ast::CallExpr,
) -> Option<Box<dyn BoolFilter>> {
    let func = call.func_name.as_str();
    if !matches!(func, "startsWith" | "endsWith" | "contains") {
        return None;
    }
    let target_expr = call.target.as_ref()?;
    let var_name = match &target_expr.expr {
        Expr::Ident(name) => name,
        _ => return None,
    };
    let arg = call.args.first()?;
    let val = match &arg.expr {
        Expr::Literal(LiteralValue::String(s)) => s.inner().to_string(),
        _ => return None,
    };
    let idx = ctx.var_idx(var_name);
    match func {
        "startsWith" => Some(Box::new(StartsWithConst { var_idx: idx, prefix: val })),
        "endsWith" => Some(Box::new(EndsWithConst { var_idx: idx, suffix: val })),
        "contains" => Some(Box::new(ContainsConst { var_idx: idx, substring: val })),
        _ => None,
    }
}

fn try_compile_in_set(
    ctx: &mut FilterCtx,
    left: &Expr,
    right: &Expr,
) -> Option<Box<dyn BoolFilter>> {
    let var_name = match left {
        Expr::Ident(name) => name,
        _ => return None,
    };
    let list = match right {
        Expr::List(list) => &list.elements,
        _ => return None,
    };
    let idx = ctx.var_idx(var_name);

    // Try all-int
    let mut ints = Vec::with_capacity(list.len());
    for item in list {
        if let Expr::Literal(LiteralValue::Int(i)) = &item.expr {
            ints.push(*i.inner());
        } else {
            ints.clear();
            break;
        }
    }
    if ints.len() == list.len() {
        if ints.len() <= 16 {
            return Some(Box::new(InIntLinearSet { var_idx: idx, vals: ints }));
        }
        let set: std::collections::HashSet<i64> = ints.into_iter().collect();
        return Some(Box::new(InIntHashSet { var_idx: idx, set }));
    }

    // Try all-string
    let mut strs = Vec::with_capacity(list.len());
    for item in list {
        if let Expr::Literal(LiteralValue::String(s)) = &item.expr {
            strs.push(s.inner().to_string());
        } else {
            strs.clear();
            break;
        }
    }
    if strs.len() == list.len() {
        if strs.len() <= 16 {
            return Some(Box::new(InStrLinearSet { var_idx: idx, vals: strs }));
        }
        let set: std::collections::HashSet<String> = strs.into_iter().collect();
        return Some(Box::new(InStrHashSet { var_idx: idx, set }));
    }

    None
}
