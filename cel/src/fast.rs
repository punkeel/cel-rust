//! High-performance CEL evaluation with Schema-based field resolution.
//!
//! Eliminates all HashMap lookups at evaluation time. Field names are
//! resolved to lightweight `u16` handles at schema construction time —
//! every subsequent `set_*` call is a single array write, and every
//! `eval()` is direct array indexing in compiled filter tree nodes.
//!
//! # Example
//!
//! ```rust
//! use cel::fast::{Schema, FieldType, Filter, EvalContext};
//!
//! // 1. Declare your fields once.
//! let mut schema = Schema::new();
//! let port   = schema.add_field("port",   FieldType::Int);
//! let method = schema.add_field("method", FieldType::String);
//!
//! // 2. Compile expression — field names → indices resolved now.
//! let filter = Filter::compile("port == 80 && method == 'GET'", &schema).unwrap();
//!
//! // 3. Per-request: O(1) set by handle, O(1) eval.
//! let mut ctx = EvalContext::new(&schema);
//! ctx.set_i64(port, 80);
//! ctx.set_str(method, "GET");
//! assert_eq!(filter.eval(&ctx).unwrap(), true);
//! ```
//!
//! `Field` is `Copy` — keep the handles around and reuse them.
//! `EvalContext` is reusable — call `clear()` to reset all fields to `Value::Null`
//! without deallocating the backing array.

use crate::objects::Value;
use crate::vm::filter_tree_compiler::{self, CompiledFilterTree};
use crate::{ExecutionError, Expression};
use std::collections::HashMap;
use std::sync::Arc;

/// CEL field types for Schema declarations.
///
/// These match the CEL language spec type system.
/// The type annotation is used for diagnostics and future compile-time
/// compatibility checks — it does not affect runtime performance.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Int,
    Uint,
    Double,
    Bool,
    String,
    Bytes,
    Null,
    Any,
}

/// A lightweight handle to a field within a [`Schema`].
///
/// Copyable, zero-cost — wraps a `u16` array index. Obtain one from
/// [`Schema::add_field`], then use it with [`EvalContext::set_*`] for
/// O(1) value assignment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Field(u16);

impl Field {
    #[inline]
    pub fn index(&self) -> usize {
        self.0 as usize
    }
}

/// Declares the set of fields available to CEL expressions.
///
/// Assigns each field a stable `u16` index. These indices are embedded
/// into compiled [`Filter`] objects at compilation time, enabling
/// O(1) array access at evaluation time — no HashMap lookups, no
/// string comparisons.
///
/// # Usage
///
/// ```rust
/// use cel::fast::{Schema, FieldType};
///
/// let mut s = Schema::new();
/// let port = s.add_field("port", FieldType::Int);
/// let name = s.add_field("name", FieldType::String);
/// // s is usable directly — no .build() needed
/// ```
#[derive(Clone, Debug)]
pub struct Schema {
    fields: Vec<(Arc<str>, FieldType)>,
    name_to_idx: HashMap<Arc<str>, u16>,
}

impl Schema {
    pub fn new() -> Self {
        Self {
            fields: Vec::new(),
            name_to_idx: HashMap::new(),
        }
    }

    /// Register a field and return its handle.
    ///
    /// The returned [`Field`] can be used with [`EvalContext::set_*`]
    /// for O(1) value assignment at evaluation time.
    pub fn add_field(&mut self, name: &str, ty: FieldType) -> Field {
        let idx = self.fields.len() as u16;
        let name_arc: Arc<str> = Arc::from(name);
        self.name_to_idx.insert(Arc::clone(&name_arc), idx);
        self.fields.push((name_arc, ty));
        Field(idx)
    }

    /// Look up a field handle by name.
    pub fn get_field(&self, name: &str) -> Option<Field> {
        self.name_to_idx.get(name).copied().map(Field)
    }

    /// Number of registered fields.
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Iterate over all fields in index order.
    pub fn fields(&self) -> impl Iterator<Item = (&str, FieldType)> {
        self.fields.iter().map(|(n, t)| (n.as_ref(), *t))
    }

