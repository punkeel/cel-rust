use crate::objects::{Key, Value};
use std::collections::HashSet;
use std::sync::Arc;

// ── Fast evaluation view with typed arrays ──

/// Fast-evaluation context view exposing typed arrays.
///
/// Closures access field values directly through the appropriate typed
/// slice, completely bypassing the `Value` enum match for int/string
/// fields. The `values` fallback slice exists for compound types (lists,
/// maps) and boolean fields.
#[derive(Clone, Copy)]
pub struct EvalView<'a> {
    pub ints: &'a [i64],
    pub strings: &'a [Arc<str>],
    pub values: &'a [Value],
}

// ── Compiled node (specialized Bool vs Value) ──

/// A compiled expression node with two variants:
/// - Bool: fast path returning bool (no Value wrapping)
/// - Value: returns an arbitrary Value (future non-bool expressions)
pub enum CompiledNode {
    Bool(Box<dyn Fn(&EvalView) -> bool>),
    Value(Box<dyn Fn(&EvalView) -> Value>),
}

impl CompiledNode {
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        match self {
            Self::Bool(f) => f(ctx),
            Self::Value(f) => matches!(f(ctx), Value::Bool(true)),
        }
    }

    #[inline(always)]
    pub fn eval(&self, ctx: &EvalView) -> Value {
        match self {
            Self::Bool(f) => Value::Bool(f(ctx)),
            Self::Value(f) => f(ctx),
        }
    }
}

// ── Compiled filter node (wraps CompiledNode) ──

/// A compiled filter expression backed by a single closure.
/// Wraps a CompiledNode.
pub struct CompiledFilterNode(CompiledNode);

impl CompiledFilterNode {
    pub fn new(f: Box<dyn Fn(&EvalView) -> Value>) -> Self {
        Self(CompiledNode::Value(f))
    }

    pub fn new_bool(f: Box<dyn Fn(&EvalView) -> bool>) -> Self {
        Self(CompiledNode::Bool(f))
    }

