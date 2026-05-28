//! Core closure compiler — compiles any CEL expression to a
//! `Box<dyn Fn(&Context) -> Result<Value, ExecutionError>>`.
//!
//! Unlike the AST interpreter (which recursively walks the tree at
//! evaluation time), this compiler walks the tree ONCE at compile time
//! and captures all logic into a flat closure.  The result is a single
//! function call per evaluation — no recursion, no dynamic dispatch on
//! expression variants.
//!
//! Variable resolution, function dispatch, and overload matching all
//! happen via the [`Context`] at runtime (not via pre-assigned slots).
//! This means the compiled closure integrates naturally with the
//! existing Context-based scoping (child scopes, variable resolvers,
//! etc.) including comprehension temp variables.
//!
//! # Usage
//!
//! ```ignore
//! use cel::vm::compiler::compile_expression;
//!
//! let expr = cel::Program::compile("a.b + 1").unwrap();
//! let closure = compile_expression(&expr.expression(), &[]);
//!
//! let mut ctx = cel::Context::default();
//! ctx.add_variable_from_value("a", std::collections::HashMap::from([
//!     ("b", 41i64),
//! ]));
//! let result = closure(&ctx).unwrap();
//! assert_eq!(result, cel::Value::Int(42));
//! ```

use crate::common::ast::operators;
use crate::common::ast::{
    EntryExpr, Expr, IdedExpr, LiteralValue, SelectExpr,
};
use crate::common::types::*;
use crate::common::value::Val;
use crate::context::Context;
use crate::objects::{Key, Map, OptionalValue, Value};
use crate::parser::Expression;
use crate::{ExecutionError, FunctionContext};
use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::Arc;

/// A compiled CEL expression closure.
///
/// Takes a reference to the evaluation [`Context`] and returns either
/// a [`Value`] or an [`ExecutionError`].
pub type ValueClosure = Box<dyn Fn(&Context) -> Result<Value, ExecutionError>>;

/// Compile a parsed expression into a closure.
///
/// `reserved_names` can be supplied to pre-assign variable names for
/// external binding — typically used by the Schema/fast-path pipeline.
/// For standalone use, pass an empty slice.
///
/// The returned closure captures **everything** needed for evaluation.
/// It is `'static` and can be sent across threads.
pub fn compile_expression(expr: &Expression, reserved_names: &[&str]) -> ValueClosure {
    let owned: Vec<String> = reserved_names.iter().map(|s| s.to_string()).collect();
    let reserved: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    compile_expr(&reserved, &expr.expr)
}

// ─── Entry point — compile a single Expr node ──────────────────────────

fn compile_expr(reserved: &[&str], expr: &Expr) -> ValueClosure {
    match expr {
        Expr::Literal(lit) => compile_literal(lit),
        Expr::Ident(name) => compile_ident(name),
        Expr::Select(sel) => compile_select(sel, reserved),
        Expr::Call(call) => compile_call(call, reserved),
        Expr::List(list) => compile_list(list, reserved),
        Expr::Map(map) => compile_map(map, reserved),
        Expr::Comprehension(comp) => compile_comprehension(comp, reserved),
        Expr::Struct(strct) => compile_struct(strct, reserved),
        Expr::Unspecified => Box::new(|_| Ok(Value::Null)),
    }
}

/// Helper: compile a list of `IdedExpr` arguments into closures.
fn compile_arg_list<'a>(reserved: &[&str], args: &[IdedExpr]) -> Vec<ValueClosure> {
    args.iter()
        .map(|a| compile_expr(reserved, &a.expr))
        .collect()
}

/// Convert a `Value` into a `Cow<'_, dyn Val>` so it can be passed to
/// Val-trait methods (add, compare, index, etc.).
///
/// Uses the existing `Value → Box<dyn Val>` conversion from `objects.rs`,
/// which correctly handles all types including opaque values and structs.
fn to_cow(val: &Value) -> Cow<'static, dyn Val> {
    match TryInto::<Box<dyn Val>>::try_into(val.clone()) {
        Ok(boxed) => Cow::Owned(boxed),
        Err(_) => Cow::Owned(Box::new(CelNull) as Box<dyn Val>),
    }
}

// ─── Literals ──────────────────────────────────────────────────────────

fn compile_literal(lit: &LiteralValue) -> ValueClosure {
    match lit {
        LiteralValue::Int(i) => {
            let v = *i.inner();
            Box::new(move |_| Ok(Value::Int(v)))
        }
        LiteralValue::UInt(u) => {
            let v = *u.inner();
            Box::new(move |_| Ok(Value::UInt(v)))
        }
        LiteralValue::Double(f) => {
            let v = *f.inner();
            Box::new(move |_| Ok(Value::Float(v)))
        }
        LiteralValue::String(s) => {
            let inner = s.inner().to_string();
            let v: Arc<str> = Arc::from(inner);
            Box::new(move |_| Ok(Value::String(Arc::clone(&v))))
        }
        LiteralValue::Bytes(b) => {
            let v: Arc<Vec<u8>> = Arc::new(b.inner().to_vec());
            Box::new(move |_| Ok(Value::Bytes(Arc::clone(&v))))
        }
        LiteralValue::Boolean(b) => {
            let v = *b.inner();
            Box::new(move |_| Ok(Value::Bool(v)))
        }
        LiteralValue::Null => Box::new(|_| Ok(Value::Null)),
    }
}

// ─── Identifiers ───────────────────────────────────────────────────────

fn compile_ident(name: &str) -> ValueClosure {
    let name = name.to_string();
    Box::new(move |ctx| {
        let cow = ctx
            .get_variable(&name)
            .ok_or_else(|| ExecutionError::UndeclaredReference(Arc::new(name.clone())))?;
        Value::try_from(cow.as_ref())
    })
}

// ─── Member access (Select) ────────────────────────────────────────────

