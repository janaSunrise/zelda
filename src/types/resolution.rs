//! AST to Type resolution.
//!
//! This module converts oxc TypeScript AST nodes to our Type representation.

use oxc_ast::ast::*;

use super::{IndexSignature, Param, Property, Type, TypeArena, TypeId, TypeParam};

/// Convert an oxc TSType AST node to our Type representation.
pub fn resolve_ts_type(ts_type: &TSType, arena: &mut TypeArena) -> TypeId {
    match ts_type {
        // Primitive keywords - use pre-cached TypeId constants
        TSType::TSStringKeyword(_) => TypeId::STRING,
        TSType::TSNumberKeyword(_) => TypeId::NUMBER,
        TSType::TSBooleanKeyword(_) => TypeId::BOOLEAN,
        TSType::TSNullKeyword(_) => TypeId::NULL,
        TSType::TSUndefinedKeyword(_) => TypeId::UNDEFINED,
        TSType::TSVoidKeyword(_) => TypeId::VOID,
        TSType::TSAnyKeyword(_) => TypeId::ANY,
        TSType::TSUnknownKeyword(_) => TypeId::UNKNOWN,
        TSType::TSNeverKeyword(_) => TypeId::NEVER,

        // Literal types
        TSType::TSLiteralType(lit) => match &lit.literal {
            TSLiteral::StringLiteral(s) => {
                arena.intern(Type::StringLiteral(s.value.to_string()))
            }
            TSLiteral::NumericLiteral(n) => arena.intern(Type::NumberLiteral(n.value)),
            TSLiteral::BooleanLiteral(b) => {
                if b.value {
                    TypeId::TRUE
                } else {
                    TypeId::FALSE
                }
            }
            _ => TypeId::ANY,
        },

        // Array types
        TSType::TSArrayType(arr) => {
            let elem_id = resolve_ts_type(&arr.element_type, arena);
            arena.array(elem_id)
        }

        // Tuple types
        TSType::TSTupleType(tuple) => {
            let types: Vec<TypeId> = tuple
                .element_types
                .iter()
                .map(|e| resolve_tuple_element(e, arena))
                .collect();
            arena.tuple(types)
        }

        // Union types
        TSType::TSUnionType(union) => {
            let types: Vec<TypeId> = union
                .types
                .iter()
                .map(|t| resolve_ts_type(t, arena))
                .collect();
            arena.union(types)
        }

        // Intersection types
        TSType::TSIntersectionType(inter) => {
            let types: Vec<TypeId> = inter
                .types
                .iter()
                .map(|t| resolve_ts_type(t, arena))
                .collect();
            arena.intersection(types)
        }

        // Type references (e.g., Array<T>, Promise<T>, custom types)
        TSType::TSTypeReference(type_ref) => {
            let name = match &type_ref.type_name {
                TSTypeName::IdentifierReference(ident) => ident.name.to_string(),
                TSTypeName::QualifiedName(qual) => qual.right.name.to_string(),
                TSTypeName::ThisExpression(_) => "this".to_string(),
            };

            let type_args: Vec<TypeId> = type_ref
                .type_arguments
                .as_ref()
                .map(|params| params.params.iter().map(|t| resolve_ts_type(t, arena)).collect())
                .unwrap_or_default();

            arena.type_ref(name, type_args)
        }

        // Function types
        TSType::TSFunctionType(func) => resolve_function_type(func, arena),

        // Type literals (inline object types)
        TSType::TSTypeLiteral(lit) => resolve_type_literal(lit, arena),

        // Parenthesized types
        TSType::TSParenthesizedType(paren) => resolve_ts_type(&paren.type_annotation, arena),

        // Type operators (keyof, unique, readonly)
        TSType::TSTypeOperatorType(op) => {
            match op.operator {
                TSTypeOperatorOperator::Keyof => {
                    let inner_id = resolve_ts_type(&op.type_annotation, arena);
                    arena.intern(Type::KeyOf(inner_id))
                }
                TSTypeOperatorOperator::Readonly => {
                    // For now, pass through (readonly modifier on mapped types)
                    resolve_ts_type(&op.type_annotation, arena)
                }
                TSTypeOperatorOperator::Unique => {
                    // unique symbol - just treat as the underlying type
                    resolve_ts_type(&op.type_annotation, arena)
                }
            }
        }

        // Indexed access types: T[K]
        TSType::TSIndexedAccessType(access) => {
            let object_id = resolve_ts_type(&access.object_type, arena);
            let index_id = resolve_ts_type(&access.index_type, arena);
            arena.intern(Type::IndexedAccess {
                object_type: object_id,
                index_type: index_id,
            })
        }

        // Mapped types: { [K in keyof T]: T[K] }
        TSType::TSMappedType(mapped) => {
            // Key type parameter name (e.g., "P" in [P in keyof T])
            let type_param = mapped.key.name.to_string();

            // The constraint (e.g., "keyof T" in [P in keyof T])
            let constraint_id = resolve_ts_type(&mapped.constraint, arena);

            // The template is the value type (e.g., "T[K]" in "{ [K in keyof T]: T[K] }")
            let template_id = mapped
                .type_annotation
                .as_ref()
                .map(|a| resolve_ts_type(a, arena))
                .unwrap_or(TypeId::ANY);

            // Handle modifiers
            // readonly_modifier: +readonly, -readonly, or none
            let readonly_modifier = mapped.readonly.map(|op| {
                matches!(
                    op,
                    TSMappedTypeModifierOperator::True | TSMappedTypeModifierOperator::Plus
                )
            });

            // optional_modifier: +?, -?, or none
            let optional_modifier = mapped.optional.map(|op| {
                matches!(
                    op,
                    TSMappedTypeModifierOperator::True | TSMappedTypeModifierOperator::Plus
                )
            });

            arena.intern(Type::MappedType {
                type_param,
                constraint: constraint_id,
                template: template_id,
                readonly_modifier,
                optional_modifier,
            })
        }

        // Type predicates resolve to boolean when used as standalone types.
        // The predicate details are extracted in build_function_type for narrowing.
        TSType::TSTypePredicate(_) => TypeId::BOOLEAN,

        // Conditional types: T extends U ? X : Y
        TSType::TSConditionalType(cond) => {
            let check_id = resolve_ts_type(&cond.check_type, arena);
            let extends_id = resolve_ts_type(&cond.extends_type, arena);
            let true_id = resolve_ts_type(&cond.true_type, arena);
            let false_id = resolve_ts_type(&cond.false_type, arena);
            arena.intern(Type::ConditionalType {
                check_type: check_id,
                extends_type: extends_id,
                true_type: true_id,
                false_type: false_id,
            })
        }

        // Infer types: infer R (used in conditional type extends clauses)
        TSType::TSInferType(infer) => {
            let constraint = infer
                .type_parameter
                .constraint
                .as_ref()
                .map(|c| resolve_ts_type(c, arena));
            arena.intern(Type::InferType {
                name: infer.type_parameter.name.name.to_string(),
                constraint,
            })
        }

        // Template literal types: `hello${string}world`
        TSType::TSTemplateLiteralType(template) => {
            let mut texts = Vec::new();
            let mut types = Vec::new();

            // Build texts and types from quasis and types
            for (i, quasi) in template.quasis.iter().enumerate() {
                texts.push(quasi.value.raw.to_string());
                if let Some(ty) = template.types.get(i) {
                    types.push(resolve_ts_type(ty, arena));
                }
            }

            arena.intern(Type::TemplateLiteralType { texts, types })
        }

        _ => TypeId::ANY,
    }
}

