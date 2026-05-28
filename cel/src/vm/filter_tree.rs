use crate::objects::{Key, Value};
use std::sync::Arc;

// ── Core types ──

#[derive(Clone, Copy)]
pub struct EvalView<'a> {
    pub ints: &'a [i64],
    pub strings: &'a [Arc<str>],
    pub values: &'a [Value],
}

#[derive(Clone)]
pub struct ItemPredicate(Arc<dyn Fn(&Value) -> bool>);
impl std::fmt::Debug for ItemPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct("ItemPredicate").finish() }
}
impl ItemPredicate {
    pub fn new(f: Box<dyn Fn(&Value) -> bool>) -> Self { Self(Arc::from(f)) }
    pub fn call(&self, v: &Value) -> bool { (self.0)(v) }
}

// ── Sub-expr types ──

#[derive(Clone, Debug)]
pub enum StrExpr { Literal(String), Var(usize), Concat(Box<StrExpr>, Box<StrExpr>) }
impl StrExpr {
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> { match self {
        Self::Literal(s) => { let l = s.len(); Box::new(move |_| l) }
        Self::Var(i) => { let i = *i; Box::new(move |ctx| ctx.strings[i].len()) }
        Self::Concat(a,b) => { let a=a.compile_len(); let b=b.compile_len(); Box::new(move |ctx| a(ctx)+b(ctx)) }
    } }
}

#[derive(Clone, Debug)]
pub enum ListExpr { Var(usize) }
impl ListExpr {
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> { match self {
        Self::Var(i) => { let i=*i; Box::new(move |ctx| match &ctx.values[i]{Value::List(l)=>l.len(),_=>0}) }
    } }
}

#[derive(Clone, Debug)]
pub enum I64Expr {
    Literal(i64), Var(usize),
    Add(Box<I64Expr>,Box<I64Expr>), Sub(Box<I64Expr>,Box<I64Expr>),
    Mul(Box<I64Expr>,Box<I64Expr>), Div(Box<I64Expr>,Box<I64Expr>), Mod(Box<I64Expr>,Box<I64Expr>),
    Neg(Box<I64Expr>), StrLen(Box<StrExpr>), ListLen(Box<ListExpr>),
}
impl I64Expr {
    pub fn compile(&self) -> Box<dyn Fn(&EvalView) -> i64> { match self {
        Self::Literal(v) => { let v=*v; Box::new(move |_| v) }
        Self::Var(i) => { let i=*i; Box::new(move |ctx| ctx.ints[i]) }
        Self::Add(a,b) => { let a=a.compile();let b=b.compile();Box::new(move |ctx| a(ctx).wrapping_add(b(ctx))) }
        Self::Sub(a,b) => { let a=a.compile();let b=b.compile();Box::new(move |ctx| a(ctx).wrapping_sub(b(ctx))) }
        Self::Mul(a,b) => { let a=a.compile();let b=b.compile();Box::new(move |ctx| a(ctx).wrapping_mul(b(ctx))) }
        Self::Div(a,b) => { let a=a.compile();let b=b.compile();Box::new(move |ctx|{let bv=b(ctx);if bv==0{0}else{a(ctx).wrapping_div(bv)}}) }
        Self::Mod(a,b) => { let a=a.compile();let b=b.compile();Box::new(move |ctx|{let bv=b(ctx);if bv==0{0}else{a(ctx).wrapping_rem(bv)}}) }
        Self::Neg(a) => { let a=a.compile();Box::new(move |ctx| a(ctx).wrapping_neg()) }
        Self::StrLen(s) => { let s=s.compile_len();Box::new(move |ctx| s(ctx) as i64) }
        Self::ListLen(l) => { let l=l.compile_len();Box::new(move |ctx| l(ctx) as i64) }
    } }
    pub fn eval_i64(&self, ctx: &EvalView) -> i64 { match self {
        Self::Literal(v) => *v,
        Self::Var(i) => ctx.ints[*i],
        Self::Add(a,b) => a.eval_i64(ctx).wrapping_add(b.eval_i64(ctx)),
        Self::Sub(a,b) => a.eval_i64(ctx).wrapping_sub(b.eval_i64(ctx)),
        Self::Mul(a,b) => a.eval_i64(ctx).wrapping_mul(b.eval_i64(ctx)),
        Self::Div(a,b) => { let bv=b.eval_i64(ctx); if bv==0{0}else{a.eval_i64(ctx).wrapping_div(bv)} }
        Self::Mod(a,b) => { let bv=b.eval_i64(ctx); if bv==0{0}else{a.eval_i64(ctx).wrapping_rem(bv)} }
        Self::Neg(a) => a.eval_i64(ctx).wrapping_neg(),
        Self::StrLen(s) => s.compile_len()(ctx) as i64,
        Self::ListLen(l) => l.compile_len()(ctx) as i64,
    } }
}

