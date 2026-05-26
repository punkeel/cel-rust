use crate::objects::Value;
use std::sync::Arc;

/// A compact 8-byte tagged value for the register VM fast path.
///
/// Encoding:
/// - `tag == TAG_INT`   → payload is a sign-extended i63 stored in the low 63 bits
/// - `tag == TAG_NULL`  → null
/// - `tag == TAG_TRUE`  → bool true
/// - `tag == TAG_FALSE` → bool false
/// - `tag == TAG_PTR`   → payload is a *const BoxPayload (heap-allocated Value)
///
/// This lets integer equality run without touching memory beyond the register
/// itself — no enum discriminant load, no branch mispredict.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct FastValue(pub u64);

const TAG_INT: u64 = 0x8000_0000_0000_0000; // top bit set
const TAG_MASK: u64 = 0xE000_0000_0000_0000; // top 3 bits for tag

// Small tag values (top 3 bits = 000) with special payload patterns
const TAG_NULL: u64 = 0x0000_0000_0000_0001;
const TAG_TRUE: u64 = 0x0000_0000_0000_0002;
const TAG_FALSE: u64 = 0x0000_0000_0000_0003;
const TAG_PTR: u64 = 0x0000_0000_0000_0004;

/// Heap-allocated payload for types that don't fit inline.
pub struct BoxPayload {
    pub value: Value,
}

impl FastValue {
    #[inline(always)]
    pub fn null() -> Self {
        Self(TAG_NULL)
    }

    #[inline(always)]
    pub fn from_bool(b: bool) -> Self {
        if b {
            Self(TAG_TRUE)
        } else {
            Self(TAG_FALSE)
        }
    }

    #[inline(always)]
    pub fn from_int(i: i64) -> Self {
        // Sign-extend to 63 bits, then set the top bit.
        let tagged = ((i as u64) & 0x7FFF_FFFF_FFFF_FFFF) | TAG_INT;
        Self(tagged)
    }

    #[inline(always)]
    pub fn from_value(v: Value) -> Self {
        match v {
            Value::Null => Self::null(),
            Value::Bool(true) => Self::from_bool(true),
            Value::Bool(false) => Self::from_bool(false),
            Value::Int(i) => Self::from_int(i),
            other => {
                let ptr = Box::into_raw(Box::new(BoxPayload { value: other }));
                Self(ptr as u64 | TAG_PTR)
            }
        }
    }

    #[inline(always)]
    pub fn is_int(&self) -> bool {
        (self.0 & TAG_INT) != 0
    }

    #[inline(always)]
    pub fn as_int(&self) -> Option<i64> {
        if self.is_int() {
            // Sign-extend from 63 bits back to 64 bits
            Some(((self.0 << 1) as i64) >> 1)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        self.0 == TAG_TRUE || self.0 == TAG_FALSE
    }

    #[inline(always)]
    pub fn as_bool(&self) -> Option<bool> {
        match self.0 {
            TAG_TRUE => Some(true),
            TAG_FALSE => Some(false),
            _ => None,
        }
    }

    #[inline(always)]
    pub fn is_null(&self) -> bool {
        self.0 == TAG_NULL
    }

    #[inline(always)]
    pub fn is_ptr(&self) -> bool {
        (self.0 & 0xFFFF_FFFF_FFFF_FFFF) == TAG_PTR
            || ((self.0 & TAG_MASK) == 0 && self.0 >= TAG_PTR && self.0 <= 0x000F_FFFF_FFFF_FFFF)
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> Option<*const BoxPayload> {
        if (self.0 & 0xF) == TAG_PTR {
            Some((self.0 & !0xF) as *const BoxPayload)
        } else {
            None
        }
    }

    /// Convert back to a full Value.  May allocate for ptr payloads.
    #[inline(always)]
    pub fn to_value(&self) -> Value {
        if self.is_int() {
            Value::Int(self.as_int().unwrap())
        } else if self.is_bool() {
            Value::Bool(self.as_bool().unwrap())
        } else if self.is_null() {
            Value::Null
        } else if let Some(ptr) = self.as_ptr() {
            unsafe { (*ptr).value.clone() }
        } else {
            Value::Null
        }
    }
}

impl Drop for BoxPayload {
    fn drop(&mut self) {
        // nothing special, just let value drop naturally
    }
}