/// Resolve a tuple element type.
///
/// Tuple elements can be:
/// - Regular: `[string, number]`
/// - Optional: `[string, number?]`
/// - Rest: `[string, ...number[]]`
/// - Named: `[name: string, age: number]`
pub fn resolve_tuple_element(elem: &TSTupleElement, arena: &mut TypeArena) -> TypeId {
    match elem {
        TSTupleElement::TSOptionalType(opt) => resolve_ts_type(&opt.type_annotation, arena),
        TSTupleElement::TSRestType(rest) => resolve_ts_type(&rest.type_annotation, arena),
        TSTupleElement::TSNamedTupleMember(named) => {
            resolve_tuple_element(&named.element_type, arena)
        }
        _ => {
            if let Some(ty) = elem.as_ts_type() {
                resolve_ts_type(ty, arena)
            } else {
                TypeId::ANY
            }
        }
    }
}

pub fn resolve_function_type(func: &TSFunctionType, arena: &mut TypeArena) -> TypeId {
    let params = resolve_formal_parameters(&func.params, arena);
    let return_type_id = resolve_ts_type(&func.return_type.type_annotation, arena);

    arena.function(params, return_type_id)
}

pub fn resolve_type_literal(lit: &TSTypeLiteral, arena: &mut TypeArena) -> TypeId {
    let mut properties = Vec::new();
    let mut index_signature = None;

    for member in &lit.members {
        match member {
            TSSignature::TSPropertySignature(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty_id = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                        .unwrap_or(TypeId::ANY);
                    let mut property = Property::new(name, ty_id);
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
                    let params = resolve_formal_parameters(&method.params, arena);
                    let return_type_id = method
                        .return_type
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                        .unwrap_or(TypeId::VOID);
                    let method_ty_id = arena.function(params, return_type_id);
                    let mut property = Property::new(name, method_ty_id);
                    if method.optional {
                        property = property.optional();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSIndexSignature(idx) => {
                // Index signature: [key: string]: T or [key: number]: T
                if let Some(param) = idx.parameters.first() {
                    let key_type_id = resolve_ts_type(&param.type_annotation.type_annotation, arena);
                    let value_type_id = resolve_ts_type(&idx.type_annotation.type_annotation, arena);
                    index_signature = Some(IndexSignature {
                        key_type: key_type_id,
                        value_type: value_type_id,
                    });
                }
            }
            _ => {}
        }
    }

    arena.intern(Type::Object {
        properties,
        index_signature,
        extends: vec![],
        type_params: vec![],
    })
}

pub fn resolve_formal_parameters(params: &FormalParameters, arena: &mut TypeArena) -> Vec<Param> {
    let mut result: Vec<Param> = params
        .items
        .iter()
        .map(|p| {
            let name = match &p.pattern {
                BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                _ => "_".to_string(),
            };
            let ty_id = p
                .type_annotation
                .as_ref()
                .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                .unwrap_or(TypeId::ANY);
            let mut param = Param::new(name, ty_id);
            if p.optional {
                param = param.optional();
            }
            param
        })
        .collect();

    // Handle rest parameter (...args) if present
    // Rest parameter is stored separately in params.rest, not in params.items
    if let Some(rest_param) = &params.rest {
        let name = match &rest_param.rest.argument {
            BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
            _ => "args".to_string(),
        };
        // Rest parameter type should be the array type (e.g., any[] for ...data: any[])
        let ty_id = rest_param
            .type_annotation
            .as_ref()
            .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
            .unwrap_or_else(|| arena.array(TypeId::ANY));
        result.push(Param::new(name, ty_id).rest());
    }

    result
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
pub fn build_function_type(func: &Function, arena: &mut TypeArena) -> TypeId {
    let mut params: Vec<Param> = func
        .params
        .items
        .iter()
        .map(|p| {
            let name = match &p.pattern {
                BindingPattern::BindingIdentifier(ident) => ident.name.to_string(),
                _ => "_".to_string(),
            };
            let ty_id = p
                .type_annotation
                .as_ref()
                .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                .unwrap_or(TypeId::ANY);
            let mut param = Param::new(name, ty_id);
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
        let ty_id = rest_param
            .type_annotation
            .as_ref()
            .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
            .unwrap_or_else(|| arena.array(TypeId::ANY));

        let param = Param::new(name, ty_id).rest();
        params.push(param);
    }

    // Handle type predicates specially. Assertion functions like "asserts val is string"
    // have void as their effective return type since they throw on failure. Regular type
    // guards like "val is string" return boolean.
    let (return_type_id, type_predicate) = if let Some(ann) = &func.return_type {
        if let TSType::TSTypePredicate(pred) = &ann.type_annotation {
            let parameter_name = match &pred.parameter_name {
                TSTypePredicateName::Identifier(ident) => ident.name.to_string(),
                TSTypePredicateName::This(_) => "this".to_string(),
            };

            let type_annotation = pred
                .type_annotation
                .as_ref()
                .map(|ann| resolve_ts_type(&ann.type_annotation, arena));

            let predicate = super::TypePredicate {
                parameter_name,
                asserts: pred.asserts,
                type_annotation,
            };

            let ret_id = if pred.asserts {
                TypeId::VOID
            } else {
                TypeId::BOOLEAN
            };
            (ret_id, Some(predicate))
        } else {
            (resolve_ts_type(&ann.type_annotation, arena), None)
        }
    } else {
        (TypeId::VOID, None)
    };

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
                        tp = tp.with_constraint(resolve_ts_type(constraint, arena));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    arena.intern(Type::Function {
        params,
        return_type: return_type_id,
        type_params,
        type_predicate,
    })
}

/// Build an object type from an interface declaration.
pub fn build_interface_type(decl: &TSInterfaceDeclaration, arena: &mut TypeArena) -> TypeId {
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
                        tp = tp.with_constraint(resolve_ts_type(constraint, arena));
                    }
                    if let Some(default) = &p.default {
                        tp = tp.with_default(resolve_ts_type(default, arena));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    let extends: Vec<TypeId> = decl
        .extends
        .iter()
        .map(|heritage| {
            let name = match &heritage.expression {
                Expression::Identifier(ident) => ident.name.to_string(),
                _ => return TypeId::ANY,
            };
            let type_args: Vec<TypeId> = heritage
                .type_arguments
                .as_ref()
                .map(|args| args.params.iter().map(|t| resolve_ts_type(t, arena)).collect())
                .unwrap_or_default();
            arena.type_ref(name, type_args)
        })
        .collect();

    for member in &decl.body.body {
        match member {
            TSSignature::TSPropertySignature(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty_id = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                        .unwrap_or(TypeId::ANY);
                    let mut property = Property::new(name, ty_id);
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
                    let params = resolve_formal_parameters(&method.params, arena);
                    let return_type_id = method
                        .return_type
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                        .unwrap_or(TypeId::VOID);
                    let method_ty_id = arena.function(params, return_type_id);
                    let mut property = Property::new(name, method_ty_id);
                    if method.optional {
                        property = property.optional();
                    }
                    properties.push(property);
                }
            }
            TSSignature::TSIndexSignature(idx) => {
                // Index signature: [key: string]: T or [key: number]: T
                if let Some(param) = idx.parameters.first() {
                    let key_type_id = resolve_ts_type(&param.type_annotation.type_annotation, arena);
                    let value_type_id = resolve_ts_type(&idx.type_annotation.type_annotation, arena);
                    index_signature = Some(IndexSignature {
                        key_type: key_type_id,
                        value_type: value_type_id,
                    });
                }
            }
            _ => {}
        }
    }

    arena.intern(Type::Object {
        properties,
        index_signature,
        extends,
        type_params,
    })
}

/// Build an object type from a class declaration.
///
/// Returns three TypeIds:
/// - instance_type: The type of instances (properties and methods)
/// - constructor_type: The type of the constructor function
/// - static_type: Object type with static properties and methods
pub fn build_class_type(decl: &Class, arena: &mut TypeArena) -> (TypeId, Option<TypeId>, TypeId) {
    let mut properties = Vec::new();
    let mut static_properties = Vec::new();
    let mut constructor_type_id = None;

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
                        tp = tp.with_constraint(resolve_ts_type(constraint, arena));
                    }
                    if let Some(default) = &p.default {
                        tp = tp.with_default(resolve_ts_type(default, arena));
                    }
                    tp
                })
                .collect()
        })
        .unwrap_or_default();

    // Build extends list from super class
    let extends: Vec<TypeId> =
        if let Some(Expression::Identifier(ident)) = decl.super_class.as_ref() {
            let type_args: Vec<TypeId> = decl
                .super_type_arguments
                .as_ref()
                .map(|args| args.params.iter().map(|t| resolve_ts_type(t, arena)).collect())
                .unwrap_or_default();
            vec![arena.type_ref(ident.name.to_string(), type_args)]
        } else {
            vec![]
        };

    // Process class elements
    for element in &decl.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                if let Some(name) = get_property_key_name(&prop.key) {
                    let ty_id = prop
                        .type_annotation
                        .as_ref()
                        .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                        .unwrap_or(TypeId::ANY);
                    let mut property = Property::new(name, ty_id);
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
                            let ty_id = p
                                .type_annotation
                                .as_ref()
                                .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                                .unwrap_or(TypeId::ANY);
                            let mut param = Param::new(name, ty_id);
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
                        if is_param_property
                            && let BindingPattern::BindingIdentifier(ident) = &p.pattern
                        {
                            let name = ident.name.to_string();
                            let ty_id = p
                                .type_annotation
                                .as_ref()
                                .map(|ann| resolve_ts_type(&ann.type_annotation, arena))
                                .unwrap_or(TypeId::ANY);
                            let mut property = Property::new(name, ty_id);
                            if p.readonly {
                                property = property.readonly();
                            }
                            properties.push(property);
                        }
                    }

                    constructor_type_id = Some(arena.intern(Type::Function {
                        params,
                        return_type: TypeId::VOID, // Constructor's "return" is the instance
                        type_params: type_params.clone(), // Use class's type parameters
                        type_predicate: None,
                    }));
                    continue;
                }

                if let Some(name) = get_property_key_name(&method.key) {
                    let func_type_id = build_function_type(&method.value, arena);
                    let mut property = Property::new(name, func_type_id);
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

    let instance_type_id = arena.intern(Type::Object {
        properties,
        index_signature: None,
        extends,
        type_params,
    });

    let static_type_id = arena.object(static_properties);

    (instance_type_id, constructor_type_id, static_type_id)
}