// ── CompiledNode (2-variant, fast dispatch) ──

enum CompiledNodeInner { Bool(Box<dyn Fn(&EvalView) -> bool>), Value(Box<dyn Fn(&EvalView) -> Value>) }
pub struct CompiledNode(CompiledNodeInner);
impl CompiledNode {
    pub fn new_bool(f: Box<dyn Fn(&EvalView) -> bool>) -> Self { Self(CompiledNodeInner::Bool(f)) }
    pub fn new_value(f: Box<dyn Fn(&EvalView) -> Value>) -> Self { Self(CompiledNodeInner::Value(f)) }
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        match &self.0 { CompiledNodeInner::Bool(f) => f(ctx), CompiledNodeInner::Value(f) => matches!(f(ctx), Value::Bool(true)) }
    }
    #[inline(always)]
    pub fn eval(&self, ctx: &EvalView) -> Value {
        match &self.0 { CompiledNodeInner::Bool(f) => Value::Bool(f(ctx)), CompiledNodeInner::Value(f) => f(ctx) }
    }
}

// ── Boxed data structs ──

#[derive(Clone, Debug)] pub struct RegexData { pub idx: usize, pub regex: regex::Regex }
#[derive(Clone, Debug)] pub struct AhoData { pub idx: usize, pub ac: aho_corasick::AhoCorasick, pub min: usize }
#[derive(Clone, Debug)] pub struct ContainsAnyData { pub idx: usize, pub needles: Vec<String> }

// ── Op enums ──

#[derive(Clone, Copy, Debug)] pub enum IntOp { Eq, Ne, Lt, Le, Gt, Ge }
#[derive(Clone, Copy, Debug)] pub enum IntArithOp {
    AddEq, AddNe, AddLt, AddLe, AddGt, AddGe,
    SubEq, SubNe, SubLt, SubLe, SubGt, SubGe,
    MulEq, MulNe, MulLt, MulLe, MulGt, MulGe,
}
#[derive(Clone, Copy, Debug)] pub enum CmpOp { Ge, Gt, Le, Lt, Eq, Ne }

impl IntOp { fn eval(&self, l: i64, r: i64) -> bool { match self {
    Self::Eq => l==r, Self::Ne => l!=r, Self::Lt => l<r, Self::Le => l<=r, Self::Gt => l>r, Self::Ge => l>=r,
} } }
impl IntArithOp { fn eval(&self, v:i64,a:i64,c:i64) -> bool { match self {
    Self::AddEq=>v.wrapping_add(a)==c,Self::AddNe=>v.wrapping_add(a)!=c,Self::AddLt=>v.wrapping_add(a)<c,Self::AddLe=>v.wrapping_add(a)<=c,Self::AddGt=>v.wrapping_add(a)>c,Self::AddGe=>v.wrapping_add(a)>=c,
    Self::SubEq=>v.wrapping_sub(a)==c,Self::SubNe=>v.wrapping_sub(a)!=c,Self::SubLt=>v.wrapping_sub(a)<c,Self::SubLe=>v.wrapping_sub(a)<=c,Self::SubGt=>v.wrapping_sub(a)>c,Self::SubGe=>v.wrapping_sub(a)>=c,
    Self::MulEq=>v.wrapping_mul(a)==c,Self::MulNe=>v.wrapping_mul(a)!=c,Self::MulLt=>v.wrapping_mul(a)<c,Self::MulLe=>v.wrapping_mul(a)<=c,Self::MulGt=>v.wrapping_mul(a)>c,Self::MulGe=>v.wrapping_mul(a)>=c,
} } }
impl CmpOp { fn eval(&self, l:i64, r:i64) -> bool { match self {
    Self::Ge=>l>=r,Self::Gt=>l>r,Self::Le=>l<=r,Self::Lt=>l<r,Self::Eq=>l==r,Self::Ne=>l!=r,
} } }

