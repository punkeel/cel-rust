use crate::objects::{Key, Value};
use std::collections::HashSet;
use std::sync::Arc;

// ── Compiled node (specialized Bool vs Value) ──

/// A compiled expression node with two variants:
/// - Bool: fast path returning bool (no Value wrapping)
/// - Value: returns an arbitrary Value (future non-bool expressions)
#[derive(Clone)]
pub enum CompiledNode {
    Bool(Arc<dyn Fn(&[Value]) -> bool>),
    Value(Arc<dyn Fn(&[Value]) -> Value>),
}

impl CompiledNode {
    #[inline(always)]
    pub fn eval_bool(&self, vars: &[Value]) -> bool {
        match self {
            Self::Bool(f) => f(vars),
            Self::Value(f) => matches!(f(vars), Value::Bool(true)),
        }
    }

    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> Value {
        match self {
            Self::Bool(f) => Value::Bool(f(vars)),
            Self::Value(f) => f(vars),
        }
    }
}

// ── Compiled filter node (wraps CompiledNode) ──

/// A compiled filter expression backed by a single closure.
/// Cheap to clone (Arc). Wraps a CompiledNode.
#[derive(Clone)]
pub struct CompiledFilterNode(CompiledNode);

impl CompiledFilterNode {
    pub fn new(f: Arc<dyn Fn(&[Value]) -> Value>) -> Self {
        Self(CompiledNode::Value(f))
    }

    pub fn new_bool(f: Arc<dyn Fn(&[Value]) -> bool>) -> Self {
        Self(CompiledNode::Bool(f))
    }

    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> Value {
        self.0.eval(vars)
    }

    #[inline(always)]
    pub fn eval_bool(&self, vars: &[Value]) -> bool {
        self.0.eval_bool(vars)
    }
}