/// Widen literal types to their base types.
///
/// Used for `let` and `var` declarations where the type should be mutable:
/// - `"hello"` -> `string`
/// - `42` -> `number`
/// - `true` -> `boolean`
///
/// Also handles nested types (arrays, tuples, objects, unions).
pub fn widen_type(ty_id: TypeId, arena: &mut TypeArena) -> TypeId {
    // Handle boolean literals (TRUE and FALSE need widening to BOOLEAN)
    if ty_id == TypeId::TRUE || ty_id == TypeId::FALSE {
        return TypeId::BOOLEAN;
    }

    // Fast path: primitives don't need widening
    if ty_id.is_primitive() {
        return ty_id;
    }

    // Clone the type to avoid borrow issues
    let ty = arena.get(ty_id).clone();

    match ty {
        Type::StringLiteral(_) => TypeId::STRING,
        Type::NumberLiteral(_) => TypeId::NUMBER,
        Type::BooleanLiteral(_) => TypeId::BOOLEAN,
        Type::Array(inner_id) => {
            let widened_inner = widen_type(inner_id, arena);
            arena.array(widened_inner)
        }
        Type::Tuple(type_ids) => {
            let widened: Vec<TypeId> = type_ids
                .into_iter()
                .map(|id| widen_type(id, arena))
                .collect();
            arena.tuple(widened)
        }
        Type::Union(type_ids) => {
            let widened: Vec<TypeId> = type_ids
                .into_iter()
                .map(|id| widen_type(id, arena))
                .collect();
            arena.union(widened)
        }
        Type::Object {
            properties,
            index_signature,
            extends,
            type_params,
        } => {
            let widened_properties: Vec<Property> = properties
                .into_iter()
                .map(|mut p| {
                    p.ty = widen_type(p.ty, arena);
                    p
                })
                .collect();
            arena.intern(Type::Object {
                properties: widened_properties,
                index_signature,
                extends,
                type_params,
            })
        }
        _ => ty_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_widen_string_literal() {
        let mut arena = TypeArena::new();
        let str_lit_id = arena.intern(Type::StringLiteral("hello".into()));
        assert_eq!(widen_type(str_lit_id, &mut arena), TypeId::STRING);
    }

    #[test]
    fn test_widen_number_literal() {
        let mut arena = TypeArena::new();
        let num_lit_id = arena.intern(Type::NumberLiteral(42.0));
        assert_eq!(widen_type(num_lit_id, &mut arena), TypeId::NUMBER);
    }

    #[test]
    fn test_widen_boolean_literal() {
        let mut arena = TypeArena::new();
        // BooleanLiteral(true) is pre-cached as TypeId::TRUE
        assert_eq!(widen_type(TypeId::TRUE, &mut arena), TypeId::BOOLEAN);
    }

    #[test]
    fn test_widen_array() {
        let mut arena = TypeArena::new();
        let str_lit_id = arena.intern(Type::StringLiteral("test".into()));
        let arr_id = arena.array(str_lit_id);
        let widened_id = widen_type(arr_id, &mut arena);
        // Should become string[]
        let expected_id = arena.array(TypeId::STRING);
        assert_eq!(widened_id, expected_id);
    }

    #[test]
    fn test_widen_tuple() {
        let mut arena = TypeArena::new();
        let str_lit_id = arena.intern(Type::StringLiteral("a".into()));
        let num_lit_id = arena.intern(Type::NumberLiteral(1.0));
        let tuple_id = arena.tuple(vec![str_lit_id, num_lit_id]);
        let widened_id = widen_type(tuple_id, &mut arena);
        let expected_id = arena.tuple(vec![TypeId::STRING, TypeId::NUMBER]);
        assert_eq!(widened_id, expected_id);
    }

    #[test]
    fn test_widen_union() {
        let mut arena = TypeArena::new();
        let str_lit_id = arena.intern(Type::StringLiteral("a".into()));
        let num_lit_id = arena.intern(Type::NumberLiteral(1.0));
        let union_id = arena.union(vec![str_lit_id, num_lit_id]);
        let widened_id = widen_type(union_id, &mut arena);
        let expected_id = arena.union(vec![TypeId::STRING, TypeId::NUMBER]);
        assert_eq!(widened_id, expected_id);
    }

    #[test]
    fn test_widen_preserves_primitives() {
        let mut arena = TypeArena::new();
        assert_eq!(widen_type(TypeId::STRING, &mut arena), TypeId::STRING);
        assert_eq!(widen_type(TypeId::NUMBER, &mut arena), TypeId::NUMBER);
        assert_eq!(widen_type(TypeId::BOOLEAN, &mut arena), TypeId::BOOLEAN);
    }
}
