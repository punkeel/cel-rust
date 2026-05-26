use crate::objects::Value;

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
    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> i64 {
        match self {
            Self::Literal(v) => *v,
            Self::Var(idx) => match &vars[*idx] {
                Value::Int(i) => *i,
                _ => 0,
            },
            Self::Add(a, b) => a.eval(vars).wrapping_add(b.eval(vars)),
            Self::Sub(a, b) => a.eval(vars).wrapping_sub(b.eval(vars)),
            Self::Mul(a, b) => a.eval(vars).wrapping_mul(b.eval(vars)),
            Self::Div(a, b) => {
                let bv = b.eval(vars);
                if bv == 0 { 0 } else { a.eval(vars).wrapping_div(bv) }
            }
            Self::Mod(a, b) => {
                let bv = b.eval(vars);
                if bv == 0 { 0 } else { a.eval(vars).wrapping_rem(bv) }
            }
            Self::Neg(a) => a.eval(vars).wrapping_neg(),
            Self::StrLen(a) => a.len(vars) as i64,
            Self::ListLen(a) => a.len(vars) as i64,
        }
    }
}

/// A typed string expression that evaluates directly to `&str` (via reference to var storage).
/// For owned results (concatenation), we return `Option<String>` and the caller stores it.
#[derive(Clone, Debug)]
pub enum StrExpr {
    Literal(String),
    Var(usize),
    Concat(Box<StrExpr>, Box<StrExpr>),
}

impl StrExpr {
    /// Evaluate to a borrowed string if possible (Var or Literal).
    /// Returns None for Concat which requires allocation.
    #[inline(always)]
    pub fn eval_borrow<'a>(&'a self, vars: &'a [Value]) -> Option<&'a str> {
        match self {
            Self::Literal(s) => Some(s.as_str()),
            Self::Var(idx) => match &vars[*idx] {
                Value::String(s) => Some(&**s),
            _ => Some(""),
            },
            Self::Concat(_, _) => None,
        }
    }

    /// Evaluate to an owned string (allocates for Concat).
    #[inline(always)]
    pub fn eval_owned(&self, vars: &[Value]) -> String {
        match self {
            Self::Literal(s) => s.clone(),
            Self::Var(idx) => match &vars[*idx] {
                Value::String(s) => (&**s).to_string(),
                _ => String::new(),
            },
            Self::Concat(a, b) => {
                let mut result = a.eval_owned(vars);
                result.push_str(&b.eval_owned(vars));
                result
            }
        }
    }

    /// Length in bytes.
    #[inline(always)]
    pub fn len(&self, vars: &[Value]) -> usize {
        match self {
            Self::Literal(s) => s.len(),
            Self::Var(idx) => match &vars[*idx] {
                Value::String(s) => s.len(),
                _ => 0,
            },
            Self::Concat(a, b) => a.len(vars) + b.len(vars),
        }
    }
}

/// A typed list expression.
#[derive(Clone, Debug)]
pub enum ListExpr {
    Var(usize),
}

impl ListExpr {
    #[inline(always)]
    pub fn eval<'a>(&'a self, vars: &'a [Value]) -> Option<&'a Vec<Value>> {
        match self {
            Self::Var(idx) => match &vars[*idx] {
                Value::List(list) => Some(list),
                _ => None,
            },
        }
    }

    #[inline(always)]
    pub fn len(&self, vars: &[Value]) -> usize {
        self.eval(vars).map_or(0, |list| list.len())
    }
}

// =====================================================================
// FilterNode — concrete enum replacing `Box<dyn BoolFilter>` + 25 structs
// =====================================================================

