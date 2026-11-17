//! Numbers as part of a type's identity.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

/// An `f64` that can be part of a type's identity (`Type::NumberLiteral`,
/// enum member values).
///
/// Types are interned through a hash map, so `Eq`, `Hash`, and `Ord` must agree.
/// Plain `f64` can't guarantee that (NaN != NaN, and `0.0 == -0.0` despite
/// different bits), so this compares by IEEE 754 total order and hashes the
/// bit pattern: two values are equal exactly when their bits are equal.
#[derive(Debug, Clone, Copy)]
pub struct NumberValue(f64);

impl NumberValue {
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl From<f64> for NumberValue {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

impl PartialEq for NumberValue {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for NumberValue {}

impl PartialOrd for NumberValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NumberValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for NumberValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl fmt::Display for NumberValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash(n: NumberValue) -> u64 {
        let mut h = DefaultHasher::new();
        n.hash(&mut h);
        h.finish()
    }

    #[test]
    fn eq_and_hash_agree() {
        let pairs = [
            (0.0, -0.0),
            (f64::NAN, f64::NAN),
            (1.5, 1.5),
            (f64::INFINITY, f64::INFINITY),
        ];
        for (a, b) in pairs {
            let (a, b) = (NumberValue::from(a), NumberValue::from(b));
            assert_eq!(a == b, hash(a) == hash(b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn nan_equals_itself() {
        let nan = NumberValue::from(f64::NAN);
        assert_eq!(nan, nan);
    }

    #[test]
    fn displays_like_f64() {
        assert_eq!(NumberValue::from(42.0).to_string(), "42");
        assert_eq!(NumberValue::from(-1.5).to_string(), "-1.5");
    }
}
