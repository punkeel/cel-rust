use crate::common::ast::operators;
use crate::common::ast::{Expr, LiteralValue};
use crate::vm::filter_tree::{
    And, BoolFilter, EqIntConst, EqStrConst, GeIntConst, GtIntConst, InIntHashSet, InStrHashSet,
    LeIntConst, LtIntConst, NeIntConst, Not, Or,
};
use crate::Expression;
use std::collections::HashMap;

pub fn compile_filter_tree(expr: &Expression) -> Result<Box<dyn BoolFilter>, String> {
    let mut ctx = FilterCtx::new();
    let filter = compile_expr(&mut ctx, &expr.expr)?;
    Ok(filter)
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

fn compile_expr(
    ctx: &mut FilterCtx,
    expr: &Expr,
) -> Result<Box<dyn BoolFilter>, String> {
    match expr {
        Expr::Call(call) => {
            let name = call.func_name.as_str();

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

            if call.args.len() == 2 {
                if let Some(f) = try_compile_int_cmp(ctx, name, &call.args[0].expr, &call.args[1].expr) {
                    return Ok(f);
                }
                if let Some(f) = try_compile_str_cmp(ctx, name, &call.args[0].expr, &call.args[1].expr) {
                    return Ok(f);
                }
                if name == operators::IN {
                    if let Some(f) = try_compile_in_set(ctx, &call.args[0].expr, &call.args[1].expr) {
                        return Ok(f);
                    }
                }
            }

            Err(format!("unsupported filter expr: {}", name))
        }
        _ => Err("unsupported expr kind in filter tree".into()),
    }
}

fn try_compile_int_cmp(
    ctx: &mut FilterCtx,
    op: &str,
    left: &Expr,
    right: &Expr,
) -> Option<Box<dyn BoolFilter>> {
    let (var_name, val) = match (left, right) {
        (Expr::Ident(name), Expr::Literal(LiteralValue::Int(i))) => (name, *i.inner()),
        (Expr::Literal(LiteralValue::Int(i)), Expr::Ident(name)) => (name, *i.inner()),
        _ => return None,
    };
    let idx = ctx.var_idx(var_name);
    let f: Box<dyn BoolFilter> = match op {
        operators::EQUALS => Box::new(EqIntConst { var_idx: idx, val }),
        operators::NOT_EQUALS => Box::new(NeIntConst { var_idx: idx, val }),
        operators::LESS => Box::new(LtIntConst { var_idx: idx, val }),
        operators::LESS_EQUALS => Box::new(LeIntConst { var_idx: idx, val }),
        operators::GREATER => Box::new(GtIntConst { var_idx: idx, val }),
        operators::GREATER_EQUALS => Box::new(GeIntConst { var_idx: idx, val }),
        _ => return None,
    };
    Some(f)
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
        let set: std::collections::HashSet<String> = strs.into_iter().collect();
        return Some(Box::new(InStrHashSet { var_idx: idx, set }));
    }

    None
}
