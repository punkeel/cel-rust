// ============================================================================
// FunctionHandle Design: Fast function dispatch for CEL
// ============================================================================
//
// PROBLEM:
// Currently CallExpr stores `func_name: String`. At runtime, executing a call
// requires: BTreeMap lookup by name → iterate overloads → Type::is_assignable
// per arg → finally call fn pointer. For simple calls like `x.contains("y")`,
// this dispatch costs 30-50 ns. The actual str::contains is ~1 ns.
//
// SOLUTION:
// Store a resolved function handle in CallExpr at parse time. For stdlib
// functions with known overloads, this collapses to a direct array lookup.
//
// EXTERNAL API: Unchanged.
//   context.add_function("myFn", my_fn);  // works exactly as before
//
// INTERNAL PIPELINE:
//   Parse("abs") → Resolve("abs", [Int]) → Handle(3, 0) → store in CallExpr
//   Eval(Call(Handle(3,0), args)) → FUNCS[3].overloads[0].op(args)  // O(1)
//
// DYNAMIC FALLBACK:
//   User functions without type info still use string lookup. We pay the old
//   cost only when we must.

use std::sync::Arc;

// A compact handle representing a resolved function overload.
// Layout: [16 bits fn_id | 16 bits overload_idx]
// Supports 65K functions with 65K overloads each. More than enough for CEL.
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

// What CallExpr stores instead of `func_name: String`
pub enum FuncRef {
    // Fast path: resolved at parse/compile time. Zero-cost dispatch.
    Handle(FunctionHandle),
    // Fallback: user-defined or macro-expanded functions that couldn't be
    // resolved statically. Same behavior as today.
    Name(String),
}

// The new Env storage: functions are stored in a Vec indexed by fn_id.
// A side BTreeMap maps names → fn_id for the initial lookup during parsing.
pub struct FastEnv {
    // Indexed by fn_id. Each entry holds all overloads for one function name.
    functions: Vec<FunctionDecl>,
    // Maps function name → fn_id. Used once at parse time, never at runtime.
    name_to_id: std::collections::BTreeMap<String, u16>,
}

impl FastEnv {
    pub fn add_function(&mut self, name: &str, decl: FunctionDecl) -> u16 {
        let id = self.functions.len() as u16;
        self.functions.push(decl);
        self.name_to_id.insert(name.to_string(), id);
        id
    }

    // Called during parsing to resolve a function name to a handle.
    // If no overloads match the given arg types (or types are unknown),
    // returns None and the parser stores FuncRef::Name instead.
    pub fn resolve(
        &self,
        name: &str,
        member: bool,
        arg_types: &[TypeHint],
    ) -> Option<FunctionHandle> {
        let fn_id = self.name_to_id.get(name)?;
        let decl = &self.functions[*fn_id as usize];

        // Find the first matching overload
        for (idx, overload) in decl.overloads.iter().enumerate() {
            if overload.member_function == member
                && arg_types.len() == overload.arg_types.len()
                && arg_types.iter().zip(&overload.arg_types).all(|(a, t)| a.fits(t))
            {
                return Some(FunctionHandle::new(*fn_id, idx as u16));
            }
        }
        None
    }

    // Runtime dispatch via handle: O(1), no string comparison, no BTreeMap.
    #[inline(always)]
    pub fn call(&self, handle: FunctionHandle, ctx: &FunctionContext, args: Vec<Cow<dyn Val>>) -> Result<Value> {
        let decl = &self.functions[handle.fn_id() as usize];
        let overload = &decl.overloads[handle.overload_idx() as usize];
        (overload.op)(ctx, args)
    }
}

// TypeHint is used during parsing for overload resolution.
// Many expressions have a known type without full type inference.
pub enum TypeHint {
    String,    // `LiteralValue::String`
    Int,       // `LiteralValue::Int`
    Bool,      // `LiteralValue::Bool`
    List,      // `Expr::List`
    Map,       // `Expr::Map`
    Unknown,   // variable access — can't resolve without schema
}

impl TypeHint {
    fn fits(&self, target: &Type) -> bool {
        matches!((self, target),
            (Self::String, Type::String) |
            (Self::Int, Type::Int) |
            (Self::Bool, Type::Bool) |
            (Self::List, Type::List(_)) |
            (Self::Map, Type::Map { .. }) |
            (_, _)  // Unknown matches anything — we trust the overload
        )
    }
}

// ============================================================================
// Expected impact
// ============================================================================
//
// Before (string dispatch):
//   "hello".contains("he")  →  BTreeMap.get("contains")  →  O(log N) strings
//                            →  iterate overloads         →  O(M) compares
//                            →  Type::is_assignable       →  virtual call
//                            →  call fn pointer
//   Estimated: 30-50 ns
//
// After (handle dispatch):
//   "hello".contains("he")  →  FUNCS[handle.fn_id()].ops[handle.overload_idx()]
//                            →  call fn pointer
//   Estimated: 2-3 ns
//
// Speedup: ~15× for stdlib function calls.
//
// For user-defined functions:
//   - If arg types are literals or known: resolved to Handle at parse time.
//   - If arg types are variables without schema: falls back to Name.
//   - External API (add_function) is completely unchanged.
//
// ============================================================================
// Migration path
// ============================================================================
//
// Step 1: Add FunctionHandle + FastEnv (new module, keep old Env working)
// Step 2: Modify parser to call FastEnv::resolve() for CallExpr
// Step 3: Modify VM eval to check FuncRef::Handle first
// Step 4: Benchmark abs(-5), size("hello"), etc. to measure speedup
// Step 5: Deprecate old Env::find_overload once all paths use handles
