use crate::objects::{Key, Value};
use std::collections::HashSet;
use std::sync::Arc;

// ── Compiled filter node (single eval path) ──

/// A compiled filter expression backed by a closure.
/// Clone is cheap (Arc). Eval calls the inner closure.
#[derive(Clone)]
pub struct CompiledFilterNode(Arc<dyn Fn(&[Value]) -> Value>);

impl CompiledFilterNode {
    pub fn new(f: Arc<dyn Fn(&[Value]) -> Value>) -> Self {
        Self(f)
    }

    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> Value {
        (self.0)(vars)
    }
}

impl std::fmt::Debug for CompiledFilterNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledFilterNode").finish()
    }
}

// ── Compiled filter with named variables ──

/// A compiled filter expression with known variable names.
pub struct CompiledFilter {
    pub compiled: CompiledFilterNode,
    pub var_names: Vec<String>,
}

impl CompiledFilter {
    pub fn eval(&self, vars: &[Value]) -> Value {
        self.compiled.eval(vars)
    }

    pub fn bind_vars(&self, ctx: &crate::Context) -> Vec<Value> {
        self.var_names
            .iter()
            .map(|name| {
                ctx.get_variable(name)
                    .and_then(|cow| Value::try_from(cow.as_ref()).ok())
                    .unwrap_or(Value::Null)
            })
            .collect()
    }
}
// ── Item predicate closure for ExistsClosure ──

/// A boxed predicate on a single `&Value` reference.
/// Used by `ExistsClosure` to evaluate predicates on each list element
/// without allocating a scratch vector or cloning items.
///
/// Clone is via `Arc`, Debug is a stub — the closure itself is opaque.
#[derive(Clone)]
pub struct ItemPredicate(Arc<dyn Fn(&Value) -> bool>);

impl std::fmt::Debug for ItemPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ItemPredicate").finish()
    }
}

impl ItemPredicate {
    pub fn new(f: Box<dyn Fn(&Value) -> bool>) -> Self {
        Self(Arc::from(f))
    }

    #[inline(always)]
    pub fn call(&self, v: &Value) -> bool {
        (self.0)(v)
    }
}
/// A typed string expression that evaluates to `&str` (via reference to var storage).
#[derive(Clone, Debug)]
pub enum StrExpr {
    Literal(String),
    Var(usize),
    Concat(Box<StrExpr>, Box<StrExpr>),
}

impl StrExpr {
    /// Compile length calculation into a closure.
    pub fn compile_len(&self) -> Arc<dyn Fn(&[Value]) -> usize> {
        match self {
            Self::Literal(s) => {
                let len = s.len();
                Arc::new(move |_| len)
            }
            Self::Var(idx) => {
                let i = *idx;
                Arc::new(move |vars| match &vars[i] {
                    Value::String(s) => s.len(),
                    _ => 0,
                })
            }
            Self::Concat(a, b) => {
                let a_fn = a.compile_len();
                let b_fn = b.compile_len();
                Arc::new(move |vars| a_fn(vars) + b_fn(vars))
            }
        }
    }
}
/// A typed list expression.
#[derive(Clone, Debug)]
pub enum ListExpr {
    Var(usize),
}

impl ListExpr {
    /// Compile length calculation into a closure.
    pub fn compile_len(&self) -> Arc<dyn Fn(&[Value]) -> usize> {
        match self {
            Self::Var(idx) => {
                let i = *idx;
                Arc::new(move |vars| match &vars[i] {
                    Value::List(list) => list.len(),
                    _ => 0,
                })
            }
        }
    }
}
/// A typed integer expression that evaluates directly to `i64`.
/// Used as a sub-expression inside boolean filters (e.g. `port + 100 >= 1024`).
#[derive(Clone, Debug)]
pub enum I64Expr {
    Literal(i64),
    Var(usize),
    Add(Box<I64Expr>, Box<I64Expr>),
    Sub(Box<I64Expr>, Box<I64Expr>),
    Mul(Box<I64Expr>, Box<I64Expr>),
    Div(Box<I64Expr>, Box<I64Expr>),
    Mod(Box<I64Expr>, Box<I64Expr>),
    Neg(Box<I64Expr>),
    /// String length: size(string)
    StrLen(Box<StrExpr>),
    /// List length: size(list)
    ListLen(Box<ListExpr>),
}

