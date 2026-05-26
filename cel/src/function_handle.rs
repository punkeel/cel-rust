use crate::common::{
    ast::{CallExpr, EntryExpr, Expr, IdedExpr},
    decls::FunctionDecl,
    types::{Kind, Type},
    value::Val,
};
use crate::Env;
use std::borrow::Cow;

/// A compact handle representing a resolved function overload.
/// Layout: [16 bits fn_id | 16 bits overload_idx]
/// Supports 65K functions with 65K overloads each — far more than needed for CEL stdlib.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FunctionHandle(u32);

impl FunctionHandle {
    pub const UNRESOLVED: Self = Self(u32::MAX);

    #[inline(always)]
    pub fn new(fn_id: u16, overload_idx: u16) -> Self {
        Self((fn_id as u32) << 16 | overload_idx as u32)
    }

    #[inline(always)]
    pub fn fn_id(self) -> u16 {
        (self.0 >> 16) as u16
    }

    #[inline(always)]
    pub fn overload_idx(self) -> u16 {
        self.0 as u16
    }

    #[inline(always)]
    pub fn is_resolved(self) -> bool {
        self.0 != u32::MAX
    }
}

/// Information needed to resolve a function call at parse/compile time.
pub struct FunctionResolver<'a> {
    env: &'a Env,
    name_to_id: std::collections::BTreeMap<String, u16>,
    functions: Vec<&'a FunctionDecl>,
}

impl<'a> FunctionResolver<'a> {
    pub fn new(env: &'a Env) -> Self {
        // Build flat index from env's BTreeMap.
        // We iterate in order so the IDs are deterministic.
        let mut name_to_id = std::collections::BTreeMap::new();
        let mut functions = Vec::new();
        for (name, decl) in env.functions() {
            let id = functions.len() as u16;
            name_to_id.insert(name.clone(), id);
            functions.push(decl);
        }
        Self {
            env,
            name_to_id,
            functions,
        }
    }

    /// Resolve a function call to its function pointer given compile-time type hints.
    pub fn resolve_op(
        &self,
        name: &str,
        member: bool,
        arg_types: &[TypeHint],
    ) -> Option<crate::common::functions::Function> {
        let fn_id = self.name_to_id.get(name)?;
        let decl = self.functions[*fn_id as usize];

        for overload in decl.overloads.iter() {
            if overload.member_function == member
                && arg_types.len() == overload.arg_types.len()
                && arg_types
                    .iter()
                    .zip(&overload.arg_types)
                    .all(|(a, t)| a.fits(t))
            {
                return Some(overload.op);
            }
        }
        None
    }

    /// Resolve a function call to a handle given its name, whether it's a member
    /// function, and the compile-time-known types of its arguments.
    pub fn resolve(
        &self,
        name: &str,
        member: bool,
        arg_types: &[TypeHint],
    ) -> Option<FunctionHandle> {
        let fn_id = self.name_to_id.get(name)?;
        let decl = self.functions[*fn_id as usize];

        for (idx, overload) in decl.overloads.iter().enumerate() {
            if overload.member_function == member
                && arg_types.len() == overload.arg_types.len()
                && arg_types
                    .iter()
                    .zip(&overload.arg_types)
                    .all(|(a, t)| a.fits(t))
            {
                return Some(FunctionHandle::new(*fn_id, idx as u16));
            }
        }
        None
    }

    /// Resolve a function call using runtime type checking fallback.
    /// Called when compile-time types are unknown.
    pub fn resolve_runtime(
        &self,
        name: &str,
        member: bool,
        args: &[Cow<'_, dyn Val>],
    ) -> Option<FunctionHandle> {
        let fn_id = self.name_to_id.get(name)?;
        let decl = self.functions[*fn_id as usize];

        for (idx, overload) in decl.overloads.iter().enumerate() {
            if overload.member_function == member
                && args.len() == overload.arg_types.len()
                && args
                    .iter()
                    .zip(&overload.arg_types)
                    .all(|(a, t)| t.is_assignable(a.as_ref()))
            {
                return Some(FunctionHandle::new(*fn_id, idx as u16));
            }
        }
        None
    }

    /// Get the function pointer for a resolved handle.
    #[inline(always)]
    pub fn get_function(&self, handle: FunctionHandle) -> crate::common::functions::Function {
        let decl = self.functions[handle.fn_id() as usize];
        decl.overloads[handle.overload_idx() as usize].op
    }

    pub fn functions(&self) -> &[&'a FunctionDecl] {
        &self.functions
    }
}

/// Lightweight type hint for compile-time function overload resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeHint {
    String,
    Int,
    UInt,
    Float,
    Bool,
    Bytes,
    List,
    Map,
    Null,
    Unknown,
}

