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
            Self::Var(idx) => match vars.get(*idx) {
                Some(Value::Int(i)) => *i,
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
            Self::Var(idx) => match vars.get(*idx) {
                Some(Value::String(s)) => Some(s.as_str()),
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
            Self::Var(idx) => match vars.get(*idx) {
                Some(Value::String(s)) => s.as_str().to_string(),
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
            Self::Var(idx) => match vars.get(*idx) {
                Some(Value::String(s)) => s.len(),
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
            Self::Var(idx) => match vars.get(*idx) {
                Some(Value::List(list)) => Some(list),
                _ => None,
            },
        }
    }

    #[inline(always)]
    pub fn len(&self, vars: &[Value]) -> usize {
        self.eval(vars).map_or(0, |list| list.len())
    }
}

/// A compiled boolean filter with zero VM dispatch overhead.
/// The expression tree is turned into nested structs; Rust monomorphizes
/// `eval()` into essentially a flat function.
pub trait BoolFilter {
    fn eval(&self, vars: &[Value]) -> bool;
}

impl<T: BoolFilter + ?Sized> BoolFilter for Box<T> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.as_ref().eval(vars)
    }
}

// ---------- Leaf nodes ----------

pub struct EqIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for EqIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i == self.val,
            _ => false,
        }
    }
}

pub struct NeIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for NeIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i != self.val,
            _ => false,
        }
    }
}

pub struct LtIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for LtIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i < self.val,
            _ => false,
        }
    }
}

pub struct LeIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for LeIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i <= self.val,
            _ => false,
        }
    }
}

pub struct GtIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for GtIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i > self.val,
            _ => false,
        }
    }
}

pub struct GeIntConst {
    pub var_idx: usize,
    pub val: i64,
}
impl BoolFilter for GeIntConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => *i >= self.val,
            _ => false,
        }
    }
}

// ---------- Arithmetic + comparison fused structs ----------
// These inline the arithmetic operation into the comparison, eliminating
// I64Expr::eval() recursion overhead.  Used for patterns like `port + 100 >= 1024`.

macro_rules! define_arith_cmp {
    ($name:ident, $arith_op:ident, $cmp_op:tt) => {
        pub struct $name {
            pub var_idx: usize,
            pub arith: i64,
            pub cmp: i64,
        }
        impl BoolFilter for $name {
            #[inline(always)]
            fn eval(&self, vars: &[Value]) -> bool {
                match vars.get(self.var_idx) {
                    Some(Value::Int(i)) => i.$arith_op(self.arith) $cmp_op self.cmp,
                    _ => false,
                }
            }
        }
    };
}

define_arith_cmp!(AddConstEq, wrapping_add, ==);
define_arith_cmp!(AddConstNe, wrapping_add, !=);
define_arith_cmp!(AddConstLt, wrapping_add, <);
define_arith_cmp!(AddConstLe, wrapping_add, <=);
define_arith_cmp!(AddConstGt, wrapping_add, >);
define_arith_cmp!(AddConstGe, wrapping_add, >=);

define_arith_cmp!(SubConstEq, wrapping_sub, ==);
define_arith_cmp!(SubConstNe, wrapping_sub, !=);
define_arith_cmp!(SubConstLt, wrapping_sub, <);
define_arith_cmp!(SubConstLe, wrapping_sub, <=);
define_arith_cmp!(SubConstGt, wrapping_sub, >);
define_arith_cmp!(SubConstGe, wrapping_sub, >=);

define_arith_cmp!(MulConstEq, wrapping_mul, ==);
define_arith_cmp!(MulConstNe, wrapping_mul, !=);
define_arith_cmp!(MulConstLt, wrapping_mul, <);
define_arith_cmp!(MulConstLe, wrapping_mul, <=);
define_arith_cmp!(MulConstGt, wrapping_mul, >);
define_arith_cmp!(MulConstGe, wrapping_mul, >=);

pub struct EqStrConst {
    pub var_idx: usize,
    pub val: String,
}
impl BoolFilter for EqStrConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => s.as_str() == self.val,
            _ => false,
        }
    }
}

// Small-set membership using linear scan (faster than HashSet for N ≤ ~8)
pub struct InIntSet<const N: usize> {
    pub var_idx: usize,
    pub vals: [i64; N],
}

impl<const N: usize> BoolFilter for InIntSet<N> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => self.vals.contains(i),
            _ => false,
        }
    }
}

pub struct InStrSet<const N: usize> {
    pub var_idx: usize,
    pub vals: [String; N],
}

impl<const N: usize> BoolFilter for InStrSet<N> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => self.vals.iter().any(|v| s.as_str() == v),
            _ => false,
        }
    }
}

// Small-set membership with linear scan (faster than HashSet for N ≤ ~16)
pub struct InIntLinearSet {
    pub var_idx: usize,
    pub vals: Vec<i64>,
}

impl BoolFilter for InIntLinearSet {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => self.vals.contains(i),
            _ => false,
        }
    }
}

pub struct InStrLinearSet {
    pub var_idx: usize,
    pub vals: Vec<String>,
}