impl std::fmt::Debug for CompiledFilterNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledFilterNode").finish()
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
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i == val,
                    _ => false,
                }))
            }
            Self::NeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i != val,
                    _ => false,
                }))
            }
            Self::LtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i < val,
                    _ => false,
                }))
            }
            Self::LeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i <= val,
                    _ => false,
                }))
            }
            Self::GtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i > val,
                    _ => false,
                }))
            }
            Self::GeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => *i >= val,
                    _ => false,
                }))
            }
            Self::AddEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) == cmp,
                    _ => false,
                }))
            }
            Self::AddNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) != cmp,
                    _ => false,
                }))
            }
            Self::AddLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) < cmp,
                    _ => false,
                }))
            }
            Self::AddLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) <= cmp,
                    _ => false,
                }))
            }
            Self::AddGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) > cmp,
                    _ => false,
                }))
            }
            Self::AddGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_add(arith) >= cmp,
                    _ => false,
                }))
            }
            Self::SubEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) == cmp,
                    _ => false,
                }))
            }
            Self::SubNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) != cmp,
                    _ => false,
                }))
            }
            Self::SubLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) < cmp,
                    _ => false,
                }))
            }
            Self::SubLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) <= cmp,
                    _ => false,
                }))
            }
            Self::SubGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) > cmp,
                    _ => false,
                }))
            }
            Self::SubGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_sub(arith) >= cmp,
                    _ => false,
                }))
            }
            Self::MulEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) == cmp,
                    _ => false,
                }))
            }
            Self::MulNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) != cmp,
                    _ => false,
                }))
            }
            Self::MulLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) < cmp,
                    _ => false,
                }))
            }
            Self::MulLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) <= cmp,
                    _ => false,
                }))
            }
            Self::MulGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) > cmp,
                    _ => false,
                }))
            }
            Self::MulGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => i.wrapping_mul(arith) >= cmp,
                    _ => false,
                }))
            }
            Self::EqStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => s.as_ref() == val.as_ref(),
                    _ => false,
                }))
            }
            Self::NeStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => s.as_ref() != val.as_ref(),
                    _ => false,
                }))
            }
            Self::BoolVar { idx } => {
                let idx = *idx;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Bool(b) => *b,
                    _ => false,
                }))
            }
            Self::InIntLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => vals.contains(i),
                    _ => false,
                }))
            }
            Self::InIntHash { idx, set } => {
                let idx = *idx; let set: HashSet<i64> = set.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::Int(i) => set.contains(i),
                    _ => false,
                }))
            }
            Self::InStrLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => vals.iter().any(|v| s.as_ref() == v),
                    _ => false,
                }))
            }
            Self::InStrHash { idx, set } => {
                let idx = *idx; let set: HashSet<String> = set.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => set.contains(s.as_ref()),
                    _ => false,
                }))
            }
            Self::StartsWith { idx, prefix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(prefix.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => v.starts_with(s.as_ref()),
                    _ => false,
                }))
            }
            Self::EndsWith { idx, suffix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(suffix.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => v.ends_with(s.as_ref()),
                    _ => false,
                }))
            }
            Self::Contains { idx, substring } => {
                let idx = *idx; let s: Arc<str> = Arc::from(substring.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(v) => v.contains(s.as_ref()),
                    _ => false,
                }))
            }
            Self::Matches { idx, regex } => {
                let idx = *idx; let regex = regex.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => regex.is_match(s),
                    _ => false,
                }))
            }
            Self::ContainsAny { idx, needles } => {
                let idx = *idx; let needles = needles.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => {
                        let text: &str = s;
                        for n in &needles { if text.contains(n.as_str()) { return true; } }
                        false
                    }
                    _ => false,
                }))
            }
            Self::AhoContains { idx, ac, min } => {
                let idx = *idx; let ac = ac.clone(); let min = *min;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[idx] {
                    Value::String(s) => {
                        let text = s.as_bytes();
                        if min <= 1 { return ac.is_match(text); }
                        let mut matched = 0u64;
                        for mat in ac.find_iter(text) {
                            let pid = mat.pattern().as_u64();
                            if pid < 64 {
                                matched |= 1u64 << pid;
                                if matched.count_ones() as usize >= min { return true; }
                            }
                        }
                        false
                    }
                    _ => false,
                }))
            }
            Self::GeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) >= r_fn(vars)))
            }
            Self::GtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) > r_fn(vars)))
            }
            Self::LeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) <= r_fn(vars)))
            }
            Self::LtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) < r_fn(vars)))
            }
            Self::EqExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) == r_fn(vars)))
            }
            Self::NeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| l_fn(vars) != r_fn(vars)))
            }
            Self::And(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| a_fn.eval_bool(vars) && b_fn.eval_bool(vars)))
            }
            Self::Or(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| {
                    if matches!(a_fn.eval_bool(vars), true) { return true; }
                    b_fn.eval_bool(vars)
                }))
            }
            Self::Not(inner) => {
                let inner_fn = inner.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| !inner_fn.eval_bool(vars)))
            }
            Self::Exists { list_idx, item_idx, predicate } => {
                let list_idx = *list_idx; let item_idx = *item_idx; let pred = predicate.compile();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => {
                        let mut extended = vars.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[item_idx] = item.clone();
                            if pred.eval_bool(&extended) { return true; }
                        }
                        false
                    }
                    _ => false,
                }))
            }
            Self::ExistsClosure { list_idx, predicate } => {
                let list_idx = *list_idx; let pred = predicate.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => list.iter().any(|item| pred.call(item)),
                    _ => false,
                }))
            }
            Self::MapKeyContains { map_idx, key, needle } => {
                let map_idx = *map_idx;
                let key: Arc<str> = Arc::from(key.as_str());
                let needle: Arc<str> = Arc::from(needle.as_str());
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[map_idx] {
                    Value::Map(m) => {
                        let k = Key::String(Arc::clone(&key));
                        match m.map.get(&k) {
                            Some(Value::String(s)) => s.as_ref() == needle.as_ref(),
                            Some(Value::List(list)) => list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_ref())),
                            _ => false,
                        }
                    }
                    _ => false,
                }))
            }
            Self::ExistsInIntSet { list_idx, vals } => {
                let list_idx = *list_idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                    _ => false,
                }))
            }
            Self::ExistsEqInt { list_idx, val } => {
                let list_idx = *list_idx; let val = *val;
                CompiledFilterNode::new_bool(Arc::new(move |vars| match &vars[list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == val)),
                    _ => false,
                }))
            }
        }
    }
}