    #[inline(always)]
    pub fn eval(&self, ctx: &EvalView) -> Value {
        self.0.eval(ctx)
    }

    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        self.0.eval_bool(ctx)
    }

    /// Unwrap into the inner CompiledNode.
    pub fn into_inner(self) -> CompiledNode {
        self.0
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
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> {
        match self {
            Self::Literal(s) => {
                let len = s.len();
                Box::new(move |_| len)
            }
            Self::Var(idx) => {
                let i = *idx;
                Box::new(move |ctx| ctx.strings[i].len())
            }
            Self::Concat(a, b) => {
                let a_fn = a.compile_len();
                let b_fn = b.compile_len();
                Box::new(move |ctx| a_fn(ctx) + b_fn(ctx))
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
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> {
        match self {
            Self::Var(idx) => {
                let i = *idx;
                Box::new(move |ctx| match &ctx.values[i] {
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
    pub fn compile(&self) -> Box<dyn Fn(&EvalView) -> i64> {
        match self {
            Self::Literal(v) => {
                let v = *v;
                Box::new(move |_| v)
            }
            Self::Var(idx) => {
                let i = *idx;
                Box::new(move |ctx| ctx.ints[i])
            }
            Self::Add(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Box::new(move |ctx| a_fn(ctx).wrapping_add(b_fn(ctx)))
            }
            Self::Sub(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Box::new(move |ctx| a_fn(ctx).wrapping_sub(b_fn(ctx)))
            }
            Self::Mul(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Box::new(move |ctx| a_fn(ctx).wrapping_mul(b_fn(ctx)))
            }
            Self::Div(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Box::new(move |ctx| {
                    let bv = b_fn(ctx);
                    if bv == 0 { 0 } else { a_fn(ctx).wrapping_div(bv) }
                })
            }
            Self::Mod(a, b) => {
                let a_fn = a.compile();
                let b_fn = b.compile();
                Box::new(move |ctx| {
                    let bv = b_fn(ctx);
                    if bv == 0 { 0 } else { a_fn(ctx).wrapping_rem(bv) }
                })
            }
            Self::Neg(a) => {
                let a_fn = a.compile();
                Box::new(move |ctx| a_fn(ctx).wrapping_neg())
            }
            Self::StrLen(s) => {
                let s_fn = s.compile_len();
                Box::new(move |ctx| s_fn(ctx) as i64)
            }
            Self::ListLen(l) => {
                let l_fn = l.compile_len();
                Box::new(move |ctx| l_fn(ctx) as i64)
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
    /// The returned `CompiledFilterNode` uses typed access — int fields
    /// read from `EvalView::ints`, strings from `EvalView::strings`,
    /// skipping the Value enum match entirely.
    pub fn compile(&self) -> CompiledFilterNode {
        match self {
            // ── Int comparisons: direct ctx.ints access ──
            Self::EqInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] == val))
            }
            Self::NeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] != val))
            }
            Self::LtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] < val))
            }
            Self::LeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] <= val))
            }
            Self::GtInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] > val))
            }
            Self::GeInt { idx, val } => {
                let idx = *idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx] >= val))
            }

            // ── Fused arithmetic + comparison: ctx.ints with wrapping arith ──
            Self::AddEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) == cmp))
            }
            Self::AddNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) != cmp))
            }
            Self::AddLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) < cmp))
            }
            Self::AddLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) <= cmp))
            }
            Self::AddGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) > cmp))
            }
            Self::AddGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith) >= cmp))
            }
            Self::SubEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) == cmp))
            }
            Self::SubNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) != cmp))
            }
            Self::SubLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) < cmp))
            }
            Self::SubLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) <= cmp))
            }
            Self::SubGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) > cmp))
            }
            Self::SubGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith) >= cmp))
            }
            Self::MulEq { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) == cmp))
            }
            Self::MulNe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) != cmp))
            }
            Self::MulLt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) < cmp))
            }
            Self::MulLe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) <= cmp))
            }
            Self::MulGt { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) > cmp))
            }
            Self::MulGe { idx, arith, cmp } => {
                let idx = *idx; let arith = *arith; let cmp = *cmp;
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith) >= cmp))
            }

            // ── String comparison: direct ctx.strings access ──
            Self::EqStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.strings[idx].as_ref() == val.as_ref()))
            }
            Self::NeStr { idx, val } => {
                let idx = *idx; let val: Arc<str> = Arc::from(val.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.strings[idx].as_ref() != val.as_ref()))
            }

            // ── Bool: via values fallback (not in typed arrays) ──
            Self::BoolVar { idx } => {
                let idx = *idx;
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[idx] {
                    Value::Bool(b) => *b,
                    _ => false,
                }))
            }

            // ── Int set membership: ctx.ints ──
            Self::InIntLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| vals.contains(&ctx.ints[idx])))
            }
            Self::InIntHash { idx, set } => {
                let idx = *idx; let set: HashSet<i64> = set.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| set.contains(&ctx.ints[idx])))
            }

            // ── Str set membership: ctx.strings ──
            Self::InStrLinear { idx, vals } => {
                let idx = *idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| {
                    let v = ctx.strings[idx].as_ref();
                    vals.iter().any(|s| s == v)
                }))
            }
            Self::InStrHash { idx, set } => {
                let idx = *idx; let set: HashSet<String> = set.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| set.contains(ctx.strings[idx].as_ref())))
            }

            // ── String methods: ctx.strings ──
            Self::StartsWith { idx, prefix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(prefix.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.strings[idx].starts_with(s.as_ref())))
            }
            Self::EndsWith { idx, suffix } => {
                let idx = *idx; let s: Arc<str> = Arc::from(suffix.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.strings[idx].ends_with(s.as_ref())))
            }
            Self::Contains { idx, substring } => {
                let idx = *idx; let s: Arc<str> = Arc::from(substring.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| ctx.strings[idx].contains(s.as_ref())))
            }
            Self::Matches { idx, regex } => {
                let idx = *idx; let regex = regex.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| regex.is_match(ctx.strings[idx].as_ref())))
            }

            // ── Multi-pattern contains: ctx.strings ──
            Self::ContainsAny { idx, needles } => {
                let idx = *idx; let needles = needles.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| {
                    let text = ctx.strings[idx].as_ref();
                    for n in &needles { if text.contains(n.as_str()) { return true; } }
                    false
                }))
            }
            Self::AhoContains { idx, ac, min } => {
                let idx = *idx; let ac = ac.clone(); let min = *min;
                CompiledFilterNode::new_bool(Box::new(move |ctx| {
                    let text = ctx.strings[idx].as_ref();
                    let bytes = text.as_bytes();
                    if min <= 1 { return ac.is_match(bytes); }
                    let mut matched = 0u64;
                    for mat in ac.find_iter(bytes) {
                        let pid = mat.pattern().as_u64();
                        if pid < 64 {
                            matched |= 1u64 << pid;
                            if matched.count_ones() as usize >= min { return true; }
                        }
                    }
                    false
                }))
            }

            // ── I64Expr comparisons: pass ctx through to sub-closures ──
            Self::GeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) >= r_fn(ctx)))
            }
            Self::GtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) > r_fn(ctx)))
            }
            Self::LeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) <= r_fn(ctx)))
            }
            Self::LtExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) < r_fn(ctx)))
            }
            Self::EqExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) == r_fn(ctx)))
            }
            Self::NeExpr { left, right } => {
                let l_fn = left.compile(); let r_fn = right.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| l_fn(ctx) != r_fn(ctx)))
            }

            // ── Logic combinators: compose via ctx ──
            Self::And(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| a_fn.eval_bool(ctx) && b_fn.eval_bool(ctx)))
            }
            Self::Or(a, b) => {
                let a_fn = a.compile(); let b_fn = b.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| {
                    if a_fn.eval_bool(ctx) { return true; }
                    b_fn.eval_bool(ctx)
                }))
            }
            Self::Not(inner) => {
                let inner_fn = inner.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| !inner_fn.eval_bool(ctx)))
            }

            // ── General Exists: allocates scratch vector ──
            Self::Exists { list_idx, item_idx, predicate } => {
                let list_idx = *list_idx; let item_idx = *item_idx; let pred = predicate.compile();
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[list_idx] {
                    Value::List(list) => {
                        let mut extended = ctx.values.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[item_idx] = item.clone();
                            let tmp = EvalView {
                                ints: ctx.ints,
                                strings: ctx.strings,
                                values: &extended,
                            };
                            if pred.eval_bool(&tmp) { return true; }
                        }
                        false
                    }
                    _ => false,
                }))
            }

            // ── ExistsClosure: no alloc, ItemPredicate works on &Value ──
            Self::ExistsClosure { list_idx, predicate } => {
                let list_idx = *list_idx; let pred = predicate.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[list_idx] {
                    Value::List(list) => list.iter().any(|item| pred.call(item)),
                    _ => false,
                }))
            }

            // ── Map-key contains: via values fallback ──
            Self::MapKeyContains { map_idx, key, needle } => {
                let map_idx = *map_idx;
                let key: Arc<str> = Arc::from(key.as_str());
                let needle: Arc<str> = Arc::from(needle.as_str());
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[map_idx] {
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

            // ── Specialized exists ──
            Self::ExistsInIntSet { list_idx, vals } => {
                let list_idx = *list_idx; let vals = vals.clone();
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                    _ => false,
                }))
            }
            Self::ExistsEqInt { list_idx, val } => {
                let list_idx = *list_idx; let val = *val;
                CompiledFilterNode::new_bool(Box::new(move |ctx| match &ctx.values[list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == val)),
                    _ => false,
                }))
            }
        }
    }
}