impl BoolFilter for InStrLinearSet {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => self.vals.iter().any(|v| s.as_str() == v),
            _ => false,
        }
    }
}

// For larger sets, use a HashSet
pub struct InIntHashSet {
    pub var_idx: usize,
    pub set: std::collections::HashSet<i64>,
}

impl BoolFilter for InIntHashSet {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::Int(i)) => self.set.contains(i),
            _ => false,
        }
    }
}

pub struct InStrHashSet {
    pub var_idx: usize,
    pub set: std::collections::HashSet<String>,
}

impl BoolFilter for InStrHashSet {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => self.set.contains(s.as_ref()),
            _ => false,
        }
    }
}

// ---------- String expression boolean filters ----------

pub struct StartsWithConst {
    pub var_idx: usize,
    pub prefix: String,
}
impl BoolFilter for StartsWithConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => s.starts_with(&self.prefix),
            _ => false,
        }
    }
}

pub struct EndsWithConst {
    pub var_idx: usize,
    pub suffix: String,
}
impl BoolFilter for EndsWithConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => s.ends_with(&self.suffix),
            _ => false,
        }
    }
}

pub struct ContainsConst {
    pub var_idx: usize,
    pub substring: String,
}
impl BoolFilter for ContainsConst {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => s.contains(&self.substring),
            _ => false,
        }
    }
}

/// Multi-pattern contains using naive search (str::contains in a loop).
/// Faster than Aho-Corasick for very small pattern counts (≤4) on short strings.
pub struct ContainsAny {
    pub var_idx: usize,
    pub needles: Vec<String>,
}

impl BoolFilter for ContainsAny {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => {
                let text = s.as_str();
                // Unroll-like: the compiler usually unrolls this for small Vecs
                for needle in &self.needles {
                    if text.contains(needle.as_str()) {
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

/// Multi-pattern contains using Aho-Corasick.
/// Scans the input string once for all patterns simultaneously.
pub struct AhoCorasickContains {
    pub var_idx: usize,
    pub automaton: aho_corasick::AhoCorasick,
    pub min_matches: usize, // 1 for OR semantics, N for AND of N distinct patterns
}

impl BoolFilter for AhoCorasickContains {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        match vars.get(self.var_idx) {
            Some(Value::String(s)) => {
                let text = s.as_bytes();
                // Fast path: OR semantics (any single match is enough).
                // is_match() is measurably faster than find_iter() for simple yes/no.
                if self.min_matches <= 1 {
                    return self.automaton.is_match(text);
                }
                // AND semantics: need N distinct patterns to match.
                // Track which patterns matched using a small stack bitset.
                // Supports up to 64 patterns.
                let mut matched = 0u64;
                for mat in self.automaton.find_iter(text) {
                    let pid = mat.pattern().as_u64();
                    if pid < 64 {
                        matched |= 1u64 << pid;
                        if matched.count_ones() as usize >= self.min_matches {
                            return true;
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }
}

// Range check using I64Expr on both sides
pub struct GeI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for GeI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) >= self.right.eval(vars)
    }
}

pub struct GtI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for GtI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) > self.right.eval(vars)
    }
}

pub struct LeI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for LeI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) <= self.right.eval(vars)
    }
}

pub struct LtI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for LtI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) < self.right.eval(vars)
    }
}

pub struct EqI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for EqI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) == self.right.eval(vars)
    }
}

pub struct NeI64Expr {
    pub left: I64Expr,
    pub right: I64Expr,
}
impl BoolFilter for NeI64Expr {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.left.eval(vars) != self.right.eval(vars)
    }
}

// ---------- Batch / ruleset evaluation ----------

/// Evaluates multiple boolean filters against the same variable slice.
/// Useful for "100 rules against 1 request" workloads.
pub struct FilterBatch {
    pub filters: Vec<(Box<dyn BoolFilter>, String)>, // (filter, rule_name)
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
    filters: Vec<(Box<dyn BoolFilter>, String, u8)>, // filter, name, cost
}

impl SmartBatch {
    pub fn new(filters: Vec<(Box<dyn BoolFilter>, String)>) -> Self {
        let mut with_cost: Vec<_> = filters
            .into_iter()
            .map(|(f, name)| {
                // rough cost: 1 for int eq, 2 for str eq, 5 for contains, etc.
                // We could make this richer with a cost() trait method.
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

// ---------- Logic combinators ----------

pub struct And<A, B> {
    pub a: A,
    pub b: B,
}
impl<A: BoolFilter, B: BoolFilter> BoolFilter for And<A, B> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.a.eval(vars) && self.b.eval(vars)
    }
}

pub struct Or<A, B> {
    pub a: A,
    pub b: B,
}
impl<A: BoolFilter, B: BoolFilter> BoolFilter for Or<A, B> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        self.a.eval(vars) || self.b.eval(vars)
    }
}

pub struct Not<A> {
    pub inner: A,
}
impl<A: BoolFilter> BoolFilter for Not<A> {
    #[inline(always)]
    fn eval(&self, vars: &[Value]) -> bool {
        !self.inner.eval(vars)
    }
}