impl TypeHint {
    /// Check if this hint can match the given target type.
    /// Unknown is a wildcard — it matches anything because we don't have
    /// enough information at compile time to reject it.
    pub fn fits(&self, target: &Type) -> bool {
        if matches!(self, Self::Unknown) {
            return true;
        }
        let target_kind = target.kind();
        matches!(
            (self, target_kind),
            (Self::String, Kind::String)
                | (Self::Int, Kind::Int)
                | (Self::UInt, Kind::UInt)
                | (Self::Float, Kind::Double)
                | (Self::Bool, Kind::Boolean)
                | (Self::Bytes, Kind::Bytes)
                | (Self::List, Kind::List)
                | (Self::Map, Kind::Map)
                | (Self::Null, Kind::NullType)
        )
    }
}

/// Derive a TypeHint from an expression — returns None for variables etc.
pub fn expr_type_hint(expr: &Expr) -> TypeHint {
    match expr {
        Expr::Literal(lit) => match lit {
            crate::common::ast::LiteralValue::Boolean(_) => TypeHint::Bool,
            crate::common::ast::LiteralValue::Bytes(_) => TypeHint::Bytes,
            crate::common::ast::LiteralValue::Double(_) => TypeHint::Float,
            crate::common::ast::LiteralValue::Int(_) => TypeHint::Int,
            crate::common::ast::LiteralValue::Null => TypeHint::Null,
            crate::common::ast::LiteralValue::String(_) => TypeHint::String,
            crate::common::ast::LiteralValue::UInt(_) => TypeHint::UInt,
        },
        Expr::List(_) => TypeHint::List,
        Expr::Map(_) => TypeHint::Map,
        _ => TypeHint::Unknown,
    }
}

/// Walk an AST and resolve function handles where possible.
pub fn resolve_ast(expr: &mut IdedExpr, resolver: &FunctionResolver<'_>) {
    resolve_expr(&mut expr.expr, resolver);
}

fn resolve_expr(expr: &mut Expr, resolver: &FunctionResolver<'_>) {
    match expr {
        Expr::Call(call) => {
            // First resolve children
            if let Some(target) = call.target.as_mut() {
                resolve_expr(&mut target.expr, resolver);
            }
            for arg in &mut call.args {
                resolve_expr(&mut arg.expr, resolver);
            }

            // Try to resolve this call
            let member = call.target.is_some();
            let arg_hints: Vec<_> = call
                .args
                .iter()
                .map(|a| expr_type_hint(&a.expr))
                .collect();

            call.resolved_op = resolver.resolve_op(&call.func_name, member, &arg_hints);
        }
        Expr::Select(select) => {
            resolve_expr(&mut select.operand.expr, resolver);
        }
        Expr::List(list) => {
            for elem in &mut list.elements {
                resolve_expr(&mut elem.expr, resolver);
            }
        }
        Expr::Map(map) => {
            for entry in &mut map.entries {
                match &mut entry.expr {
                    EntryExpr::MapEntry(me) => {
                        resolve_expr(&mut me.key.expr, resolver);
                        resolve_expr(&mut me.value.expr, resolver);
                    }
                    EntryExpr::StructField(sf) => {
                        resolve_expr(&mut sf.value.expr, resolver);
                    }
                }
            }
        }
        Expr::Comprehension(comp) => {
            resolve_expr(&mut comp.iter_range.expr, resolver);
            resolve_expr(&mut comp.accu_init.expr, resolver);
            resolve_expr(&mut comp.loop_cond.expr, resolver);
            resolve_expr(&mut comp.loop_step.expr, resolver);
            resolve_expr(&mut comp.result.expr, resolver);
        }
        _ => {}
    }
}