/// A compiled boolean expression evaluating directly to `bool`.
///
/// This is a concrete enum with no trait-object dispatch. Every variant
/// is specialized for a specific pattern — the eval method is a single
/// match that the compiler can fully inline and optimize.
///
/// # Performance
///
/// No vtable dispatch, no generic monomorphization across combinators.
/// The `eval` function is a single `match` — the compiler sees every
/// branch at once and can optimize across them.
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
    // var + arith op cmp
    AddEq { idx: usize, arith: i64, cmp: i64 },
    AddNe { idx: usize, arith: i64, cmp: i64 },
    AddLt { idx: usize, arith: i64, cmp: i64 },
    AddLe { idx: usize, arith: i64, cmp: i64 },
    AddGt { idx: usize, arith: i64, cmp: i64 },
    AddGe { idx: usize, arith: i64, cmp: i64 },
    // var - arith op cmp
    SubEq { idx: usize, arith: i64, cmp: i64 },
    SubNe { idx: usize, arith: i64, cmp: i64 },
    SubLt { idx: usize, arith: i64, cmp: i64 },
    SubLe { idx: usize, arith: i64, cmp: i64 },
    SubGt { idx: usize, arith: i64, cmp: i64 },
    SubGe { idx: usize, arith: i64, cmp: i64 },
    // var * arith op cmp
    MulEq { idx: usize, arith: i64, cmp: i64 },
    MulNe { idx: usize, arith: i64, cmp: i64 },
    MulLt { idx: usize, arith: i64, cmp: i64 },
    MulLe { idx: usize, arith: i64, cmp: i64 },
    MulGt { idx: usize, arith: i64, cmp: i64 },
    MulGe { idx: usize, arith: i64, cmp: i64 },

    // --- String comparison ---
    EqStr { idx: usize, val: String },

    // --- Set membership: int ---
    InIntLinear { idx: usize, vals: Vec<i64> },
    InIntHash { idx: usize, set: std::collections::HashSet<i64> },

    // --- Set membership: str ---
    InStrLinear { idx: usize, vals: Vec<String> },
    InStrHash { idx: usize, set: std::collections::HashSet<String> },

    // --- String methods ---
    StartsWith { idx: usize, prefix: String },
    EndsWith { idx: usize, suffix: String },
    Contains { idx: usize, substring: String },

    // --- Multi-pattern contains ---
    ContainsAny { idx: usize, needles: Vec<String> },
    AhoContains {
        idx: usize,
        ac: aho_corasick::AhoCorasick,
        min: usize,
    },

    // --- I64Expr comparisons (general i64 expression) ---
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
}

impl FilterNode {
    /// Evaluate this expression against a variable slice.
    ///
    /// Single match — no vtable, no generic dispatch. The compiler
    /// can inline the entire eval path at each call site.
    #[inline(always)]
    pub fn eval(&self, vars: &[Value]) -> bool {
        match self {
            // ── Int comparisons ──
            Self::EqInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i == *val,
                _ => false,
            },
            Self::NeInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i != *val,
                _ => false,
            },
            Self::LtInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i < *val,
                _ => false,
            },
            Self::LeInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i <= *val,
                _ => false,
            },
            Self::GtInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i > *val,
                _ => false,
            },
            Self::GeInt { idx, val } => match &vars[*idx] {
                Value::Int(i) => *i >= *val,
                _ => false,
            },

            // ── Fused arithmetic + comparison ──
            Self::AddEq { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) == *cmp,
                _ => false,
            },
            Self::AddNe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) != *cmp,
                _ => false,
            },
            Self::AddLt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) < *cmp,
                _ => false,
            },
            Self::AddLe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) <= *cmp,
                _ => false,
            },
            Self::AddGt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) > *cmp,
                _ => false,
            },
            Self::AddGe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_add(*arith) >= *cmp,
                _ => false,
            },
            Self::SubEq { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) == *cmp,
                _ => false,
            },
            Self::SubNe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) != *cmp,
                _ => false,
            },
            Self::SubLt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) < *cmp,
                _ => false,
            },
            Self::SubLe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) <= *cmp,
                _ => false,
            },
            Self::SubGt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) > *cmp,
                _ => false,
            },
            Self::SubGe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_sub(*arith) >= *cmp,
                _ => false,
            },
            Self::MulEq { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) == *cmp,
                _ => false,
            },
            Self::MulNe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) != *cmp,
                _ => false,
            },
            Self::MulLt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) < *cmp,
                _ => false,
            },
            Self::MulLe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) <= *cmp,
                _ => false,
            },
            Self::MulGt { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) > *cmp,
                _ => false,
            },
            Self::MulGe { idx, arith, cmp } => match &vars[*idx] {
                Value::Int(i) => i.wrapping_mul(*arith) >= *cmp,
                _ => false,
            },

            // ── String comparison ──
            Self::EqStr { idx, val } => match &vars[*idx] {
                Value::String(s) => &**s == val,
                _ => false,
            },

            // ── Set membership: int ──
            Self::InIntLinear { idx, vals } => match &vars[*idx] {
                Value::Int(i) => vals.contains(i),
                _ => false,
            },
            Self::InIntHash { idx, set } => match &vars[*idx] {
                Value::Int(i) => set.contains(i),
                _ => false,
            },

            // ── Set membership: str ──
            Self::InStrLinear { idx, vals } => match &vars[*idx] {
                Value::String(s) => vals.iter().any(|v| s.as_ref() == v),
                _ => false,
            },
            Self::InStrHash { idx, set } => match &vars[*idx] {
                Value::String(s) => set.contains(s.as_ref()),
                _ => false,
            },

            // ── String methods ──
            Self::StartsWith { idx, prefix } => match &vars[*idx] {
                Value::String(s) => s.starts_with(prefix),
                _ => false,
            },
            Self::EndsWith { idx, suffix } => match &vars[*idx] {
                Value::String(s) => s.ends_with(suffix),
                _ => false,
            },
            Self::Contains { idx, substring } => match &vars[*idx] {
                Value::String(s) => s.contains(substring.as_str()),
                _ => false,
            },

            // ── Multi-pattern contains ──
            Self::ContainsAny { idx, needles } => match &vars[*idx] {
                Value::String(s) => {
                    let text: &str = &**s;
                    for needle in needles {
                        if text.contains(needle.as_str()) {
                            return true;
                        }
                    }
                    false
                }
                _ => false,
            },
            Self::AhoContains { idx, ac, min } => match &vars[*idx] {
                Value::String(s) => {
                    let text = s.as_bytes();
                    if *min <= 1 {
                        return ac.is_match(text);
                    }
                    let mut matched = 0u64;
                    for mat in ac.find_iter(text) {
                        let pid = mat.pattern().as_u64();
                        if pid < 64 {
                            matched |= 1u64 << pid;
                            if matched.count_ones() as usize >= *min {
                                return true;
                            }
                        }
                    }
                    false
                }
                _ => false,
            },

            // ── I64Expr comparisons ──
            Self::GeExpr { left, right } => left.eval(vars) >= right.eval(vars),
            Self::GtExpr { left, right } => left.eval(vars) > right.eval(vars),
            Self::LeExpr { left, right } => left.eval(vars) <= right.eval(vars),
            Self::LtExpr { left, right } => left.eval(vars) < right.eval(vars),
            Self::EqExpr { left, right } => left.eval(vars) == right.eval(vars),
            Self::NeExpr { left, right } => left.eval(vars) != right.eval(vars),

            // ── Logic combinators ──
            Self::And(a, b) => a.eval(vars) && b.eval(vars),
            Self::Or(a, b) => a.eval(vars) || b.eval(vars),
            Self::Not(inner) => !inner.eval(vars),
        }
    }
}