impl I64Expr {
    /// Compile this expression into a callable closure returning i64.
    pub fn compile(&self) -> Arc<dyn Fn(&[Value]) -> i64> {
        match self {
            Self::Literal(v) => {
                let v = *v;
                Arc::new(move |_| v)
            }
            Self::Var(idx) => {
                let i = *idx;
                Arc::new(move |vars| match &vars[i] {
                    Value::Int(val) => *val,
                    _ => 0,
                })
            }
            Self::Add(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Arc::new(move |vars| a_fn(vars).wrapping_add(b_fn(vars)))
            }
            Self::Sub(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Arc::new(move |vars| a_fn(vars).wrapping_sub(b_fn(vars)))
            }
            Self::Mul(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Arc::new(move |vars| a_fn(vars).wrapping_mul(b_fn(vars)))
            }
            Self::Div(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Arc::new(move |vars| {
                    let bv = b_fn(vars);
                    if bv == 0 { 0 } else { a_fn(vars).wrapping_div(bv) }
                })
            }
            Self::Mod(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Arc::new(move |vars| {
                    let bv = b_fn(vars);
                    if bv == 0 { 0 } else { a_fn(vars).wrapping_rem(bv) }
                })
            }
            Self::Neg(a) => {
                let a_fn = a.compile();
                Arc::new(move |vars| a_fn(vars).wrapping_neg())
            }
            Self::StrLen(s) => {
                let s_fn = s.compile_len();
                Arc::new(move |vars| s_fn(vars) as i64)
            }
            Self::ListLen(l) => {
                let l_fn = l.compile_len();
                Arc::new(move |vars| l_fn(vars) as i64)
            }
        }
    }
}
// =====================================================================
// FilterNode — concrete enum replaced by CompiledFilterNode at runtime
// =====================================================================

/// A boolean expression tree, used as a compile-time IR.
/// Compiled to a closure via `compile()` before evaluation.
#[derive(Clone, Debug)]
pub enum FilterNode {
    // --- Int comparisons (var op literal) ---
    EqInt { idx: usize, val: i64 },
    NeInt { idx: usize, val: i64 },
    LtInt { idx: usize, val: i64 },
    LeInt { idx: usize, val: i64 },
    GtInt { idx: usize, val: i64 },
    GeInt { idx: usize, val: i64 },

    // --- Fused arithmetic + comparison ---
    AddEq { idx: usize, arith: i64, cmp: i64 },
    AddNe { idx: usize, arith: i64, cmp: i64 },
    AddLt { idx: usize, arith: i64, cmp: i64 },
    AddLe { idx: usize, arith: i64, cmp: i64 },
    AddGt { idx: usize, arith: i64, cmp: i64 },
    AddGe { idx: usize, arith: i64, cmp: i64 },
    SubEq { idx: usize, arith: i64, cmp: i64 },
    SubNe { idx: usize, arith: i64, cmp: i64 },
    SubLt { idx: usize, arith: i64, cmp: i64 },
    SubLe { idx: usize, arith: i64, cmp: i64 },
    SubGt { idx: usize, arith: i64, cmp: i64 },
    SubGe { idx: usize, arith: i64, cmp: i64 },
    MulEq { idx: usize, arith: i64, cmp: i64 },
    MulNe { idx: usize, arith: i64, cmp: i64 },
    MulLt { idx: usize, arith: i64, cmp: i64 },
    MulLe { idx: usize, arith: i64, cmp: i64 },
    MulGt { idx: usize, arith: i64, cmp: i64 },
    MulGe { idx: usize, arith: i64, cmp: i64 },

    // --- String comparison ---
    EqStr { idx: usize, val: String },
    NeStr { idx: usize, val: String },

    // --- Boolean field predicates ---
    BoolVar { idx: usize },

    // --- Set membership: int ---
    InIntLinear { idx: usize, vals: Vec<i64> },
    InIntHash { idx: usize, set: HashSet<i64> },

    // --- Set membership: str ---
    InStrLinear { idx: usize, vals: Vec<String> },
    InStrHash { idx: usize, set: HashSet<String> },

