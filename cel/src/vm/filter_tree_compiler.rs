use crate::common::ast::operators;
use crate::common::ast::{Expr, LiteralValue};
use crate::common::types::CelBool;
use crate::objects::Value;
use crate::vm::filter_tree::{FilterNode, I64Expr, ListExpr, StrExpr};
use crate::{ExecutionError, Expression};
use std::collections::HashMap;
use std::sync::Arc;

use std::fmt;

// ============================================================================
// CompiledExpr — a type-directed compiled expression.
// Every variant holds a closure over typed arrays (&[i64], &[Arc<str>]).
// The compiler picks the most specific variant based on type info.
// ============================================================================

/// A compiled CEL expression — captured as a closure over typed data arrays.
///
/// The compiler selects the most specific variant (Bool > I64 > Str > Value)
/// based on literal types, schema field types, and operator signatures.
///
/// # Zero-overhead bool
///
/// `Bool` closures are the same ones that powered `fast_eval` before.
/// `eval_bool()` extracts them directly — no match overhead.
pub enum CompiledExpr {
    /// Boolean expression: `port == 80`, `a && b`, `startsWith(path, "/api")`
    Bool(Box<dyn Fn(&[i64], &[Arc<str>]) -> bool + Send + Sync>),
    /// Integer expression: `port + 1`, `size(path)`, `count - 5`
    I64(Box<dyn Fn(&[i64], &[Arc<str>]) -> i64 + Send + Sync>),
    /// Floating-point expression: `lat + 0.5`, `rate * 1.1`
    F64(Box<dyn Fn(&[i64], &[Arc<str>]) -> f64 + Send + Sync>),
    /// String expression: `name + " (" + region + ")"`
    Str(Box<dyn Fn(&[i64], &[Arc<str>]) -> Arc<str> + Send + Sync>),
    /// Escape hatch for any CEL value — still uses typed arrays.
    /// Needed for list/map/bytes/duration/struct types that don't
    /// have dedicated typed array columns.
    Value(Box<dyn Fn(&[i64], &[Arc<str>]) -> Result<Value, ExecutionError> + Send + Sync>),
}

impl CompiledExpr {
    /// Evaluate as boolean — panics if this is not a Bool variant.
    /// This is the zero-overhead path for `eval_bool()`.
    #[inline(always)]
    pub fn eval_bool(&self, ints: &[i64], strings: &[Arc<str>]) -> bool {
        match self {
            CompiledExpr::Bool(f) => f(ints, strings),
            _ => panic!("CompiledExpr::eval_bool called on non-Bool variant"),
        }
    }

    /// Evaluate and return a Value no matter the inner type.
    /// Bool → Value::Bool, I64 → Value::Int, etc.
    /// This is the zero-overhead path for `execute()`.
    #[inline(always)]
    pub fn eval_value(&self, ints: &[i64], strings: &[Arc<str>]) -> Result<Value, ExecutionError> {
        match self {
            CompiledExpr::Bool(f) => Ok(Value::Bool(f(ints, strings))),
            CompiledExpr::I64(f) => Ok(Value::Int(f(ints, strings))),
            CompiledExpr::F64(f) => Ok(Value::Float(f(ints, strings))),
            CompiledExpr::Str(f) => Ok(Value::String(f(ints, strings))),
            CompiledExpr::Value(f) => f(ints, strings),
        }
    }

    /// Downcast to Bool — returns None if not Bool.
    pub fn as_bool(&self) -> Option<&Box<dyn Fn(&[i64], &[Arc<str>]) -> bool + Send + Sync>> {
        match self {
            CompiledExpr::Bool(f) => Some(f),
            _ => None,
        }
    }

    /// Downcast to I64 — returns None if not I64.
    pub fn as_i64(&self) -> Option<&Box<dyn Fn(&[i64], &[Arc<str>]) -> i64 + Send + Sync>> {
        match self {
            CompiledExpr::I64(f) => Some(f),
            _ => None,
        }
    }
}

