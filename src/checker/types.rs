//! Type resolution and manipulation helpers.

use oxc_ast::ast::*;

use crate::types::Type;

use super::Checker;

impl<'a> Checker<'a> {
    /// Resolve a type annotation to our Type representation.
    pub(super) fn resolve_type(&self, ts_type: &TSType) -> Type {
        match ts_type {
            // Primitive keywords
            TSType::TSStringKeyword(_) => Type::String,
            TSType::TSNumberKeyword(_) => Type::Number,
            TSType::TSBooleanKeyword(_) => Type::Boolean,
            TSType::TSNullKeyword(_) => Type::Null,
            TSType::TSUndefinedKeyword(_) => Type::Undefined,
            TSType::TSVoidKeyword(_) => Type::Void,
            TSType::TSAnyKeyword(_) => Type::Any,
            TSType::TSUnknownKeyword(_) => Type::Unknown,
            TSType::TSNeverKeyword(_) => Type::Never,

            // Literal types
            TSType::TSLiteralType(lit) => match &lit.literal {
                TSLiteral::StringLiteral(s) => Type::StringLiteral(s.value.to_string()),
                TSLiteral::NumericLiteral(n) => Type::NumberLiteral(n.value),
                TSLiteral::BooleanLiteral(b) => Type::BooleanLiteral(b.value),
                _ => Type::Any,
            },

            // Array types
            TSType::TSArrayType(arr) => {
                Type::Array(Box::new(self.resolve_type(&arr.element_type)))
            }

            // Union types
            TSType::TSUnionType(union) => {
                let types = union.types.iter().map(|t| self.resolve_type(t)).collect();
                Type::Union(types)
            }

            // Intersection types
            TSType::TSIntersectionType(inter) => {
                let types = inter.types.iter().map(|t| self.resolve_type(t)).collect();
                Type::Intersection(types)
            }

            // Type references
            TSType::TSTypeReference(type_ref) => {
                let name = match &type_ref.type_name {
                    TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                    TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                    TSTypeName::ThisExpression(_) => "this".to_string(),
                };
                let type_args = type_ref
                    .type_arguments
                    .as_ref()
                    .map(|params| params.params.iter().map(|t| self.resolve_type(t)).collect())
                    .unwrap_or_default();
                Type::TypeRef { name, type_args }
            }

            // Parenthesized types
            TSType::TSParenthesizedType(paren) => self.resolve_type(&paren.type_annotation),

            _ => Type::Any,
        }
    }

    /// Widen literal types to their base types.
    ///
    /// Used for `let` and `var` declarations where the type should be mutable:
    /// - `"hello"` to `string`
    /// - `42` to `number`
    /// - `true` to `boolean`
    pub fn widen_type(&self, ty: Type) -> Type {
        match ty {
            Type::StringLiteral(_) => Type::String,
            Type::NumberLiteral(_) => Type::Number,
            Type::BooleanLiteral(_) => Type::Boolean,
            Type::Array(inner) => Type::Array(Box::new(self.widen_type(*inner))),
            Type::Tuple(types) => {
                Type::Tuple(types.into_iter().map(|t| self.widen_type(t)).collect())
            }
            Type::Union(types) => {
                Type::Union(types.into_iter().map(|t| self.widen_type(t)).collect())
            }
            Type::Object {
                properties,
                index_signature,
            } => Type::Object {
                properties: properties
                    .into_iter()
                    .map(|mut p| {
                        p.ty = self.widen_type(p.ty);
                        p
                    })
                    .collect(),
                index_signature,
            },
            other => other,
        }
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

        // Deduplicate by sorting
        types.sort_by(|a, b| format!("{a}").cmp(&format!("{b}")));
        types.dedup();

        if types.len() == 1 {
            types.pop().unwrap()
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
            return types.into_iter().next().unwrap();
        }

        let mut iter = types.into_iter();
        let mut result = iter.next().unwrap();
        for ty in iter {
            result = self.union_types(result, ty);
        }
        result
    }
}