fn compile_select(sel: &SelectExpr, reserved: &[&str]) -> ValueClosure {
    let operand_fn = compile_expr(reserved, &sel.operand.expr);
    let field = sel.field.clone();
    let is_test = sel.test;

    Box::new(move |ctx| {
        let operand = operand_fn(ctx)?;
        let key: CelString = field.as_str().into();

        if is_test {
            let cow = to_cow(&operand);
            let exists = match cow.as_indexer() {
                Some(indexer) => indexer.get(&key).is_ok(),
                None => false,
            };
            Ok(Value::Bool(exists))
        } else {
            let cow = to_cow(&operand);
            let indexer = cow
                .as_indexer()
                .ok_or(ExecutionError::NoSuchOverload)?;
            let result = indexer.get(&key)?;
            Value::try_from(result.as_ref())
        }
    })
}

// ─── Call / function dispatch ──────────────────────────────────────────

fn compile_call(call: &crate::common::ast::CallExpr, reserved: &[&str]) -> ValueClosure {
    let name_str = call.func_name.clone();
    let name = name_str.as_str();
    let arity = call.args.len();

    // ── Ternary (conditional) ──
    if arity == 3 && name == operators::CONDITIONAL {
        let cond = compile_expr(reserved, &call.args[0].expr);
        let t = compile_expr(reserved, &call.args[1].expr);
        let f = compile_expr(reserved, &call.args[2].expr);
        return Box::new(move |ctx| {
            let c = cond(ctx)?;
            match c {
                Value::Bool(true) => t(ctx),
                Value::Bool(false) => f(ctx),
                _ => Err(ExecutionError::NoSuchOverload),
            }
        });
    }

    // ── Unary operators ──
    if arity == 1 {
        let inner = compile_expr(reserved, &call.args[0].expr);
        match name {
            operators::LOGICAL_NOT => {
                return Box::new(move |ctx| {
                    let val = inner(ctx)?;
                    match val {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        _ => Err(ExecutionError::UnsupportedUnaryOperator("!", val)),
                    }
                });
            }
            operators::NEGATE => {
                return Box::new(move |ctx| {
                    let val = inner(ctx)?;
                    match val {
                        Value::Int(i) => Ok(Value::Int(i.wrapping_neg())),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => {
                            let cow = to_cow(&val);
                            let negator = cow
                                .as_negator()
                                .ok_or(ExecutionError::NoSuchOverload)?;
                            let result = negator.negate()?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::NOT_STRICTLY_FALSE => {
                return Box::new(move |ctx| {
                    let val = inner(ctx)?;
                    match val {
                        Value::Bool(false) | Value::Null => Ok(Value::Bool(false)),
                        _ => Ok(Value::Bool(true)),
                    }
                });
            }
            _ => {}
        }
    }

    // ── Binary operators ──
    if arity == 2 {
        match name {
            operators::LOGICAL_OR => {
                let left = compile_expr(reserved, &call.args[0].expr);
                let right = compile_expr(reserved, &call.args[1].expr);
                return Box::new(move |ctx| {
                    let l = left(ctx);
                    // CEL try_bool: extract bool or treat as error
                    match l {
                        Ok(Value::Bool(true)) => return Ok(Value::Bool(true)),
                        Ok(Value::Bool(false)) => {
                            // Left false: return right (which must be bool)
                            match right(ctx) {
                                Ok(v @ Value::Bool(_)) => return Ok(v),
                                Ok(_) => return Err(ExecutionError::NoSuchOverload),
                                Err(e) => return Err(e),
                            }
                        }
                        Ok(_) | Err(_) => {
                            // Left error or non-bool: short-circuit if right is true
                            let l_err = l.err();
                            match right(ctx) {
                                Ok(Value::Bool(true)) => return Ok(Value::Bool(true)),
                                Ok(Value::Bool(false)) => {
                                    return Err(l_err.unwrap_or(ExecutionError::NoSuchOverload));
                                }
                                Ok(_) => return Err(ExecutionError::NoSuchOverload),
                                Err(e) => return Err(e),
                            }
                        }
                    }
                });
            }
            operators::LOGICAL_AND => {
                let left = compile_expr(reserved, &call.args[0].expr);
                let right = compile_expr(reserved, &call.args[1].expr);
                return Box::new(move |ctx| {
                    let l = left(ctx);
                    // CEL try_bool: extract bool or treat as error
                    match l {
                        Ok(Value::Bool(false)) => return Ok(Value::Bool(false)),
                        Ok(Value::Bool(true)) => {
                            // Left true: return right (which must be bool)
                            match right(ctx) {
                                Ok(v @ Value::Bool(_)) => return Ok(v),
                                Ok(_) => return Err(ExecutionError::NoSuchOverload),
                                Err(e) => return Err(e),
                            }
                        }
                        Ok(_) | Err(_) => {
                            // Left error or non-bool: short-circuit if right is false
                            let l_err = l.err();
                            match right(ctx) {
                                Ok(Value::Bool(false)) => return Ok(Value::Bool(false)),
                                Ok(Value::Bool(true)) => {
                                    return Err(l_err.unwrap_or(ExecutionError::NoSuchOverload));
                                }
                                Ok(_) => return Err(ExecutionError::NoSuchOverload),
                                Err(e) => return Err(e),
                            }
                        }
                    }
                });
            }
            _ => {}
        }

        let lhs_fn = compile_expr(reserved, &call.args[0].expr);
        let rhs_fn = compile_expr(reserved, &call.args[1].expr);

        match name {
            operators::EQUALS => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    Ok(Value::Bool(lhs == rhs))
                });
            }
            operators::NOT_EQUALS => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    Ok(Value::Bool(lhs != rhs))
                });
            }
            operators::LESS => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    let result = match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => l < r,
                        (Value::UInt(l), Value::UInt(r)) => l < r,
                        (Value::Float(l), Value::Float(r)) => {
                            if l.is_nan() || r.is_nan() {
                                return Err(ExecutionError::ValuesNotComparable(lhs, rhs));
                            }
                            l < r
                        }
                        (Value::String(l), Value::String(r)) => l.as_ref() < r.as_ref(),
                        (Value::Bool(l), Value::Bool(r)) => l < r,
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let ord = lcow
                                .as_comparer()
                                .ok_or(ExecutionError::NoSuchOverload)?
                                .compare(rcow.as_ref())?;
                            ord == std::cmp::Ordering::Less
                        }
                    };
                    Ok(Value::Bool(result))
                });
            }
            operators::LESS_EQUALS => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    let result = match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => l <= r,
                        (Value::UInt(l), Value::UInt(r)) => l <= r,
                        (Value::Float(l), Value::Float(r)) => {
                            if l.is_nan() || r.is_nan() {
                                return Err(ExecutionError::ValuesNotComparable(lhs, rhs));
                            }
                            l <= r
                        }
                        (Value::String(l), Value::String(r)) => l.as_ref() <= r.as_ref(),
                        (Value::Bool(l), Value::Bool(r)) => l <= r,
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let ord = lcow
                                .as_comparer()
                                .ok_or(ExecutionError::NoSuchOverload)?
                                .compare(rcow.as_ref())?;
                            ord != std::cmp::Ordering::Greater
                        }
                    };
                    Ok(Value::Bool(result))
                });
            }
            operators::GREATER => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    let result = match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => l > r,
                        (Value::UInt(l), Value::UInt(r)) => l > r,
                        (Value::Float(l), Value::Float(r)) => {
                            if l.is_nan() || r.is_nan() {
                                return Err(ExecutionError::ValuesNotComparable(lhs, rhs));
                            }
                            l > r
                        }
                        (Value::String(l), Value::String(r)) => l.as_ref() > r.as_ref(),
                        (Value::Bool(l), Value::Bool(r)) => l > r,
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let ord = lcow
                                .as_comparer()
                                .ok_or(ExecutionError::NoSuchOverload)?
                                .compare(rcow.as_ref())?;
                            ord == std::cmp::Ordering::Greater
                        }
                    };
                    Ok(Value::Bool(result))
                });
            }
            operators::GREATER_EQUALS => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    let result = match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => l >= r,
                        (Value::UInt(l), Value::UInt(r)) => l >= r,
                        (Value::Float(l), Value::Float(r)) => {
                            if l.is_nan() || r.is_nan() {
                                return Err(ExecutionError::ValuesNotComparable(lhs, rhs));
                            }
                            l >= r
                        }
                        (Value::String(l), Value::String(r)) => l.as_ref() >= r.as_ref(),
                        (Value::Bool(l), Value::Bool(r)) => l >= r,
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let ord = lcow
                                .as_comparer()
                                .ok_or(ExecutionError::NoSuchOverload)?
                                .compare(rcow.as_ref())?;
                            ord != std::cmp::Ordering::Less
                        }
                    };
                    Ok(Value::Bool(result))
                });
            }
            operators::ADD => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => {
                            let res = l
                                .checked_add(*r)
                                .ok_or(ExecutionError::Overflow("add", lhs, rhs))?;
                            Ok(Value::Int(res))
                        }
                        (Value::UInt(l), Value::UInt(r)) => {
                            let res = l
                                .checked_add(*r)
                                .ok_or(ExecutionError::Overflow("add", lhs, rhs))?;
                            Ok(Value::UInt(res))
                        }
                        (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l + r)),
                        (Value::String(l), Value::String(r)) => {
                            let mut s = String::with_capacity(l.len() + r.len());
                            s.push_str(l);
                            s.push_str(r);
                            Ok(Value::String(Arc::from(s.into_boxed_str())))
                        }
                        (Value::List(l), Value::List(r)) => {
                            let mut merged = Vec::with_capacity(l.len() + r.len());
                            merged.extend_from_slice(l);
                            merged.extend_from_slice(r);
                            Ok(Value::List(Arc::new(merged)))
                        }
                        #[cfg(feature = "chrono")]
                        (Value::Duration(l), Value::Duration(r)) => {
                            Ok(Value::Duration(*l + *r))
                        }
                        #[cfg(feature = "chrono")]
                        (Value::Timestamp(l), Value::Duration(r)) => {
                            let result = *l + *r;
                            if result > *crate::common::types::timestamp::MAX_TIMESTAMP
                                || result < *crate::common::types::timestamp::MIN_TIMESTAMP
                            {
                                return Err(ExecutionError::Overflow("add", lhs, rhs));
                            }
                            Ok(Value::Timestamp(result))
                        }
                        #[cfg(feature = "chrono")]
                        (Value::Duration(l), Value::Timestamp(r)) => {
                            // chrono: DateTime + Duration, not Duration + DateTime
                            let result = *r + *l;
                            if result > *crate::common::types::timestamp::MAX_TIMESTAMP
                                || result < *crate::common::types::timestamp::MIN_TIMESTAMP
                            {
                                return Err(ExecutionError::Overflow("add", lhs, rhs));
                            }
                            Ok(Value::Timestamp(result))
                        }
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let adder = lcow
                                .as_adder()
                                .ok_or(ExecutionError::UnsupportedBinaryOperator(
                                    "add",
                                    lhs,
                                    rhs,
                                ))?;
                            let result = adder.add(rcow.as_ref())?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::SUBSTRACT => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => {
                            let res = l
                                .checked_sub(*r)
                                .ok_or(ExecutionError::Overflow("sub", lhs, rhs))?;
                            Ok(Value::Int(res))
                        }
                        (Value::UInt(l), Value::UInt(r)) => {
                            let res = l
                                .checked_sub(*r)
                                .ok_or(ExecutionError::Overflow("sub", lhs, rhs))?;
                            Ok(Value::UInt(res))
                        }
                        (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l - r)),
                        #[cfg(feature = "chrono")]
                        (Value::Timestamp(l), Value::Timestamp(r)) => {
                            Ok(Value::Duration(*l - *r))
                        }
                        #[cfg(feature = "chrono")]
                        (Value::Timestamp(l), Value::Duration(r)) => {
                            let result = *l - *r;
                            if result > *crate::common::types::timestamp::MAX_TIMESTAMP
                                || result < *crate::common::types::timestamp::MIN_TIMESTAMP
                            {
                                return Err(ExecutionError::Overflow("sub", lhs, rhs));
                            }
                            Ok(Value::Timestamp(result))
                        }
                        #[cfg(feature = "chrono")]
                        (Value::Duration(l), Value::Duration(r)) => {
                            Ok(Value::Duration(*l - *r))
                        }
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let sub = lcow
                                .as_subtractor()
                                .ok_or(ExecutionError::UnsupportedBinaryOperator(
                                    "sub",
                                    lhs,
                                    rhs,
                                ))?;
                            let result = sub.sub(rcow.as_ref())?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::MULTIPLY => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match (&lhs, &rhs) {
                        (Value::Int(l), Value::Int(r)) => {
                            let res = l
                                .checked_mul(*r)
                                .ok_or(ExecutionError::Overflow("mul", lhs, rhs))?;
                            Ok(Value::Int(res))
                        }
                        (Value::UInt(l), Value::UInt(r)) => {
                            let res = l
                                .checked_mul(*r)
                                .ok_or(ExecutionError::Overflow("mul", lhs, rhs))?;
                            Ok(Value::UInt(res))
                        }
                        (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l * r)),
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let mul = lcow
                                .as_multiplier()
                                .ok_or(ExecutionError::UnsupportedBinaryOperator(
                                    "mul",
                                    lhs,
                                    rhs,
                                ))?;
                            let result = mul.mul(rcow.as_ref())?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::DIVIDE => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match (&lhs, &rhs) {
                        (Value::Int(_), Value::Int(0)) => {
                            Err(ExecutionError::DivisionByZero(lhs))
                        }
                        (Value::Int(l), Value::Int(r)) => {
                            let res = l
                                .checked_div(*r)
                                .ok_or(ExecutionError::Overflow("div", lhs, rhs))?;
                            Ok(Value::Int(res))
                        }
                        (Value::UInt(_), Value::UInt(0)) => {
                            Err(ExecutionError::DivisionByZero(lhs))
                        }
                        (Value::UInt(l), Value::UInt(r)) => Ok(Value::UInt(l / r)),
                        (Value::Float(l), Value::Float(r)) => Ok(Value::Float(l / r)),
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let div = lcow
                                .as_divider()
                                .ok_or(ExecutionError::UnsupportedBinaryOperator(
                                    "div",
                                    lhs,
                                    rhs,
                                ))?;
                            let result = div.div(rcow.as_ref())?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::MODULO => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match (&lhs, &rhs) {
                        (Value::Int(_), Value::Int(0)) => {
                            Err(ExecutionError::RemainderByZero(lhs))
                        }
                        (Value::Int(l), Value::Int(r)) => {
                            let res = l
                                .checked_rem(*r)
                                .ok_or(ExecutionError::Overflow("rem", lhs, rhs))?;
                            Ok(Value::Int(res))
                        }
                        (Value::UInt(_), Value::UInt(0)) => {
                            Err(ExecutionError::RemainderByZero(lhs))
                        }
                        (Value::UInt(l), Value::UInt(r)) => Ok(Value::UInt(l % r)),
                        _ => {
                            let lcow = to_cow(&lhs);
                            let rcow = to_cow(&rhs);
                            let m = lcow
                                .as_modder()
                                .ok_or(ExecutionError::UnsupportedBinaryOperator(
                                    "rem",
                                    lhs,
                                    rhs,
                                ))?;
                            let result = m.modulo(rcow.as_ref())?;
                            Value::try_from(result.as_ref())
                        }
                    }
                });
            }
            operators::IN => {
                return Box::new(move |ctx| {
                    let lhs = lhs_fn(ctx)?;
                    let rhs = rhs_fn(ctx)?;
                    match &rhs {
                        Value::List(list) => Ok(Value::Bool(list.contains(&lhs))),
                        Value::Map(map) => {
                            let key: Result<Key, ExecutionError> = match &lhs {
                                Value::String(s) => Ok(Key::String(Arc::clone(s))),
                                Value::Int(i) => Ok(Key::Int(*i)),
                                Value::UInt(u) => Ok(Key::Uint(*u)),
                                Value::Bool(b) => Ok(Key::Bool(*b)),
                                _ => Err(ExecutionError::UnsupportedMapIndex(lhs)),
                            };
                            match key {
                                Ok(k) => Ok(Value::Bool(map.map.contains_key(&k))),
                                Err(e) => Err(e),
                            }
                        }
                        _ => {
                            let rcow = to_cow(&rhs);
                            let container = rcow
                                .as_container()
                                .ok_or(ExecutionError::NoSuchOverload)?;
                            let lcow = to_cow(&lhs);
                            Ok(Value::Bool(container.contains(lcow.as_ref())?))
                        }
                    }
                });
            }
            operators::INDEX | operators::OPT_INDEX => {
                let is_optional_op = name == operators::OPT_INDEX;
                return Box::new(move |ctx| {
                    let container = lhs_fn(ctx)?;
                    let index = rhs_fn(ctx)?;

                    let (container, is_opt) = match container {
                        Value::Opaque(ref opaque) => {
                            if let Some(opt) = opaque.downcast_ref::<OptionalValue>() {
                                match opt.inner() {
                                    Some(inner) => (inner.clone(), true),
                                    None => {
                                        return Ok(Value::Opaque(Arc::new(
                                            OptionalValue::none(),
                                        )))
                                    }
                                }
                            } else {
                                (container, false)
                            }
                        }
                        _ => (container, false),
                    };

                    let result = match (&container, &index) {
                        (Value::List(list), Value::Int(i)) => {
                            let idx = if *i < 0 {
                                let len = list.len() as i64;
                                (len + i) as usize
                            } else {
                                *i as usize
                            };
                            list.get(idx).cloned()
                                .ok_or(ExecutionError::IndexOutOfBounds(index))
                        }
                        (Value::Map(map), _) => {
                            let key: Result<Key, ExecutionError> = match &index {
                                Value::String(s) => Ok(Key::String(Arc::clone(s))),
                                Value::Int(i) => Ok(Key::Int(*i)),
                                Value::UInt(u) => Ok(Key::Uint(*u)),
                                Value::Bool(b) => Ok(Key::Bool(*b)),
                                _ => Err(ExecutionError::UnsupportedMapIndex(index)),
                            };
                            match key {
                                Ok(k) => map.get(&k).cloned()
                                    .ok_or_else(|| ExecutionError::NoSuchKey(
                                        Arc::new(format!("{:?}", k))
                                    )),
                                Err(e) => Err(e),
                            }
                        }
                        _ => {
                            let cow = to_cow(&container);
                            let indexer = cow
                                .as_indexer()
                                .ok_or(ExecutionError::NoSuchOverload)?;
                            let idx_cow = to_cow(&index);
                            indexer.get(idx_cow.as_ref())
                                .and_then(|v| Value::try_from(v.as_ref()).map_err(|_| ExecutionError::NoSuchOverload))
                        }
                    };

                    if is_opt || is_optional_op {
                        Ok(match result {
                            Ok(v) => Value::Opaque(Arc::new(OptionalValue::of(v))),
                            Err(_) => Value::Opaque(Arc::new(OptionalValue::none())),
                        })
                    } else {
                        result
                    }
                });
            }
            operators::OPT_SELECT => {
                return Box::new(move |ctx| {
                    let operand = lhs_fn(ctx)?;
                    let field_val = rhs_fn(ctx)?;
                    let field_str = match &field_val {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err(ExecutionError::function_error(
                                "_?._",
                                "field must be string",
                            ))
                        }
                    };
                    let key: CelString = field_str.as_ref().into();

                    if let Value::Opaque(ref opaque) = operand {
                        if let Some(opt) = opaque.downcast_ref::<OptionalValue>() {
                            return match opt.inner() {
                                Some(inner) => {
                                    let cow = to_cow(inner);
                                    match cow.as_indexer() {
                                        Some(indexer) => match indexer.get(&key) {
                                            Ok(v) => Ok(Value::Opaque(Arc::new(
                                                OptionalValue::of(
                                                    Value::try_from(v.as_ref()).unwrap_or(Value::Null),
                                                ),
                                            ))),
                                            Err(_) => Ok(Value::Opaque(Arc::new(
                                                OptionalValue::none(),
                                            ))),
                                        },
                                        None => Ok(Value::Opaque(Arc::new(OptionalValue::none()))),
                                    }
                                }
                                None => Ok(Value::Opaque(Arc::new(OptionalValue::none()))),
                            };
                        }
                    }

                    let cow = to_cow(&operand);
                    let indexer = cow
                        .as_indexer()
                        .ok_or(ExecutionError::NoSuchOverload)?;
                    let val = indexer.get(&key)?;
                    Ok(Value::Opaque(Arc::new(
                        OptionalValue::of(Value::try_from(val.as_ref()).unwrap_or(Value::Null)),
                    )))
                });
            }
            _ => {}
        }
    }

    // ── General function dispatch (non-operator) ──

    let arg_fns = compile_arg_list(reserved, &call.args);
    let func_name = call.func_name.clone();
    let resolved_op = call.resolved_op;

    match &call.target {
        None => {
            if let Some(op) = resolved_op {
                return Box::new(move |ctx| {
                    let mut args = Vec::with_capacity(arg_fns.len());
                    for arg_fn in &arg_fns {
                        let val = arg_fn(ctx)?;
                        args.push(to_cow(&val));
                    }
                    let result = op(args)?;
                    Value::try_from(result.as_ref())
                });
            }

            let name = func_name.clone();
            Box::new(move |ctx| {
                let mut args = Vec::with_capacity(arg_fns.len());
                for arg_fn in &arg_fns {
                    let val = arg_fn(ctx)?;
                    args.push(to_cow(&val));
                }

                if let Some(op) = ctx.env().find_overload(&name, &args) {
                    let result = op(args)?;
                    return Value::try_from(result.as_ref());
                }

                let func = ctx
                    .get_function(&name)
                    .ok_or_else(|| ExecutionError::UndeclaredReference(Arc::new(name.clone())))?;

                let mut fn_ctx = FunctionContext::new(&name, None, ctx, args);
                let v = (func)(&mut fn_ctx)?;
                Ok(v)
            })
        }
        Some(target) => {
            let target_fn = compile_expr(reserved, &target.expr);

            match &target.expr {
                Expr::Ident(ref prefix) => {
                    let qualified = format!("{}.{}", prefix, func_name);

                    if let Some(op) = resolved_op {
                        return Box::new(move |ctx| {
                            let mut args = Vec::with_capacity(1 + arg_fns.len());
                            args.push(to_cow(&target_fn(ctx)?));
                            for arg_fn in &arg_fns {
                                args.push(to_cow(&arg_fn(ctx)?));
                            }
                            let result = op(args)?;
                            Value::try_from(result.as_ref())
                        });
                    }

                    let qname = qualified.clone();
                    let fname = func_name.clone();
                    Box::new(move |ctx| {
                        let mut args = Vec::with_capacity(arg_fns.len());
                        for arg_fn in &arg_fns {
                            let val = arg_fn(ctx)?;
                            args.push(to_cow(&val));
                        }

                        if let Some(op) = ctx.env().find_overload(&qname, &args) {
                            let result = op(args)?;
                            return Value::try_from(result.as_ref());
                        }

                        let func = ctx.get_function(&qname);
                        if let Some(func) = func {
                            let mut fn_ctx =
                                FunctionContext::new(&fname, None, ctx, args);
                            let v = (func)(&mut fn_ctx)?;
                            return Ok(v);
                        }

                        let target_val = target_fn(ctx)?;
                        let mut args = Vec::with_capacity(1 + arg_fns.len());
                        args.push(to_cow(&target_val));
                        for arg_fn in &arg_fns {
                            args.push(to_cow(&arg_fn(ctx)?));
                        }

                        if let Some(op) = ctx.env().find_member_overload(&fname, &args) {
                            let result = op(args)?;
                            return Value::try_from(result.as_ref());
                        }

                        let func = ctx
                            .get_function(&fname)
                            .ok_or_else(|| {
                                ExecutionError::UndeclaredReference(Arc::new(fname.clone()))
                            })?;

                        let target_val = target_fn(ctx)?;
                        let mut fn_ctx = FunctionContext::new(
                            &fname,
                            Some(to_cow(&target_val)),
                            ctx,
                            args.into_iter().skip(1).collect(),
                        );
                        let v = (func)(&mut fn_ctx)?;
                        Ok(v)
                    })
                }
                _ => {
                    let fname = func_name.clone();
                    Box::new(move |ctx| {
                        let target_val = target_fn(ctx)?;
                        let mut args = Vec::with_capacity(1 + arg_fns.len());
                        args.push(to_cow(&target_val));
                        for arg_fn in &arg_fns {
                            let val = arg_fn(ctx)?;
                            args.push(to_cow(&val));
                        }

                        if let Some(op) =
                            ctx.env().find_member_overload(&fname, &args)
                        {
                            let result = op(args)?;
                            return Value::try_from(result.as_ref());
                        }

                        let func = ctx.get_function(&fname).ok_or_else(|| {
                            ExecutionError::UndeclaredReference(Arc::new(fname.clone()))
                        })?;

                        let target_val = target_fn(ctx)?;
                        let mut fn_ctx = FunctionContext::new(
                            &fname,
                            Some(to_cow(&target_val)),
                            ctx,
                            args.into_iter().skip(1).collect(),
                        );
                        let v = (func)(&mut fn_ctx)?;
                        Ok(v)
                    })
                }
            }
        }
    }
}