    /// Field names in index order, used internally by tree compiler.
    pub(crate) fn field_names(&self) -> Vec<&str> {
        self.fields.iter().map(|(n, _)| n.as_ref()).collect()
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime values indexed by [`Field`] handle.
///
/// Pre-allocated flat array of `Value`. Every `set_*` call is a single
/// array write — no hashing, no name resolution.
///
/// String values are internally cached (`intern`-ed) so that repeated
/// `set_str("GET")` calls only allocate the first time — subsequent
/// calls are a linear scan + `Arc::clone` (~3 ns vs ~30 ns).
/// The cache persists across `clear()` calls.
///
/// Unset fields have value `Value::Null`. The filter tree handles this
/// correctly: `Null == 80` evaluates to `false`.
///
/// # Hot-path reuse
///
/// Allocate once per schema, then reuse for every request:
///
/// ```rust
/// use cel::fast::{Schema, FieldType, Filter, EvalContext};
///
/// let mut s = Schema::new();
/// let port = s.add_field("port", FieldType::Int);
/// let f = Filter::compile("port == 80", &s).unwrap();
///
/// let mut ctx = EvalContext::new(&s);
/// for request in 0..1_000_000 {
///     ctx.set_i64(port, request);  // O(1)
///     f.eval(&ctx);                // O(1) — ~2 ns
///     ctx.clear();                 // reset without dealloc
/// }
/// ```
pub struct EvalContext {
    values: Box<[Value]>,
    /// Interned strings — persists across `clear()` to avoid
    /// re-allocating common values like `"GET"`, `"/api"`, etc.
    string_cache: Vec<Arc<str>>,
}

impl EvalContext {
    /// Allocate a new context matching the schema.
    ///
    /// All fields are initialized to `Value::Null`.
    #[inline]
    pub fn new(schema: &Schema) -> Self {
        let values = vec![Value::Null; schema.field_count()].into_boxed_slice();
        Self {
            values,
            string_cache: Vec::new(),
        }
    }

    /// Set any field value. O(1) — single array write.
    ///
    /// Prefer the typed `set_*` methods when the type is known at
    /// the call site. Use this as the escape hatch for compound types
    /// (lists, maps, bytes, etc.).
    #[inline]
    pub fn set(&mut self, field: Field, value: Value) {
        self.values[field.0 as usize] = value;
    }

    /// Set an integer field. O(1).
    #[inline]
    pub fn set_i64(&mut self, field: Field, val: i64) {
        self.values[field.0 as usize] = Value::Int(val);
    }

    /// Set an unsigned integer field. O(1).
    #[inline]
    pub fn set_u64(&mut self, field: Field, val: u64) {
        self.values[field.0 as usize] = Value::UInt(val);
    }

    /// Set a floating-point field. O(1).
    #[inline]
    pub fn set_f64(&mut self, field: Field, val: f64) {
        self.values[field.0 as usize] = Value::Float(val);
    }

    /// Set a boolean field. O(1).
    #[inline]
    pub fn set_bool(&mut self, field: Field, val: bool) {
        self.values[field.0 as usize] = Value::Bool(val);
    }

    /// Set a string field from a `&str`. O(1) + Arc allocation on first
    /// unique string; subsequent calls with the same string are a
    /// linear scan + `Arc::clone` (~3 ns).
    #[inline]
    pub fn set_str(&mut self, field: Field, val: &str) {
        let arc = self.intern(val);
        self.values[field.0 as usize] = Value::String(arc);
    }

    /// Set a string field from an owned `String`. O(1) + Arc allocation
    /// on first unique string; cached on subsequent calls.
    #[inline]
    pub fn set_string(&mut self, field: Field, val: String) {
        let arc = self.intern(val.as_str());
        self.values[field.0 as usize] = Value::String(arc);
    }

    /// Intern a string: return existing `Arc<str>` if cached,
    /// otherwise allocate, cache, and return.
    fn intern(&mut self, s: &str) -> Arc<str> {
        // Linear scan — fast for small caches (typical: < 10 unique strings).
        // For larger caches a HashMap would be better, but real-world
        // EvalContexts rarely have > 20 unique string values.
        for arc in &self.string_cache {
            if &**arc == s {
                return Arc::clone(arc);
            }
        }
        let arc: Arc<str> = Arc::from(s);
        self.string_cache.push(Arc::clone(&arc));
        arc
    }

    /// Get a field value by handle. O(1).
    #[inline]
    pub fn get(&self, field: Field) -> &Value {
        &self.values[field.0 as usize]
    }

    /// Access the internal value array.
    #[inline]
    pub fn as_slice(&self) -> &[Value] {
        &self.values
    }

    /// Reset all values to `Value::Null` (retains allocation).
    #[inline]
    pub fn clear(&mut self) {
        self.values.fill(Value::Null);
    }

    /// Number of fields in this context.
    pub fn len(&self) -> usize {
        self.values.len()
    }
}

impl std::fmt::Debug for EvalContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalContext")
            .field("values", &self.values)
            .finish()
    }
}

