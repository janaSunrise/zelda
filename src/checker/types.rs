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

            // Object type literals: { a: number; b?: string }
            TSType::TSTypeLiteral(lit) => self.resolve_type_literal(lit),

            // Tuple types: [number, string]
            TSType::TSTupleType(tuple) => {
                let types = tuple
                    .element_types
                    .iter()
                    .map(|elem| self.resolve_tuple_element(elem))
                    .collect();
                Type::Tuple(types)
            }

            // Function types: (a: number) => string
            TSType::TSFunctionType(func) => self.resolve_function_type(func),

            _ => Type::Any,
        }
    }

    /// Resolve a type literal (object type) to our Type representation.
    fn resolve_type_literal(&self, lit: &TSTypeLiteral) -> Type {
        let mut properties = Vec::new();

        for member in &lit.members {
            match member {
                TSSignature::TSPropertySignature(prop) => {
                    if let Some(name) = self.get_ts_property_key_name(&prop.key) {
                        let ty = prop
                            .type_annotation
                            .as_ref()
                            .map(|ann| self.resolve_type(&ann.type_annotation))
                            .unwrap_or(Type::Any);
                        let optional = prop.optional;
                        properties.push(crate::types::Property {
                            name,
                            ty,
                            optional,
                            readonly: prop.readonly,
                        });
                    }
                }
                TSSignature::TSMethodSignature(method) => {
                    if let Some(name) = self.get_ts_property_key_name(&method.key) {
                        // Build function type from method signature
                        let params = self.resolve_formal_parameters(&method.params);
                        let return_type = method
                            .return_type
                            .as_ref()
                            .map(|ann| self.resolve_type(&ann.type_annotation))
                            .unwrap_or(Type::Void);
                        let ty = Type::Function {
                            params,
                            return_type: Box::new(return_type),
                            type_params: vec![],
                        };
                        properties.push(crate::types::Property {
                            name,
                            ty,
                            optional: method.optional,
                            readonly: false,
                        });
                    }
                }
                // TODO: Index signatures
                _ => {}
            }
        }

        Type::Object {
            properties,
            index_signature: None,
        }
    }

    /// Get property key name from TSPropertyKey.
    fn get_ts_property_key_name(&self, key: &PropertyKey) -> Option<String> {
        match key {
            PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
            PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
            PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
            _ => None,
        }
    }

    /// Resolve a tuple element type.
    fn resolve_tuple_element(&self, elem: &TSTupleElement) -> Type {
        match elem {
            TSTupleElement::TSOptionalType(opt) => self.resolve_type(&opt.type_annotation),
            TSTupleElement::TSRestType(rest) => self.resolve_type(&rest.type_annotation),
            _ => {
                if let Some(ty) = elem.as_ts_type() {
                    self.resolve_type(ty)
                } else {
                    Type::Any
                }
            }
        }
    }

    /// Resolve a function type.
    fn resolve_function_type(&self, func: &TSFunctionType) -> Type {
        let params = self.resolve_formal_parameters(&func.params);
        let return_type = self.resolve_type(&func.return_type.type_annotation);

        Type::Function {
            params,
            return_type: Box::new(return_type),
            type_params: vec![],
        }
    }

    /// Resolve formal parameters to our Param representation.
    fn resolve_formal_parameters(&self, params: &FormalParameters) -> Vec<crate::types::Param> {
        params
            .items
            .iter()
            .map(|p| {
                let name = match &p.pattern {
                    BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                    _ => "_".to_string(),
                };
                let ty = p
                    .type_annotation
                    .as_ref()
                    .map(|ann| self.resolve_type(&ann.type_annotation))
                    .unwrap_or(Type::Any);
                crate::types::Param {
                    name,
                    ty,
                    optional: p.optional,
                    rest: false,
                }
            })
            .collect()
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
