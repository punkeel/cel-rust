use crate::common::ast::operators;
use crate::common::ast::{Expr, LiteralValue};
use crate::vm::filter_tree::{CompiledFilterNode, FilterNode, I64Expr, ItemPredicate, ListExpr, StrExpr};
use crate::objects::Value;
use crate::Expression;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct CompiledFilterTree {
    pub compiled: CompiledFilterNode,
    pub var_names: Vec<String>,
}

impl CompiledFilterTree {
    pub fn eval(&self, vars: &[crate::objects::Value]) -> bool {
        matches!(self.compiled.eval(vars), Value::Bool(true))
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

/// Compile an expression to a filter tree, using a Schema's field ordering
/// for variable index assignment instead of auto-assignment.
///
/// `field_names` provides the name → index mapping: the position in the Vec
/// is the index used in tree nodes and expected by EvalContext.
pub fn compile_filter_tree_with_schema(
    expr: &Expression,
    field_names: &[&str],
    bool_fields: &HashSet<String>,
) -> Result<CompiledFilterTree, String> {
    let mut ctx = FilterCtx::with_schema(field_names, bool_fields);
    let filter = compile_expr(&mut ctx, &expr.expr)?;
    let compiled = filter.compile();
    Ok(CompiledFilterTree {
        compiled,
        var_names: ctx.var_names,
    })
}

pub fn compile_filter_tree(expr: &Expression) -> Result<CompiledFilterTree, String> {
    let mut ctx = FilterCtx::new();
    let filter = compile_expr(&mut ctx, &expr.expr)?;
    let compiled = filter.compile();
    Ok(CompiledFilterTree {
        compiled,
        var_names: ctx.var_names,
    })
}

struct FilterCtx {
    var_map: HashMap<String, usize>,
    var_names: Vec<String>,
    bool_fields: HashSet<String>,
}

impl FilterCtx {
    fn new() -> Self {
        Self {
            var_map: HashMap::new(),
            var_names: Vec::new(),
            bool_fields: HashSet::new(),
        }
    }

    /// Create a context pre-populated with schema field names.
    /// The field_names Vec provides the name → index mapping.
    fn with_schema(field_names: &[&str], bool_fields: &HashSet<String>) -> Self {
        let mut var_map = HashMap::with_capacity(field_names.len());
        let mut var_names = Vec::with_capacity(field_names.len());
        for (idx, name) in field_names.iter().enumerate() {
            var_map.insert((*name).to_string(), idx);
            var_names.push((*name).to_string());
        }
        Self {
            var_map,
            var_names,
            bool_fields: bool_fields.clone(),
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

fn compile_expr(ctx: &mut FilterCtx, expr: &Expr) -> Result<Box<FilterNode>, String> {
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
                return Ok(Box::new(FilterNode::And(a, b)));
            }
            if name == operators::LOGICAL_OR && call.args.len() == 2 {
                let a = compile_expr(ctx, &call.args[0].expr)?;
                let b = compile_expr(ctx, &call.args[1].expr)?;
                return Ok(Box::new(FilterNode::Or(a, b)));
            }
            if name == operators::LOGICAL_NOT && call.args.len() == 1 {
                let inner = compile_expr(ctx, &call.args[0].expr)?;
                return Ok(Box::new(FilterNode::Not(inner)));
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
        Expr::Comprehension(comp) => {
            // Detect exists() pattern: accu_var="@result", accu_init=false,
            // loop_step = @result || <predicate>, iter_var2 = None.
            let is_exists = matches!(&comp.accu_init.expr,
                Expr::Literal(LiteralValue::Boolean(b)) if !*b.inner()
            ) && comp.accu_var == "@result"
              && comp.iter_var2.is_none();

            if !is_exists {
                return Err("unsupported comprehension pattern".into());
            }

            // Detect map["key"].exists(x, x == "val") — single lookup, no alloc
            // iter_range must be _[_](map, "literal_key")
            // predicate must be x == "literal_val" or "literal_val" == x
            if let Some(f) = try_compile_map_key_exists(ctx, comp) {
                return Ok(f);
            }

            // Detect list.exists(it, it in [int1, int2, ...]) — direct scan, no alloc
            if let Some(f) = try_compile_exists_in_int_set(ctx, comp) {
                return Ok(f);
            }

            // Detect list.exists(it, it == int_val) — direct scan, no alloc
            if let Some(f) = try_compile_exists_eq_int(ctx, comp) {
                return Ok(f);
            }
            // Try to compile the predicate into a closure (any supported pattern)
            // This avoids the scratch vector allocation entirely.
            if let Some(f) = try_compile_exists_closure(ctx, comp) {
                return Ok(f);
            }

            // Fall through to generic Exists with scratch vector

            // Extract the list variable from iter_range
            let list_name = match &comp.iter_range.expr {
                Expr::Ident(name) => name.clone(),
                _ => return Err("exists() on non-ident range not supported".into()),
            };
            let list_idx = ctx.var_idx(&list_name);

            // Register the iteration variable (gets index = field_count)
            let item_idx = ctx.var_idx(&comp.iter_var);

            // Extract predicate from loop_step: @result || <predicate>
            let predicate = match &comp.loop_step.expr {
                Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR
                    && call.args.len() == 2 =>
                {
                    match &call.args[0].expr {
                        Expr::Ident(name) if name == "@result" => {
                            compile_expr(ctx, &call.args[1].expr)?
                        }
                        _ => return Err("unsupported exists() predicate structure".into()),
                    }
                }
                _ => return Err("unsupported exists() pattern".into()),
            };

            Ok(Box::new(FilterNode::Exists {
                list_idx,
                item_idx,
                predicate,
            }))
        }
        Expr::Ident(name) => {
            // Boolean-typed fields can be used directly as predicates.
            if ctx.bool_fields.contains(name.as_str()) {
                let idx = ctx.var_idx(name);
                return Ok(Box::new(FilterNode::BoolVar { idx }));
            }
            Err("unsupported expr kind in filter tree".into())
        }
        _ => Err("unsupported expr kind in filter tree".into()),
    }
}

/// Try to compile `map["key"].exists(x, x == "value")` as a single
/// `MapKeyContains` node — no Vec allocation, no per-item iteration.
fn try_compile_map_key_exists(
    ctx: &mut FilterCtx,
    comp: &crate::common::ast::ComprehensionExpr,
) -> Option<Box<FilterNode>> {
    // iter_range must be _[_](map, "literal_key")
    let (map_name, map_key) = match &comp.iter_range.expr {
        Expr::Call(call) if call.func_name.as_str() == operators::INDEX
            && call.args.len() >= 1 =>
        {
            let map_name = match &call.args[0].expr {
                Expr::Ident(name) => name.clone(),
                _ => return None,
            };
            let map_key = match call.args.get(1).map(|a| &a.expr) {
                Some(Expr::Literal(LiteralValue::String(s))) => s.inner().to_string(),
                _ => return None, // non-literal key
            };
            (map_name, map_key)
        }
        _ => return None,
    };

    // Predicate must be x == "value" or "value" == x (inside @result || predicate)
    let pred = match &comp.loop_step.expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR
            && call.args.len() == 2 =>
        {
            match &call.args[0].expr {
                Expr::Ident(name) if name == "@result" => &call.args[1].expr,
                _ => return None,
            }
        }
        _ => return None,
    };

    // Extract needle from the equality check
    let needle = match pred {
        Expr::Call(call) if call.func_name.as_str() == operators::EQUALS
            && call.args.len() == 2 =>
        {
            let ident_name = comp.iter_var.as_str();
            // Match either: x == "val" or "val" == x
            match (&call.args[0].expr, &call.args[1].expr) {
                (Expr::Ident(name), Expr::Literal(LiteralValue::String(s)))
                    if name == ident_name => s.inner().to_string(),
                (Expr::Literal(LiteralValue::String(s)), Expr::Ident(name))
                    if name == ident_name => s.inner().to_string(),
                _ => return None,
            }
        }
        _ => return None,
    };

    let map_idx = ctx.var_idx(&map_name);
    Some(Box::new(FilterNode::MapKeyContains {
        map_idx,
        key: map_key,
        needle,
    }))
}

/// Try to compile `list.exists(it, it in [int1, int2, ...])` as a single
/// `ExistsInIntSet` node — no allocation, no scratch vector, no item clone.
/// Scans the list directly and checks each element against the embedded set.
fn try_compile_exists_in_int_set(
    ctx: &mut FilterCtx,
    comp: &crate::common::ast::ComprehensionExpr,
) -> Option<Box<FilterNode>> {
    // iter_range must be Expr::Ident (a field name)
    let list_name = match &comp.iter_range.expr {
        Expr::Ident(name) => name.clone(),
        _ => return None,
    };

    // Extract predicate from loop_step: @result || <predicate>
    let pred = match &comp.loop_step.expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR
            && call.args.len() == 2 =>
        {
            match &call.args[0].expr {
                Expr::Ident(name) if name == "@result" => &call.args[1].expr,
                _ => return None,
            }
        }
        _ => return None,
    };

    // Predicate must be: it @in [int_literals]
    let vals = match pred {
        Expr::Call(call) if call.func_name.as_str() == operators::IN
            && call.args.len() == 2 =>
        {
            // Left must be the iteration variable
            match &call.args[0].expr {
                Expr::Ident(name) if name == comp.iter_var.as_str() => {},
                _ => return None,
            }
            // Right must be a list of int literals
            match &call.args[1].expr {
                Expr::List(list) => {
                    let mut vals = Vec::with_capacity(list.elements.len());
                    for item in &list.elements {
                        match &item.expr {
                            Expr::Literal(LiteralValue::Int(n)) => vals.push(*n.inner()),
                            _ => return None, // non-int literal in list
                        }
                    }
                    vals
                }
                _ => return None,
            }
        }
        _ => return None,
    };

    let list_idx = ctx.var_idx(&list_name);
    Some(Box::new(FilterNode::ExistsInIntSet { list_idx, vals }))
}

/// Try to compile `list.exists(it, it == int_val)` as a single
/// `ExistsEqInt` node — no allocation, no scratch vector.
fn try_compile_exists_eq_int(
    ctx: &mut FilterCtx,
    comp: &crate::common::ast::ComprehensionExpr,
) -> Option<Box<FilterNode>> {
    // iter_range must be Expr::Ident
    let list_name = match &comp.iter_range.expr {
        Expr::Ident(name) => name.clone(),
        _ => return None,
    };

    // Extract predicate from loop_step
    let pred = match &comp.loop_step.expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR
            && call.args.len() == 2 =>
        {
            match &call.args[0].expr {
                Expr::Ident(name) if name == "@result" => &call.args[1].expr,
                _ => return None,
            }
        }
        _ => return None,
    };

    // Predicate must be: it == int_val or int_val == it
    let val = match pred {
        Expr::Call(call) if call.func_name.as_str() == operators::EQUALS
            && call.args.len() == 2 =>
        {
            let ident_name = comp.iter_var.as_str();
            match (&call.args[0].expr, &call.args[1].expr) {
                (Expr::Ident(name), Expr::Literal(LiteralValue::Int(n)))
                    if name == ident_name => *n.inner(),
                (Expr::Literal(LiteralValue::Int(n)), Expr::Ident(name))
                    if name == ident_name => *n.inner(),
                _ => return None,
            }
        }
        _ => return None,
    };

    let list_idx = ctx.var_idx(&list_name);
    Some(Box::new(FilterNode::ExistsEqInt { list_idx, val }))
}

/// Try to compile `list.exists(it, predicate)` as `ExistsClosure`.
/// Compiles the predicate to a `Box<dyn Fn(&Value) -> bool>` closure,
/// avoiding scratch vector allocation at eval time.
fn try_compile_exists_closure(
    ctx: &mut FilterCtx,
    comp: &crate::common::ast::ComprehensionExpr,
) -> Option<Box<FilterNode>> {
    let list_name = match &comp.iter_range.expr {
        Expr::Ident(name) => name.clone(),
        _ => return None,
    };
    let pred = match &comp.loop_step.expr {
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR
            && call.args.len() == 2 =>
        {
            match &call.args[0].expr {
                Expr::Ident(name) if name == "@result" => &call.args[1].expr,
                _ => return None,
            }
        }
        _ => return None,
    };
    let closure = try_compile_item_predicate(pred, comp.iter_var.as_str())?;
    let list_idx = ctx.var_idx(&list_name);
    Some(Box::new(FilterNode::ExistsClosure {
        list_idx,
        predicate: ItemPredicate::new(closure),
    }))
}

/// Compile a CEL predicate expression (operating on a single item variable)
/// into a `Box<dyn Fn(&Value) -> bool>` closure.
///
/// Patterns handled:
///   - `x == literal` (int, string)
///   - `x != literal` (int, string)
///   - `x @in [literal, ...]` (int list, string list)
///   - `x.startsWith("str")`, `x.endsWith("str")`, `x.contains("str")`
///   - `x.matches("regex")`
///   - `a && b`, `a || b`, `!a`
fn try_compile_item_predicate(
    pred: &Expr,
    iter_var: &str,
) -> Option<Box<dyn Fn(&Value) -> bool>> {
    match pred {
        Expr::Call(call) if call.func_name.as_str() == operators::EQUALS && call.args.len() == 2 => {
            let ident_name = iter_var;
            if let (Expr::Ident(name), Expr::Literal(LiteralValue::Int(n))) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let val = *n.inner(); return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i == val))); }
            }
            if let (Expr::Literal(LiteralValue::Int(n)), Expr::Ident(name)) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let val = *n.inner(); return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i == val))); }
            }
            if let (Expr::Ident(name), Expr::Literal(LiteralValue::String(s))) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let needle: Arc<str> = Arc::from(s.inner()); return Some(Box::new(move |v| matches!(v, Value::String(s) if s.as_ref() == needle.as_ref()))); }
            }
            if let (Expr::Literal(LiteralValue::String(s)), Expr::Ident(name)) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let needle: Arc<str> = Arc::from(s.inner()); return Some(Box::new(move |v| matches!(v, Value::String(s) if s.as_ref() == needle.as_ref()))); }
            }
            None
        }
        Expr::Call(call) if call.func_name.as_str() == operators::NOT_EQUALS && call.args.len() == 2 => {
            let ident_name = iter_var;
            if let (Expr::Ident(name), Expr::Literal(LiteralValue::Int(n))) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let val = *n.inner(); return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i != val))); }
            }
            if let (Expr::Literal(LiteralValue::Int(n)), Expr::Ident(name)) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let val = *n.inner(); return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i != val))); }
            }
            if let (Expr::Ident(name), Expr::Literal(LiteralValue::String(s))) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let needle: Arc<str> = Arc::from(s.inner()); return Some(Box::new(move |v| matches!(v, Value::String(s) if s.as_ref() != needle.as_ref()))); }
            }
            if let (Expr::Literal(LiteralValue::String(s)), Expr::Ident(name)) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name { let needle: Arc<str> = Arc::from(s.inner()); return Some(Box::new(move |v| matches!(v, Value::String(s) if s.as_ref() != needle.as_ref()))); }
            }
            None
        }
        Expr::Call(call) if call.func_name.as_str() == operators::IN && call.args.len() == 2 => {
            match &call.args[0].expr { Expr::Ident(name) if name == iter_var => {} _ => return None }
            match &call.args[1].expr {
                Expr::List(list) => {
                    let mut ints = Vec::new();
                    let mut all_int = true;
                    for item in &list.elements {
                        match &item.expr { Expr::Literal(LiteralValue::Int(n)) => ints.push(*n.inner()), _ => { all_int = false; break; } }
                    }
                    if all_int && !ints.is_empty() { return Some(Box::new(move |v| matches!(v, Value::Int(i) if ints.contains(i)))); }
                    let mut strs: Vec<Arc<str>> = Vec::new();
                    let mut all_str = true;
                    for item in &list.elements {
                        match &item.expr { Expr::Literal(LiteralValue::String(s)) => strs.push(Arc::from(s.inner())), _ => { all_str = false; break; } }
                    }
                    if all_str && !strs.is_empty() { return Some(Box::new(move |v| matches!(v, Value::String(s) if strs.iter().any(|x| x.as_ref() == s.as_ref())))); }
                    None
                }
                _ => None,
            }
        }
        Expr::Call(call) if call.args.len() == 1 => {
            let func = call.func_name.as_str();
            if !matches!(func, "startsWith" | "endsWith" | "contains" | "matches") { return None; }
            let target_expr = call.target.as_ref()?;
            match &target_expr.expr { Expr::Ident(name) if name == iter_var => {} _ => return None }
            let val = match &call.args[0].expr { Expr::Literal(LiteralValue::String(s)) => s.inner().to_string(), _ => return None };
            match func {
                "startsWith" => { let p: Arc<str> = Arc::from(val.as_str()); Some(Box::new(move |v| matches!(v, Value::String(s) if s.starts_with(p.as_ref())))) }
                "endsWith" => { let suffix: Arc<str> = Arc::from(val.as_str()); Some(Box::new(move |v| matches!(v, Value::String(s) if s.ends_with(suffix.as_ref())))) }
                "contains" => { let sub: Arc<str> = Arc::from(val.as_str()); Some(Box::new(move |v| matches!(v, Value::String(s) if s.contains(sub.as_ref())))) }
                "matches" => match regex::Regex::new(&val) { Ok(re) => Some(Box::new(move |v| matches!(v, Value::String(s) if re.is_match(s.as_ref())))), Err(_) => None }
                _ => None,
            }
        }
        // ── Comparison operators (it > literal, etc.) ──
        Expr::Call(call) if call.args.len() == 2 && matches!(call.func_name.as_str(), operators::GREATER_EQUALS | operators::GREATER | operators::LESS_EQUALS | operators::LESS) => {
            let ident_name = iter_var;
            if let (Expr::Ident(name), Expr::Literal(LiteralValue::Int(n))) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name {
                    let val = *n.inner();
                    match call.func_name.as_str() {
                        operators::GREATER_EQUALS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i >= val))),
                        operators::GREATER => return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i > val))),
                        operators::LESS_EQUALS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i <= val))),
                        operators::LESS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if *i < val))),
                        _ => {}
                    }
                }
            }
            if let (Expr::Literal(LiteralValue::Int(n)), Expr::Ident(name)) = (&call.args[0].expr, &call.args[1].expr) {
                if name == ident_name {
                    let val = *n.inner();
                    match call.func_name.as_str() {
                        operators::GREATER_EQUALS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if val >= *i))),
                        operators::GREATER => return Some(Box::new(move |v| matches!(v, Value::Int(i) if val > *i))),
                        operators::LESS_EQUALS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if val <= *i))),
                        operators::LESS => return Some(Box::new(move |v| matches!(v, Value::Int(i) if val < *i))),
                        _ => {}
                    }
                }
            }
            None
        }
        // ── AND composition ──
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_AND && call.args.len() == 2 => {
            let a = try_compile_item_predicate(&call.args[0].expr, iter_var)?;
            let b = try_compile_item_predicate(&call.args[1].expr, iter_var)?;
            Some(Box::new(move |v| a(v) && b(v)))
        }
        // ── OR composition ──
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_OR && call.args.len() == 2 => {
            let a = try_compile_item_predicate(&call.args[0].expr, iter_var)?;
            let b = try_compile_item_predicate(&call.args[1].expr, iter_var)?;
            Some(Box::new(move |v| a(v) || b(v)))
        }
        Expr::Call(call) if call.func_name.as_str() == operators::LOGICAL_NOT && call.args.len() == 1 => {
            let inner = try_compile_item_predicate(&call.args[0].expr, iter_var)?;
            Some(Box::new(move |v| !inner(v)))
        }
        _ => None,
    }
}