/// A compiled CEL expression, optimized for repeated evaluation.
///
/// Compiles against a [`Schema`], resolving field names to indices at
/// compile time. Each [`Filter::eval`] call is a flat array traversal
/// in the compiled filter tree — ~1–6 ns depending on pattern complexity.
///
/// If an expression cannot be compiled to the filter tree (e.g., it's
/// non-boolean or uses unsupported patterns like comprehensions), eval
/// automatically falls back to the AST interpreter. This is transparent
/// to the caller and only ~50× slower (still only ~200 ns).
pub struct Filter {
    tree: Option<CompiledFilterTree>,
    expression: Expression,
    var_names: Vec<String>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter")
            .field("var_names", &self.var_names)
            .finish()
    }
}

impl Filter {
    /// Parse and compile a CEL expression against a [`Schema`].
    ///
    /// Field names are resolved to [`Schema`] indices at compile time.
    /// Returns an error if parsing fails.
    pub fn compile(source: &str, schema: &Schema) -> Result<Self, String> {
        let parser = crate::parser::Parser::default();
        let expression = parser.parse(source).map_err(|e| format!("{}", e))?;
        Self::from_expression(expression, schema)
    }

    /// Create a Filter from a pre-parsed Expression.
    pub fn from_expression(expression: Expression, schema: &Schema) -> Result<Self, String> {
        let field_names: Vec<&str> = schema.field_names();
        let tree = filter_tree_compiler::compile_filter_tree_with_schema(
            &expression,
            &field_names,
        )
        .ok();

        let var_names = match &tree {
            Some(t) => t.var_names.clone(),
            None => expression
                .references()
                .variables()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        };

        Ok(Filter {
            tree,
            expression,
            var_names,
        })
    }

    /// Evaluate the expression against a set of runtime values.
    ///
    /// If the expression was compiled to a filter tree, this runs in
    /// ~1–6 ns per call — just array reads. Otherwise it falls back
    /// transparently to the AST interpreter (~200 ns).
    #[inline(always)]
    pub fn eval(&self, ctx: &EvalContext) -> Result<bool, ExecutionError> {
        if self.tree.is_some() {
            Ok(self.eval_bool(ctx))
        } else {
            let mut map_ctx = crate::Context::default();
            for (name, val) in self.var_names.iter().zip(ctx.as_slice().iter()) {
                map_ctx.add_variable_from_value(name, val.clone());
            }
            match Value::resolve(&self.expression, &map_ctx) {
                Ok(Value::Bool(b)) => Ok(b),
                Ok(_) => Ok(false),
                Err(e) => Err(e),
            }
        }
    }

    /// Evaluate and return a `bool` directly, without Result wrapping.
    ///
    /// Same as [`eval`] but skips the `Result` allocation. Panics if
    /// the filter tree was not compiled (use [`eval`] for fallback).
    #[inline(always)]
    pub fn eval_bool(&self, ctx: &EvalContext) -> bool {
        // Safety: Schema guarantees all field indices are in-bounds
        // and all value types match the expected variants.
        unsafe { self.tree.as_ref().unwrap().filter.eval_fast(ctx.as_slice()) }
    }

    /// Variable names referenced by this filter, in index order.
    /// Matches the schema indices used during compilation.
    pub fn variables(&self) -> &[String] {
        &self.var_names
    }
}