impl fmt::Debug for CompiledExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompiledExpr::Bool(_) => f.write_str("CompiledExpr::Bool"),
            CompiledExpr::I64(_) => f.write_str("CompiledExpr::I64"),
            CompiledExpr::F64(_) => f.write_str("CompiledExpr::F64"),
            CompiledExpr::Str(_) => f.write_str("CompiledExpr::Str"),
            CompiledExpr::Value(_) => f.write_str("CompiledExpr::Value"),
        }
    }
}

pub struct CompiledFilterTree {
    pub filter: Box<FilterNode>,
    pub compiled: CompiledExpr,
    pub var_names: Vec<String>,
}

impl fmt::Debug for CompiledFilterTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledFilterTree")
            .field("filter", &self.filter)
            .field("var_names", &self.var_names)
            .finish()
    }
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

    /// Extract typed arrays from a Context for use with fast_eval.
    /// Returns (ints, strings) in variable index order.
    pub fn bind_typed(&self, ctx: &crate::Context) -> (Vec<i64>, Vec<std::sync::Arc<str>>) {
        use crate::common::types::{CelInt, CelString};
        use crate::common::value::Val;
        let mut ints = Vec::with_capacity(self.var_names.len());
        let mut strings = Vec::with_capacity(self.var_names.len());
        for name in &self.var_names {
            match ctx.get_variable(name) {
                Some(cow) => {
                    let v: &dyn Val = cow.as_ref();
                    match v.get_type().kind() {
                        crate::common::types::Kind::Int => {
                            let i = *v.downcast_ref::<CelInt>().unwrap().inner();
                            ints.push(i);
                            strings.push(std::sync::Arc::from(""));
                        }
                        crate::common::types::Kind::String => {
                            let s = v.downcast_ref::<CelString>().unwrap().inner();
                            ints.push(0);
                            strings.push(std::sync::Arc::from(s));
                        }
                        crate::common::types::Kind::Boolean => {
                            let b = *v.downcast_ref::<CelBool>().unwrap().inner();
                            ints.push(if b { 1 } else { 0 });
                            strings.push(std::sync::Arc::from(""));
                        }
                        _ => {
                            ints.push(0);
                            strings.push(std::sync::Arc::from(""));
                        }
                    }
                }
                None => {
                    ints.push(0);
                    strings.push(std::sync::Arc::from(""));
                }
            }
        }
        (ints, strings)
    }
}

/// Generate a closure for an I64Expr (integer sub-expression).
/// Enables closure-based eval for `size(path) > 5` and similar patterns.
fn compile_closure_i64(expr: &I64Expr) -> Box<dyn Fn(&[i64], &[Arc<str>]) -> i64 + Send + Sync> {
    match expr {
        I64Expr::Literal(v) => {
            let v = *v;
            Box::new(move |_, _| v)
        }
        I64Expr::Var(idx) => {
            let i = *idx;
            Box::new(move |ints, _| ints[i])
        }
        I64Expr::Add(a, b) => {
            let a_fn = compile_closure_i64(a);
            let b_fn = compile_closure_i64(b);
            Box::new(move |ints, s| a_fn(ints, s).wrapping_add(b_fn(ints, s)))
        }
        I64Expr::Sub(a, b) => {
            let a_fn = compile_closure_i64(a);
            let b_fn = compile_closure_i64(b);
            Box::new(move |ints, s| a_fn(ints, s).wrapping_sub(b_fn(ints, s)))
        }
        I64Expr::Mul(a, b) => {
            let a_fn = compile_closure_i64(a);
            let b_fn = compile_closure_i64(b);
            Box::new(move |ints, s| a_fn(ints, s).wrapping_mul(b_fn(ints, s)))
        }
        I64Expr::Div(a, b) => {
            let a_fn = compile_closure_i64(a);
            let b_fn = compile_closure_i64(b);
            Box::new(move |ints, s| {
                let bv = b_fn(ints, s);
                if bv == 0 { 0 } else { a_fn(ints, s).wrapping_div(bv) }
            })
        }
        I64Expr::Mod(a, b) => {
            let a_fn = compile_closure_i64(a);
            let b_fn = compile_closure_i64(b);
            Box::new(move |ints, s| {
                let bv = b_fn(ints, s);
                if bv == 0 { 0 } else { a_fn(ints, s).wrapping_rem(bv) }
            })
        }
        I64Expr::Neg(a) => {
            let a_fn = compile_closure_i64(a);
            Box::new(move |ints, s| a_fn(ints, s).wrapping_neg())
        }
        I64Expr::StrLen(s) => compile_str_len(s),
        I64Expr::ListLen(_) => {
            // List length not available from typed arrays.
            // Falls back to AST for correctness via CompiledFilterTree::filter.
            Box::new(|_, _| 0i64)
        }
    }
}