// ─── List literal ──────────────────────────────────────────────────────

fn compile_list(
    list: &crate::common::ast::ListExpr,
    reserved: &[&str],
) -> ValueClosure {
    let elem_fns: Vec<ValueClosure> = list
        .elements
        .iter()
        .map(|e| compile_expr(reserved, &e.expr))
        .collect();
    let optional_indices = list.optional_indices.clone();

    Box::new(move |ctx| {
        let mut result = Vec::with_capacity(elem_fns.len());
        for (idx, elem_fn) in elem_fns.iter().enumerate() {
            let val = elem_fn(ctx)?;
            if optional_indices.contains(&idx) {
                if let Value::Opaque(ref opaque) = val {
                    if let Some(opt) = opaque.downcast_ref::<OptionalValue>() {
                        if let Some(inner) = opt.inner() {
                            result.push(inner.clone());
                        }
                        continue;
                    }
                }
                result.push(val);
            } else {
                result.push(val);
            }
        }
        Ok(Value::List(Arc::new(result)))
    })
}

// ─── Map literal ───────────────────────────────────────────────────────

fn compile_map(
    map: &crate::common::ast::MapExpr,
    reserved: &[&str],
) -> ValueClosure {
    let mut key_val_pairs: Vec<(ValueClosure, ValueClosure, bool)> = Vec::new();
    for entry in &map.entries {
        match &entry.expr {
            EntryExpr::MapEntry(e) => {
                let key_fn = compile_expr(reserved, &e.key.expr);
                let val_fn = compile_expr(reserved, &e.value.expr);
                key_val_pairs.push((key_fn, val_fn, e.optional));
            }
            EntryExpr::StructField(_) => {}
        }
    }

    Box::new(move |ctx| {
        let mut hmap: HashMap<Key, Value> = HashMap::with_capacity(key_val_pairs.len());
        for (key_fn, val_fn, is_optional) in &key_val_pairs {
            let key_val = key_fn(ctx)?;
            let val = val_fn(ctx)?;

            let key = match &key_val {
                Value::String(s) => Key::String(Arc::clone(s)),
                Value::Int(i) => Key::Int(*i),
                Value::UInt(u) => Key::Uint(*u),
                Value::Bool(b) => Key::Bool(*b),
                _ => return Err(ExecutionError::UnsupportedKeyType(key_val)),
            };

            if *is_optional {
                if let Value::Opaque(ref opaque) = val {
                    if let Some(opt) = opaque.downcast_ref::<OptionalValue>() {
                        if let Some(inner) = opt.inner() {
                            hmap.insert(key, inner.clone());
                        }
                        continue;
                    }
                }
            }
            hmap.insert(key, val);
        }
        Ok(Value::Map(Map {
            map: Arc::new(hmap),
        }))
    })
}

