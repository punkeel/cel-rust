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

    /// Fast eval — no bounds checks or type checks.
    ///
    /// # Safety
    ///
    /// Same safety guarantees as [`FilterNode::eval_fast`].
    #[inline(always)]
    pub unsafe fn eval_fast(&self, vars: &[Value]) -> i64 {
        match self {
            Self::Literal(v) => *v,
            Self::Var(idx) => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::Add(a, b) => a.eval_fast(vars).wrapping_add(b.eval_fast(vars)),
            Self::Sub(a, b) => a.eval_fast(vars).wrapping_sub(b.eval_fast(vars)),
            Self::Mul(a, b) => a.eval_fast(vars).wrapping_mul(b.eval_fast(vars)),
            Self::Div(a, b) => {
                let bv = b.eval_fast(vars);
                if bv == 0 { 0 } else { a.eval_fast(vars).wrapping_div(bv) }
            }
            Self::Mod(a, b) => {
                let bv = b.eval_fast(vars);
                if bv == 0 { 0 } else { a.eval_fast(vars).wrapping_rem(bv) }
            }
            Self::Neg(a) => a.eval_fast(vars).wrapping_neg(),
            Self::StrLen(a) => a.len_unchecked(vars) as i64,
            Self::ListLen(a) => a.len_unchecked(vars) as i64,
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

    /// Fast length — no bounds checks or type checks.
    ///
    /// # Safety
    ///
    /// Same safety guarantees as [`FilterNode::eval_fast`].
    #[inline(always)]
    pub unsafe fn len_unchecked(&self, vars: &[Value]) -> usize {
        match self {
            Self::Literal(s) => s.len(),
            Self::Var(idx) => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => s.len(),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::Concat(a, b) => a.len_unchecked(vars) + b.len_unchecked(vars),
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

    /// Fast length — no bounds checks or type checks.
    ///
    /// # Safety
    ///
    /// Same safety guarantees as [`FilterNode::eval_fast`].
    #[inline(always)]
    pub unsafe fn len_unchecked(&self, vars: &[Value]) -> usize {
        match self {
            Self::Var(idx) => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::List(list) => list.len(),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
        }
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
    /// String != comparison.
    NeStr { idx: usize, val: String },

    // --- Boolean field predicates ---
    /// Bare boolean variable used as condition (e.g. `flag && ...`).
    BoolVar { idx: usize },

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
    /// Pre-compiled regex match. Regex is compiled at Filter::compile time.
    Matches { idx: usize, regex: regex::Regex },

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

    // --- Comprehension: exists() ---
    /// `list.exists(x, predicate)`. Iterates a list field and evaluates
    /// the predicate against each element placed at `item_idx`.
    /// Covers all 5 remaining corpus rules (rule_000/006/014/016/022).
    Exists {
        list_idx: usize,
        item_idx: usize,
        predicate: Box<FilterNode>,
    },

    // --- Map-key contains (map["key"].exists(x, x == "val")) ---
    /// Optimized path for `map["key"].exists(x, x == "value")`.
    /// No allocation, no iteration — just a HashMap lookup + comparison.
    /// Handles both single-string and list-of-strings values.
    MapKeyContains {
        map_idx: usize,
        key: String,
        needle: String,
    },

    // --- List membership check: list.exists(x, x in [vals]) ---
    /// Optimized path for `list.exists(x, x in [int1, int2, ...])`.
    /// No allocation, no scratch vector, no item clone — scans the list
    /// directly and compares each element against the embedded int set.
    /// Eliminates the per-eval `vars.to_vec()` + `item.clone()` overhead
    /// of the generic `Exists` node.
    ExistsInIntSet {
        list_idx: usize,
        vals: Vec<i64>,
    },

    // --- List membership check: list.exists(x, x == val) ---
    /// Same as ExistsInIntSet but for a single literal comparison.
    ExistsEqInt {
        list_idx: usize,
        val: i64,
    },
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
            Self::NeStr { idx, val } => match &vars[*idx] {
                Value::String(s) => &**s != val,
                _ => false,
            },

            // ── Boolean field predicates ──
            Self::BoolVar { idx } => match &vars[*idx] {
                Value::Bool(b) => *b,
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
            Self::Matches { idx, regex } => match &vars[*idx] {
                Value::String(s) => regex.is_match(s),
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

            // ── Comprehension: exists() ──
            Self::Exists { list_idx, item_idx, predicate } => {
                match &vars[*list_idx] {
                    Value::List(list) => {
                        let mut extended: Vec<Value> = vars.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[*item_idx] = item.clone();
                            if predicate.eval(&extended) {
                                return true;
                            }
                        }
                        false
                    }
                    _ => false,
                }
            }

            // ── Map-key contains (single lookup, no alloc) ──
            Self::MapKeyContains { map_idx, key, needle } => {
                match &vars[*map_idx] {
                    Value::Map(m) => {
                        let k = crate::objects::Key::String(std::sync::Arc::from(key.as_str()));
                        match m.map.get(&k) {
                            Some(Value::String(s)) => s.as_ref() == needle.as_str(),
                            Some(Value::List(list)) => {
                                list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_str()))
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }

            // ── Specialized exists-in-int-set (no alloc, no scratch vec) ──
            Self::ExistsInIntSet { list_idx, vals } => match &vars[*list_idx] {
                Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                _ => false,
            },
            // ── Specialized exists-eq-int (no alloc, no scratch vec) ──
            Self::ExistsEqInt { list_idx, val } => match &vars[*list_idx] {
                Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == *val)),
                _ => false,
            },
        }
    }

    /// Evaluate without bounds checks or type checks.
    ///
    /// # Safety
    ///
    /// Caller must guarantee:
    /// - Every `idx` in this tree is `< vars.len()`
    /// - Every field access matches the expected `Value` variant
    ///   (e.g. `EqInt` only encounters `Value::Int` at its index)
    ///
    /// These are guaranteed when the tree was compiled against a [`Schema`](crate::fast::Schema)
    /// and all fields have been set to the correct types.
    #[inline(always)]
    pub unsafe fn eval_fast(&self, vars: &[Value]) -> bool {
        match self {
            // ── Int comparisons ──
            Self::EqInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i == *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::NeInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i != *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::LtInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i < *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::LeInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i <= *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::GtInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i > *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::GeInt { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => *i >= *val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── Fused arithmetic + comparison ──
            Self::AddEq { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) == *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AddNe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) != *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AddLt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) < *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AddLe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) <= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AddGt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) > *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AddGe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_add(*arith) >= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubEq { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) == *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubNe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) != *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubLt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) < *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubLe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) <= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubGt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) > *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::SubGe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_sub(*arith) >= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulEq { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) == *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulNe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) != *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulLt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) < *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulLe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) <= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulGt { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) > *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MulGe { idx, arith, cmp } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => i.wrapping_mul(*arith) >= *cmp,
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── String comparison ──
            Self::EqStr { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => &**s == val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::NeStr { idx, val } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => &**s != val,
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── Boolean field predicates ──
            Self::BoolVar { idx } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Bool(b) => *b,
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── Set membership: int ──
            Self::InIntLinear { idx, vals } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => vals.contains(i),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::InIntHash { idx, set } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::Int(i) => set.contains(i),
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── Set membership: str ──
            Self::InStrLinear { idx, vals } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => vals.iter().any(|v| s.as_ref() == v),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::InStrHash { idx, set } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => set.contains(s.as_ref()),
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── String methods ──
            Self::StartsWith { idx, prefix } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => s.starts_with(prefix.as_str()),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::EndsWith { idx, suffix } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => s.ends_with(suffix.as_str()),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::Contains { idx, substring } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => s.contains(substring.as_str()),
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::Matches { idx, regex } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => regex.is_match(s),
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── Multi-pattern contains ──
            Self::ContainsAny { idx, needles } => {
                let v = vars.get_unchecked(*idx);
                match v {
                    Value::String(s) => {
                        let text: &str = &**s;
                        for needle in needles {
                            if text.contains(needle.as_str()) {
                                return true;
                            }
                        }
                        false
                    }
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::AhoContains { idx, ac, min } => {
                let v = vars.get_unchecked(*idx);
                match v {
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
                    _ => std::hint::unreachable_unchecked(),
                }
            }

            // ── I64Expr comparisons ──
            Self::GeExpr { left, right } => left.eval_fast(vars) >= right.eval_fast(vars),
            Self::GtExpr { left, right } => left.eval_fast(vars) > right.eval_fast(vars),
            Self::LeExpr { left, right } => left.eval_fast(vars) <= right.eval_fast(vars),
            Self::LtExpr { left, right } => left.eval_fast(vars) < right.eval_fast(vars),
            Self::EqExpr { left, right } => left.eval_fast(vars) == right.eval_fast(vars),
            Self::NeExpr { left, right } => left.eval_fast(vars) != right.eval_fast(vars),

            // ── Logic combinators (recursively call eval_fast) ──
            Self::And(a, b) => a.eval_fast(vars) && b.eval_fast(vars),
            Self::Or(a, b) => a.eval_fast(vars) || b.eval_fast(vars),
            Self::Not(inner) => !inner.eval_fast(vars),

            // ── Comprehension: exists() (extend vars, unsafe indexing) ──
            Self::Exists { list_idx, item_idx, predicate } => {
                let v = vars.get_unchecked(*list_idx);
                match v {
                    Value::List(list) => {
                        let mut extended: Vec<Value> = vars.to_vec();
                        extended.push(Value::Null);
                        for item in list.iter() {
                            extended[*item_idx] = item.clone();
                            if predicate.eval_fast(&extended) {
                                return true;
                            }
                        }
                        false
                    }
                    _ => false,
                }
            }

            // ── Map-key contains (single lookup, no alloc) ──
            Self::MapKeyContains { map_idx, key, needle } => {
                let v = vars.get_unchecked(*map_idx);
                match v {
                    Value::Map(m) => {
                        let k = crate::objects::Key::String(std::sync::Arc::from(key.as_str()));
                        match m.map.get(&k) {
                            Some(Value::String(s)) => s.as_ref() == needle.as_str(),
                            Some(Value::List(list)) => {
                                list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_str()))
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }

            // ── Specialized exists-in-int-set (no alloc, no scratch vec) ──
            Self::ExistsInIntSet { list_idx, vals } => {
                let v = vars.get_unchecked(*list_idx);
                match v {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                    _ => false,
                }
            }
            // ── Specialized exists-eq-int (no alloc, no scratch vec) ──
            Self::ExistsEqInt { list_idx, val } => {
                let v = vars.get_unchecked(*list_idx);
                match v {
                    Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == *val)),
                    _ => false,
                }
            }
        }
    }

    /// Evaluate using pre-extracted typed arrays — no Value enum access.
    ///
    ///
    /// Same as [`eval_fast`], plus: the typed arrays must have been populated
    /// by [`EvalContext`](crate::fast::EvalContext) (which the fast path does).
    #[inline(always)]
    pub unsafe fn eval_fast_typed(
        &self,
        ints: &[i64],
        strings: &[std::sync::Arc<str>],
    ) -> bool {
        match self {
            // ── Int comparisons (direct i64 access) ──
            Self::EqInt { idx, val } => *ints.get_unchecked(*idx) == *val,
            Self::NeInt { idx, val } => *ints.get_unchecked(*idx) != *val,
            Self::LtInt { idx, val } => *ints.get_unchecked(*idx) < *val,
            Self::LeInt { idx, val } => *ints.get_unchecked(*idx) <= *val,
            Self::GtInt { idx, val } => *ints.get_unchecked(*idx) > *val,
            Self::GeInt { idx, val } => *ints.get_unchecked(*idx) >= *val,

            // ── Fused arithmetic + comparison (direct i64 access) ──
            Self::AddEq { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) == *cmp
            }
            Self::AddNe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) != *cmp
            }
            Self::AddLt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) < *cmp
            }
            Self::AddLe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) <= *cmp
            }
            Self::AddGt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) > *cmp
            }
            Self::AddGe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_add(*arith) >= *cmp
            }
            Self::SubEq { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) == *cmp
            }
            Self::SubNe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) != *cmp
            }
            Self::SubLt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) < *cmp
            }
            Self::SubLe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) <= *cmp
            }
            Self::SubGt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) > *cmp
            }
            Self::SubGe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_sub(*arith) >= *cmp
            }
            Self::MulEq { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) == *cmp
            }
            Self::MulNe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) != *cmp
            }
            Self::MulLt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) < *cmp
            }
            Self::MulLe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) <= *cmp
            }
            Self::MulGt { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) > *cmp
            }
            Self::MulGe { idx, arith, cmp } => {
                ints.get_unchecked(*idx).wrapping_mul(*arith) >= *cmp
            }

            // ── String comparison (direct Arc<str> access) ──
            Self::EqStr { idx, val } => strings.get_unchecked(*idx).as_ref() == val.as_str(),
            Self::NeStr { idx, val } => strings.get_unchecked(*idx).as_ref() != val.as_str(),

            // ── Boolean field predicates (fall back to vars) ──
            Self::BoolVar { .. } => core::hint::unreachable_unchecked(),

            // ── Set membership: int ──
            Self::InIntLinear { idx, vals } => vals.contains(ints.get_unchecked(*idx)),
            Self::InIntHash { idx, set } => set.contains(ints.get_unchecked(*idx)),

            // ── Set membership: str ──
            Self::InStrLinear { idx, vals } => {
                let s: &str = strings.get_unchecked(*idx).as_ref();
                vals.iter().any(|v| s == v)
            }
            Self::InStrHash { idx, set } => {
                let s: &str = strings.get_unchecked(*idx).as_ref();
                set.contains(s)
            }

            // ── String methods ──
            Self::StartsWith { idx, prefix } => {
                strings.get_unchecked(*idx).starts_with(prefix.as_str())
            }
            Self::EndsWith { idx, suffix } => {
                strings.get_unchecked(*idx).ends_with(suffix.as_str())
            }
            Self::Contains { idx, substring } => {
                strings.get_unchecked(*idx).contains(substring.as_str())
            }
            Self::Matches { idx, regex } => {
                regex.is_match(strings.get_unchecked(*idx).as_ref())
            }

            // ── Multi-pattern contains ──
            Self::ContainsAny { idx, needles } => {
                let text: &str = strings.get_unchecked(*idx).as_ref();
                for needle in needles {
                    if text.contains(needle.as_str()) {
                        return true;
                    }
                }
                false
            }
            Self::AhoContains { idx, ac, min } => {
                let text = strings.get_unchecked(*idx).as_bytes();
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

            // ── I64Expr / other — fall back to vars path ──
            Self::GeExpr { .. }
            | Self::GtExpr { .. }
            | Self::LeExpr { .. }
            | Self::LtExpr { .. }
            | Self::EqExpr { .. }
            | Self::NeExpr { .. } => {
                // These use I64Expr which needs the full Value enum
                // This path is rarely hit for Schema-compiled expressions
                core::hint::unreachable_unchecked()
            }

            // ── Logic combinators (recursively call eval_fast_typed) ──
            Self::And(a, b) => a.eval_fast_typed(ints, strings) && b.eval_fast_typed(ints, strings),
            Self::Or(a, b) => a.eval_fast_typed(ints, strings) || b.eval_fast_typed(ints, strings),
            Self::Not(inner) => !inner.eval_fast_typed(ints, strings),

            // ── Comprehension: exists() (can't extend typed arrays) ──
            Self::Exists { .. } => core::hint::unreachable_unchecked(),
            Self::MapKeyContains { .. } => core::hint::unreachable_unchecked(),
            Self::ExistsInIntSet { .. } => core::hint::unreachable_unchecked(),
            Self::ExistsEqInt { .. } => core::hint::unreachable_unchecked(),
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