/// Compile a StrExpr to a length-returning closure.
fn compile_str_len(expr: &StrExpr) -> Box<dyn Fn(&[i64], &[Arc<str>]) -> i64 + Send + Sync> {
    match expr {
        StrExpr::Literal(st) => {
            let len = st.len() as i64;
            Box::new(move |_, _| len)
        }
        StrExpr::Var(idx) => {
            let i = *idx;
            Box::new(move |_, strings| strings[i].len() as i64)
        }
        StrExpr::Concat(a, b) => {
            let a_fn = compile_str_len(a);
            let b_fn = compile_str_len(b);
            Box::new(move |ints, strings| a_fn(ints, strings) + b_fn(ints, strings))
        }
    }
}

/// Compile a FilterNode into a CompiledExpr.
///
/// All boolean FilterNode variants produce `CompiledExpr::Bool`.
/// This is the unified entry point — legacy callers use `eval_bool()`.
pub fn compile_closure(node: &FilterNode) -> CompiledExpr {
    compile_closure_bool(node)
}

/// Core boolean closure compiler — same as the old compile_closure.
fn compile_closure_bool(node: &FilterNode) -> CompiledExpr {
    CompiledExpr::Bool(match node {
        // ── Int comparisons ──
        FilterNode::EqInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] == v)
        }
        FilterNode::NeInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] != v)
        }
        FilterNode::LtInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] < v)
        }
        FilterNode::LeInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] <= v)
        }
        FilterNode::GtInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] > v)
        }
        FilterNode::GeInt { idx, val } => {
            let (i, v) = (*idx, *val);
            Box::new(move |ints, _| ints[i] >= v)
        }

        // ── Fused arithmetic + comparison ──
        FilterNode::AddEq { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) == c)
        }
        FilterNode::AddNe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) != c)
        }
        FilterNode::AddLt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) < c)
        }
        FilterNode::AddLe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) <= c)
        }
        FilterNode::AddGt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) > c)
        }
        FilterNode::AddGe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_add(a) >= c)
        }
        FilterNode::SubEq { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) == c)
        }
        FilterNode::SubNe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) != c)
        }
        FilterNode::SubLt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) < c)
        }
        FilterNode::SubLe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) <= c)
        }
        FilterNode::SubGt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) > c)
        }
        FilterNode::SubGe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_sub(a) >= c)
        }
        FilterNode::MulEq { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) == c)
        }
        FilterNode::MulNe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) != c)
        }
        FilterNode::MulLt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) < c)
        }
        FilterNode::MulLe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) <= c)
        }
        FilterNode::MulGt { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) > c)
        }
        FilterNode::MulGe { idx, arith, cmp } => {
            let (i, a, c) = (*idx, *arith, *cmp);
            Box::new(move |ints, _| ints[i].wrapping_mul(a) >= c)
        }

        // ── String comparison ──
        FilterNode::EqStr { idx, val } => {
            let i = *idx;
            let v: Arc<str> = Arc::from(val.as_str());
            Box::new(move |_, strings| strings[i].as_ref() == v.as_ref())
        }
        FilterNode::NeStr { idx, val } => {
            let i = *idx;
            let v: Arc<str> = Arc::from(val.as_str());
            Box::new(move |_, strings| strings[i].as_ref() != v.as_ref())
        }

        // ── Set membership: int ──
        FilterNode::InIntLinear { idx, vals } => {
            let i = *idx;
            let v = vals.clone();
            Box::new(move |ints, _| v.contains(&ints[i]))
        }
        FilterNode::InIntHash { idx, set } => {
            let i = *idx;
            let set = set.clone();
            Box::new(move |ints, _| set.contains(&ints[i]))
        }

        // ── Set membership: str ──
        FilterNode::InStrLinear { idx, vals } => {
            let i = *idx;
            let v = vals.clone();
            Box::new(move |_, strings| {
                let s: &str = strings[i].as_ref();
                v.iter().any(|x| x == s)
            })
        }
        FilterNode::InStrHash { idx, set } => {
            let i = *idx;
            let set = set.clone();
            Box::new(move |_, strings| {
                let s: &str = strings[i].as_ref();
                set.contains(s)
            })
        }

        // ── String methods ──
        FilterNode::StartsWith { idx, prefix } => {
            let i = *idx;
            let p: Arc<str> = Arc::from(prefix.as_str());
            Box::new(move |_, strings| strings[i].starts_with(p.as_ref()))
        }
        FilterNode::EndsWith { idx, suffix } => {
            let i = *idx;
            let s: Arc<str> = Arc::from(suffix.as_str());
            Box::new(move |_, strings| strings[i].ends_with(s.as_ref()))
        }
        FilterNode::Contains { idx, substring } => {
            let i = *idx;
            let sub: Arc<str> = Arc::from(substring.as_str());
            Box::new(move |_, strings| strings[i].contains(sub.as_ref()))
        }

        // ── Regex matches (pre-compiled regex captured in closure) ──
        FilterNode::Matches { idx, regex } => {
            let i = *idx;
            let re = regex.clone();
            Box::new(move |_, strings| re.is_match(strings[i].as_ref()))
        }

        // ── Multi-pattern contains ──
        FilterNode::ContainsAny { idx, needles } => {
            let i = *idx;
            let n = needles.clone();
            Box::new(move |_, strings| {
                let text: &str = strings[i].as_ref();
                for needle in &n {
                    if text.contains(needle.as_str()) {
                        return true;
                    }
                }
                false
            })
        }
        FilterNode::AhoContains { idx, ac, min } => {
            let i = *idx;
            let ac = ac.clone();
            let m = *min;
            Box::new(move |_, strings| {
                let text = strings[i].as_bytes();
                if m <= 1 { return ac.is_match(text); }
                let mut matched = 0u64;
                for mat in ac.find_iter(text) {
                    let pid = mat.pattern().as_u64();
                    if pid < 64 {
                        matched |= 1u64 << pid;
                        if matched.count_ones() as usize >= m { return true; }
                    }
                }
                false
            })
        }

        // ── Logic combinators ──
        FilterNode::And(a, b) => {
            let CompiledExpr::Bool(a_fn) = compile_closure(a) else {
                unreachable!("FilterNode children always compile to Bool")
            };
            let CompiledExpr::Bool(b_fn) = compile_closure(b) else {
                unreachable!("FilterNode children always compile to Bool")
            };
            Box::new(move |ints, strings| a_fn(ints, strings) && b_fn(ints, strings))
        }
        FilterNode::Or(a, b) => {
            let CompiledExpr::Bool(a_fn) = compile_closure(a) else {
                unreachable!("FilterNode children always compile to Bool")
            };
            let CompiledExpr::Bool(b_fn) = compile_closure(b) else {
                unreachable!("FilterNode children always compile to Bool")
            };
            Box::new(move |ints, strings| a_fn(ints, strings) || b_fn(ints, strings))
        }
        FilterNode::Not(inner) => {
            let CompiledExpr::Bool(inner_fn) = compile_closure(inner) else {
                unreachable!("FilterNode children always compile to Bool")
            };
            Box::new(move |ints, strings| !inner_fn(ints, strings))
        }

        // ── I64Expr comparisons (compiled to closures via compile_closure_i64) ──
        FilterNode::GeExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) >= b(ints, strings))
        }
        FilterNode::GtExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) > b(ints, strings))
        }
        FilterNode::LeExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) <= b(ints, strings))
        }
        FilterNode::LtExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) < b(ints, strings))
        }
        FilterNode::EqExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) == b(ints, strings))
        }
        FilterNode::NeExpr { left, right } => {
            let a = compile_closure_i64(left);
            let b = compile_closure_i64(right);
            Box::new(move |ints, strings| a(ints, strings) != b(ints, strings))
        }

        // ── Boolean variable / literal ──
        FilterNode::BoolLiteral { val } => {
            let v = *val;
            Box::new(move |_, _| v)
        }
        FilterNode::BoolVar { idx } => {
            let i = *idx;
            Box::new(move |ints, _| ints[i] != 0)
        }
    })
}