// ─── Comprehension ─────────────────────────────────────────────────────

fn compile_comprehension(
    comp: &crate::common::ast::ComprehensionExpr,
    reserved: &[&str],
) -> ValueClosure {
    let iter_var = comp.iter_var.clone();
    let iter_var2 = comp.iter_var2.clone();
    let accu_var = comp.accu_var.clone();

    let iter_range_fn = compile_expr(reserved, &comp.iter_range.expr);
    let accu_init_fn = compile_expr(reserved, &comp.accu_init.expr);

    let loop_cond_expr = comp.loop_cond.expr.clone();
    let loop_step_expr = comp.loop_step.expr.clone();
    let result_expr = comp.result.expr.clone();

    // Clone reserved into owned strings so the closure is 'static
    let owned_reserved: Vec<String> = reserved.iter().map(|s| s.to_string()).collect();

    Box::new(move |ctx| {
        let init_val = accu_init_fn(ctx)?;

        let iter_val = iter_range_fn(ctx)?;
        let is_map_iter: bool;
        let items: Vec<Value> = match &iter_val {
            Value::List(list) => {
                is_map_iter = false;
                list.iter().cloned().collect()
            }
            Value::Map(map) => {
                is_map_iter = true;
                map
                    .map
                    .iter()
                    .map(|(k, v)| {
                        Value::List(Arc::new(vec![k.clone().into(), v.clone()]))
                    })
                    .collect()
            }
            _ => return Err(ExecutionError::NoSuchOverload),
        };

        let mut accu = init_val;

        // Convert owned_reserved back to &str for compile_expr calls
        let r_names: Vec<&str> = owned_reserved.iter().map(|s| s.as_str()).collect();

        for item in &items {
            let mut inner = ctx.new_inner_scope();

            match &iter_var2 {
                Some(var2) => match item {
                    Value::List(pair) if pair.len() >= 2 => {
                        let cow1: Box<dyn Val> = pair[0].clone().try_into()
                            .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
                        let cow2: Box<dyn Val> = pair[1].clone().try_into()
                            .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
                        inner.add_variable_as_val(&iter_var, cow1);
                        inner.add_variable_as_val(var2, cow2);
                    }
                    _ => {
                        let cow: Box<dyn Val> = item.clone().try_into()
                            .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
                        inner.add_variable_as_val(&iter_var, cow);
                        inner.add_variable_as_val(var2, Box::new(CelNull) as Box<dyn Val>);
                    }
                },
                None => {
                    let key_val = if is_map_iter {
                        match item {
                            Value::List(pair) if pair.len() >= 2 => pair[0].clone(),
                            _ => item.clone(),
                        }
                    } else {
                        item.clone()
                    };
                    let cow: Box<dyn Val> = key_val
                        .clone()
                        .try_into()
                        .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
                    inner.add_variable_as_val(&iter_var, cow);
                }
            }

            let accu_cow: Box<dyn Val> = accu.clone().try_into()
                .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
            inner.add_variable_as_val(&accu_var, accu_cow);

            let cond_fn = compile_expr(&r_names, &loop_cond_expr);
            let cond_val = cond_fn(&inner)?;
            let should_continue = match cond_val {
                Value::Bool(b) => b,
                _ => break,
            };
            if !should_continue {
                break;
            }

            let step_fn = compile_expr(&r_names, &loop_step_expr);
            accu = step_fn(&inner)?;
        }

        let mut final_ctx = ctx.new_inner_scope();
        let final_accu_cow: Box<dyn Val> = accu.clone().try_into()
            .unwrap_or_else(|_| Box::new(CelNull) as Box<dyn Val>);
        final_ctx.add_variable_as_val(&accu_var, final_accu_cow);
        let result_fn = compile_expr(&r_names, &result_expr);
        result_fn(&final_ctx)
    })
}

