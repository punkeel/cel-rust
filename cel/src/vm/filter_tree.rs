use crate::objects::{Key, Value};
use std::collections::HashSet;
use std::sync::Arc;

// ── Fast evaluation view with typed arrays ──

/// Fast-evaluation context view exposing typed arrays.
///
/// Int and string fields access their dedicated slices directly,
/// bypassing the `Value` enum match entirely. Compound types (lists,
/// maps, booleans) fall back to `values`.
#[derive(Clone, Copy)]
pub struct EvalView<'a> {
    pub ints: &'a [i64],
    pub strings: &'a [Arc<str>],
    pub values: &'a [Value],
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

/// A typed string expression that evaluates to a length.
#[derive(Clone, Debug)]
pub enum StrExpr {
    Literal(String),
    Var(usize),
    Concat(Box<StrExpr>, Box<StrExpr>),
}

impl StrExpr {
    pub fn eval_len(&self, ctx: &EvalView) -> usize {
        match self {
            Self::Literal(s) => s.len(),
            Self::Var(idx) => ctx.strings[*idx].len(),
            Self::Concat(a, b) => a.eval_len(ctx) + b.eval_len(ctx),
        }
    }
}

/// A typed list expression.
#[derive(Clone, Debug)]
pub enum ListExpr {
    Var(usize),
}