/// Compile an expression to a filter tree, using a Schema's field ordering
/// for variable index assignment instead of auto-assignment.
///
/// `field_names` provides the name → index mapping: the position in the Vec
/// is the index used in tree nodes and expected by EvalContext.
pub fn compile_filter_tree_with_schema(
    expr: &Expression,
    field_names: &[&str],
) -> Result<CompiledFilterTree, String> {
    let mut ctx = FilterCtx::with_schema(field_names);
    let mut filter = compile_expr(&mut ctx, &expr.expr)?;
    filter.optimize_order();
    let compiled = compile_closure(&filter);
    Ok(CompiledFilterTree { filter, compiled, var_names: ctx.var_names })
}

pub fn compile_filter_tree(expr: &Expression) -> Result<CompiledFilterTree, String> {
    let mut ctx = FilterCtx::new();
    let mut filter = compile_expr(&mut ctx, &expr.expr)?;
    filter.optimize_order();
    let compiled = compile_closure(&filter);
    Ok(CompiledFilterTree { filter, compiled, var_names: ctx.var_names })
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

    /// Create a context pre-populated with schema field names.
    /// The field_names Vec provides the name → index mapping.
    fn with_schema(field_names: &[&str]) -> Self {
        let mut var_map = HashMap::with_capacity(field_names.len());
        let mut var_names = Vec::with_capacity(field_names.len());
        for (idx, name) in field_names.iter().enumerate() {
            var_map.insert((*name).to_string(), idx);
            var_names.push((*name).to_string());
        }
        Self { var_map, var_names }
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

fn flatten_ident(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(name) => Some(name.clone()),
        Expr::Select(sel) => {
            let prefix = flatten_ident(&sel.operand.expr)?;
            Some(format!("{}.{}", prefix, sel.field))
        }
        _ => None,
    }
}

fn resolve_var(ctx: &mut FilterCtx, expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Ident(name) => Some(ctx.var_idx(name)),
        Expr::Select(sel) => {
            let dotted = flatten_ident(expr)?;
            if ctx.var_map.contains_key(&dotted) {
                Some(ctx.var_idx(&dotted))
            } else if let Some(bare) = dotted.rsplit('.').next() {
                if ctx.var_map.contains_key(bare) {
                    Some(ctx.var_idx(bare))
                } else {
                    None
                }
            } else {
                None
            }
        }
        Expr::Call(call) if call.func_name.as_str() == operators::INDEX && call.args.len() == 1 => {
            let target = call.target.as_ref()?;
            let key = match &call.args[0].expr {
                Expr::Literal(LiteralValue::String(s)) => s.inner(),
                _ => return None,
            };
            let prefix = flatten_ident(&target.expr)?;
            let dotted = format!("{}.{}", prefix, key);
            if ctx.var_map.contains_key(&dotted) {
                Some(ctx.var_idx(&dotted))
            } else if let Some(bare) = dotted.rsplit('.').next() {
                if ctx.var_map.contains_key(bare) {
                    Some(ctx.var_idx(bare))
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

fn compile_expr(ctx: &mut FilterCtx, expr: &Expr) -> Result<Box<FilterNode>, String> {
    match expr {
        // ── Boolean literal: `true`, `false` ──
        Expr::Literal(LiteralValue::Boolean(b)) => {
            Ok(Box::new(FilterNode::BoolLiteral { val: *b.inner() }))
        }
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
        _ => Err("unsupported expr kind in filter tree".into()),
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
        let idx = resolve_var(ctx, var_expr)?;
        let val = match lit_expr {
            Expr::Literal(LiteralValue::Int(i)) => *i.inner(),
            _ => return None,
        };
        Some((idx, val))
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
    // Reject if both sides are bare identifiers — we can't guarantee both are ints
    // (e.g. opaque values, strings, etc. fall through to AST for correct comparison)
    if !matches!(left, Expr::Literal(_)) && !matches!(right, Expr::Literal(_))
        && matches!(left, Expr::Ident(_)) && matches!(right, Expr::Ident(_))
    {
        return None;
    }
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
        Expr::Literal(LiteralValue::String(_)) | Expr::Literal(LiteralValue::Bytes(_)) => None,
        _ => None,
    }
}

fn try_compile_str_cmp(
    ctx: &mut FilterCtx,
    op: &str,
    left: &Expr,
    right: &Expr,
) -> Option<Box<FilterNode>> {
    if op != operators::EQUALS && op != operators::NOT_EQUALS {
        return None;
    }
    let (idx, val) = if let Some(idx) = resolve_var(ctx, left) {
        match right {
            Expr::Literal(LiteralValue::String(s)) => (idx, s.inner().to_string()),
            _ => return None,
        }
    } else if let Some(idx) = resolve_var(ctx, right) {
        match left {
            Expr::Literal(LiteralValue::String(s)) => (idx, s.inner().to_string()),
            _ => return None,
        }
    } else {
        return None;
    };
    match op {
        operators::EQUALS => Some(Box::new(FilterNode::EqStr { idx, val })),
        _ => Some(Box::new(FilterNode::NeStr { idx, val })),
    }
}

fn try_compile_str_bool(
    ctx: &mut FilterCtx,
    func: &str,
    receiver: &Expr,
    arg: &Expr,
) -> Option<Box<FilterNode>> {
    let idx = resolve_var(ctx, receiver)?;
    let val = match arg {
        Expr::Literal(LiteralValue::String(s)) => s.inner().to_string(),
        _ => return None,
    };
    match func {
        "startsWith" => Some(Box::new(FilterNode::StartsWith { idx, prefix: val })),
        "endsWith" => Some(Box::new(FilterNode::EndsWith { idx, suffix: val })),
        "contains" => Some(Box::new(FilterNode::Contains { idx, substring: val })),
        "matches" => {
            let regex = regex::Regex::new(&val).ok()?;
            Some(Box::new(FilterNode::Matches { idx, regex }))
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
    let idx = resolve_var(ctx, &target_expr.expr)?;
    let arg = call.args.first()?;
    let val = match &arg.expr {
        Expr::Literal(LiteralValue::String(s)) => s.inner().to_string(),
        _ => return None,
    };
    match func {
        "startsWith" => Some(Box::new(FilterNode::StartsWith { idx, prefix: val })),
        "endsWith" => Some(Box::new(FilterNode::EndsWith { idx, suffix: val })),
        "contains" => Some(Box::new(FilterNode::Contains { idx, substring: val })),
        "matches" => {
            let regex = regex::Regex::new(&val).ok()?;
            Some(Box::new(FilterNode::Matches { idx, regex }))
        }
        _ => None,
    }
}

fn try_compile_in_set(ctx: &mut FilterCtx, left: &Expr, right: &Expr) -> Option<Box<FilterNode>> {
    let idx = resolve_var(ctx, left)?;
    let list = match right {
        Expr::List(list) => &list.elements,
        _ => return None,
    };

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