// ─── Struct literal ────────────────────────────────────────────────────

#[cfg(feature = "structs")]
fn compile_struct(
    strct: &crate::common::ast::StructExpr,
    reserved: &[&str],
) -> ValueClosure {
    use std::collections::BTreeMap;

    let type_name = strct.type_name.clone();

    // Compile each field value expression into a closure at compile time.
    // (field_name, optional_flag, compiled_closure)
    let mut field_fns: Vec<(String, bool, ValueClosure)> = Vec::new();
    for entry in &strct.entries {
        if let crate::common::ast::EntryExpr::StructField(sf) = &entry.expr {
            let field_name = sf.field.clone();
            let optional = sf.optional;
            let val_fn = compile_expr(reserved, &sf.value.expr);
            field_fns.push((field_name, optional, val_fn));
        }
        // `MapEntry` in a struct context is invalid CEL — skip silently.
    }

    Box::new(move |ctx| {
        // Resolve the struct definition from the environment.
        let struct_def = ctx
            .env()
            .find_struct(&type_name)
            .ok_or_else(|| ExecutionError::UnexpectedType {
                got: type_name.clone(),
                want: String::from("known struct"),
            })?;

        // Evaluate field values and collect into BTreeMap.
        let mut fields: BTreeMap<String, std::borrow::Cow<dyn Val>> = BTreeMap::new();
        for (field_name, optional, val_fn) in &field_fns {
            let val = val_fn(ctx)?;
            // `optional` field that evaluates to Null → skip, use StructDef default.
            if *optional && val == Value::Null {
                continue;
            }
            fields.insert(field_name.clone(), to_cow(&val));
        }

        // Build and type-check via StructDef (handles defaults & error messages).
        let cel_struct = struct_def.new_struct(fields)?;
        Ok(Value::Struct(Arc::new(cel_struct)))
    })
}

