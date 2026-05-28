use crate::objects::{Key, Value};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct EvalView<'a> {
    pub ints: &'a [i64],
    pub strings: &'a [Arc<str>],
    pub values: &'a [Value],
}

// ── Compiled node: 2-variant dispatch ──

enum CompiledNodeInner {
    Bool(Box<dyn Fn(&EvalView) -> bool>),
    Value(Box<dyn Fn(&EvalView) -> Value>),
}

pub struct CompiledNode(CompiledNodeInner);

impl CompiledNode {
    pub fn new_bool(f: Box<dyn Fn(&EvalView) -> bool>) -> Self {
        Self(CompiledNodeInner::Bool(f))
    }
    pub fn new_value(f: Box<dyn Fn(&EvalView) -> Value>) -> Self {
        Self(CompiledNodeInner::Value(f))
    }
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalView) -> bool {
        match &self.0 {
            CompiledNodeInner::Bool(f) => f(ctx),
            CompiledNodeInner::Value(f) => matches!(f(ctx), Value::Bool(true)),
        }
    }
    #[inline(always)]
    pub fn eval(&self, ctx: &EvalView) -> Value {
        match &self.0 {
            CompiledNodeInner::Bool(f) => Value::Bool(f(ctx)),
            CompiledNodeInner::Value(f) => f(ctx),
        }
    }
}

// ── Item predicate ──

#[derive(Clone)]
pub struct ItemPredicate(Arc<dyn Fn(&Value) -> bool>);

impl std::fmt::Debug for ItemPredicate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ItemPredicate").finish()
    }
}

impl ItemPredicate {
    pub fn new(f: Box<dyn Fn(&Value) -> bool>) -> Self { Self(Arc::from(f)) }
    pub fn call(&self, v: &Value) -> bool { (self.0)(v) }
}

// ── Sub-expressions ──

#[derive(Clone, Debug)]
pub enum StrExpr {
    Literal(String), Var(usize), Concat(Box<StrExpr>, Box<StrExpr>),
}
impl StrExpr {
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> {
        match self {
            Self::Literal(s) => { let l = s.len(); Box::new(move |_| l) }
            Self::Var(i) => { let i = *i; Box::new(move |ctx| ctx.strings[i].len()) }
            Self::Concat(a,b) => { let a=a.compile_len(); let b=b.compile_len(); Box::new(move |ctx| a(ctx)+b(ctx)) }
        }
    }
}

#[derive(Clone, Debug)]
pub enum ListExpr { Var(usize) }
impl ListExpr {
    pub fn compile_len(&self) -> Box<dyn Fn(&EvalView) -> usize> {
        match self { Self::Var(i) => { let i=*i; Box::new(move |ctx| match &ctx.values[i] { Value::List(l) => l.len(), _ => 0 }) } }
    }
}

#[derive(Clone, Debug)]
pub enum I64Expr {
    Literal(i64), Var(usize), Add(Box<I64Expr>,Box<I64Expr>), Sub(Box<I64Expr>,Box<I64Expr>),
    Mul(Box<I64Expr>,Box<I64Expr>), Div(Box<I64Expr>,Box<I64Expr>), Mod(Box<I64Expr>,Box<I64Expr>),
    Neg(Box<I64Expr>), StrLen(Box<StrExpr>), ListLen(Box<ListExpr>),
}
impl I64Expr {
    pub fn compile(&self) -> Box<dyn Fn(&EvalView) -> i64> {
        match self {
            Self::Literal(v) => { let v=*v; Box::new(move |_| v) }
            Self::Var(i) => { let i=*i; Box::new(move |ctx| ctx.ints[i]) }
            Self::Add(a,b) => { let a=a.compile(); let b=b.compile(); Box::new(move |ctx| a(ctx).wrapping_add(b(ctx))) }
            Self::Sub(a,b) => { let a=a.compile(); let b=b.compile(); Box::new(move |ctx| a(ctx).wrapping_sub(b(ctx))) }
            Self::Mul(a,b) => { let a=a.compile(); let b=b.compile(); Box::new(move |ctx| a(ctx).wrapping_mul(b(ctx))) }
            Self::Div(a,b) => { let a=a.compile(); let b=b.compile(); Box::new(move |ctx| { let bv=b(ctx); if bv==0 {0} else {a(ctx).wrapping_div(bv)} }) }
            Self::Mod(a,b) => { let a=a.compile(); let b=b.compile(); Box::new(move |ctx| { let bv=b(ctx); if bv==0 {0} else {a(ctx).wrapping_rem(bv)} }) }
            Self::Neg(a) => { let a=a.compile(); Box::new(move |ctx| a(ctx).wrapping_neg()) }
            Self::StrLen(s) => { let s=s.compile_len(); Box::new(move |ctx| s(ctx) as i64) }
            Self::ListLen(l) => { let l=l.compile_len(); Box::new(move |ctx| l(ctx) as i64) }
        }
    }
}

