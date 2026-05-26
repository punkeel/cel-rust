use crate::objects::Value;

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
