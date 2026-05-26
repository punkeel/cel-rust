use crate::objects::Value;
use std::sync::Arc;

/// A compact 8-byte tagged value for the register VM fast path.
///
/// Encoding:
/// - `tag == TAG_INT`      → payload is a sign-extended i63 stored in the low 63 bits
/// - `tag == TAG_NULL`     → null
/// - `tag == TAG_TRUE`     → bool true
/// - `tag == TAG_FALSE`    → bool false
/// - `tag == TAG_STRING`   → payload is a pointer to an `Arc<String>` heap allocation
/// - `tag == TAG_PTR`      → payload is a *const BoxPayload (heap-allocated Value)
/// - `tag == TAG_SSO`      → payload is a short string (≤ 7 bytes) stored inline
///
/// For integers and booleans, this fits entirely in a register with no indirection.
/// For short strings (≤ 7 bytes), bytes are stored inline — no heap allocation.
/// For longer strings, we store the Arc pointer directly, skipping the BoxPayload wrapper.
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct FastValue(pub u64);

const TAG_INT: u64 = 0x8000_0000_0000_0000; // top bit set
const TAG_MASK: u64 = 0xE000_0000_0000_0000; // top 3 bits for tag

// Small tag values (top 3 bits = 000) with special payload patterns
const TAG_NULL: u64 = 0x0000_0000_0000_0001;
const TAG_TRUE: u64 = 0x0000_0000_0000_0002;
const TAG_FALSE: u64 = 0x0000_0000_0000_0003;
const TAG_STRING: u64 = 0x0000_0000_0000_0005;
const TAG_SSO: u64 = 0x0000_0000_0000_0006;
const TAG_PTR: u64 = 0x0000_0000_0000_0004;