    // --- String methods ---
    StartsWith { idx: usize, prefix: String },
    EndsWith { idx: usize, suffix: String },
    Contains { idx: usize, substring: String },
    Matches { idx: usize, regex: regex::Regex },

    // --- Multi-pattern contains ---
    ContainsAny { idx: usize, needles: Vec<String> },
    AhoContains { idx: usize, ac: aho_corasick::AhoCorasick, min: usize },

    // --- I64Expr comparisons ---
    GeExpr { left: I64Expr, right: I64Expr },
    GtExpr { left: I64Expr, right: I64Expr },
    LeExpr { left: I64Expr, right: I64Expr },
    LtExpr { left: I64Expr, right: I64Expr },
    EqExpr { left: I64Expr, right: I64Expr },
    NeExpr { left: I64Expr, right: I64Expr },

    // --- Logic combinators ---
    And(Box<FilterNode>, Box<FilterNode>),
    Or(Box<FilterNode>, Box<FilterNode>),
    Not(Box<FilterNode>),

    // --- Comprehension: exists() ---
    Exists { list_idx: usize, item_idx: usize, predicate: Box<FilterNode> },

    // --- Exists with closure predicate (no alloc) ---
    ExistsClosure { list_idx: usize, predicate: ItemPredicate },

    // --- Map-key contains ---
    MapKeyContains { map_idx: usize, key: String, needle: String },