// ── FilterNode (IR) → CompiledNode (eval) ──

#[derive(Clone, Debug)]
pub enum FilterNode {
    EqInt{idx:usize,val:i64}, NeInt{idx:usize,val:i64}, LtInt{idx:usize,val:i64},
    LeInt{idx:usize,val:i64}, GtInt{idx:usize,val:i64}, GeInt{idx:usize,val:i64},
    AddEq{idx:usize,arith:i64,cmp:i64}, AddNe{idx:usize,arith:i64,cmp:i64},
    AddLt{idx:usize,arith:i64,cmp:i64}, AddLe{idx:usize,arith:i64,cmp:i64},
    AddGt{idx:usize,arith:i64,cmp:i64}, AddGe{idx:usize,arith:i64,cmp:i64},
    SubEq{idx:usize,arith:i64,cmp:i64}, SubNe{idx:usize,arith:i64,cmp:i64},
    SubLt{idx:usize,arith:i64,cmp:i64}, SubLe{idx:usize,arith:i64,cmp:i64},
    SubGt{idx:usize,arith:i64,cmp:i64}, SubGe{idx:usize,arith:i64,cmp:i64},
    MulEq{idx:usize,arith:i64,cmp:i64}, MulNe{idx:usize,arith:i64,cmp:i64},
    MulLt{idx:usize,arith:i64,cmp:i64}, MulLe{idx:usize,arith:i64,cmp:i64},
    MulGt{idx:usize,arith:i64,cmp:i64}, MulGe{idx:usize,arith:i64,cmp:i64},
    EqStr{idx:usize,val:String}, NeStr{idx:usize,val:String}, BoolVar{idx:usize},
    InIntLinear{idx:usize,vals:Vec<i64>}, InIntHash{idx:usize,set:HashSet<i64>},
    InStrLinear{idx:usize,vals:Vec<String>}, InStrHash{idx:usize,set:HashSet<String>},
    StartsWith{idx:usize,prefix:String}, EndsWith{idx:usize,suffix:String},
    Contains{idx:usize,substring:String}, Matches{idx:usize,regex:regex::Regex},
    ContainsAny{idx:usize,needles:Vec<String>},
    AhoContains{idx:usize,ac:aho_corasick::AhoCorasick,min:usize},
    GeExpr{left:I64Expr,right:I64Expr}, GtExpr{left:I64Expr,right:I64Expr},
    LeExpr{left:I64Expr,right:I64Expr}, LtExpr{left:I64Expr,right:I64Expr},
    EqExpr{left:I64Expr,right:I64Expr}, NeExpr{left:I64Expr,right:I64Expr},
    And(Box<FilterNode>,Box<FilterNode>), Or(Box<FilterNode>,Box<FilterNode>), Not(Box<FilterNode>),
    Exists{list_idx:usize,item_idx:usize,predicate:Box<FilterNode>},
    ExistsClosure{list_idx:usize,predicate:ItemPredicate},
    MapKeyContains{map_idx:usize,key:String,needle:String},
    ExistsInIntSet{list_idx:usize,vals:Vec<i64>}, ExistsEqInt{list_idx:usize,val:i64},
}

