//! AST to Type resolution.
//!
//! This module converts oxc TypeScript AST nodes to our Type representation.

use oxc_ast::ast::*;

use super::{IndexSignature, Param, Property, Type, TypeParam};

/// Convert an oxc TSType AST node to our Type representation.
pub fn resolve_ts_type(ts_type: &TSType) -> Type {
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
        TSType::TSArrayType(arr) => Type::Array(Box::new(resolve_ts_type(&arr.element_type))),

        // Tuple types
        TSType::TSTupleType(tuple) => {
            let types: Vec<Type> = tuple
                .element_types
                .iter()
                .map(resolve_tuple_element)
                .collect();
            Type::Tuple(types)
        }

        // Union types
        TSType::TSUnionType(union) => {
            let types: Vec<Type> = union.types.iter().map(resolve_ts_type).collect();
            Type::Union(types)
        }

        // Intersection types
        TSType::TSIntersectionType(inter) => {
            let types: Vec<Type> = inter.types.iter().map(resolve_ts_type).collect();
            Type::Intersection(types)
        }

        // Type references (e.g., Array<T>, Promise<T>, custom types)
        TSType::TSTypeReference(type_ref) => {
            let name = match &type_ref.type_name {
                TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                TSTypeName::ThisExpression(_) => "this".to_string(),
            };

            let type_args: Vec<Type> = type_ref
                .type_arguments
                .as_ref()
                .map(|params| params.params.iter().map(resolve_ts_type).collect())
                .unwrap_or_default();

            Type::TypeRef { name, type_args }
        }

        // Function types
        TSType::TSFunctionType(func) => resolve_function_type(func),

        // Type literals (inline object types)
        TSType::TSTypeLiteral(lit) => resolve_type_literal(lit),

        // Parenthesized types
        TSType::TSParenthesizedType(paren) => resolve_ts_type(&paren.type_annotation),

        _ => Type::Any,
    }
}

/// Resolve a tuple element type.
///
/// Tuple elements can be:
/// - Regular: `[string, number]`
/// - Optional: `[string, number?]`
/// - Rest: `[string, ...number[]]`
/// - Named: `[name: string, age: number]`
pub fn resolve_tuple_element(elem: &TSTupleElement) -> Type {
    match elem {
        TSTupleElement::TSOptionalType(opt) => resolve_ts_type(&opt.type_annotation),
        TSTupleElement::TSRestType(rest) => resolve_ts_type(&rest.type_annotation),
        TSTupleElement::TSNamedTupleMember(named) => resolve_tuple_element(&named.element_type),
        _ => {
            if let Some(ty) = elem.as_ts_type() {
                resolve_ts_type(ty)
            } else {
                Type::Any
            }
        }
    }
}

pub fn resolve_function_type(func: &TSFunctionType) -> Type {
    let params = resolve_formal_parameters(&func.params);
    let return_type = resolve_ts_type(&func.return_type.type_annotation);

    Type::Function {
        params,
        return_type: Box::new(return_type),
        type_params: vec![],
    }
}