// ---------- Batch / ruleset evaluation ----------

/// Evaluates multiple filter nodes against the same variable slice.
/// Useful for "100 rules against 1 request" workloads.
pub struct FilterBatch {
    pub filters: Vec<(Box<FilterNode>, String)>, // (filter, rule_name)
}

impl FilterBatch {
    pub fn eval_all(&self, vars: &[Value]) -> Vec<(String, bool)> {
        self.filters
            .iter()
            .map(|(f, name)| (name.clone(), f.eval(vars)))
            .collect()
    }

    /// Find first matching filter (short-circuit across rules).
    pub fn eval_any(&self, vars: &[Value]) -> Option<String> {
        self.filters
            .iter()
            .find(|(f, _)| f.eval(vars))
            .map(|(_, name)| name.clone())
    }

    /// Evaluate all returning a bool slice (no allocation).
    pub fn eval_into(&self, vars: &[Value], out: &mut [bool]) {
        for (i, (f, _)) in self.filters.iter().enumerate() {
            out[i] = f.eval(vars);
        }
    }
}

/// A compiled ruleset that reorders filters by estimated cost.
/// Cheaper filters run first for `eval_any` short-circuiting.
pub struct SmartBatch {
    filters: Vec<(Box<FilterNode>, String, u8)>, // filter, name, cost
}

impl SmartBatch {
    pub fn new(filters: Vec<(Box<FilterNode>, String)>) -> Self {
        let mut with_cost: Vec<_> = filters
            .into_iter()
            .map(|(f, name)| {
                // rough cost: 1 for int eq, 2 for str eq, 5 for contains, etc.
                // We could make this richer with a cost() method.
                let cost = 1u8;
                (f, name, cost)
            })
            .collect();
        with_cost.sort_by_key(|(_, _, cost)| *cost);
        Self { filters: with_cost }
    }

    pub fn eval_any(&self, vars: &[Value]) -> Option<String> {
        self.filters
            .iter()
            .find(|(f, _, _)| f.eval(vars))
            .map(|(_, name, _)| name.clone())
    }

    pub fn eval_all(&self, vars: &[Value]) -> Vec<(String, bool)> {
        self.filters
            .iter()
            .map(|(f, name, _)| (name.clone(), f.eval(vars)))
            .collect()
    }
}