impl FilterNode {
    pub fn compile(&self) -> CompiledNode {
        macro_rules! int_cmp { ($id:ident, $op:tt) => { let idx=*self.$id.idx; let val=*self.$id.val; CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx] $op val)) }; }
        macro_rules! arith_cmp { ($v:ident, $op:ident) => { let idx=*self.$v.idx; let arith=*self.$v.arith; let cmp=*self.$v.cmp; CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].$op(arith) cmp)) }; }
        match self {
            Self::EqInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]==val))}
            Self::NeInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]!=val))}
            Self::LtInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]<val))}
            Self::LeInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]<=val))}
            Self::GtInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]>val))}
            Self::GeInt{idx,val}=>{let idx=*idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx]>=val))}
            Self::AddEq{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)==cmp))}
            Self::AddNe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)!=cmp))}
            Self::AddLt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)<cmp))}
            Self::AddLe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)<=cmp))}
            Self::AddGt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)>cmp))}
            Self::AddGe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_add(arith)>=cmp))}
            Self::SubEq{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)==cmp))}
            Self::SubNe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)!=cmp))}
            Self::SubLt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)<cmp))}
            Self::SubLe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)<=cmp))}
            Self::SubGt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)>cmp))}
            Self::SubGe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_sub(arith)>=cmp))}
            Self::MulEq{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)==cmp))}
            Self::MulNe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)!=cmp))}
            Self::MulLt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)<cmp))}
            Self::MulLe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)<=cmp))}
            Self::MulGt{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)>cmp))}
            Self::MulGe{idx,arith,cmp}=>{let idx=*idx;let arith=*arith;let cmp=*cmp;CompiledNode::new_bool(Box::new(move |ctx| ctx.ints[idx].wrapping_mul(arith)>=cmp))}
            Self::EqStr{idx,val}=>{let idx=*idx;let val:Arc<str>=Arc::from(val.as_str());CompiledNode::new_bool(Box::new(move |ctx| ctx.strings[idx].as_ref()==val.as_ref()))}
            Self::NeStr{idx,val}=>{let idx=*idx;let val:Arc<str>=Arc::from(val.as_str());CompiledNode::new_bool(Box::new(move |ctx| ctx.strings[idx].as_ref()!=val.as_ref()))}
            Self::BoolVar{idx}=>{let idx=*idx;CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[idx]{Value::Bool(b)=>*b,_=>false}))}
            Self::InIntLinear{idx,vals}=>{let idx=*idx;let vals=vals.clone();CompiledNode::new_bool(Box::new(move |ctx| vals.contains(&ctx.ints[idx])))}
            Self::InIntHash{idx,set}=>{let idx=*idx;let set:HashSet<i64>=set.clone();CompiledNode::new_bool(Box::new(move |ctx| set.contains(&ctx.ints[idx])))}
            Self::InStrLinear{idx,vals}=>{let idx=*idx;let vals=vals.clone();CompiledNode::new_bool(Box::new(move |ctx|{let v=ctx.strings[idx].as_ref();vals.iter().any(|s|s==v)}))}
            Self::InStrHash{idx,set}=>{let idx=*idx;let set:HashSet<String>=set.clone();CompiledNode::new_bool(Box::new(move |ctx| set.contains(ctx.strings[idx].as_ref())))}
            Self::StartsWith{idx,prefix}=>{let idx=*idx;let s:Arc<str>=Arc::from(prefix.as_str());CompiledNode::new_bool(Box::new(move |ctx| ctx.strings[idx].starts_with(s.as_ref())))}
            Self::EndsWith{idx,suffix}=>{let idx=*idx;let s:Arc<str>=Arc::from(suffix.as_str());CompiledNode::new_bool(Box::new(move |ctx| ctx.strings[idx].ends_with(s.as_ref())))}
            Self::Contains{idx,substring}=>{let idx=*idx;let s:Arc<str>=Arc::from(substring.as_str());CompiledNode::new_bool(Box::new(move |ctx| ctx.strings[idx].contains(s.as_ref())))}
            Self::Matches{idx,regex}=>{let idx=*idx;let regex=regex.clone();CompiledNode::new_bool(Box::new(move |ctx| regex.is_match(ctx.strings[idx].as_ref())))}
            Self::ContainsAny{idx,needles}=>{let idx=*idx;let needles=needles.clone();CompiledNode::new_bool(Box::new(move |ctx|{let t=ctx.strings[idx].as_ref();for n in &needles{if t.contains(n.as_str()){return true}}false}))}
            Self::AhoContains{idx,ac,min}=>{let idx=*idx;let ac=ac.clone();let min=*min;CompiledNode::new_bool(Box::new(move |ctx|{let b=ctx.strings[idx].as_ref().as_bytes();if min<=1{return ac.is_match(b)}let mut m=0u64;for mat in ac.find_iter(b){let p=mat.pattern().as_u64();if p<64{m|=1u64<<p;if m.count_ones()as usize>=min{return true}}}false}))}
            Self::GeExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)>=r(ctx)))}
            Self::GtExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)>r(ctx)))}
            Self::LeExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)<=r(ctx)))}
            Self::LtExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)<r(ctx)))}
            Self::EqExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)==r(ctx)))}
            Self::NeExpr{left,right}=>{let l=left.compile();let r=right.compile();CompiledNode::new_bool(Box::new(move |ctx| l(ctx)!=r(ctx)))}
            Self::And(a,b)=>{let a=a.compile();let b=b.compile();CompiledNode::new_bool(Box::new(move |ctx| a.eval_bool(ctx)&&b.eval_bool(ctx)))}
            Self::Or(a,b)=>{let a=a.compile();let b=b.compile();CompiledNode::new_bool(Box::new(move |ctx|{if a.eval_bool(ctx){return true}b.eval_bool(ctx)}))}
            Self::Not(i)=>{let i=i.compile();CompiledNode::new_bool(Box::new(move |ctx|!i.eval_bool(ctx)))}
            Self::Exists{list_idx,item_idx,predicate}=>{let li=*list_idx;let ii=*item_idx;let p=predicate.compile();CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[li]{Value::List(list)=>{let mut e=ctx.values.to_vec();e.push(Value::Null);for item in list.iter(){e[ii]=item.clone();let tv=EvalView{ints:ctx.ints,strings:ctx.strings,values:&e};if p.eval_bool(&tv){return true}}false}_=>false}))}
            Self::ExistsClosure{list_idx,predicate}=>{let li=*list_idx;let p=predicate.clone();CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|item|p.call(item)),_=>false}))}
            Self::MapKeyContains{map_idx,key,needle}=>{let mi=*map_idx;let k:Arc<str>=Arc::from(key.as_str());let n:Arc<str>=Arc::from(needle.as_str());CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[mi]{Value::Map(m)=>{let kk=Key::String(Arc::clone(&k));match m.map.get(&kk){Some(Value::String(s))=>s.as_ref()==n.as_ref(),Some(Value::List(list))=>list.iter().any(|v|matches!(v,Value::String(s)if s.as_ref()==n.as_ref())),_=>false}}_=>false}))}
            Self::ExistsInIntSet{list_idx,vals}=>{let li=*list_idx;let vals=vals.clone();CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|v| matches!(v,Value::Int(i)if vals.contains(i))),_=>false}))}
            Self::ExistsEqInt{list_idx,val}=>{let li=*list_idx;let val=*val;CompiledNode::new_bool(Box::new(move |ctx| match &ctx.values[li]{Value::List(list)=>list.iter().any(|v| matches!(v,Value::Int(i)if *i==val)),_=>false}))}
        }
    }
}