pub fn resolve_type_literal(lit: &TSTypeLiteral) -> Type {
    let mut properties = Vec::new();
    let mut index_signature = None;

    for member in &lit.members {
        match member {
            TSSignature::TSPropertySignature(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);
                    let mut property = Property::new(name, ty);
                    if prop.optional {
                        property = property.optional();
                    }
                    if prop.readonly {
                        property = property.readonly();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSMethodSignature(method) => {
                if let Some(name) = get_property_key_name(&method.key) {
                    let params = resolve_formal_parameters(&method.params);
                    let return_type = method
                        .return_type
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Void);
                    let ty = Type::Function {
                        params,
                        return_type: Box::new(return_type),
                        type_params: vec![],
                    };
                    let mut property = Property::new(name, ty);
                    if method.optional {
                        property = property.optional();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSIndexSignature(idx) => {
                // Index signature: [key: string]: T or [key: number]: T
                if let Some(param) = idx.parameters.first() {
                    let key_type = resolve_ts_type(&param.type_annotation.type_annotation);
                    let value_type = resolve_ts_type(&idx.type_annotation.type_annotation);
                    index_signature = Some(IndexSignature {
                        key_type: Box::new(key_type),
                        value_type: Box::new(value_type),
                    });
                }
            }
            _ => {}
        }
    }

    Type::Object {
        properties,
        index_signature,
        extends: vec![],
        type_params: vec![],
    }
}

pub fn resolve_formal_parameters(params: &FormalParameters) -> Vec<Param> {
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
                .map(|ann| resolve_ts_type(&ann.type_annotation))
                .unwrap_or(Type::Any);
            let mut param = Param::new(name, ty);
            if p.optional {
                param = param.optional();
            }
            param
        })
        .collect()
}

pub fn get_property_key_name(key: &PropertyKey) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
        PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
        _ => None,
    }
}

