//! Type resolution and manipulation helpers.

use oxc_ast::ast::*;

use crate::type_resolution;
use crate::types::Type;

use super::Checker;

impl<'a> Checker<'a> {
    /// Resolve a type annotation to our Type representation.
    pub(super) fn resolve_ts_type(&self, ts_type: &TSType) -> Type {
        type_resolution::resolve_ts_type(ts_type)
    }

    /// Widen literal types to their base types.
    pub fn widen_type(&self, ty: Type) -> Type {
        type_resolution::widen_type(ty)
    }

    /// Create a union of two types.
    ///
    /// Handles deduplication and flattening of nested unions.
    pub(super) fn union_types(&self, a: Type, b: Type) -> Type {
        if a == b {
            return a;
        }

        // Flatten nested unions
        let mut types = Vec::new();

        match a {
            Type::Union(inner) => types.extend(inner),
            other => types.push(other),
        }

        match b {
            Type::Union(inner) => types.extend(inner),
            other => types.push(other),
        }

        // Deduplicate by sorting (uses Type's Ord implementation)
        types.sort();
        types.dedup();

        if types.len() == 1 {
            types.pop().expect("checked len == 1")
        } else {
            Type::Union(types)
        }
    }

    /// Unify multiple types into one (for arrays, return types, etc).
    ///
    /// Empty list is `never`, single type to that type, multiple to union.
    pub(super) fn unify_types(&self, types: Vec<Type>) -> Type {
        if types.is_empty() {
            return Type::Never;
        }

        if types.len() == 1 {
            return types.into_iter().next().expect("checked len == 1");
        }

        let mut iter = types.into_iter();
        let mut result = iter.next().expect("checked len > 1");
        for ty in iter {
            result = self.union_types(result, ty);
        }
        result
    }
}