// ── Compact FilterNode (19 variants, heavy data boxed) ──

#[derive(Clone, Debug)]
pub enum FilterNode {
    IntCmp { op: IntOp, idx: usize, val: i64 },
    IntArith { op: IntArithOp, idx: usize, arith: i64, cmp: i64 },
    StrEq { idx: usize, neg: bool, val: String },
    BoolVar { idx: usize },
    InIntSet { idx: usize, vals: Vec<i64> },
    InStrSet { idx: usize, vals: Vec<String> },
    StartsWith { idx: usize, prefix: String },
    EndsWith { idx: usize, suffix: String },
    Contains { idx: usize, substring: String },
    Matches(Box<RegexData>),
    ContainsAny(Box<ContainsAnyData>),
    AhoContains(Box<AhoData>),
    I64Cmp { op: CmpOp, left: Box<I64Expr>, right: Box<I64Expr> },
    And(Box<FilterNode>, Box<FilterNode>),
    Or(Box<FilterNode>, Box<FilterNode>),
    Not(Box<FilterNode>),
    Exists { list_idx: usize, item_idx: usize, predicate: Box<FilterNode> },
    ExistsClosure { list_idx: usize, predicate: ItemPredicate },
    MapKeyContains { map_idx: usize, key: String, needle: String },
    ExistsInIntSet { list_idx: usize, vals: Vec<i64> },
    ExistsEqInt { list_idx: usize, val: i64 },
}