    // --- Specialized list membership ---
    ExistsInIntSet { list_idx: usize, vals: Vec<i64> },
    ExistsEqInt { list_idx: usize, val: i64 },
}
impl FilterNode {
    /// Compile this node into a callable closure.
    /// Replaces eval(), eval_fast(), eval_fast_typed(), and compile_closure().
    /// The returned `CompiledFilterNode` is a single closure — call `.eval(vars)` to run it.
    pub fn compile(&self) -> CompiledFilterNode {
        match self {
            Self::EqInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i == val),
                    _ => Value::Bool(false),
                }))
            }
            Self::NeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i != val),
                    _ => Value::Bool(false),
                }))
            }
            Self::LtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i < val),
                    _ => Value::Bool(false),
                }))
            }
            Self::LeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i <= val),
                    _ => Value::Bool(false),
                }))
            }
            Self::GtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i > val),
                    _ => Value::Bool(false),
                }))
            }
            Self::GeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(*i >= val),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) == cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) != cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) < cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) <= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) > cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::AddGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_add(arith) >= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) == cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) != cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) < cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) <= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) > cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::SubGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_sub(arith) >= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) == cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) != cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) < cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) <= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) > cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::MulGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(i.wrapping_mul(arith) >= cmp),
                    _ => Value::Bool(false),
                }))
            }
            Self::EqStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => Value::Bool(s.as_ref() == val.as_ref()),
                    _ => Value::Bool(false),
                }))
            }
            Self::NeStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => Value::Bool(s.as_ref() != val.as_ref()),
                    _ => Value::Bool(false),
                }))
            }
            Self::BoolVar { idx } => {
                let idx = *idx;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Bool(b) => Value::Bool(*b),
                    _ => Value::Bool(false),
                }))
            }
            Self::InIntLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(vals.contains(i)),
                    _ => Value::Bool(false),
                }))
            }
            Self::InIntHash { idx, set } => {
                let idx = *idx; let set: HashSet<i64> = set.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => Value::Bool(set.contains(i)),
                    _ => Value::Bool(false),
                }))
            }
            Self::InStrLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => Value::Bool(vals.iter().any(|v| s.as_ref() == v)),
                    _ => Value::Bool(false),
                }))
            }
            Self::InStrHash { idx, set } => {
                let idx = *idx; let set: HashSet<String> = set.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => Value::Bool(set.contains(s.as_ref())),
                    _ => Value::Bool(false),
                }))
            }
            Self::StartsWith { idx, prefix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(prefix.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => Value::Bool(v.starts_with(s.as_ref())),
                    _ => Value::Bool(false),
                }))
            }
            Self::EndsWith { idx, suffix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(suffix.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => Value::Bool(v.ends_with(s.as_ref())),
                    _ => Value::Bool(false),
                }))
            }
            Self::Contains { idx, substring } => {
                let idx = *idx; let s: Arc<str> = Arc::from(substring.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => Value::Bool(v.contains(s.as_ref())),
                    _ => Value::Bool(false),
                }))
            }
            Self::Matches { idx, regex } => {
                let idx = *idx; let regex = regex.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => Value::Bool(regex.is_match(s)),
                    _ => Value::Bool(false),
                }))
            }
            Self::ContainsAny { idx, needles } => {
                let idx = *idx; let needles = needles.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => {
                        let text: &str = s;
                        for n in &needles { if text.contains(n.as_str()) { return Value::Bool(true); } }
                        Value::Bool(false)
                    }
                    _ => Value::Bool(false),
                }))
            }
            Self::AhoContains { idx, ac, min } => {
                let idx = *idx; let ac = ac.clone(); let min = *min;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => {
                        let text = s.as_bytes();
                        if min <= 1 { return Value::Bool(ac.is_match(text)); }
                        let mut matched = 0u64;
                        for mat in ac.find_iter(text) {
                            let pid = mat.pattern().as_u64();
                            if pid < 64 {
                                matched |= 1u64 << pid;
                                if matched.count_ones() as usize >= min { return Value::Bool(true); }
                            }
                        }
                        Value::Bool(false)
                    }
                    _ => Value::Bool(false),
                }))
            }
            Self::GeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) >= r_fn(vars))))
            }
            Self::GtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) > r_fn(vars))))
            }
            Self::LeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) <= r_fn(vars))))
            }
            Self::LtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) < r_fn(vars))))
            }
            Self::EqExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) == r_fn(vars))))
            }
            Self::NeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new(Arc::new(move |vars| Value::Bool(l_fn(vars) != r_fn(vars))))
            }
            Self::And(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new(Arc::new(move |vars| match a_fn.eval(vars) {
                    Value::Bool(true) => b_fn.eval(vars),
                    _ => Value::Bool(false),
                }))
            }
            Self::Or(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new(Arc::new(move |vars| {
                    if matches!(a_fn.eval(vars), Value::Bool(true)) { return Value::Bool(true); }
                    b_fn.eval(vars)
                }))
            }
            Self::Not(inner) => {
                let inner_fn = inner.compile();
                CompiledFilterNode::new(Arc::new(move |vars| match inner_fn.eval(vars) {
                    Value::Bool(b) => Value::Bool(!b),
                    _ => Value::Bool(false),
                }))
            }
            Self::Exists { list_idx, item_idx, predicate } => {
                let list_idx = *list_idx; let item_idx = *item_idx; let pred = predicate.compile();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => {
                        let mut extended = vars.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[item_idx] = item.clone();
                            if let Value::Bool(true) = pred.eval(&extended) { return Value::Bool(true); }
                        }
                        Value::Bool(false)
                    }
                    _ => Value::Bool(false),
                }))
            }
            Self::ExistsClosure { list_idx, predicate } => {
                let list_idx = *list_idx; let pred = predicate.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => Value::Bool(list.iter().any(|item| pred.call(item))),
                    _ => Value::Bool(false),
                }))
            }
            Self::MapKeyContains { map_idx, key, needle } => {
                let map_idx = *map_idx;
                let key: Arc<str> = Arc::from(key.as_str());
                let needle: Arc<str> = Arc::from(needle.as_str());
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[map_idx] {
                    Value::Map(m) => {
                        let k = Key::String(Arc::clone(&key));
                        match m.map.get(&k) {
                            Some(Value::String(s)) => Value::Bool(s.as_ref() == needle.as_ref()),
                            Some(Value::List(list)) => Value::Bool(list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_ref()))),
                            _ => Value::Bool(false),
                        }
                    }
                    _ => Value::Bool(false),
                }))
            }
            Self::ExistsInIntSet { list_idx, vals } => {
                let list_idx = *list_idx; let vals = vals.clone();
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => Value::Bool(list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i)))),
                    _ => Value::Bool(false),
                }))
            }
            Self::ExistsEqInt { list_idx, val } => {
                let list_idx = *list_idx; let val = *val;
                CompiledFilterNode::new(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => Value::Bool(list.iter().any(|v| matches!(v, Value::Int(i) if *i == val))),
                    _ => Value::Bool(false),
                }))
            }
        }
    }
}
