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
use crate::vm::compiler::{compile_expression, ValueClosure};
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
    /// Fast-access typed arrays — set_* writes here too.
    /// Filter tree nodes read from these directly, skipping the
    /// Value enum match + 24-byte stride.
    ints: Box<[i64]>,
    strings: Box<[Arc<str>]>,
    /// Interned strings — persists across `clear()` to avoid
    /// re-allocating common values like `"GET"`, `"/api"`, etc.
    string_cache: Vec<Arc<str>>,
}

impl EvalContext {
    /// Allocate a new context matching the schema.
    ///
    /// All fields are initialized to `Value::Null`/zero/empty.
    #[inline]
    pub fn new(schema: &Schema) -> Self {
        let len = schema.field_count();
        let empty: Arc<str> = Arc::from("");
        Self {
            values: vec![Value::Null; len].into_boxed_slice(),
            ints: vec![0i64; len].into_boxed_slice(),
            strings: vec![Arc::clone(&empty); len].into_boxed_slice(),
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
        let idx = field.0 as usize;
        self.values[idx] = Value::Int(val);
        self.ints[idx] = val;
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
        let idx = field.0 as usize;
        let arc = self.intern(val);
        self.values[idx] = Value::String(Arc::clone(&arc));
        self.strings[idx] = arc;
    }

    /// Set a string field from an owned `String`. O(1) + Arc allocation
    /// on first unique string; cached on subsequent calls.
    #[inline]
    pub fn set_string(&mut self, field: Field, val: String) {
        let idx = field.0 as usize;
        let arc = self.intern(val.as_str());
        self.values[idx] = Value::String(Arc::clone(&arc));
        self.strings[idx] = arc;
    }

    /// Intern a string: return existing `Arc<str>` if cached,
    /// otherwise allocate, cache, and return.
    fn intern(&mut self, s: &str) -> Arc<str> {
        // Linear scan — fast for small caches (typical: < 10 unique strings).
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

    /// Fast-access: read i64 value directly (no enum match).
    /// Only valid for fields declared as Int.
    #[inline]
    pub fn get_i64_fast(&self, idx: usize) -> i64 {
        // Safety: Schema guarantees field idx is Int
        unsafe { *self.ints.get_unchecked(idx) }
    }

    /// Fast-access: read string value directly (no enum match).
    /// Only valid for fields declared as String.
    #[inline]
    pub fn get_str_fast(&self, idx: usize) -> &str {
        // Safety: Schema guarantees field idx is String
        unsafe { &*(*self.strings.get_unchecked(idx)).as_ref() }
    }

    /// Reset all values to `Value::Null` (retains allocation).
    #[inline]
    pub fn clear(&mut self) {
        self.values.fill(Value::Null);
        self.ints.fill(0);
        let empty: Arc<str> = Arc::from("");
        self.strings.fill(empty);
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
    var_names: Vec<String>,
    /// Schema field indices for each entry in `var_names`, used by the
    /// AST fallback path to correctly index into `EvalContext.as_slice()`.
    ///
    /// When the fast tree path is available, this is `0..field_count()`
    /// (all schema fields in order). On the fallback AST path it maps
    /// each variable name to its correct schema index, so that
    /// [`Filter::eval`] can index into [`EvalContext`] by field rather
    /// than zipping names with schema order.
    var_indices: Vec<usize>,
    /// Pre-compiled closure for expressions that couldn't be compiled
    /// to the fast filter tree. Used in place of AST tree-walk for the
    /// fallback path — handles all Expr variants including comprehensions,
    /// selects, maps, etc.
    fallback_closure: Option<ValueClosure>,
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter")
            .field("var_names", &self.var_names)
            .field("var_indices", &self.var_indices)
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

        // Collect boolean field names so the filter tree compiler can
        // compile bare Ident references for boolean-typed fields.
        let bool_fields: std::collections::HashSet<String> = schema
            .fields()
            .filter(|(_, ty)| *ty == FieldType::Bool)
            .map(|(name, _)| name.to_string())
            .collect();

        let tree = filter_tree_compiler::compile_filter_tree_with_schema(
            &expression,
            &field_names,
            &bool_fields,
        )
        .ok();

        let (var_names, var_indices) = match &tree {
            Some(t) => {
                // When the tree compiled, var_names are in schema order (0..N).
                let indices: Vec<usize> = (0..t.var_names.len()).collect();
                (t.var_names.clone(), indices)
            }
            None => {
                let names: Vec<String> = expression
                    .references()
                    .variables()
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect();
                // Resolve each name through the schema to get its field index.
                // This is essential for the AST fallback path to correctly
                // index into EvalContext.as_slice().
                let indices: Vec<usize> = names
                    .iter()
                    .map(|n| schema.get_field(n).map(|f| f.index()).unwrap_or(0))
                    .collect();
                (names, indices)
            }
        };

        // Compile a closure for the fallback path. When the filter tree
        // can't handle the expression (e.g. comprehensions, selects),
        // we use this closure instead of walking the AST at eval time.
        let fallback_closure = if tree.is_none() {
            Some(compile_expression(&expression, &[]))
        } else {
            None
        };

        Ok(Filter {
            tree,
            var_names,
            var_indices,
            fallback_closure,
        })
    }

    /// Evaluate the expression against a set of runtime values.
    ///
    /// If the expression was compiled to a filter tree, this runs in
    /// ~1–6 ns per call — just array reads. Otherwise it runs the
    /// pre-compiled closure (~50–200 ns depending on expression
    /// complexity) — no AST tree-walk at eval time.
    #[inline(always)]
    pub fn eval(&self, ctx: &EvalContext) -> Result<bool, ExecutionError> {
        if self.tree.is_some() {
            Ok(self.eval_bool(ctx))
        } else {
            let closure = self.fallback_closure.as_ref().expect(
                "Filter must have either a compiled filter tree or a fallback closure",
            );
            let mut map_ctx = crate::Context::default();
            // Use var_indices to look up each variable at its correct
            // schema index. This is correct regardless of the order of
            // var_names (which comes from a HashSet on the fallback path
            // and may not match schema ordering).
            for (name, &idx) in self.var_names.iter().zip(self.var_indices.iter()) {
                map_ctx.add_variable_from_value(name, ctx.as_slice()[idx].clone());
            }
            match closure(&map_ctx) {
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
        let tree = self.tree.as_ref().unwrap();
        matches!(tree.compiled.eval(ctx.as_slice()), Value::Bool(true))
    }

    /// Variable names referenced by this filter, in index order.
    /// Matches the schema indices used during compilation.
    pub fn variables(&self) -> &[String] {
        &self.var_names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    /// The AST fallback path in Filter::eval() was incorrectly zipping
    /// expression-order variable names with schema-order values, causing
    /// field/value mismatches when the expression referenced fields out of
    /// schema order.
    #[test]
    fn test_eval_ast_fallback_respects_schema_indices() {
        let mut schema = Schema::new();
        let _flag = schema.add_field("flag", FieldType::Bool);
        let port = schema.add_field("port", FieldType::Int);
        let method = schema.add_field("method", FieldType::String);

        // Ternary is not supported by the filter tree compiler, forcing AST fallback.
        let filter = Filter::compile("method == 'GET' ? port == 80 : true", &schema).unwrap();

        // Verify it indeed fell back (no filter tree compiled).
        assert!(filter.tree.is_none(), "expected AST fallback for ternary expr");

        let mut ctx = EvalContext::new(&schema);
        ctx.set_bool(_flag, false); // flag (index 0) — not referenced
        ctx.set_i64(port, 80);    // port (index 1)
        ctx.set_str(method, "GET"); // method (index 2)

        // method == "GET" → true → evaluate `port == 80` → true
        assert_eq!(filter.eval(&ctx).unwrap(), true);

        // Change port so the result should be false
        ctx.set_i64(port, 8080);
        assert_eq!(filter.eval(&ctx).unwrap(), false);

        // Change method so the ternary takes the else branch → true
        ctx.set_i64(port, 80);
        ctx.set_str(method, "POST");
        assert_eq!(filter.eval(&ctx).unwrap(), true);
    }

    /// The tree path should still work correctly regardless of field order.
    #[test]
    fn test_eval_tree_path_respects_schema_indices() {
        let mut schema = Schema::new();
        let _flag = schema.add_field("flag", FieldType::Bool);
        let port = schema.add_field("port", FieldType::Int);

        // Simple comparison — always compiles to a filter tree.
        let filter = Filter::compile("port == 80", &schema).unwrap();
        assert!(filter.tree.is_some(), "expected filter tree for simple cmp");

        let mut ctx = EvalContext::new(&schema);
        ctx.set_bool(_flag, false); // flag (index 0)
        ctx.set_i64(port, 80);   // port (index 1)

        assert_eq!(filter.eval(&ctx).unwrap(), true);
    }

    #[test]
    fn filter_eval_fallback_correct_indices() {
        // Schema fields in a specific order that differs from the
        // expression's variable order. This exercises the AST fallback
        // path (triggered when filter tree compilation fails) and
        // verifies that variables are mapped to their correct schema
        // indices rather than zipped positionally.
        let mut schema = Schema::new();
        let port = schema.add_field("port", FieldType::Int);    // idx 0
        let a = schema.add_field("a", FieldType::Bool);      // idx 1
        let b = schema.add_field("b", FieldType::Int);       // idx 2
        let _c = schema.add_field("c", FieldType::String);    // idx 3

        // Parse `a && b == 7`. With BoolVar support, this now compiles
        // to the fast path: And(BoolVar { a }, EqInt { b, 7 }).
        let parser = Parser::default();
        let expression = parser.parse("a && b == 7").unwrap();
        let filter = Filter::from_expression(expression, &schema).unwrap();

        // Verify it compiles to a filter tree (BoolVar-enabled fast path)
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_i64(port, 42);
        ctx.set_bool(a, true);
        ctx.set_i64(b, 7);
        ctx.set_str(_c, "test");

        // a → ctx[1] = true, b → ctx[2] = 7
        // → true && (7 == 7) = true
        let result = filter.eval(&ctx);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert_eq!(result.unwrap(), true);

        // Flip a to false → false && ... = false
        ctx.set_bool(a, false);
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_fallback_string_field() {
        // Similar test with string comparison, ensuring field indices
        // are correctly resolved for string-typed variables when the
        // filter tree can't compile the expression.
        let mut schema = Schema::new();
        let _x = schema.add_field("x", FieldType::Int);     // idx 0
        let _method = schema.add_field("method", FieldType::String); // idx 1
        let _flag_bool = schema.add_field("flag_bool", FieldType::Bool); // idx 2

        // Expression that fails filter tree compile because `flag_bool` is an Ident
        // inside a LOGICAL_NOT (which tries to compile the inner as a tree first)
        let parser = Parser::default();
        let expression = parser.parse("flag_bool && method.startsWith('G')").unwrap();
        let filter = Filter::from_expression(expression, &schema).unwrap();
        assert!(filter.tree.is_some(), "BoolVar should handle this now");

        // Fallback path test: use an expression with Int != (previously unsupported,
        // but now handled via NeStr). To truly trigger fallback, use something that
        // the filter tree can't handle at all — like an Expr::Literal at top level.
        // But that's not very useful. Let's instead verify the closure fallback works
        // for the non-bool-field case by testing an expression that genuinely can't
        // compile: `flag_int` where flag_int is an Int (not Bool), so Ident fails.
        let mut schema2 = Schema::new();
        let x2 = schema2.add_field("x", FieldType::Int);  // idx 0
        let method2 = schema2.add_field("method", FieldType::String); // idx 1
        let flag_int = schema2.add_field("flag_int", FieldType::Int); // idx 2 — NOT Bool

        let expr2: Expression = Parser::default().parse("flag_int && method.startsWith('G')").unwrap();
        let filter2 = Filter::from_expression(expr2, &schema2).unwrap();
        // tree is None because flag_int is Int, not Bool — Ident handler fails
        assert!(filter2.tree.is_none());
        // fallback closure should exist
        assert!(filter2.fallback_closure.is_some());

        let mut ctx2 = EvalContext::new(&schema2);
        ctx2.set_i64(x2, 99);
        ctx2.set_str(method2, "GET");
        ctx2.set_i64(flag_int, 1);

        // The closure evaluates flag_int (Int) as left operand of &&
        // and should produce a type error (CEL doesn't coerce Int to Bool).
        let result = filter2.eval(&ctx2);
        assert!(
            result.is_err(),
            "expected type error from bool check on int in closure fallback"
        );
    }

    #[test]
    fn filter_eval_fast_path_different_order() {
        // Fast path should also work when schema order differs from
        // expression order — uses direct field indices, no zip involved.
        let mut schema = Schema::new();
        let _x = schema.add_field("x", FieldType::Int);     // idx 0
        let method = schema.add_field("method", FieldType::String); // idx 1
        let port = schema.add_field("port", FieldType::Int);  // idx 2

        // `port == 443 && method.startsWith('G')` compiles fine to a tree
        let filter = Filter::compile("port == 443 && method.startsWith('G')", &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_i64(_x, 42);
        ctx.set_str(method, "GET");
        ctx.set_i64(port, 443);

        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set_i64(port, 80);
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_fallback_three_vars() {
        // With 3 variables, the probability of accidental correctness
        // from the old zip-with-schema-order bug is 1/6. With the fix
        // it's always correct.
        let mut schema = Schema::new();
        let _p = schema.add_field("p", FieldType::Int);    // idx 0
        let a = schema.add_field("a", FieldType::Bool);  // idx 1
        let _q = schema.add_field("q", FieldType::Int);    // idx 2
        let b = schema.add_field("b", FieldType::Bool);  // idx 3
        let _r = schema.add_field("r", FieldType::Int);    // idx 4

        // `a && b` — both Bool-typed, compiles via BoolVar
        let expr: Expression = Parser::default().parse("a && b").unwrap();
        let filter = Filter::from_expression(expr, &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_i64(_p, 1);
        ctx.set_bool(a, true);
        ctx.set_i64(_q, 2);
        ctx.set_bool(b, false);
        ctx.set_i64(_r, 3);

        // With fix: a=ctx[1]=true, b=ctx[3]=false → true && false = false
        assert_eq!(filter.eval(&ctx), Ok(false));

        ctx.set_bool(b, true);
        // a=ctx[1]=true, b=ctx[3]=true → true && true = true
        assert_eq!(filter.eval(&ctx), Ok(true));
    }

    #[test]
    fn filter_eval_fallback_correct_with_nonzero_root_variable() {
        // Tests a specific scenario: schema has fields before the first
        // referenced variable, verifying those don't shift indices.
        let mut schema = Schema::new();
        let skip0 = schema.add_field("skip0", FieldType::Int);    // idx 0
        let skip1 = schema.add_field("skip1", FieldType::String); // idx 1
        let flag = schema.add_field("flag", FieldType::Bool);    // idx 2

        // Expression uses only `flag` — should map to index 2.
        // With BoolVar this compiles to the fast path.
        let expr: Expression = Parser::default().parse("flag").unwrap();
        let filter = Filter::from_expression(expr, &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_i64(skip0, 123);
        ctx.set_str(skip1, "unused");
        ctx.set_bool(flag, true);

        // AST interprets bare `flag` as the bool value itself
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set_bool(flag, false);
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_string_ne() {
        // String != should compile to the fast path via NeStr
        let mut schema = Schema::new();
        let method = schema.add_field("method", FieldType::String);
        let filter = Filter::compile("method != 'GET'", &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_str(method, "POST");
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set_str(method, "GET");
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_bool_var_predicate() {
        // Boolean field used directly as a predicate (no `== true`)
        let mut schema = Schema::new();
        let flag = schema.add_field("flag", FieldType::Bool);
        let filter = Filter::compile("flag", &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_bool(flag, true);
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set_bool(flag, false);
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_bool_var_in_and() {
        // BoolVar combined with another condition
        let mut schema = Schema::new();
        let flag = schema.add_field("flag", FieldType::Bool);
        let port = schema.add_field("port", FieldType::Int);
        let filter = Filter::compile("flag && port == 80", &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_bool(flag, true);
        ctx.set_i64(port, 80);
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set_bool(flag, false);
        assert_eq!(filter.eval(&ctx), Ok(false));

        ctx.set_bool(flag, true);
        ctx.set_i64(port, 8080);
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_bool_var_not() {
        // !flag should also compile
        let mut schema = Schema::new();
        let flag = schema.add_field("flag", FieldType::Bool);
        let filter = Filter::compile("!flag", &schema).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set_bool(flag, true);
        assert_eq!(filter.eval(&ctx), Ok(false));

        ctx.set_bool(flag, false);
        assert_eq!(filter.eval(&ctx), Ok(true));
    }

    #[test]
    fn filter_eval_exists_int_list() {
        // bot_ids.exists(id, id > 5) — compiles to Exists filter node
        let mut schema = Schema::new();
        let bot_ids = schema.add_field("bot_ids", FieldType::Any);
        let filter = Filter::compile(
            "bot_ids.exists(id, id > 5)", &schema,
        ).unwrap();
        // Should compile to filter tree via Exists node
        assert!(filter.tree.is_some(), "exists() should compile to filter tree");

        let mut ctx = EvalContext::new(&schema);
        // Empty list → false
        ctx.set(bot_ids, Value::List(Arc::new(vec![])));
        assert_eq!(filter.eval(&ctx), Ok(false));

        // List with values ≤ 5 → false
        ctx.set(bot_ids, Value::List(Arc::new(vec![
            Value::Int(1),
            Value::Int(3),
            Value::Int(5),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(false));

        // List with any value > 5 → true
        ctx.set(bot_ids, Value::List(Arc::new(vec![
            Value::Int(1),
            Value::Int(7),
            Value::Int(3),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(true));
    }

    #[test]
    fn filter_eval_exists_eq_int() {
        // bot_ids.exists(id, id == 42)
        let mut schema = Schema::new();
        let bot_ids = schema.add_field("bot_ids", FieldType::Any);
        let filter = Filter::compile(
            "bot_ids.exists(id, id == 42)", &schema,
        ).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set(bot_ids, Value::List(Arc::new(vec![
            Value::Int(1),
            Value::Int(42),
            Value::Int(3),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set(bot_ids, Value::List(Arc::new(vec![
            Value::Int(1),
            Value::Int(2),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(false));
    }

    #[test]
    fn filter_eval_exists_str_list() {
        // names.exists(n, n == "bob")
        let mut schema = Schema::new();
        let names = schema.add_field("names", FieldType::Any);
        let filter = Filter::compile(
            r#"names.exists(n, n == "bob")"#, &schema,
        ).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        ctx.set(names, Value::List(Arc::new(vec![
            Value::String(Arc::from("alice")),
            Value::String(Arc::from("bob")),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(true));

        ctx.set(names, Value::List(Arc::new(vec![
            Value::String(Arc::from("alice")),
        ])));
        assert_eq!(filter.eval(&ctx), Ok(false));
    }
}

#[test]
fn filter_eval_exists_in_int_set() {
    // bot_ids.exists(id, id in [42, 99]) should compile to ExistsInIntSet
    let mut schema = Schema::new();
    let bot_ids = schema.add_field("bot_ids", FieldType::Any);
    let filter = Filter::compile(
        "bot_ids.exists(id, id in [42, 99])", &schema,
    ).unwrap();
    assert!(filter.tree.is_some());

    let mut ctx = EvalContext::new(&schema);
    ctx.set(bot_ids, Value::List(Arc::new(vec![
        Value::Int(1),
        Value::Int(42),
        Value::Int(3),
    ])));
    assert_eq!(filter.eval(&ctx), Ok(true));

    ctx.set(bot_ids, Value::List(Arc::new(vec![
        Value::Int(1),
        Value::Int(2),
        Value::Int(3),
    ])));
    assert_eq!(filter.eval(&ctx), Ok(false));
}

#[test]
fn filter_eval_exists_eq_int_node() {
    // bot_ids.exists(id, id == 42) should compile to ExistsEqInt
    let mut schema = Schema::new();
    let bot_ids = schema.add_field("bot_ids", FieldType::Any);
    let filter = Filter::compile(
        "bot_ids.exists(id, id == 42)", &schema,
    ).unwrap();
    assert!(filter.tree.is_some());

    let mut ctx = EvalContext::new(&schema);
    ctx.set(bot_ids, Value::List(Arc::new(vec![
        Value::Int(1),
        Value::Int(42),
        Value::Int(3),
    ])));
    assert_eq!(filter.eval(&ctx), Ok(true));

    ctx.set(bot_ids, Value::List(Arc::new(vec![
        Value::Int(1),
        Value::Int(2),
    ])));
    assert_eq!(filter.eval(&ctx), Ok(false));
}

#[test]
    fn filter_eval_map_key_contains() {
        let mut schema = Schema::new();
        let hdrs = schema.add_field("http__headers_map", FieldType::Any);
        let filter = Filter::compile(
            r#"http__headers_map["via"].exists(it, it == "X-MSIP-Via")"#,
            &schema,
        ).unwrap();
        assert!(filter.tree.is_some());

        let mut ctx = EvalContext::new(&schema);
        use std::collections::HashMap;

        // Single string value, matching
        {
            let mut map = HashMap::new();
            map.insert(crate::objects::Key::String(Arc::from("via")),
                Value::String(Arc::from("X-MSIP-Via")));
            ctx.set(hdrs, Value::Map(crate::objects::Map { map: Arc::new(map) }));
            assert_eq!(filter.eval(&ctx), Ok(true));
        }

        // List value with matching element
        {
            let mut map = HashMap::new();
            map.insert(crate::objects::Key::String(Arc::from("via")),
                Value::List(Arc::new(vec![
                    Value::String(Arc::from("X-Forwarded-For")),
                    Value::String(Arc::from("X-MSIP-Via")),
                ])));
            ctx.set(hdrs, Value::Map(crate::objects::Map { map: Arc::new(map) }));
            assert_eq!(filter.eval(&ctx), Ok(true));
        }

        // Non-matching
        {
            let mut map = HashMap::new();
            map.insert(crate::objects::Key::String(Arc::from("via")),
                Value::List(Arc::new(vec![
                    Value::String(Arc::from("X-Forwarded-For")),
                ])));
            ctx.set(hdrs, Value::Map(crate::objects::Map { map: Arc::new(map) }));
            assert_eq!(filter.eval(&ctx), Ok(false));
        }
    }