impl FilterNode {
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        match self {
            Self::IntCmp { op, idx, val } => op.eval(ctx.ints[*idx], *val),
            Self::IntArith { op, idx, arith, cmp } => op.eval(ctx.ints[*idx], *arith, *cmp),
            Self::StrEq { idx, neg, val } => {
                let eq = ctx.strings[*idx].as_ref() == val.as_str();
                if *neg { !eq } else { eq }
            }
            Self::BoolVar { idx } => match &ctx.values[*idx] { Value::Bool(b) => *b, _ => false },
            Self::InIntSet { idx, vals } => vals.contains(&ctx.ints[*idx]),
            Self::InStrSet { idx, vals } => {
                let v = ctx.strings[*idx].as_ref();
                vals.iter().any(|s| s == v)
            }
            Self::StartsWith { idx, prefix } => ctx.strings[*idx].starts_with(prefix.as_str()),
            Self::EndsWith { idx, suffix } => ctx.strings[*idx].ends_with(suffix.as_str()),
            Self::Contains { idx, substring } => ctx.strings[*idx].contains(substring.as_str()),
            Self::Matches(d) => d.regex.is_match(ctx.strings[d.idx].as_ref()),
            Self::ContainsAny(d) => {
                let t = ctx.strings[d.idx].as_ref();
                for n in &d.needles { if t.contains(n.as_str()) { return true } } false
            }
            Self::AhoContains(d) => {
                let b = ctx.strings[d.idx].as_ref().as_bytes();
                if d.min <= 1 { return d.ac.is_match(b) }
                let mut m = 0u64;
                for mat in d.ac.find_iter(b) {
                    let p = mat.pattern().as_u64();
                    if p < 64 { m |= 1u64 << p; if m.count_ones() as usize >= d.min { return true } }
                } false
            }
            Self::I64Cmp { op, left, right } => op.eval(left.eval_i64(ctx), right.eval_i64(ctx)),
            Self::And(a, b) => a.eval_bool(ctx) && b.eval_bool(ctx),
            Self::Or(a, b) => a.eval_bool(ctx) || b.eval_bool(ctx),
            Self::Not(i) => !i.eval_bool(ctx),
            Self::Exists { list_idx, item_idx, predicate } => match &ctx.values[*list_idx] {
                Value::List(list) => {
                    let mut e = ctx.values.to_vec(); e.push(Value::Null);
                    for item in list.iter() {
                        e[*item_idx] = item.clone();
                        let tv = EvalView { ints: ctx.ints, strings: ctx.strings, values: &e };
                        if predicate.eval_bool(&tv) { return true }
                    } false
                } _ => false,
            }
            Self::ExistsClosure { list_idx, predicate } => match &ctx.values[*list_idx] {
                Value::List(list) => list.iter().any(|item| predicate.call(item)),
                _ => false,
            }
            Self::MapKeyContains { map_idx, key, needle } => match &ctx.values[*map_idx] {
                Value::Map(m) => {
                    let k = Key::String(Arc::from(key.as_str()));
                    match m.map.get(&k) {
                        Some(Value::String(s)) => s.as_ref() == needle.as_str(),
                        Some(Value::List(list)) => list.iter().any(|v| matches!(v, Value::String(s) if s.as_ref() == needle.as_str())),
                        _ => false,
                    }
                } _ => false,
            }
            Self::ExistsInIntSet { list_idx, vals } => match &ctx.values[*list_idx] {
                Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if vals.contains(i))),
                _ => false,
            }
            Self::ExistsEqInt { list_idx, val } => match &ctx.values[*list_idx] {
                Value::List(list) => list.iter().any(|v| matches!(v, Value::Int(i) if *i == *val)),
                _ => false,
            }
        }
    }

    pub fn compile(&self) -> CompiledNode {
        match self {
            Self::IntCmp { op, idx, val } => { let idx=*idx; let val=*val; CompiledNode::new_bool(match op {
                IntOp::Eq => Box::new(move |ctx| ctx.ints[idx]==val),
                IntOp::Ne => Box::new(move |ctx| ctx.ints[idx]!=val),
                IntOp::Lt => Box::new(move |ctx| ctx.ints[idx]<val),
                IntOp::Le => Box::new(move |ctx| ctx.ints[idx]<=val),
                IntOp::Gt => Box::new(move |ctx| ctx.ints[idx]>val),
                IntOp::Ge => Box::new(move |ctx| ctx.ints[idx]>=val),
            })}
            Self::IntArith { op, idx, arith, cmp } => { let idx=*idx;let arith=*arith;let cmp=*cmp; CompiledNode::new_bool(match op {
                IntArithOp::AddEq=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)==cmp),
                IntArithOp::AddNe=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)!=cmp),
                IntArithOp::AddLt=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)<cmp),
                IntArithOp::AddLe=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)<=cmp),
                IntArithOp::AddGt=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)>cmp),
                IntArithOp::AddGe=>Box::new(move|ctx|ctx.ints[idx].wrapping_add(arith)>=cmp),
                IntArithOp::SubEq=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)==cmp),
                IntArithOp::SubNe=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)!=cmp),
                IntArithOp::SubLt=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)<cmp),
                IntArithOp::SubLe=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)<=cmp),
                IntArithOp::SubGt=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)>cmp),
                IntArithOp::SubGe=>Box::new(move|ctx|ctx.ints[idx].wrapping_sub(arith)>=cmp),
                IntArithOp::MulEq=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)==cmp),
                IntArithOp::MulNe=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)!=cmp),
                IntArithOp::MulLt=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)<cmp),
                IntArithOp::MulLe=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)<=cmp),
                IntArithOp::MulGt=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)>cmp),
                IntArithOp::MulGe=>Box::new(move|ctx|ctx.ints[idx].wrapping_mul(arith)>=cmp),
            })}
            Self::StrEq { idx, neg, val } => { let idx=*idx;let neg=*neg;let val:Arc<str>=Arc::from(val.as_str()); CompiledNode::new_bool(if neg {Box::new(move|ctx|ctx.strings[idx].as_ref()!=val.as_ref())}else{Box::new(move|ctx|ctx.strings[idx].as_ref()==val.as_ref())}) }
            Self::BoolVar { idx } => { let idx=*idx; CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[idx]{Value::Bool(b)=>*b,_=>false})) }
            Self::InIntSet { idx, vals } => { let idx=*idx;let vals=vals.clone(); CompiledNode::new_bool(Box::new(move|ctx| vals.contains(&ctx.ints[idx]))) }
            Self::InStrSet { idx, vals } => { let idx=*idx;let vals=vals.clone(); CompiledNode::new_bool(Box::new(move|ctx|{let v=ctx.strings[idx].as_ref();vals.iter().any(|s|s==v)})) }
            Self::StartsWith { idx, prefix } => { let idx=*idx;let pf:Arc<str>=Arc::from(prefix.as_str()); CompiledNode::new_bool(Box::new(move|ctx| ctx.strings[idx].starts_with(pf.as_ref()))) }
            Self::EndsWith { idx, suffix } => { let idx=*idx;let sf:Arc<str>=Arc::from(suffix.as_str()); CompiledNode::new_bool(Box::new(move|ctx| ctx.strings[idx].ends_with(sf.as_ref()))) }
            Self::Contains { idx, substring } => { let idx=*idx;let sub:Arc<str>=Arc::from(substring.as_str()); CompiledNode::new_bool(Box::new(move|ctx| ctx.strings[idx].contains(sub.as_ref()))) }
            Self::Matches(d)=>{let idx=d.idx;let re=d.regex.clone();CompiledNode::new_bool(Box::new(move|ctx| re.is_match(ctx.strings[idx].as_ref())))}
            Self::ContainsAny(d)=>{let idx=d.idx;let ndls=d.needles.clone();CompiledNode::new_bool(Box::new(move|ctx|{let t=ctx.strings[idx].as_ref();for n in &ndls{if t.contains(n.as_str()){return true}}false}))}
            Self::AhoContains(d)=>{let idx=d.idx;let ac=d.ac.clone();let min=d.min;CompiledNode::new_bool(Box::new(move|ctx|{let b=ctx.strings[idx].as_ref().as_bytes();if min<=1{return ac.is_match(b)}let mut m=0u64;for mat in ac.find_iter(b){let p=mat.pattern().as_u64();if p<64{m|=1u64<<p;if m.count_ones()as usize>=min{return true}}}false}))}
            Self::I64Cmp { op, left, right } => { let l=left.compile();let r=right.compile(); CompiledNode::new_bool(match op {
                CmpOp::Ge => Box::new(move|ctx| l(ctx)>=r(ctx)),
                CmpOp::Gt => Box::new(move|ctx| l(ctx)>r(ctx)),
                CmpOp::Le => Box::new(move|ctx| l(ctx)<=r(ctx)),
                CmpOp::Lt => Box::new(move|ctx| l(ctx)<r(ctx)),
                CmpOp::Eq => Box::new(move|ctx| l(ctx)==r(ctx)),
                CmpOp::Ne => Box::new(move|ctx| l(ctx)!=r(ctx)),
            })}
            Self::And(a,b) => { let a=a.compile();let b=b.compile(); CompiledNode::new_bool(Box::new(move|ctx| a.eval_bool(ctx)&&b.eval_bool(ctx))) }
            Self::Or(a,b) => { let a=a.compile();let b=b.compile(); CompiledNode::new_bool(Box::new(move|ctx|{if a.eval_bool(ctx){return true}b.eval_bool(ctx)})) }
            Self::Not(i) => { let i=i.compile(); CompiledNode::new_bool(Box::new(move|ctx|!i.eval_bool(ctx))) }
            Self::Exists{list_idx,item_idx,predicate} => { let li=*list_idx;let ii=*item_idx;let p=predicate.compile(); CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[li]{Value::List(list)=>{let mut e=ctx.values.to_vec();e.push(Value::Null);for item in list.iter(){e[ii]=item.clone();let tv=EvalView{ints:ctx.ints,strings:ctx.strings,values:&e};if p.eval_bool(&tv){return true}}false}_=>false})) }
            Self::ExistsClosure{list_idx,predicate} => { let li=*list_idx;let p=predicate.clone(); CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|item|p.call(item)),_=>false})) }
            Self::MapKeyContains{map_idx,key,needle} => { let mi=*map_idx;let k:Arc<str>=Arc::from(key.as_str());let n:Arc<str>=Arc::from(needle.as_str()); CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[mi]{Value::Map(m)=>{let kk=Key::String(Arc::clone(&k));match m.map.get(&kk){Some(Value::String(s))=>s.as_ref()==n.as_ref(),Some(Value::List(list))=>list.iter().any(|v|matches!(v,Value::String(s)if s.as_ref()==n.as_ref())),_=>false}}_=>false})) }
            Self::ExistsInIntSet{list_idx,vals} => { let li=*list_idx;let vals=vals.clone(); CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|v|matches!(v,Value::Int(i)if vals.contains(i))),_=>false})) }
            Self::ExistsEqInt{list_idx,val} => { let li=*list_idx;let val=*val; CompiledNode::new_bool(Box::new(move|ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|v|matches!(v,Value::Int(i)if *i==val)),_=>false})) }
        }
    }
}