/// Try to compile a pure OR-tree of `.contains(literal)` into a single AC scan.
fn try_compile_ac_or(ctx: &mut FilterCtx, expr: &Expr) -> Option<Box<FilterNode>> {
    let mut patterns = Vec::new();
    let mut var_name: Option<String> = None;
    collect_contains_or(expr, &mut var_name, &mut patterns)?;
    if patterns.len() < 2 {
        return None; // Not worth AC for a single pattern
    }
    let var_name = var_name?;
    let idx = ctx.var_idx(&var_name);

    // For very small pattern counts on short strings, naive search wins over AC.
    if patterns.len() <= 4 {
        return Some(Box::new(FilterNode::ContainsAny {
            idx,
            needles: patterns,
        }));
    }

    let automaton = aho_corasick::AhoCorasick::new(&patterns).ok()?;
    Some(Box::new(FilterNode::AhoContains {
        idx,
        ac: automaton,
        min: 1,
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
fn try_compile_ac_and(ctx: &mut FilterCtx, expr: &Expr) -> Option<Box<FilterNode>> {
    let mut patterns = Vec::new();
    let mut var_name: Option<String> = None;
    collect_contains_and(expr, &mut var_name, &mut patterns)?;
    if patterns.len() < 2 {
        return None;
    }
    let var_name = var_name?;
    let idx = ctx.var_idx(&var_name);
    let automaton = aho_corasick::AhoCorasick::new(&patterns).ok()?;
    Some(Box::new(FilterNode::AhoContains {
        idx,
        ac: automaton,
        min: patterns.len(),
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
) -> Option<Box<FilterNode>> {
    // Fast path: var op literal  (e.g. port == 80)
    fn try_var_lit(ctx: &mut FilterCtx, var_expr: &Expr, lit_expr: &Expr) -> Option<(usize, i64)> {
        let name = match var_expr {
            Expr::Ident(name) => name,
            _ => return None,
        };
        let val = match lit_expr {
            Expr::Literal(LiteralValue::Int(i)) => *i.inner(),
            _ => return None,
        };
        Some((ctx.var_idx(name), val))
    }

    if let Some((idx, val)) = try_var_lit(ctx, left, right) {
        return Some(match op {
            operators::EQUALS => Box::new(FilterNode::EqInt { idx, val }),
            operators::NOT_EQUALS => Box::new(FilterNode::NeInt { idx, val }),
            operators::LESS => Box::new(FilterNode::LtInt { idx, val }),
            operators::LESS_EQUALS => Box::new(FilterNode::LeInt { idx, val }),
            operators::GREATER => Box::new(FilterNode::GtInt { idx, val }),
            operators::GREATER_EQUALS => Box::new(FilterNode::GeInt { idx, val }),
            _ => return None,
        });
    }

    // Fast path: literal op var  (e.g. 80 == port)
    if let Some((idx, val)) = try_var_lit(ctx, right, left) {
        // Swap operator: 80 == port  =>  port == 80
        return Some(match op {
            operators::EQUALS => Box::new(FilterNode::EqInt { idx, val }),
            operators::NOT_EQUALS => Box::new(FilterNode::NeInt { idx, val }),
            operators::LESS => Box::new(FilterNode::GtInt { idx, val }), // 80 < port => port > 80
            operators::LESS_EQUALS => Box::new(FilterNode::GeInt { idx, val }), // 80 <= port => port >= 80
            operators::GREATER => Box::new(FilterNode::LtInt { idx, val }), // 80 > port => port < 80
            operators::GREATER_EQUALS => Box::new(FilterNode::LeInt { idx, val }), // 80 >= port => port <= 80
            _ => return None,
        });
    }

    // Fast path: (var + lit) op lit  or  (var - lit) op lit  or  (var * lit) op lit
    fn try_arith_lit(
        ctx: &mut FilterCtx,
        expr: &Expr,
        op: &str,
        lit_expr: &Expr,
    ) -> Option<Box<FilterNode>> {
        let cmp_val = match lit_expr {
            Expr::Literal(LiteralValue::Int(i)) => *i.inner(),
            _ => return None,
        };
        let call = match expr {
            Expr::Call(c) => c,
            _ => return None,
        };
        if call.args.len() != 2 {
            return None;
        }
        let name = call.func_name.as_str();

        // Helper: extract var_idx from an expr if it's an Ident
        let mut var_idx = |e: &Expr| match e {
            Expr::Ident(n) => Some(ctx.var_idx(n)),
            _ => None,
        };
        // Helper: extract literal i64 from an expr
        let mut lit_val = |e: &Expr| match e {
            Expr::Literal(LiteralValue::Int(i)) => Some(*i.inner()),
            _ => None,
        };

        // Try var op lit and lit op var patterns for the arithmetic
        if let Some((idx, arith)) = var_idx(&call.args[0].expr).zip(lit_val(&call.args[1].expr)) {
            return Some(match (name, op) {
                (operators::ADD, operators::EQUALS) => Box::new(FilterNode::AddEq { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::NOT_EQUALS) => Box::new(FilterNode::AddNe { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::LESS) => Box::new(FilterNode::AddLt { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::LESS_EQUALS) => Box::new(FilterNode::AddLe { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::GREATER) => Box::new(FilterNode::AddGt { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::GREATER_EQUALS) => Box::new(FilterNode::AddGe { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::EQUALS) => Box::new(FilterNode::SubEq { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::NOT_EQUALS) => Box::new(FilterNode::SubNe { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::LESS) => Box::new(FilterNode::SubLt { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::LESS_EQUALS) => Box::new(FilterNode::SubLe { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::GREATER) => Box::new(FilterNode::SubGt { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::GREATER_EQUALS) => Box::new(FilterNode::SubGe { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::EQUALS) => Box::new(FilterNode::MulEq { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::NOT_EQUALS) => Box::new(FilterNode::MulNe { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::LESS) => Box::new(FilterNode::MulLt { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::LESS_EQUALS) => Box::new(FilterNode::MulLe { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::GREATER) => Box::new(FilterNode::MulGt { idx, arith, cmp: cmp_val }),
                (operators::MULTIPLY, operators::GREATER_EQUALS) => Box::new(FilterNode::MulGe { idx, arith, cmp: cmp_val }),
                _ => return None,
            });
        }
        // lit op var (e.g. 100 + port == 1024)
        if let Some((arith, idx)) = lit_val(&call.args[0].expr).zip(var_idx(&call.args[1].expr)) {
            return Some(match (name, op) {
                (operators::ADD, operators::EQUALS) => Box::new(FilterNode::AddEq { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::NOT_EQUALS) => Box::new(FilterNode::AddNe { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::LESS) => Box::new(FilterNode::AddGt { idx, arith, cmp: cmp_val }), // swapped: lit < var+arith => var+arith > lit
                (operators::ADD, operators::LESS_EQUALS) => Box::new(FilterNode::AddGe { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::GREATER) => Box::new(FilterNode::AddLt { idx, arith, cmp: cmp_val }),
                (operators::ADD, operators::GREATER_EQUALS) => Box::new(FilterNode::AddLe { idx, arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::EQUALS) => Box::new(FilterNode::SubEq { idx, arith: -arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::NOT_EQUALS) => Box::new(FilterNode::SubNe { idx, arith: -arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::LESS) => Box::new(FilterNode::SubGt { idx, arith: -arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::LESS_EQUALS) => Box::new(FilterNode::SubGe { idx, arith: -arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::GREATER) => Box::new(FilterNode::SubLt { idx, arith: -arith, cmp: cmp_val }),
                (operators::SUBSTRACT, operators::GREATER_EQUALS) => Box::new(FilterNode::SubLe { idx, arith: -arith, cmp: cmp_val }),
                _ => return None,
            });
        }
        None
    }

    if let Some(f) = try_arith_lit(ctx, left, op, right) {
        return Some(f);
    }
    if let Some(f) = try_arith_lit(ctx, right, op, left) {
        return Some(f);
    }

    // General path: FilterNode::*Expr on both sides (supports arithmetic)
    let left_expr = try_compile_i64_expr(ctx, left)?;
    let right_expr = try_compile_i64_expr(ctx, right)?;
    let f: Box<FilterNode> = match op {
        operators::EQUALS => Box::new(FilterNode::EqExpr { left: left_expr, right: right_expr }),
        operators::NOT_EQUALS => Box::new(FilterNode::NeExpr { left: left_expr, right: right_expr }),
        operators::LESS => Box::new(FilterNode::LtExpr { left: left_expr, right: right_expr }),
        operators::LESS_EQUALS => Box::new(FilterNode::LeExpr { left: left_expr, right: right_expr }),
        operators::GREATER => Box::new(FilterNode::GtExpr { left: left_expr, right: right_expr }),
        operators::GREATER_EQUALS => Box::new(FilterNode::GeExpr { left: left_expr, right: right_expr }),
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
) -> Option<Box<FilterNode>> {
    let (var_name, val) = match (left, right) {
        (Expr::Ident(name), Expr::Literal(LiteralValue::String(s))) => {
            (name, s.inner().to_string())
        }
        (Expr::Literal(LiteralValue::String(s)), Expr::Ident(name)) => {
            (name, s.inner().to_string())
        }
        _ => return None,
    };
    let idx = ctx.var_idx(var_name);
    match op {
        operators::EQUALS => Some(Box::new(FilterNode::EqStr { idx, val })),
        operators::NOT_EQUALS => Some(Box::new(FilterNode::NeStr { idx, val })),
        _ => None,
    }
}

fn try_compile_str_bool(
    ctx: &mut FilterCtx,
    func: &str,
    receiver: &Expr,
    arg: &Expr,
) -> Option<Box<FilterNode>> {
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
        "startsWith" => Some(Box::new(FilterNode::StartsWith { idx, prefix: val })),
        "endsWith" => Some(Box::new(FilterNode::EndsWith { idx, suffix: val })),
        "contains" => Some(Box::new(FilterNode::Contains { idx, substring: val })),
        "matches" => {
            match regex::Regex::new(&val) {
                Ok(re) => Some(Box::new(FilterNode::Matches { idx, regex: re })),
                Err(_) => None, // invalid regex → fall back to AST
            }
        }
        _ => None,
    }
}

// Also handle target-based method calls (e.g. path.contains("/api"))
fn try_compile_target_str_bool(
    ctx: &mut FilterCtx,
    call: &crate::common::ast::CallExpr,
) -> Option<Box<FilterNode>> {
    let func = call.func_name.as_str();
    if !matches!(func, "startsWith" | "endsWith" | "contains" | "matches") {
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
        "startsWith" => Some(Box::new(FilterNode::StartsWith { idx, prefix: val })),
        "endsWith" => Some(Box::new(FilterNode::EndsWith { idx, suffix: val })),
        "contains" => Some(Box::new(FilterNode::Contains { idx, substring: val })),
        "matches" => {
            match regex::Regex::new(&val) {
                Ok(re) => Some(Box::new(FilterNode::Matches { idx, regex: re })),
                Err(_) => None,
            }
        }
        _ => None,
    }
}

fn try_compile_in_set(
    ctx: &mut FilterCtx,
    left: &Expr,
    right: &Expr,
) -> Option<Box<FilterNode>> {
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
            return Some(Box::new(FilterNode::InIntLinear { idx, vals: ints }));
        }
        let set: std::collections::HashSet<i64> = ints.into_iter().collect();
        return Some(Box::new(FilterNode::InIntHash { idx, set }));
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
            return Some(Box::new(FilterNode::InStrLinear { idx, vals: strs }));
        }
        let set: std::collections::HashSet<String> = strs.into_iter().collect();
        return Some(Box::new(FilterNode::InStrHash { idx, set }));
    }

    None
}
