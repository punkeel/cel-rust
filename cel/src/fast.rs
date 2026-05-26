//! High-performance CEL evaluation with Schema-based field resolution.
//!
//! Eliminates HashMap lookups at evaluation time by pre-assigning field
//! indices at schema construction time. Target: ~1 ns per eval for boolean expressions.
//!
//! # Example
//!
//! ```rust
//! use cel::fast::{Schema, FieldType, Filter, EvalContext};
//! use cel::objects::Value;
//!
//! // Setup: define fields once, get typed handles
//! let mut schema = Schema::new();
//! let port   = schema.add_field("port",   FieldType::Int);
//! let method = schema.add_field("method", FieldType::String);
//! let schema = schema.build();
//!
//! // Compile: resolves field names to indices at compile time
//! let filter = Filter::compile("port == 80 && method == 'GET'", &schema).unwrap();
//!
//! // Per-request: O(1) set by handle, no string hashing
//! let mut ctx = EvalContext::new(&schema);
//! ctx.set(port, Value::Int(80));
//! ctx.set(method, Value::String(std::sync::Arc::new("GET".to_string())));
//! assert_eq!(filter.eval(&ctx).unwrap(), true);
//! ```

use crate::objects::Value;
use crate::vm::filter_tree_compiler::{self, CompiledFilterTree};
use crate::{ExecutionError, Expression};
use std::collections::HashMap;
use std::sync::Arc;

/// CEL field types for Schema declarations.
///
/// Matches the CEL language spec type system.
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

/// A lightweight handle to a field within a Schema.
///
/// Copyable, zero-cost — encodes an array index as a `u16`.
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
/// Assigns indices to field names at construction time. These indices
/// are embedded into compiled `Filter` objects, enabling O(1) array
/// access at evaluation time — no HashMap lookups, no string comparisons.
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
    /// The returned `Field` can be used with `EvalContext::set()` for
    /// O(1) value assignment at evaluation time.
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

    /// Get the name and type of every field (in index order).
    pub fn fields(&self) -> impl Iterator<Item = (&str, FieldType)> {
        self.fields.iter().map(|(n, t)| (n.as_ref(), *t))
    }

    /// Finalize schema construction.
    /// Returns self for chaining if no additional methods needed.
    pub fn build(self) -> Self {
        self
    }

    /// Create a name→index mapping for the tree compiler.
    pub(crate) fn field_names(&self) -> Vec<&str> {
        self.fields.iter().map(|(n, _)| n.as_ref()).collect()
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime values indexed by Field handle.
///
/// Pre-allocated flat array of `Value`. Setting a value by `Field` handle
/// is a single array write — no hashing, no allocation.
///
/// Unset fields evaluate to `Value::Null`, which produces `false` for
/// any comparison in the filter tree.
pub struct EvalContext {
    values: Box<[Value]>,
}

impl EvalContext {
    /// Allocate a new context matching the schema.
    ///
    /// All fields are initialized to `Value::Null`.
    pub fn new(schema: &Schema) -> Self {
        let values = vec![Value::Null; schema.field_count()].into_boxed_slice();
        Self { values }
    }

    /// Set a field value by handle. O(1) — single array write.
    #[inline]
    pub fn set(&mut self, field: Field, value: Value) {
        self.values[field.0 as usize] = value;
    }

    /// Set an integer field value. O(1).
    #[inline]
    pub fn set_i64(&mut self, field: Field, val: i64) {
        self.values[field.0 as usize] = Value::Int(val);
    }

    /// Set an unsigned integer field value. O(1).
    #[inline]
    pub fn set_u64(&mut self, field: Field, val: u64) {
        self.values[field.0 as usize] = Value::UInt(val);
    }

    /// Set a floating-point field value. O(1).
    #[inline]
    pub fn set_f64(&mut self, field: Field, val: f64) {
        self.values[field.0 as usize] = Value::Float(val);
    }

    /// Set a boolean field value. O(1).
    #[inline]
    pub fn set_bool(&mut self, field: Field, val: bool) {
        self.values[field.0 as usize] = Value::Bool(val);
    }

    /// Set a string field value from a `&str`. O(1) — allocates Arc<String>.
    #[inline]
    pub fn set_str(&mut self, field: Field, val: &str) {
        self.values[field.0 as usize] = Value::String(Arc::new(val.to_string()));
    }

    /// Set a string field value from a `String`. O(1).
    #[inline]
    pub fn set_string(&mut self, field: Field, val: String) {
        self.values[field.0 as usize] = Value::String(Arc::new(val));
    }

    /// Get a field value by handle. O(1).
    #[inline]
    pub fn get(&self, field: Field) -> &Value {
        &self.values[field.0 as usize]
    }

    /// Access the internal value array (for tree evaluators).
    #[inline]
    pub fn as_slice(&self) -> &[Value] {
        &self.values
    }

    /// Access the internal value array mutably.
    #[inline]
    pub fn as_slice_mut(&mut self) -> &mut [Value] {
        &mut self.values
    }

    /// Reset all values to `Value::Null` (retains allocation).
    #[inline]
    pub fn clear(&mut self) {
        for slot in self.values.iter_mut() {
            *slot = Value::Null;
        }
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

/// A compiled CEL expression optimized for repeated evaluation.
///
/// Compiles the expression against a `Schema`, resolving field names
/// to indices at compile time. `eval()` then accesses values directly
/// from `EvalContext` via array indexing — no HashMap lookups.
///
/// If the expression cannot be compiled to a filter tree (e.g. non-boolean
/// or uses unsupported patterns), evaluation falls back to the AST
/// interpreter automatically.
pub struct Filter {
    /// Compiled filter tree, if applicable.
    tree: Option<CompiledFilterTree>,
    /// Original parsed expression (for AST fallback).
    expression: Expression,
    /// Variable names referenced by the tree (in index order).
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
    /// Parse and compile a CEL expression against a Schema.
    ///
    /// Field names in the expression are resolved to Schema indices at
    /// compile time. Returns an error if parsing fails.
    pub fn compile(source: &str, schema: &Schema) -> Result<Self, String> {
        let parser = crate::parser::Parser::default();
        let expression = parser.parse(source).map_err(|e| format!("{}", e))?;
        Self::from_expression(expression, schema)
    }

    /// Create a Filter from a pre-parsed Expression.
    pub fn from_expression(expression: Expression, schema: &Schema) -> Result<Self, String> {
        // Try compiling to a filter tree with schema-assigned indices.
        let field_names: Vec<&str> = schema.field_names();
        let tree = filter_tree_compiler::compile_filter_tree_with_schema(
            &expression,
            &field_names,
        )
        .ok();

        // Collect variable names (from tree if available, otherwise from expression references).
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

    /// Evaluate the filter against a context.
    ///
    /// If the expression was successfully compiled to a filter tree,
    /// this is O(1) per field access — just array reads. If not,
    /// falls back to tree-walking AST interpretation (still fast,
    /// but ~50x slower than the tree path).
    pub fn eval(&self, ctx: &EvalContext) -> Result<bool, ExecutionError> {
        if let Some(tree) = &self.tree {
            // Tree path: direct array indexing, ~1 ns
            Ok(tree.filter.eval(ctx.as_slice()))
        } else {
            // AST fallback: create a temporary Context from our flat values
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

    /// Returns the variable names referenced by this filter (in index order).
    /// Matches the Schema indices used during compression.
    pub fn variables(&self) -> &[String] {
        &self.var_names
    }
}