// For SSO: top byte stores (len << 5 | TAG_SSO identifier), remaining 7 bytes are ASCII chars
// Bits: [7:5] = length (0-7), [4:0] = tag identifier, [63:8] = inline bytes
// We use a different encoding: the bottom 4 bits are always TAG_SSO.
// The next 3 bits store the length (0-7). The remaining 57 bits store the inline bytes.
// Layout: [63:8] = bytes, [7:3] = len, [2:0] = fixed pattern
const SSO_LEN_SHIFT: u64 = 3;
const SSO_TAG_PATTERN: u64 = 0b011; // bottom 3 bits = 011
const SSO_LEN_MASK: u64 = 0x7;     // 3 bits for length

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

    /// Store an Arc<String> pointer directly.
    #[inline(always)]
    pub fn from_arc_string(s: Arc<String>) -> Self {
        let ptr = Arc::into_raw(s);
        Self(ptr as u64 | TAG_STRING)
    }

    /// Try to store a string inline (SSO). Returns None if string is too long.
    #[inline(always)]
    pub fn try_from_sso(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() > 7 {
            return None;
        }
        let mut payload: u64 = 0;
        for (i, &b) in bytes.iter().enumerate() {
            payload |= (b as u64) << (8 + i * 8);
        }
        let len_tag = ((bytes.len() as u64) & SSO_LEN_MASK) << SSO_LEN_SHIFT;
        payload |= len_tag | SSO_TAG_PATTERN;
        Some(Self(payload))
    }

    #[inline(always)]
    pub fn from_value(v: Value) -> Self {
        match v {
            Value::Null => Self::null(),
            Value::Bool(true) => Self::from_bool(true),
            Value::Bool(false) => Self::from_bool(false),
            Value::Int(i) => Self::from_int(i),
            Value::String(s) => {
                // Try SSO first, then fall back to Arc pointer
                if let Some(sso) = Self::try_from_sso(&s) {
                    sso
                } else {
                    Self::from_arc_string(s)
                }
            }
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
        (self.0 & 0xF) == TAG_PTR
    }

    #[inline(always)]
    pub fn is_string(&self) -> bool {
        (self.0 & 0xF) == TAG_STRING
    }

    #[inline(always)]
    pub fn is_sso(&self) -> bool {
        (self.0 & 0x7) == SSO_TAG_PATTERN
    }

    #[inline(always)]
    pub fn as_ptr(&self) -> Option<*const BoxPayload> {
        if (self.0 & 0xF) == TAG_PTR {
            Some((self.0 & !0xF) as *const BoxPayload)
        } else {
            None
        }
    }

    /// Get the Arc<String> pointer (for TAG_STRING values).
    #[inline(always)]
    pub fn as_arc_string(&self) -> Option<*const String> {
        if (self.0 & 0xF) == TAG_STRING {
            Some((self.0 & !0xF) as *const String)
        } else {
            None
        }
    }

    /// Compare an SSO value against a string.
    #[inline(always)]
    pub fn eq_sso_str(&self, other: &str) -> Option<bool> {
        if (self.0 & 0x7) != SSO_TAG_PATTERN {
            return None;
        }
        let len = ((self.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
        let bytes = self.0.to_le_bytes();
        let sso_bytes = &bytes[1..1 + len];
        Some(sso_bytes == other.as_bytes())
    }

    /// Convert back to a full Value. May allocate for ptr payloads.
    #[inline(always)]
    pub fn to_value(&self) -> Value {
        if self.is_int() {
            Value::Int(self.as_int().unwrap())
        } else if self.is_bool() {
            Value::Bool(self.as_bool().unwrap())
        } else if self.is_null() {
            Value::Null
        } else if self.is_sso() {
            let len = ((self.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
            let ptr = &self.0 as *const u64 as *const u8;
            let sso_bytes = unsafe { std::slice::from_raw_parts(ptr.add(1), len) };
            let sso = unsafe { std::str::from_utf8_unchecked(sso_bytes) };
            Value::String(Arc::new(sso.to_string()))
        } else if let Some(ptr) = self.as_arc_string() {
            // Reconstruct the Arc to manage the refcount
            let arc = unsafe { Arc::from_raw(ptr) };
            let cloned = Value::String(arc.clone());
            // Leak it back so the caller can still use the FastValue
            let _ = Arc::into_raw(arc);
            cloned
        } else if let Some(ptr) = self.as_ptr() {
            unsafe { (*ptr).value.clone() }
        } else {
            Value::Null
        }
    }

    /// Compare this FastValue against an expected string, taking the fast paths.
    #[inline(always)]
    pub fn eq_str(&self, expected: &str) -> bool {
        if let Some(result) = self.eq_sso_str(expected) {
            return result;
        }
        if let Some(ptr) = self.as_arc_string() {
            return unsafe { (*ptr).as_str() == expected };
        }
        if let Some(ptr) = self.as_ptr() {
            unsafe {
                match &(*ptr).value {
                    Value::String(s) => s.as_str() == expected,
                    _ => false,
                }
            }
        } else {
            false
        }
    }

    /// Compare two FastValues for string equality (fast path for eq_str_const).
    #[inline(always)]
    pub fn eq_str_fastvalue(&self, other: &FastValue) -> bool {
        // Both SSO? Compare raw payloads (clear tag bits first).
        if self.is_sso() && other.is_sso() {
            let len_a = ((self.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
            let len_b = ((other.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
            if len_a != len_b {
                return false;
            }
            // Compare payloads by clearing the bottom byte (tag+len) and comparing u64 directly.
            // Since shorter strings have trailing zeros in the payload, this works.
            return (self.0 & !0xFF) == (other.0 & !0xFF);
        }
        // Self is SSO, other is Arc<String>
        if self.is_sso() {
            let len = ((self.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
            let ptr = &self.0 as *const u64 as *const u8;
            let sso_bytes = unsafe { std::slice::from_raw_parts(ptr.add(1), len) };
            if let Some(ptr) = other.as_arc_string() {
                return unsafe { (*ptr).as_str().as_bytes() == sso_bytes };
            }
        }
        // Self is Arc<String>, other is SSO
        if other.is_sso() {
            let len = ((other.0 >> SSO_LEN_SHIFT) & SSO_LEN_MASK) as usize;
            let ptr = &other.0 as *const u64 as *const u8;
            let sso_bytes = unsafe { std::slice::from_raw_parts(ptr.add(1), len) };
            if let Some(ptr) = self.as_arc_string() {
                return unsafe { (*ptr).as_str().as_bytes() == sso_bytes };
            }
        }
        // Both Arc<String>
        if let (Some(a), Some(b)) = (self.as_arc_string(), other.as_arc_string()) {
            return unsafe { (*a).as_str() == (*b).as_str() };
        }
        // Fallback through BoxPayload
        self.to_value().eq_str_fallback(other)
    }
}

impl Drop for BoxPayload {
    fn drop(&mut self) {
        // nothing special, just let value drop naturally
    }
}

// Helper for fallback string comparison
trait EqStrFallback {
    fn eq_str_fallback(&self, other: &FastValue) -> bool;
}

impl EqStrFallback for Value {
    fn eq_str_fallback(&self, other: &FastValue) -> bool {
        match self {
            Value::String(s) => {
                if let Some(result) = other.eq_sso_str(s.as_str()) {
                    return result;
                }
                if let Some(ptr) = other.as_arc_string() {
                    return unsafe { s.as_str() == (*ptr).as_str() };
                }
                if let Some(ptr) = other.as_ptr() {
                    unsafe {
                        match &(*ptr).value {
                            Value::String(other_s) => s.as_str() == other_s.as_str(),
                            _ => false,
                        }
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}