#[cfg(not(feature = "structs"))]
fn compile_struct(
    strct: &crate::common::ast::StructExpr,
    _reserved: &[&str],
) -> ValueClosure {
    let name = strct.type_name.clone();
    Box::new(move |_| {
        Err(ExecutionError::InternalError(format!(
            "Found struct {}, feature not enabled!",
            name
        )))
    })
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::Map;
    use crate::Program;

    fn test_compile(script: &str, ctx: Option<Context>) -> Result<Value, ExecutionError> {
        let program = Program::compile(script).expect("parse failed");
        let closure = compile_expression(&program.expression(), &[]);
        let ctx = ctx.unwrap_or_default();
        closure(&ctx)
    }

    #[test]
    fn test_literal_int() {
        let result = test_compile("42", None).unwrap();
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn test_literal_string() {
        let result = test_compile("\"hello\"", None).unwrap();
        assert_eq!(result, Value::String(Arc::from("hello")));
    }

    #[test]
    fn test_literal_bool() {
        let result = test_compile("true", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_literal_null() {
        let result = test_compile("null", None).unwrap();
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn test_identifier() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("x", 42i64);
        let result = test_compile("x", Some(ctx)).unwrap();
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn test_undeclared_identifier() {
        let result = test_compile("undefined_var", None);
        assert!(matches!(
            result,
            Err(ExecutionError::UndeclaredReference(_))
        ));
    }

    #[test]
    fn test_add_ints() {
        let result = test_compile("1 + 2", None).unwrap();
        assert_eq!(result, Value::Int(3));
    }

    #[test]
    fn test_add_overflow() {
        let result = test_compile(&format!("{} + 1", i64::MAX), None);
        assert!(matches!(result, Err(ExecutionError::Overflow(..))));
    }

    #[test]
    fn test_sub_ints() {
        let result = test_compile("5 - 3", None).unwrap();
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn test_mul_ints() {
        let result = test_compile("3 * 4", None).unwrap();
        assert_eq!(result, Value::Int(12));
    }

    #[test]
    fn test_div_ints() {
        let result = test_compile("10 / 3", None).unwrap();
        assert_eq!(result, Value::Int(3));
    }

    #[test]
    fn test_div_by_zero() {
        let result = test_compile("1 / 0", None);
        assert!(matches!(result, Err(ExecutionError::DivisionByZero(_))));
    }

    #[test]
    fn test_mod_ints() {
        let result = test_compile("10 % 3", None).unwrap();
        assert_eq!(result, Value::Int(1));
    }

    #[test]
    fn test_negate_int() {
        let result = test_compile("-42", None).unwrap();
        assert_eq!(result, Value::Int(-42));
    }

    #[test]
    fn test_negate_float() {
        let result = test_compile("-3.14", None).unwrap();
        assert!(matches!(result, Value::Float(v) if (v - -3.14).abs() < 1e-10));
    }

    #[test]
    fn test_not() {
        let result = test_compile("!true", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_logical_and_short_circuit() {
        assert_eq!(
            test_compile("true && true", None).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            test_compile("false && false", None).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            test_compile("false && (1 / 0 == 0)", None).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn test_logical_or_short_circuit() {
        assert_eq!(
            test_compile("true || false", None).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            test_compile("false || false", None).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            test_compile("true || (1 / 0 == 0)", None).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_equals() {
        assert_eq!(test_compile("1 == 1", None).unwrap(), Value::Bool(true));
        assert_eq!(test_compile("1 == 2", None).unwrap(), Value::Bool(false));
        assert_eq!(
            test_compile("\"a\" == \"a\"", None).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn test_not_equals() {
        assert_eq!(test_compile("1 != 2", None).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_comparisons() {
        assert_eq!(test_compile("1 < 2", None).unwrap(), Value::Bool(true));
        assert_eq!(test_compile("2 <= 2", None).unwrap(), Value::Bool(true));
        assert_eq!(test_compile("3 > 2", None).unwrap(), Value::Bool(true));
        assert_eq!(test_compile("3 >= 3", None).unwrap(), Value::Bool(true));
    }

    #[test]
    fn test_string_concat() {
        let result = test_compile("\"hello\" + \" \" + \"world\"", None).unwrap();
        assert_eq!(result, Value::String(Arc::from("hello world")));
    }

    #[test]
    fn test_list_concat() {
        let result = test_compile("[1, 2] + [3, 4]", None).unwrap();
        assert_eq!(
            result,
            Value::List(Arc::new(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4),
            ]))
        );
    }

    #[test]
    fn test_list_literal() {
        let result = test_compile("[1, 2, 3]", None).unwrap();
        assert_eq!(
            result,
            Value::List(Arc::new(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
            ]))
        );
    }

    #[test]
    fn test_map_literal() {
        let result = test_compile("{\"a\": 1, \"b\": 2}", None).unwrap();
        let expected: HashMap<Key, Value> = HashMap::from([
            (Key::String(Arc::from("a")), Value::Int(1)),
            (Key::String(Arc::from("b")), Value::Int(2)),
        ]);
        assert_eq!(
            result,
            Value::Map(Map {
                map: Arc::new(expected)
            })
        );
    }

    #[test]
    fn test_nested_select() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value(
            "obj",
            HashMap::from([("inner", HashMap::from([("value", 42i64)]))]),
        );
        let result = test_compile("obj.inner.value", Some(ctx)).unwrap();
        assert_eq!(result, Value::Int(42));
    }

    #[test]
    fn test_select_no_such_key() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("obj", HashMap::from([("a", 1i64)]));
        let result = test_compile("obj.b", Some(ctx));
        assert!(matches!(result, Err(ExecutionError::NoSuchKey(_))));
    }

    #[test]
    fn test_has() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("obj", HashMap::from([("a", 1i64)]));
        let has_a = test_compile("has(obj.a)", Some(ctx)).unwrap();
        assert_eq!(has_a, Value::Bool(true));
        // "obj.a" exists, "obj.b" doesn't
        let mut ctx2 = Context::default();
        ctx2.add_variable_from_value("obj", HashMap::from([("a", 1i64)]));
        let has_b = test_compile("has(obj.b)", Some(ctx2)).unwrap();
        assert_eq!(has_b, Value::Bool(false));
    }

    #[test]
    fn test_list_index() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("arr", vec![10i64, 20, 30]);
        let result = test_compile("arr[1]", Some(ctx)).unwrap();
        assert_eq!(result, Value::Int(20));
    }

    #[test]
    fn test_list_negative_index() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("arr", vec![10i64, 20, 30]);
        let result = test_compile("arr[-1]", Some(ctx)).unwrap();
        assert_eq!(result, Value::Int(30));
    }

    #[test]
    fn test_in_list() {
        let result = test_compile("1 in [1, 2, 3]", None).unwrap();
        assert_eq!(result, Value::Bool(true));
        let result = test_compile("4 in [1, 2, 3]", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_ternary() {
        let result = test_compile("true ? 1 : 2", None).unwrap();
        assert_eq!(result, Value::Int(1));
        let result = test_compile("false ? 1 : 2", None).unwrap();
        assert_eq!(result, Value::Int(2));
    }

    #[test]
    fn test_complex_expression() {
        let mut ctx = Context::default();
        ctx.add_variable_from_value("x", 10i64);
        ctx.add_variable_from_value("y", 20i64);
        let result = test_compile("(x + y) * 2", Some(ctx)).unwrap();
        assert_eq!(result, Value::Int(60));
    }

    // ── Comprehension tests ──

    #[test]
    fn test_comprehension_all_basic() {
        let result = test_compile("[0, 1, 2].all(x, x >= 0)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_comprehension_all_false() {
        let result = test_compile("[0, -1, 2].all(x, x >= 0)", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_comprehension_all_identity() {
        let result = test_compile("[true, true, true].all(x, x)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_comprehension_all_identity_false() {
        let result = test_compile("[true, true, false].all(x, x)", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_comprehension_exists_basic() {
        let result = test_compile("[0, 1, 2].exists(x, x > 1)", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_comprehension_exists_false() {
        let result = test_compile("[0, 1, 2].exists(x, x > 5)", None).unwrap();
        assert_eq!(result, Value::Bool(false));
    }

    #[test]
    fn test_comprehension_matches_literal() {
        let result = test_compile("'abc'.matches('...')", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }

    #[test]
    fn test_comprehension_matches_in_comprehension() {
        let result = test_compile("{'1': 'abc', '2': 'def', '3': 'ghi'}.all(key, key.matches('...'))", None);
        match &result {
            Ok(v) => assert_eq!(*v, Value::Bool(false)),
            Err(e) => panic!("Expected Ok but got Err({:?})", e),
        }
    }

    #[test]
    fn test_comprehension_matches_simple() {
        // Simpler version to debug
        let result = test_compile("['abc'].all(x, x.matches('...'))", None);
        match &result {
            Ok(v) => assert_eq!(*v, Value::Bool(true)),
            Err(e) => panic!("Expected Ok but got Err({:?})", e),
        }
    }

    #[test]
    fn test_contains_in_comprehension() {
        // Test contains which is NOT gated behind regex feature
        let result = test_compile("['abc'].all(x, x.contains('a'))", None).unwrap();
        assert_eq!(result, Value::Bool(true));
    }
}