/// Build a function type from a Function AST node.
pub fn build_function_type(func: &Function) -> Type {
    let mut params: Vec<Param> = func
        .params
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
                .map(|ann| resolve_ts_type(&ann.type_annotation))
                .unwrap_or(Type::Any);
            let mut param = Param::new(name, ty);
            if p.optional {
                param = param.optional();
            }
            param
        })
        .collect();

    // Handle rest parameter (...args)
    // In oxc, rest parameter is stored separately in params.rest
    // FormalParameterRest has: span, rest (BindingRestElement), type_annotation
    // BindingRestElement has: span, argument (BindingPattern)
    if let Some(rest_param) = &func.params.rest {
        let name = match &rest_param.rest.argument {
            BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
            _ => "args".to_string(),
        };
        // Rest parameter type should be the array type (e.g., number[])
        let ty = rest_param
            .type_annotation
            .as_ref()
            .map(|ann| resolve_ts_type(&ann.type_annotation))
            .unwrap_or(Type::Array(Box::new(Type::Any)));

        let param = Param::new(name, ty).rest();
        params.push(param);
    }

    let return_type = func
        .return_type
        .as_ref()
        .map(|ann| resolve_ts_type(&ann.type_annotation))
        .unwrap_or(Type::Void);

    let type_params: Vec<TypeParam> = func
        .type_parameters
        .as_ref()
        .map(|params| {
            params
                .params
                .iter()
                .map(|p| {
                    let mut tp = TypeParam::new(p.name.name.to_string());
                    if let Some(constraint) = &p.constraint {
                        tp = tp.with_constraint(resolve_ts_type(constraint));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    Type::Function {
        params,
        return_type: Box::new(return_type),
        type_params,
    }
}

/// Build an object type from an interface declaration.
pub fn build_interface_type(decl: &TSInterfaceDeclaration) -> Type {
    let mut properties = Vec::new();
    let mut index_signature = None;

    // Extract type parameters for generic interfaces
    let type_params: Vec<TypeParam> = decl
        .type_parameters
        .as_ref()
        .map(|params| {
            params
                .params
                .iter()
                .map(|p| {
                    let mut tp = TypeParam::new(p.name.name.to_string());
                    if let Some(constraint) = &p.constraint {
                        tp = tp.with_constraint(resolve_ts_type(constraint));
                    }
                    if let Some(default) = &p.default {
                        tp = tp.with_default(resolve_ts_type(default));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    let extends: Vec<Type> = decl
        .extends
        .iter()
        .map(|heritage| {
            let name = match &heritage.expression {
                Expression::Identifier(ident) => ident.name.to_string(),
                _ => return Type::Any,
            };
            let type_args: Vec<Type> = heritage
                .type_arguments
                .as_ref()
                .map(|args| args.params.iter().map(resolve_ts_type).collect())
                .unwrap_or_default();
            Type::TypeRef { name, type_args }
        })
        .collect();

    for member in &decl.body.body {
        match member {
            TSSignature::TSPropertySignature(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);
                    let mut property = Property::new(name, ty);
                    if prop.optional {
                        property = property.optional();
                    }
                    if prop.readonly {
                        property = property.readonly();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSMethodSignature(method) => {
                if let Some(name) = get_property_key_name(&method.key) {
                    let params = resolve_formal_parameters(&method.params);
                    let return_type = method
                        .return_type
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Void);
                    let ty = Type::Function {
                        params,
                        return_type: Box::new(return_type),
                        type_params: vec![],
                    };
                    let mut property = Property::new(name, ty);
                    if method.optional {
                        property = property.optional();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSIndexSignature(idx) => {
                // Index signature: [key: string]: T or [key: number]: T
                if let Some(param) = idx.parameters.first() {
                    let key_type = resolve_ts_type(&param.type_annotation.type_annotation);
                    let value_type = resolve_ts_type(&idx.type_annotation.type_annotation);
                    index_signature = Some(IndexSignature {
                        key_type: Box::new(key_type),
                        value_type: Box::new(value_type),
                    });
                }
            }
            _ => {}
        }
    }

    Type::Object {
        properties,
        index_signature,
        extends,
        type_params,
    }
}

/// Build an object type from a class declaration.
///
/// Returns three types:
/// - instance_type: The type of instances (properties and methods)
/// - constructor_type: The type of the constructor function
/// - static_type: Object type with static properties and methods
pub fn build_class_type(decl: &Class) -> (Type, Option<Type>, Type) {
    let mut properties = Vec::new();
    let mut static_properties = Vec::new();
    let mut constructor_type = None;

    // Extract type parameters for generic classes
    let type_params: Vec<TypeParam> = decl
        .type_parameters
        .as_ref()
        .map(|params| {
            params
                .params
                .iter()
                .map(|p| {
                    let mut tp = TypeParam::new(p.name.name.to_string());
                    if let Some(constraint) = &p.constraint {
                        tp = tp.with_constraint(resolve_ts_type(constraint));
                    }
                    if let Some(default) = &p.default {
                        tp = tp.with_default(resolve_ts_type(default));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    // Build extends list from super class
    let extends: Vec<Type> = if let Some(super_expr) = &decl.super_class {
        if let Expression::Identifier(ident) = super_expr {
            let type_args: Vec<Type> = decl
                .super_type_arguments
                .as_ref()
                .map(|args| args.params.iter().map(resolve_ts_type).collect())
                .unwrap_or_default();
            vec![Type::TypeRef {
                name: ident.name.to_string(),
                type_args,
            }]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    // Process class elements
    for element in &decl.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation))
                        .unwrap_or(Type::Any);
                    let mut property = Property::new(name, ty);
                    if prop.optional {
                        property = property.optional();
                    }
                    if prop.readonly {
                        property = property.readonly();
                    }
                    // Separate static vs instance properties
                    if prop.r#static {
                        static_properties.push(property);
                    } else {
                        properties.push(property);
                    }
                }
            }
            ClassElement::MethodDefinition(method) => {
                // Handle constructor specially
                if method.kind == MethodDefinitionKind::Constructor {
                    let func = &method.value;
                    let params: Vec<Param> = func
                        .params
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
                                .map(|ann| resolve_ts_type(&ann.type_annotation))
                                .unwrap_or(Type::Any);
                            let mut param = Param::new(name, ty);
                            if p.optional {
                                param = param.optional();
                            }
                            param
                        })
                        .collect();

                    // Handle parameter properties (public/private/protected/readonly on params)
                    for p in &func.params.items {
                        // Parameter property if it has accessibility modifier or readonly
                        let is_param_property = p.accessibility.is_some() || p.readonly;
                        if is_param_property {
                            if let BindingPattern::BindingIdentifier(ident) = &p.pattern {
                                let name = ident.name.to_string();
                                let ty = p
                                    .type_annotation
                                    .as_ref()
                                    .map(|ann| resolve_ts_type(&ann.type_annotation))
                                    .unwrap_or(Type::Any);
                                let mut property = Property::new(name, ty);
                                if p.readonly {
                                    property = property.readonly();
                                }
                                properties.push(property);
                            }
                        }
                    }

                    constructor_type = Some(Type::Function {
                        params,
                        return_type: Box::new(Type::Void), // Constructor's "return" is the instance
                        type_params: type_params.clone(), // Use class's type parameters
                    });
                    continue;
                }

                if let Some(name) = get_property_key_name(&method.key) {
                    let func_type = build_function_type(&method.value);
                    let mut property = Property::new(name, func_type);
                    if method.optional {
                        property = property.optional();
                    }
                    // Separate static vs instance methods
                    if method.r#static {
                        static_properties.push(property);
                    } else {
                        properties.push(property);
                    }
                }
            }
            _ => {}
        }
    }

    let instance_type = Type::Object {
        properties,
        index_signature: None,
        extends,
        type_params,
    };

    let static_type = Type::Object {
        properties: static_properties,
        index_signature: None,
        extends: vec![],
        type_params: vec![],
    };

    (instance_type, constructor_type, static_type)
}

/// Widen literal types to their base types.
///
/// Used for `let` and `var` declarations where the type should be mutable:
/// - `"hello"` -> `string`
/// - `42` -> `number`
/// - `true` -> `boolean`
///
/// Also handles nested types (arrays, tuples, objects, unions).
pub fn widen_type(ty: Type) -> Type {
    match ty {
        Type::StringLiteral(_) => Type::String,
        Type::NumberLiteral(_) => Type::Number,
        Type::BooleanLiteral(_) => Type::Boolean,
        Type::Array(inner) => Type::Array(Box::new(widen_type(*inner))),
        Type::Tuple(types) => Type::Tuple(types.into_iter().map(widen_type).collect()),
        Type::Union(types) => Type::Union(types.into_iter().map(widen_type).collect()),
        Type::Object {
            properties,
            index_signature,
            extends,
            type_params,
        } => Type::Object {
            properties: properties
                .into_iter()
                .map(|mut p| {
                    p.ty = widen_type(p.ty);
                    p
                })
                .collect(),
            index_signature,
            extends,
            type_params,
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_widen_string_literal() {
        assert_eq!(widen_type(Type::StringLiteral("hello".into())), Type::String);
    }

    #[test]
    fn test_widen_number_literal() {
        assert_eq!(widen_type(Type::NumberLiteral(42.0)), Type::Number);
    }

    #[test]
    fn test_widen_boolean_literal() {
        assert_eq!(widen_type(Type::BooleanLiteral(true)), Type::Boolean);
    }

    #[test]
    fn test_widen_array() {
        let arr = Type::Array(Box::new(Type::StringLiteral("test".into())));
        assert_eq!(widen_type(arr), Type::Array(Box::new(Type::String)));
    }

    #[test]
    fn test_widen_tuple() {
        let tuple = Type::Tuple(vec![
            Type::StringLiteral("a".into()),
            Type::NumberLiteral(1.0),
        ]);
        assert_eq!(widen_type(tuple), Type::Tuple(vec![Type::String, Type::Number]));
    }

    #[test]
    fn test_widen_union() {
        let union = Type::Union(vec![
            Type::StringLiteral("a".into()),
            Type::NumberLiteral(1.0),
        ]);
        assert_eq!(widen_type(union), Type::Union(vec![Type::String, Type::Number]));
    }

    #[test]
    fn test_widen_preserves_primitives() {
        assert_eq!(widen_type(Type::String), Type::String);
        assert_eq!(widen_type(Type::Number), Type::Number);
        assert_eq!(widen_type(Type::Boolean), Type::Boolean);
    }
}