impl ListExpr {
    pub fn eval_len(&self, ctx: &EvalView) -> usize {
        match self {
            Self::Var(idx) => match &ctx.values[*idx] {
                Value::List(list) => list.len(),
                _ => 0,
            },
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
    pub fn eval_i64(&self, ctx: &EvalView) -> i64 {
        match self {
            Self::Literal(v) => *v,
            Self::Var(idx) => ctx.ints[*idx],
            Self::Add(a, b) => a.eval_i64(ctx).wrapping_add(b.eval_i64(ctx)),
            Self::Sub(a, b) => a.eval_i64(ctx).wrapping_sub(b.eval_i64(ctx)),
            Self::Mul(a, b) => a.eval_i64(ctx).wrapping_mul(b.eval_i64(ctx)),
            Self::Div(a, b) => {
                let bv = b.eval_i64(ctx);
                if bv == 0 { 0 } else { a.eval_i64(ctx).wrapping_div(bv) }
            }
            Self::Mod(a, b) => {
                let bv = b.eval_i64(ctx);
                if bv == 0 { 0 } else { a.eval_i64(ctx).wrapping_rem(bv) }
            }
            Self::Neg(a) => a.eval_i64(ctx).wrapping_neg(),
            Self::StrLen(s) => s.eval_len(ctx) as i64,
            Self::ListLen(l) => l.eval_len(ctx) as i64,
        }
    }
}

// =====================================================================
// FilterNode — the unified expression tree: both compile-time IR and
//              evaluation engine. No separate compilation step needed.
// =====================================================================

/// A boolean expression tree that can be evaluated directly.
///
/// Dual-purpose: serves as both the compile-time IR (pattern-matchable,
/// debuggable, cheap to clone) and the runtime evaluation engine
/// (via `eval_bool` / `eval`).
///
/// There is no separate compilation step or closure indirection —
/// evaluation is a single match on this enum, fully visible to the
/// optimizer for inlining.
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
    /// Evaluate the node as a boolean.
    ///
    /// Fast path — returns `bool` directly without any `Value::Bool` wrapping.
    /// For future non-bool variants, use [`eval`] instead.
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        match self {
            // ── Int comparisons: direct ctx.ints access ──
            Self::EqInt { idx, val } => ctx.ints[*idx] == *val,
            Self::NeInt { idx, val } => ctx.ints[*idx] != *val,
            Self::LtInt { idx, val } => ctx.ints[*idx] < *val,
            Self::LeInt { idx, val } => ctx.ints[*idx] <= *val,
            Self::GtInt { idx, val } => ctx.ints[*idx] > *val,
            Self::GeInt { idx, val } => ctx.ints[*idx] >= *val,

            // ── Fused arithmetic + comparison ──
            Self::AddEq { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) == *cmp,
            Self::AddNe { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) != *cmp,
            Self::AddLt { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) < *cmp,
            Self::AddLe { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) <= *cmp,
            Self::AddGt { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) > *cmp,
            Self::AddGe { idx, arith, cmp } => ctx.ints[*idx].wrapping_add(*arith) >= *cmp,
            Self::SubEq { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) == *cmp,
            Self::SubNe { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) != *cmp,
            Self::SubLt { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) < *cmp,
            Self::SubLe { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) <= *cmp,
            Self::SubGt { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) > *cmp,
            Self::SubGe { idx, arith, cmp } => ctx.ints[*idx].wrapping_sub(*arith) >= *cmp,
            Self::MulEq { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) == *cmp,
            Self::MulNe { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) != *cmp,
            Self::MulLt { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) < *cmp,
            Self::MulLe { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) <= *cmp,
            Self::MulGt { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) > *cmp,
            Self::MulGe { idx, arith, cmp } => ctx.ints[*idx].wrapping_mul(*arith) >= *cmp,

            // ── String comparison: direct ctx.strings access ──
            Self::EqStr { idx, val } => ctx.strings[*idx].as_ref() == val.as_str(),
            Self::NeStr { idx, val } => ctx.strings[*idx].as_ref() != val.as_str(),

            // ── Bool: via values fallback ──
            Self::BoolVar { idx } => match &ctx.values[*idx] {
                Value::Bool(b) => *b,
                _ => false,
            },

            // ── Int set membership ──
            Self::InIntLinear { idx, vals } => vals.contains(&ctx.ints[*idx]),
            Self::InIntHash { idx, set } => set.contains(&ctx.ints[*idx]),

            // ── Str set membership ──
            Self::InStrLinear { idx, vals } => {
                let v = ctx.strings[*idx].as_ref();
                vals.iter().any(|s| s == v)
            }
            Self::InStrHash { idx, set } => set.contains(ctx.strings[*idx].as_ref()),

            // ── String methods ──
            Self::StartsWith { idx, prefix } => ctx.strings[*idx].starts_with(prefix.as_str()),
            Self::EndsWith { idx, suffix } => ctx.strings[*idx].ends_with(suffix.as_str()),
            Self::Contains { idx, substring } => ctx.strings[*idx].contains(substring.as_str()),
            Self::Matches { idx, regex } => regex.is_match(ctx.strings[*idx].as_ref()),

            // ── Multi-pattern contains ──
            Self::ContainsAny { idx, needles } => {
                let text = ctx.strings[*idx].as_ref();
                for n in needles { if text.contains(n.as_str()) { return true; } }
                false
            }
            Self::AhoContains { idx, ac, min } => {
                let text = ctx.strings[*idx].as_ref();
                let bytes = text.as_bytes();
                if *min <= 1 { return ac.is_match(bytes); }
                let mut matched = 0u64;
                for mat in ac.find_iter(bytes) {
                    let pid = mat.pattern().as_u64();
                    if pid < 64 {
                        matched |= 1u64 << pid;
                        if matched.count_ones() as usize >= *min { return true; }
                    }
                }
                false
            }

            // ── I64Expr comparisons: recursive eval ──
            Self::GeExpr { left, right } => left.eval_i64(ctx) >= right.eval_i64(ctx),
            Self::GtExpr { left, right } => left.eval_i64(ctx) > right.eval_i64(ctx),
            Self::LeExpr { left, right } => left.eval_i64(ctx) <= right.eval_i64(ctx),
            Self::LtExpr { left, right } => left.eval_i64(ctx) < right.eval_i64(ctx),
            Self::EqExpr { left, right } => left.eval_i64(ctx) == right.eval_i64(ctx),
            Self::NeExpr { left, right } => left.eval_i64(ctx) != right.eval_i64(ctx),

            // ── Logic combinators: recursive eval ──
            Self::And(a, b) => a.eval_bool(ctx) && b.eval_bool(ctx),
            Self::Or(a, b) => a.eval_bool(ctx) || b.eval_bool(ctx),
            Self::Not(inner) => !inner.eval_bool(ctx),

            // ── General Exists: allocates scratch vector ──
            Self::Exists { list_idx, item_idx, predicate } => {
                match &ctx.values[*list_idx] {
                    Value::List(list) => {
                        let mut extended = ctx.values.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[*item_idx] = item.clone();
                            let tmp = EvalView {
                                ints: ctx.ints,
                                strings: ctx.strings,
                                values: &extended,
                            };
                            if predicate.eval_bool(&tmp) { return true; }
                        }
                        false
                    }
                    _ => false,
                }
            }

            // ── ExistsClosure: no alloc ──
            Self::ExistsClosure { list_idx, predicate } => {
                match &ctx.values[*list_idx] {
                    Value::List(list) => list.iter().any(|item| predicate.call(item)),
                    _ => false,
                }
            }

            // ── Map-key contains ──
            Self::MapKeyContains { map_idx, key, needle } => {
                match &ctx.values[*map_idx] {
                    Value::Map(m) => {
                        let k = Key::String(Arc::from(key.as_str()));
                        match m.map.get(&k) {
                            Some(Value::String(s)) => s.as_ref() == needle.as_str(),
                            Some(Value::List(list)) => list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_str())),
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }

            // ── Specialized exists ──
            Self::ExistsInIntSet { list_idx, vals } => {
                match &ctx.values[*list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                    _ => false,
                }
            }
            Self::ExistsEqInt { list_idx, val } => {
                match &ctx.values[*list_idx] {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == *val)),
                    _ => false,
                }
            }
        }
    }

    /// Evaluate the node and return a `Value`.
    ///
    /// Current variants all return `Value::Bool(...)`. Future non-bool
    /// variants (arithmetic, UDFs) will return their typed Value directly.
    /// Prefer [`eval_bool`] when the expression is known to be boolean —
    /// it avoids the `Value::Bool` wrapping.
    #[inline(always)]
    pub fn eval(&self, ctx: &EvalView) -> Value {
        // All current variants are boolean — wrap the fast path.
        // Future non-bool variants will get their own match arms here.
        Value::Bool(self.eval_bool(ctx))
    }
}
