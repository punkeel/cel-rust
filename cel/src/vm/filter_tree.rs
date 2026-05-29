use crate::objects::Value;

/// A pre-resolved function from the FnTable.
/// Wraps the raw function pointer with Debug+Clone so it can be stored in FilterNode.
#[derive(Clone)]
pub struct FnCallPtr(pub std::sync::Arc<dyn Fn(&[Value]) -> Result<Value, crate::ExecutionError> + Send + Sync>);

impl std::fmt::Debug for FnCallPtr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FnCallPtr").finish()
    }
}

impl FnCallPtr {
    /// Call the function with the given argument slice.
    pub fn call(&self, args: &[Value]) -> Result<Value, crate::ExecutionError> {
        (self.0)(args)
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

    /// Fast eval using typed arrays — no Value enum access.
    ///
    /// # Safety
    ///
    /// Same as [`eval_fast`], but reads from pre-extracted typed arrays.
    #[inline(always)]
    pub unsafe fn eval_fast_typed(&self, ints: &[i64], strings: &[std::sync::Arc<str>]) -> i64 {
        match self {
            Self::Literal(v) => *v,
            Self::Var(idx) => *ints.get_unchecked(*idx),
            Self::Add(a, b) => a.eval_fast_typed(ints, strings).wrapping_add(b.eval_fast_typed(ints, strings)),
            Self::Sub(a, b) => a.eval_fast_typed(ints, strings).wrapping_sub(b.eval_fast_typed(ints, strings)),
            Self::Mul(a, b) => a.eval_fast_typed(ints, strings).wrapping_mul(b.eval_fast_typed(ints, strings)),
            Self::Div(a, b) => {
                let bv = b.eval_fast_typed(ints, strings);
                if bv == 0 { 0 } else { a.eval_fast_typed(ints, strings).wrapping_div(bv) }
            }
            Self::Mod(a, b) => {
                let bv = b.eval_fast_typed(ints, strings);
                if bv == 0 { 0 } else { a.eval_fast_typed(ints, strings).wrapping_rem(bv) }
            }
            Self::Neg(a) => a.eval_fast_typed(ints, strings).wrapping_neg(),
            Self::StrLen(s) => s.len_typed(strings) as i64,
            Self::ListLen(_) => 0, // List length not available from typed arrays
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

    /// Fast length from typed string array — no Value enum access.
    ///
    /// # Safety
    ///
    /// Same as [`len_unchecked`], but reads from pre-extracted `Arc<str>` array.
    #[inline(always)]
    pub unsafe fn len_typed(&self, strings: &[std::sync::Arc<str>]) -> usize {
        match self {
            Self::Literal(s) => s.len(),
            Self::Var(idx) => strings.get_unchecked(*idx).len(),
            Self::Concat(a, b) => a.len_typed(strings) + b.len_typed(strings),
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

/// Integer comparison operator for `exists` and similar nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IntCmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl IntCmp {
    #[inline(always)]
    pub fn eval(&self, a: i64, b: i64) -> bool {
        match self {
            IntCmp::Eq => a == b,
            IntCmp::Ne => a != b,
            IntCmp::Lt => a < b,
            IntCmp::Le => a <= b,
            IntCmp::Gt => a > b,
            IntCmp::Ge => a >= b,
        }
    }
}

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
    NeStr { idx: usize, val: String },

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

    // --- Boolean variable / literal ---
    /// Read a variable as a boolean value (`is_admin`, `flag_enabled`).
    BoolVar { idx: usize },
    /// A boolean literal — `true` or `false`.
    BoolLiteral { val: bool },

    // --- Comprehension / exists ---
    /// `list.exists(x, x > 5)` — iterate int list, check int condition.
    ExistsIntList {
        /// Index of the collection (list of ints) variable.
        collection_idx: usize,
        /// Each element is compared against this value.
        cmp_val: i64,
        /// Comparison operator.
        cmp: IntCmp,
    },
    /// `list.exists(x, x == "val")` — iterate string list, check string equality.
    ExistsStrEq {
        /// Index of the collection (list of strings) variable.
        collection_idx: usize,
        /// String value to compare each element against.
        cmp_val: String,
    },
    /// `list.exists(x, x in [1, 2, 3])` — int-list membership via exists.
    ExistsIntSet {
        /// Index of the collection (list of ints) variable.
        collection_idx: usize,
        /// Integer set to check membership against.
        vals: Vec<i64>,
    },
    /// `map.exists(k, v, v > 5)` — iterate map, check int value condition.
    ExistsMapInt {
        /// Index of the collection (map) variable.
        collection_idx: usize,
        /// Each value is compared against this value.
        cmp_val: i64,
        /// Comparison operator for values.
        cmp: IntCmp,
    },

    // --- Map-indexed exists ---
    /// `map["key"].exists(it, it == "str")` — index map by key, iterate result, string equality.
    MapIndexStrEq {
        /// Index of the map variable.
        map_idx: usize,
        /// The key to look up (e.g. "via").
        key: String,
        /// String value to compare each element against.
        cmp_val: String,
    },
    /// `map["key"].exists(it, it > val)` — index map by key, iterate result, int comparison.
    MapIndexIntList {
        /// Index of the map variable.
        map_idx: usize,
        /// The key to look up (e.g. "port").
        key: String,
        /// Each element is compared against this value.
        cmp_val: i64,
        /// Comparison operator.
        cmp: IntCmp,
    },

    // --- Pre-resolved function call result ---
    /// `func_name(arg_idxs...) cmp_op literal` — pre-resolved function from FnTable,
    /// called at eval time, result compared against a literal.
    FnCmpResult {
        /// Pre-resolved function pointer.
        func: FnCallPtr,
        /// Indices of argument variables.
        arg_idxs: Vec<usize>,
        /// Comparison operator.
        cmp: IntCmp,
        /// Literal to compare against.
        literal: i64,
    },

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
            Self::NeStr { idx, val } => match &vars[*idx] {
                Value::String(s) => &**s != val,
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

            // ── Boolean variable / literal ──
            Self::BoolLiteral { val } => *val,
            Self::BoolVar { idx } => match &vars[*idx] {
                Value::Bool(b) => *b,
                _ => false,
            },

            // ── Exists / comprehension ──
            Self::ExistsIntList { collection_idx, cmp_val, cmp } => {
                match &vars[*collection_idx] {
                    Value::List(list) => {
                        list.iter().any(|item| match item {
                            Value::Int(i) => cmp.eval(*i, *cmp_val),
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            Self::ExistsStrEq { collection_idx, cmp_val } => {
                match &vars[*collection_idx] {
                    Value::List(list) => {
                        list.iter().any(|item| match item {
                            Value::String(s) => s.as_ref() == cmp_val.as_str(),
                            _ => false,
                        })
                    }
                    _ => false,
                }
            }
            Self::ExistsIntSet { collection_idx, vals } => match &vars[*collection_idx] {
                Value::List(list) => list.iter().any(|item| match item {
                    Value::Int(i) => vals.contains(i),
                    _ => false,
                }),
                _ => false,
            },
            Self::ExistsMapInt { collection_idx, cmp_val, cmp } => match &vars[*collection_idx] {
                Value::Map(map) => map.map.values().any(|v| match v {
                    Value::Int(i) => cmp.eval(*i, *cmp_val),
                    _ => false,
                }),
                _ => false,
            },

            // ── Map-indexed exists ──
            Self::MapIndexStrEq { map_idx, key, cmp_val } => {
                match &vars[*map_idx] {
                    Value::Map(map) => {
                        let key_ref: crate::objects::Key = key.as_str().into();
                        match map.map.get(&key_ref) {
                            Some(Value::String(s)) => s.as_ref() == cmp_val.as_str(),
                            Some(Value::List(list)) => list.iter().any(|item| match item {
                                Value::String(s) => s.as_ref() == cmp_val.as_str(),
                                _ => false,
                            }),
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
            Self::MapIndexIntList { map_idx, key, cmp_val, cmp } => {
                match &vars[*map_idx] {
                    Value::Map(map) => {
                        let key_ref: crate::objects::Key = key.as_str().into();
                        match map.map.get(&key_ref) {
                            Some(Value::Int(i)) => cmp.eval(*i, *cmp_val),
                            Some(Value::List(list)) => list.iter().any(|item| match item {
                                Value::Int(i) => cmp.eval(*i, *cmp_val),
                                _ => false,
                            }),
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
            // ── Pre-resolved function call result ──
            Self::FnCmpResult { func, arg_idxs, cmp, literal } => {
                let mut args = [Value::Null, Value::Null, Value::Null];
                for (i, &idx) in arg_idxs.iter().enumerate() {
                    if i >= 3 { break; }
                    args[i] = vars[idx].clone();
                }
                match func.call(&args[..arg_idxs.len()]) {
                    Ok(Value::Int(i)) => cmp.eval(i, *literal),
                    _ => false,
                }
            }
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

            // ── Boolean variable / literal ──
            Self::BoolLiteral { val } => *val,
            Self::BoolVar { idx } => match vars.get_unchecked(*idx) {
                Value::Bool(b) => *b,
                _ => std::hint::unreachable_unchecked(),
            },

            // ── Exists / comprehension ──
            Self::ExistsIntList { collection_idx, cmp_val, cmp } => {
                match vars.get_unchecked(*collection_idx) {
                    Value::List(list) => {
                        list.iter().any(|item| match item {
                            Value::Int(i) => cmp.eval(*i, *cmp_val),
                            _ => false,
                        })
                    }
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::ExistsStrEq { collection_idx, cmp_val } => {
                match vars.get_unchecked(*collection_idx) {
                    Value::List(list) => {
                        list.iter().any(|item| match item {
                            Value::String(s) => s.as_ref() == cmp_val.as_str(),
                            _ => false,
                        })
                    }
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::ExistsIntSet { collection_idx, vals } => match vars.get_unchecked(*collection_idx) {
                Value::List(list) => list.iter().any(|item| match item {
                    Value::Int(i) => vals.contains(i),
                    _ => false,
                }),
                _ => std::hint::unreachable_unchecked(),
            },
            Self::ExistsMapInt { collection_idx, cmp_val, cmp } => match vars.get_unchecked(*collection_idx) {
                Value::Map(map) => map.map.values().any(|v| match v {
                    Value::Int(i) => cmp.eval(*i, *cmp_val),
                    _ => false,
                }),
                _ => std::hint::unreachable_unchecked(),
            },

            // ── Map-indexed exists (eval_fast) ──
            Self::MapIndexStrEq { map_idx, key, cmp_val } => {
                match vars.get_unchecked(*map_idx) {
                    Value::Map(map) => {
                        let key_ref: crate::objects::Key = key.as_str().into();
                        match map.map.get(&key_ref) {
                            Some(Value::String(s)) => s.as_ref() == cmp_val.as_str(),
                            Some(Value::List(list)) => list.iter().any(|item| match item {
                                Value::String(s) => s.as_ref() == cmp_val.as_str(),
                                _ => false,
                            }),
                            _ => std::hint::unreachable_unchecked(),
                        }
                    }
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            Self::MapIndexIntList { map_idx, key, cmp_val, cmp } => {
                match vars.get_unchecked(*map_idx) {
                    Value::Map(map) => {
                        let key_ref: crate::objects::Key = key.as_str().into();
                        match map.map.get(&key_ref) {
                            Some(Value::Int(i)) => cmp.eval(*i, *cmp_val),
                            Some(Value::List(list)) => list.iter().any(|item| match item {
                                Value::Int(i) => cmp.eval(*i, *cmp_val),
                                _ => false,
                            }),
                            _ => std::hint::unreachable_unchecked(),
                        }
                    }
                    _ => std::hint::unreachable_unchecked(),
                }
            }
            // ── Pre-resolved function call result ──
            Self::FnCmpResult { func, arg_idxs, cmp, literal } => {
                let mut args = [Value::Null, Value::Null, Value::Null];
                for (i, &idx) in arg_idxs.iter().enumerate() {
                    if i >= 3 { break; }
                    args[i] = vars.get_unchecked(idx).clone();
                }
                match func.call(&args[..arg_idxs.len()]) {
                    Ok(Value::Int(i)) => cmp.eval(i, *literal),
                    _ => false,
                }
            }
        }
    }

    /// Evaluate using pre-extracted typed arrays — no Value enum access.
    ///
    /// # Safety
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

            // ── I64Expr comparisons (using typed eval) ──
            Self::GeExpr { left, right } => left.eval_fast_typed(ints, strings) >= right.eval_fast_typed(ints, strings),
            Self::GtExpr { left, right } => left.eval_fast_typed(ints, strings) > right.eval_fast_typed(ints, strings),
            Self::LeExpr { left, right } => left.eval_fast_typed(ints, strings) <= right.eval_fast_typed(ints, strings),
            Self::LtExpr { left, right } => left.eval_fast_typed(ints, strings) < right.eval_fast_typed(ints, strings),
            Self::EqExpr { left, right } => left.eval_fast_typed(ints, strings) == right.eval_fast_typed(ints, strings),
            Self::NeExpr { left, right } => left.eval_fast_typed(ints, strings) != right.eval_fast_typed(ints, strings),

            // ── Logic combinators (recursively call eval_fast_typed) ──
            Self::And(a, b) => a.eval_fast_typed(ints, strings) && b.eval_fast_typed(ints, strings),
            Self::Or(a, b) => a.eval_fast_typed(ints, strings) || b.eval_fast_typed(ints, strings),
            Self::Not(inner) => !inner.eval_fast_typed(ints, strings),

            // ── Boolean variable / literal (encoded as i64 0/1 in ints array) ──
            Self::BoolLiteral { val } => *val,
            Self::BoolVar { idx } => *ints.get_unchecked(*idx) != 0,

            // ── Exists / comprehension (requires Value array — not callable from typed path) ──
            Self::ExistsIntList { .. } | Self::ExistsStrEq { .. }
            | Self::ExistsIntSet { .. } | Self::ExistsMapInt { .. }
            | Self::MapIndexStrEq { .. } | Self::MapIndexIntList { .. }
            | Self::FnCmpResult { .. } => {
                std::hint::unreachable_unchecked()
            }
        }
    }

    /// Estimate the relative evaluation cost of this node.
    /// Higher = more expensive. Used to reorder AND/OR for optimal short-circuit.
    pub fn cost(&self) -> u8 {
        match self {
            // Tier 1: dirt cheap (int register read + compare)
            Self::EqInt { .. } | Self::NeInt { .. } => 1,
            Self::LtInt { .. } | Self::LeInt { .. } => 1,
            Self::GtInt { .. } | Self::GeInt { .. } => 1,
            Self::AddEq { .. } | Self::AddNe { .. } => 1,
            Self::AddLt { .. } | Self::AddLe { .. } => 1,
            Self::AddGt { .. } | Self::AddGe { .. } => 1,
            Self::SubEq { .. } | Self::SubNe { .. } => 1,
            Self::SubLt { .. } | Self::SubLe { .. } => 1,
            Self::SubGt { .. } | Self::SubGe { .. } => 1,
            Self::MulEq { .. } | Self::MulNe { .. } => 1,
            Self::MulLt { .. } | Self::MulLe { .. } => 1,
            Self::MulGt { .. } | Self::MulGe { .. } => 1,
            // I64Expr comparisons — still cheap, slight arithmetic overhead
            Self::GeExpr { .. } | Self::GtExpr { .. } => 2,
            Self::LeExpr { .. } | Self::LtExpr { .. } => 2,
            Self::EqExpr { .. } | Self::NeExpr { .. } => 2,

            // Tier 2: cheap (string compare, small set scan)
            Self::EqStr { .. } | Self::NeStr { .. } => 5,
            Self::InIntLinear { .. } => 8,
            Self::InIntHash { .. } => 4,
            Self::InStrLinear { .. } => 15,
            Self::InStrHash { .. } => 8,

            // Tier 3: medium (string prefix/suffix scan)
            Self::StartsWith { .. } | Self::EndsWith { .. } => 8,
            Self::Contains { .. } => 15,

            // Tier 4: expensive
            Self::Matches { .. } => 50,
            Self::ContainsAny { .. } => 35,
            Self::AhoContains { .. } => 40,

            // Boolean variable / literal — cheap (register read + int-to-bool)
            Self::BoolVar { .. } | Self::BoolLiteral { .. } => 1,

            // Exists / comprehension — O(N) iteration
            Self::ExistsIntList { .. } | Self::ExistsStrEq { .. }
            | Self::ExistsIntSet { .. } | Self::ExistsMapInt { .. }
            | Self::MapIndexStrEq { .. } | Self::MapIndexIntList { .. } => 100,
            | Self::FnCmpResult { .. } => 80,

            // Recursive: sum of children
            Self::And(a, b) | Self::Or(a, b) => a.cost().saturating_add(b.cost()),
            Self::Not(inner) => inner.cost(),
        }
    }

    /// Whether this node requires Value array access (map lookups, list iteration).
    /// When true, the typed i64/Arc<str> path cannot be used for this node.
    pub fn needs_values(&self) -> bool {
        match self {
            Self::ExistsIntList { .. } | Self::ExistsStrEq { .. }
            | Self::ExistsIntSet { .. } | Self::ExistsMapInt { .. }
            | Self::MapIndexStrEq { .. } | Self::MapIndexIntList { .. }
            | Self::FnCmpResult { .. } => true,
            Self::And(a, b) | Self::Or(a, b) => a.needs_values() || b.needs_values(),
            Self::Not(inner) => inner.needs_values(),
            _ => false,
        }
    }

    /// Reorder AND/OR branches so cheaper expression evaluates first.
    /// Maximizes short-circuit benefit: AND skips right if left is false,
    /// OR skips right if left is true — so the cheap check should go first.
    pub fn optimize_order(&mut self) {
        match self {
            Self::And(ref mut a, ref mut b) | Self::Or(ref mut a, ref mut b) => {
                // Recurse first
                a.optimize_order();
                b.optimize_order();
                // Then swap if right is cheaper than left
                if a.cost() > b.cost() {
                    std::mem::swap(a, b);
                }
            }
            Self::Not(ref mut inner) => {
                inner.optimize_order();
            }
            _ => {}
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
                let cost = f.cost();
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
