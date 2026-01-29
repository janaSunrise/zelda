//! Type resolution and manipulation.

use std::collections::HashMap;

use oxc_ast::ast::*;

use crate::types::resolution;
use crate::types::{IndexSignature, Param, Property, Type, TypeParam};

use super::Checker;

impl<'a> Checker<'a> {
    pub(super) fn resolve_ts_type(&self, ts_type: &TSType) -> Type {
        // TSTypeQuery needs symbol table access, so handle it here rather than in resolution module
        if let TSType::TSTypeQuery(query) = ts_type {
            return self.resolve_type_query(query);
        }
        resolution::resolve_ts_type(ts_type)
    }

    /// `typeof x` -> look up x's type in symbol table
    fn resolve_type_query(&self, query: &oxc_ast::ast::TSTypeQuery) -> Type {
        use oxc_ast::ast::{TSTypeQueryExprName, TSTypeName};

        match &query.expr_name {
            TSTypeQueryExprName::IdentifierReference(ident) => {
                let name = ident.name.as_str();
                self.symbols.lookup(name).map(|s| s.ty.clone()).unwrap_or(Type::Any)
            }
            // TODO: resolve full qualified chain instead of just the last part
            TSTypeQueryExprName::QualifiedName(qual) => {
                let name = qual.right.name.as_str();
                self.symbols.lookup(name).map(|s| s.ty.clone()).unwrap_or(Type::Any)
            }
            TSTypeQueryExprName::TSImportType(_) => Type::Any,
            TSTypeQueryExprName::ThisExpression(_) => Type::Any,
        }
    }

    pub fn widen_type(&self, ty: Type) -> Type {
        resolution::widen_type(ty)
    }

    /// Flatten and deduplicate a union of two types.
    pub(super) fn union_types(&self, a: Type, b: Type) -> Type {
        if a == b {
            return a;
        }

        let mut types = Vec::new();
        match a {
            Type::Union(inner) => types.extend(inner),
            other => types.push(other),
        }
        match b {
            Type::Union(inner) => types.extend(inner),
            other => types.push(other),
        }

        types.sort();
        types.dedup();

        if types.len() == 1 {
            types.pop().unwrap()
        } else {
            Type::Union(types)
        }
    }

    /// [] -> never, [T] -> T, [T, U, ...] -> T | U | ...
    pub(super) fn unify_types(&self, types: Vec<Type>) -> Type {
        match types.len() {
            0 => Type::Never,
            1 => types.into_iter().next().unwrap(),
            _ => {
                let mut iter = types.into_iter();
                let mut result = iter.next().unwrap();
                for ty in iter {
                    result = self.union_types(result, ty);
                }
                result
            }
        }
    }

    /// Replace type parameters with concrete types.
    /// `Array<T>` with {T -> string} => `Array<string>`
    pub fn substitute_type_params(&self, ty: &Type, substitutions: &HashMap<String, Type>) -> Type {
        match ty {
            Type::TypeParameter { name, .. } => {
                substitutions.get(name).cloned().unwrap_or_else(|| ty.clone())
            }

            Type::TypeRef { name, type_args } => {
                // Bare TypeRef with no args might be a type parameter reference
                if type_args.is_empty() {
                    if let Some(substituted) = substitutions.get(name) {
                        return substituted.clone();
                    }
                }

                let new_args: Vec<Type> = type_args
                    .iter()
                    .map(|arg| self.substitute_type_params(arg, substitutions))
                    .collect();

                Type::TypeRef {
                    name: name.clone(),
                    type_args: new_args,
                }
            }

            // Compound types: recurse into their components
            Type::Array(elem) => {
                Type::Array(Box::new(self.substitute_type_params(elem, substitutions)))
            }

            Type::Tuple(types) => {
                Type::Tuple(types.iter().map(|t| self.substitute_type_params(t, substitutions)).collect())
            }

            Type::Union(types) => {
                Type::Union(types.iter().map(|t| self.substitute_type_params(t, substitutions)).collect())
            }

            Type::Intersection(types) => {
                Type::Intersection(types.iter().map(|t| self.substitute_type_params(t, substitutions)).collect())
            }

            Type::Object { properties, index_signature, extends, type_params } => {
                // Substitute type parameters in properties
                // Note: When instantiating Box<number>, we DO want to substitute T -> number in properties
                let new_props: Vec<Property> = properties
                    .iter()
                    .map(|p| Property {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(&p.ty, substitutions),
                        optional: p.optional,
                        readonly: p.readonly,
                    })
                    .collect();

                let new_idx = index_signature.as_ref().map(|idx| IndexSignature {
                    key_type: Box::new(self.substitute_type_params(&idx.key_type, substitutions)),
                    value_type: Box::new(self.substitute_type_params(&idx.value_type, substitutions)),
                });

                let new_extends: Vec<Type> = extends
                    .iter()
                    .map(|t| self.substitute_type_params(t, substitutions))
                    .collect();

                // After instantiation, remove the type parameters that have been substituted
                // If all type params are substituted, the result is a concrete type
                let remaining_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .filter(|tp| !substitutions.contains_key(&tp.name))
                    .cloned()
                    .collect();

                Type::Object {
                    properties: new_props,
                    index_signature: new_idx,
                    extends: new_extends,
                    type_params: remaining_type_params,
                }
            }

            Type::Function { params, return_type, type_params, type_predicate } => {
                // Don't substitute the function's own type parameters, only free variables
                let bound_names: std::collections::HashSet<_> = type_params.iter().map(|tp| tp.name.clone()).collect();
                let filtered_subs: HashMap<String, Type> = substitutions
                    .iter()
                    .filter(|(k, _)| !bound_names.contains(*k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                let new_params: Vec<Param> = params
                    .iter()
                    .map(|p| Param {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(&p.ty, &filtered_subs),
                        optional: p.optional,
                        rest: p.rest,
                    })
                    .collect();

                let new_return = self.substitute_type_params(return_type, &filtered_subs);

                // Also substitute in type param constraints and defaults
                let new_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .map(|tp| TypeParam {
                        name: tp.name.clone(),
                        constraint: tp.constraint.as_ref().map(|c| Box::new(self.substitute_type_params(c, &filtered_subs))),
                        default: tp.default.as_ref().map(|d| Box::new(self.substitute_type_params(d, &filtered_subs))),
                    })
                    .collect();

                let new_predicate = type_predicate.as_ref().map(|tp| crate::types::TypePredicate {
                    parameter_name: tp.parameter_name.clone(),
                    asserts: tp.asserts,
                    type_annotation: tp.type_annotation.as_ref().map(|ty| Box::new(self.substitute_type_params(ty, &filtered_subs))),
                });

                Type::Function {
                    params: new_params,
                    return_type: Box::new(new_return),
                    type_params: new_type_params,
                    type_predicate: new_predicate,
                }
            }

            Type::ClassConstructor { params, type_params, static_members } => {
                // Similar to Function, don't substitute the constructor's own type parameters
                let bound_names: std::collections::HashSet<_> = type_params.iter().map(|tp| tp.name.clone()).collect();
                let filtered_subs: HashMap<String, Type> = substitutions
                    .iter()
                    .filter(|(k, _)| !bound_names.contains(*k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                let new_params: Vec<Param> = params
                    .iter()
                    .map(|p| Param {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(&p.ty, &filtered_subs),
                        optional: p.optional,
                        rest: p.rest,
                    })
                    .collect();

                let new_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .map(|tp| TypeParam {
                        name: tp.name.clone(),
                        constraint: tp.constraint.as_ref().map(|c| Box::new(self.substitute_type_params(c, &filtered_subs))),
                        default: tp.default.as_ref().map(|d| Box::new(self.substitute_type_params(d, &filtered_subs))),
                    })
                    .collect();

                let new_static_members: Vec<Property> = static_members
                    .iter()
                    .map(|p| Property {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(&p.ty, &filtered_subs),
                        optional: p.optional,
                        readonly: p.readonly,
                    })
                    .collect();

                Type::ClassConstructor {
                    params: new_params,
                    type_params: new_type_params,
                    static_members: new_static_members,
                }
            }

            // KeyOf: substitute into the inner type
            Type::KeyOf(inner) => {
                Type::KeyOf(Box::new(self.substitute_type_params(inner, substitutions)))
            }

            // IndexedAccess: substitute into both parts
            Type::IndexedAccess { object_type, index_type } => {
                Type::IndexedAccess {
                    object_type: Box::new(self.substitute_type_params(object_type, substitutions)),
                    index_type: Box::new(self.substitute_type_params(index_type, substitutions)),
                }
            }

            // MappedType: substitute into constraint and template, but not the bound type_param
            Type::MappedType { type_param, constraint, template, readonly_modifier, optional_modifier } => {
                // The type_param is a bound variable in the mapped type, so filter it from substitutions
                let filtered_subs: HashMap<String, Type> = substitutions
                    .iter()
                    .filter(|(k, _)| *k != type_param)
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                Type::MappedType {
                    type_param: type_param.clone(),
                    constraint: Box::new(self.substitute_type_params(constraint, &filtered_subs)),
                    template: Box::new(self.substitute_type_params(template, &filtered_subs)),
                    readonly_modifier: *readonly_modifier,
                    optional_modifier: *optional_modifier,
                }
            }

            // ConditionalType: substitute into all parts
            Type::ConditionalType { check_type, extends_type, true_type, false_type } => {
                Type::ConditionalType {
                    check_type: Box::new(self.substitute_type_params(check_type, substitutions)),
                    extends_type: Box::new(self.substitute_type_params(extends_type, substitutions)),
                    true_type: Box::new(self.substitute_type_params(true_type, substitutions)),
                    false_type: Box::new(self.substitute_type_params(false_type, substitutions)),
                }
            }

            // InferType: the inferred variable should not be substituted (it's being defined)
            Type::InferType { name, constraint } => {
                // Filter out the infer variable name from substitutions
                let filtered_subs: HashMap<String, Type> = substitutions
                    .iter()
                    .filter(|(k, _)| *k != name)
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();

                Type::InferType {
                    name: name.clone(),
                    constraint: constraint.as_ref().map(|c| Box::new(self.substitute_type_params(c, &filtered_subs))),
                }
            }

            // TemplateLiteralType: substitute into the type placeholders
            Type::TemplateLiteralType { texts, types } => {
                Type::TemplateLiteralType {
                    texts: texts.clone(),
                    types: types.iter().map(|t| self.substitute_type_params(t, substitutions)).collect(),
                }
            }

            // Primitives and literals: no substitution needed
            Type::String
            | Type::Number
            | Type::Boolean
            | Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Any
            | Type::Unknown
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => ty.clone(),
        }
    }

    /// Resolve a keyof type to a union of string literal types.
    ///
    /// `keyof { x: number; y: string }` resolves to `"x" | "y"`
    pub fn resolve_keyof(&self, ty: &Type) -> Type {
        // First resolve the type if it's a TypeRef
        let resolved = match ty {
            Type::TypeRef { name, type_args } => {
                self.resolve_type_ref_with_args(name, type_args)
                    .unwrap_or_else(|| ty.clone())
            }
            Type::TypeParameter { constraint: Some(constraint), .. } => {
                // For a type parameter with constraint, keyof T extends C gives keyof C
                return self.resolve_keyof(constraint);
            }
            other => other.clone(),
        };

        match &resolved {
            Type::Object { properties, extends, .. } => {
                // Collect all property names including from extended interfaces
                let all_props = self.resolve_object_properties(properties, extends);
                let keys: Vec<Type> = all_props
                    .iter()
                    .map(|p| Type::StringLiteral(p.name.clone()))
                    .collect();

                if keys.is_empty() {
                    Type::Never
                } else if keys.len() == 1 {
                    keys.into_iter().next().unwrap()
                } else {
                    Type::Union(keys)
                }
            }
            Type::Union(types) => {
                // keyof (A | B) = (keyof A) & (keyof B)
                let resolved_keys: Vec<Type> = types
                    .iter()
                    .map(|t| self.resolve_keyof(t))
                    .collect();
                if resolved_keys.is_empty() {
                    Type::Never
                } else if resolved_keys.len() == 1 {
                    resolved_keys.into_iter().next().unwrap()
                } else {
                    Type::Intersection(resolved_keys)
                }
            }
            Type::Intersection(types) => {
                // keyof (A & B) = (keyof A) | (keyof B)
                let resolved_keys: Vec<Type> = types
                    .iter()
                    .map(|t| self.resolve_keyof(t))
                    .collect();
                self.unify_types(resolved_keys)
            }
            Type::Any => Type::Union(vec![Type::String, Type::Number]),
            Type::Unknown => Type::Never,
            _ => Type::Never, // Primitives have no keys
        }
    }

    /// Resolve a mapped type to a concrete object type.
    ///
    /// `{ [K in keyof Person]: Person[K] }` resolves to `{ name: string; age: number }`
    pub fn resolve_mapped_type(
        &self,
        type_param: &str,
        constraint: &Type,
        template: &Type,
        readonly_modifier: Option<bool>,
        optional_modifier: Option<bool>,
    ) -> Type {
        // First resolve the constraint to get the keys
        let resolved_constraint = if let Type::KeyOf(inner) = constraint {
            self.resolve_keyof(inner)
        } else {
            constraint.clone()
        };

        // Get the list of keys to iterate over
        let keys: Vec<Type> = match &resolved_constraint {
            Type::Union(types) => types.clone(),
            Type::StringLiteral(_) => vec![resolved_constraint.clone()],
            Type::Never => return Type::Object {
                properties: vec![],
                index_signature: None,
                extends: vec![],
                type_params: vec![],
            },
            _ => return Type::Any, // Can't resolve mapped type with this constraint
        };

        // Build properties by substituting each key
        let mut properties: Vec<Property> = Vec::new();

        for key in keys {
            if let Type::StringLiteral(key_name) = &key {
                // Create substitution map: type_param -> key
                let mut subs = HashMap::new();
                subs.insert(type_param.to_string(), key.clone());

                // Substitute in the template to get the property type
                let prop_type = self.substitute_type_params(template, &subs);

                // If the template is an indexed access like T[K], resolve it
                let resolved_prop_type = if let Type::IndexedAccess { object_type, index_type } = &prop_type {
                    self.resolve_indexed_access(object_type, index_type)
                } else {
                    prop_type
                };

                let mut prop = Property::new(key_name.clone(), resolved_prop_type);

                // Apply modifiers
                if let Some(true) = optional_modifier {
                    prop = prop.optional();
                }
                if let Some(true) = readonly_modifier {
                    prop = prop.readonly();
                }

                properties.push(prop);
            }
        }

        Type::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        }
    }

    /// Resolve an indexed access type T[K] to the property type.
    ///
    /// `Person["name"]` resolves to `string`
    /// `Person[keyof Person]` resolves to union of all property types
    pub fn resolve_indexed_access(&self, object_type: &Type, index_type: &Type) -> Type {
        // First, try to resolve KeyOf if that's what index_type is
        let resolved_index = if let Type::KeyOf(inner) = index_type {
            self.resolve_keyof(inner)
        } else {
            index_type.clone()
        };

        // Resolve the object type if it's a TypeRef
        let resolved_object = match object_type {
            Type::TypeRef { name, type_args } => {
                self.resolve_type_ref_with_args(name, type_args)
                    .unwrap_or_else(|| object_type.clone())
            }
            other => other.clone(),
        };

        match &resolved_index {
            // String literal key: T["prop"]
            Type::StringLiteral(key) => {
                self.get_property_type(&resolved_object, key)
            }
            // Union of keys: T["a" | "b"] = T["a"] | T["b"]
            Type::Union(keys) => {
                let types: Vec<Type> = keys
                    .iter()
                    .map(|k| self.resolve_indexed_access(&resolved_object, k))
                    .collect();
                self.unify_types(types)
            }
            // Number literal: mainly for tuples
            Type::NumberLiteral(idx) => {
                if let Type::Tuple(types) = &resolved_object {
                    let i = *idx as usize;
                    types.get(i).cloned().unwrap_or(Type::Any)
                } else if let Type::Array(elem) = &resolved_object {
                    (**elem).clone()
                } else {
                    Type::Any
                }
            }
            // String index: get index signature value type
            Type::String => {
                if let Type::Object { index_signature: Some(idx), .. } = &resolved_object {
                    if matches!(*idx.key_type, Type::String) {
                        return (*idx.value_type).clone();
                    }
                }
                Type::Any
            }
            // Number index: for arrays or number index signatures
            Type::Number => {
                if let Type::Array(elem) = &resolved_object {
                    return (**elem).clone();
                }
                if let Type::Object { index_signature: Some(idx), .. } = &resolved_object {
                    if matches!(*idx.key_type, Type::Number) {
                        return (*idx.value_type).clone();
                    }
                }
                Type::Any
            }
            _ => Type::Any,
        }
    }

    /// Build a substitution map from type parameters and type arguments.
    ///
    /// Given type params [T, U] and type args [number, string],
    /// builds {T -> number, U -> string}.
    pub fn build_substitution_map(
        &self,
        type_params: &[TypeParam],
        type_args: &[Type],
    ) -> HashMap<String, Type> {
        let mut map = HashMap::new();

        for (i, param) in type_params.iter().enumerate() {
            if let Some(arg) = type_args.get(i) {
                map.insert(param.name.clone(), arg.clone());
            } else if let Some(default) = &param.default {
                // Use default if no argument provided
                map.insert(param.name.clone(), (**default).clone());
            }
        }

        map
    }

    /// Infer type arguments from argument types by matching against parameter types.
    ///
    /// For `function identity<T>(x: T): T` called with `identity(42)`:
    /// - Match argument type (number literal 42) against parameter type (T)
    /// - Infer T = number
    ///
    /// Returns a substitution map of {type_param_name -> inferred_type}
    pub fn infer_type_args_from_call(
        &self,
        type_params: &[TypeParam],
        params: &[Param],
        arg_types: &[Type],
    ) -> HashMap<String, Type> {
        let mut inferred = HashMap::new();

        // For each parameter, try to infer type arguments from the corresponding argument
        for (i, param) in params.iter().enumerate() {
            if let Some(arg_type) = arg_types.get(i) {
                self.infer_from_types(&param.ty, arg_type, type_params, &mut inferred);
            }
        }

        // Fill in defaults for type parameters that weren't inferred
        for tp in type_params {
            if !inferred.contains_key(&tp.name) {
                if let Some(default) = &tp.default {
                    inferred.insert(tp.name.clone(), (**default).clone());
                }
            }
        }

        inferred
    }

    /// Check if a type argument satisfies its constraint.
    ///
    /// Returns true if the constraint is satisfied (or there is no constraint).
    pub fn satisfies_constraint(&self, type_arg: &Type, constraint: &Type) -> bool {
        self.is_assignable(type_arg, constraint)
    }

    /// Infer type arguments by matching a parameter type against an argument type.
    ///
    /// param_type: The declared type from the function signature (may contain type params)
    /// arg_type: The actual type from the call site
    fn infer_from_types(
        &self,
        param_type: &Type,
        arg_type: &Type,
        type_params: &[TypeParam],
        inferred: &mut HashMap<String, Type>,
    ) {
        match param_type {
            // If param is a TypeRef to one of our type params, infer it
            Type::TypeRef { name, type_args } if type_args.is_empty() => {
                // Check if this is a type parameter
                if type_params.iter().any(|tp| &tp.name == name) {
                    // Widen the argument type for inference
                    let widened = self.widen_type(arg_type.clone());
                    if let Some(existing) = inferred.get(name) {
                        // If already inferred, unify the types
                        if existing != &widened {
                            // Create a union of the two inferences
                            let unified = self.union_types(existing.clone(), widened);
                            inferred.insert(name.clone(), unified);
                        }
                    } else {
                        inferred.insert(name.clone(), widened);
                    }
                }
            }

            // If param is an array, recurse into element type
            Type::Array(param_elem) => {
                if let Type::Array(arg_elem) = arg_type {
                    self.infer_from_types(param_elem, arg_elem, type_params, inferred);
                }
            }

            // If param is a tuple, recurse into each element
            Type::Tuple(param_types) => {
                if let Type::Tuple(arg_types) = arg_type {
                    for (pt, at) in param_types.iter().zip(arg_types.iter()) {
                        self.infer_from_types(pt, at, type_params, inferred);
                    }
                }
            }

            // If param is a function, recurse into params and return
            Type::Function { params: param_params, return_type: param_ret, .. } => {
                if let Type::Function { params: arg_params, return_type: arg_ret, .. } = arg_type {
                    for (pp, ap) in param_params.iter().zip(arg_params.iter()) {
                        self.infer_from_types(&pp.ty, &ap.ty, type_params, inferred);
                    }
                    self.infer_from_types(param_ret, arg_ret, type_params, inferred);
                }
            }

            // For objects, try to match properties
            Type::Object { properties: param_props, .. } => {
                if let Type::Object { properties: arg_props, .. } = arg_type {
                    for pp in param_props {
                        if let Some(ap) = arg_props.iter().find(|p| p.name == pp.name) {
                            self.infer_from_types(&pp.ty, &ap.ty, type_params, inferred);
                        }
                    }
                }
            }

            _ => {}
        }
    }

    /// Evaluate a conditional type `T extends U ? X : Y`.
    ///
    /// Key behaviors:
    /// 1. If check_type is `never`, return `never`
    /// 2. If check_type is `any`, return union of true_type | false_type
    /// 3. Distributive: If check_type is a naked type parameter that resolves to a union,
    ///    distribute the conditional over each member
    /// 4. Otherwise, check if check_type extends extends_type and return appropriate branch
    pub fn evaluate_conditional_type(
        &self,
        check_type: &Type,
        extends_type: &Type,
        true_type: &Type,
        false_type: &Type,
    ) -> Type {
        // First, resolve TypeRefs in check_type to handle cases like MyExtract<MyUnion, string>
        // where MyUnion is a TypeRef to a union type
        let resolved_check = if let Type::TypeRef { name, type_args } = check_type {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| check_type.clone())
        } else {
            check_type.clone()
        };

        // Also resolve true_type and false_type if they're TypeRefs
        let resolved_true = if let Type::TypeRef { name, type_args } = true_type {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| true_type.clone())
        } else {
            true_type.clone()
        };

        let resolved_false = if let Type::TypeRef { name, type_args } = false_type {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| false_type.clone())
        } else {
            false_type.clone()
        };

        // Handle special cases with resolved types
        match &resolved_check {
            // never extends U ? X : Y = never
            Type::Never => return Type::Never,

            // any extends U ? X : Y = X | Y (both branches are possible)
            Type::Any => {
                return self.union_types(resolved_true, resolved_false);
            }

            // Distributive conditional types: Union distributes
            // (A | B) extends U ? X : Y = (A extends U ? X : Y) | (B extends U ? X : Y)
            // Important: We need to substitute each union member for occurrences in true_type/false_type
            // if they reference the same union. This handles cases like:
            // MyExtract<"a"|"b"|1, string> where T = "a"|"b"|1 and true_type contains T
            Type::Union(types) => {
                // Check if true_type or false_type is the same as check_type (common pattern)
                // In this case, we need to substitute each member during distribution
                let true_is_check = &resolved_true == &resolved_check;
                let false_is_check = &resolved_false == &resolved_check;

                let results: Vec<Type> = types
                    .iter()
                    .map(|t| {
                        // If true_type was the union, substitute with current member
                        let subst_true = if true_is_check { t.clone() } else { resolved_true.clone() };
                        // If false_type was the union, substitute with current member
                        let subst_false = if false_is_check { t.clone() } else { resolved_false.clone() };
                        self.evaluate_conditional_type(t, extends_type, &subst_true, &subst_false)
                    })
                    .collect();
                return self.unify_types(results).simplify();
            }

            _ => {}
        }

        // Check for infer types in extends_type and extract inferred variables
        let mut inferred = HashMap::new();
        let has_infer = self.collect_infer_types(extends_type);

        if !has_infer.is_empty() {
            // Pattern match and infer types
            // For function types (like from typeof), use resolved_check
            // For TypeRefs like Box<string>, use original check_type to preserve structure
            // This allows matching Box<string> extends Box<infer R>
            let check_for_infer = if matches!(check_type, Type::TypeRef { .. }) {
                check_type
            } else {
                &resolved_check
            };
            if self.infer_from_conditional(check_for_infer, extends_type, &mut inferred) {
                // Substitute inferred types into true_type
                let result = self.substitute_type_params(true_type, &inferred);
                return result;
            } else {
                // Pattern didn't match, return false_type
                return false_type.clone();
            }
        }

        // Normal assignability check
        if self.is_assignable(check_type, extends_type) {
            true_type.clone()
        } else {
            false_type.clone()
        }
    }

    /// Collect all infer type variable names from a type.
    fn collect_infer_types(&self, ty: &Type) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_infer_types_inner(ty, &mut result);
        result
    }

    fn collect_infer_types_inner(&self, ty: &Type, result: &mut Vec<String>) {
        match ty {
            Type::InferType { name, constraint } => {
                result.push(name.clone());
                if let Some(c) = constraint {
                    self.collect_infer_types_inner(c, result);
                }
            }
            Type::Array(elem) => self.collect_infer_types_inner(elem, result),
            Type::Tuple(types) => {
                for t in types {
                    self.collect_infer_types_inner(t, result);
                }
            }
            Type::Union(types) | Type::Intersection(types) => {
                for t in types {
                    self.collect_infer_types_inner(t, result);
                }
            }
            Type::Function { params, return_type, .. } => {
                for p in params {
                    self.collect_infer_types_inner(&p.ty, result);
                }
                self.collect_infer_types_inner(return_type, result);
            }
            Type::Object { properties, .. } => {
                for p in properties {
                    self.collect_infer_types_inner(&p.ty, result);
                }
            }
            Type::ConditionalType { check_type, extends_type, true_type, false_type } => {
                self.collect_infer_types_inner(check_type, result);
                self.collect_infer_types_inner(extends_type, result);
                self.collect_infer_types_inner(true_type, result);
                self.collect_infer_types_inner(false_type, result);
            }
            Type::TypeRef { type_args, .. } => {
                for t in type_args {
                    self.collect_infer_types_inner(t, result);
                }
            }
            _ => {}
        }
    }

    /// Pattern match check_type against extends_type, extracting inferred types.
    ///
    /// Returns true if the pattern matches, false otherwise.
    /// Populates `inferred` with the inferred type substitutions.
    fn infer_from_conditional(
        &self,
        check_type: &Type,
        extends_type: &Type,
        inferred: &mut HashMap<String, Type>,
    ) -> bool {
        match extends_type {
            // Infer type: capture the corresponding part of check_type
            Type::InferType { name, constraint } => {
                // Check constraint if present
                if let Some(c) = constraint {
                    if !self.is_assignable(check_type, c) {
                        return false;
                    }
                }
                inferred.insert(name.clone(), check_type.clone());
                true
            }

            // Function type: match params and return type
            Type::Function { params: ext_params, return_type: ext_return, .. } => {
                if let Type::Function { params: check_params, return_type: check_return, .. } = check_type {
                    // Check if extends_type has a rest parameter with infer type
                    // This is the pattern (...args: infer P) => R which captures params as tuple
                    let ext_has_rest_infer = ext_params.len() == 1
                        && ext_params[0].rest
                        && matches!(&ext_params[0].ty, Type::Array(elem) if matches!(**elem, Type::InferType { .. }));

                    // Check if extends_type has a rest parameter with 'any[]' type
                    // This is the pattern (...args: any[]) => R which matches any function
                    let ext_has_any_rest = ext_params.len() == 1
                        && ext_params[0].rest
                        && matches!(&ext_params[0].ty, Type::Array(elem) if matches!(**elem, Type::Any));

                    if ext_has_rest_infer {
                        // Capture all params as a tuple type for infer P
                        if let Type::Array(elem) = &ext_params[0].ty {
                            if let Type::InferType { name, .. } = elem.as_ref() {
                                // Create tuple from check_params
                                let tuple_types: Vec<Type> = check_params.iter().map(|p| p.ty.clone()).collect();
                                inferred.insert(name.clone(), Type::Tuple(tuple_types));
                            }
                        }
                    } else if !ext_has_any_rest {
                        // Strict parameter matching when not using catch-all rest param
                        for (ep, cp) in ext_params.iter().zip(check_params.iter()) {
                            if !self.infer_from_conditional(&cp.ty, &ep.ty, inferred) {
                                return false;
                            }
                        }
                    }
                    // Otherwise, ext_params can be ignored (any function matches)

                    // Match return type (covariant position)
                    self.infer_from_conditional(check_return, ext_return, inferred)
                } else {
                    false
                }
            }

            // Array type
            Type::Array(ext_elem) => {
                if let Type::Array(check_elem) = check_type {
                    self.infer_from_conditional(check_elem, ext_elem, inferred)
                } else {
                    false
                }
            }

            // Tuple type
            Type::Tuple(ext_types) => {
                if let Type::Tuple(check_types) = check_type {
                    if ext_types.len() != check_types.len() {
                        return false;
                    }
                    for (et, ct) in ext_types.iter().zip(check_types.iter()) {
                        if !self.infer_from_conditional(ct, et, inferred) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }

            // Object type: match properties
            Type::Object { properties: ext_props, .. } => {
                if let Type::Object { properties: check_props, .. } = check_type {
                    for ep in ext_props {
                        if let Some(cp) = check_props.iter().find(|p| p.name == ep.name) {
                            if !self.infer_from_conditional(&cp.ty, &ep.ty, inferred) {
                                return false;
                            }
                        } else if !ep.optional {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }

            // TypeRef type: match generic type arguments
            // This handles cases like Promise<infer R> where check_type is Promise<string>
            Type::TypeRef { name: ext_name, type_args: ext_args } => {
                // Special case: Array<infer R> matches Type::Array
                if ext_name == "Array" && ext_args.len() == 1 {
                    if let Type::Array(check_elem) = check_type {
                        return self.infer_from_conditional(check_elem, &ext_args[0], inferred);
                    }
                }

                if let Type::TypeRef { name: check_name, type_args: check_args } = check_type {
                    // Names must match (e.g., both Promise)
                    if ext_name != check_name {
                        return false;
                    }
                    // Type arguments must match or be inferable
                    if ext_args.len() != check_args.len() {
                        return false;
                    }
                    for (ea, ca) in ext_args.iter().zip(check_args.iter()) {
                        if !self.infer_from_conditional(ca, ea, inferred) {
                            return false;
                        }
                    }
                    true
                } else {
                    // Check type might be resolved - try to resolve and match
                    false
                }
            }

            // For non-infer types, just check assignability
            _ => self.is_assignable(check_type, extends_type),
        }
    }

    /// Evaluate `\`prefix${T}suffix\`` - unions produce cartesian product of all combinations.
    pub fn evaluate_template_literal_type(&self, texts: &[String], types: &[Type]) -> Type {
        if types.is_empty() {
            return Type::StringLiteral(texts.join(""));
        }

        // Resolve type aliases like `color-${Color}` where Color = "red" | "blue"
        let resolved_types: Vec<Type> = types
            .iter()
            .map(|ty| {
                if let Type::TypeRef { name, type_args } = ty {
                    self.resolve_type_ref_with_args(name, type_args).unwrap_or_else(|| ty.clone())
                } else {
                    ty.clone()
                }
            })
            .collect();

        let mut all_concrete = true;
        let mut type_values: Vec<Vec<String>> = Vec::new();

        for ty in &resolved_types {
            match ty {
                Type::StringLiteral(s) => type_values.push(vec![s.clone()]),
                Type::NumberLiteral(n) => type_values.push(vec![n.to_string()]),
                Type::BooleanLiteral(b) => type_values.push(vec![b.to_string()]),
                Type::Union(union_types) => {
                    let mut values = Vec::new();
                    for ut in union_types {
                        match ut {
                            Type::StringLiteral(s) => values.push(s.clone()),
                            Type::NumberLiteral(n) => values.push(n.to_string()),
                            Type::BooleanLiteral(b) => values.push(b.to_string()),
                            _ => {
                                all_concrete = false;
                                break;
                            }
                        }
                    }
                    if all_concrete {
                        type_values.push(values);
                    }
                }
                // Can't produce concrete literals from these
                Type::String | Type::Number | Type::Any => all_concrete = false,
                _ => all_concrete = false,
            }
            if !all_concrete {
                break;
            }
        }

        if !all_concrete {
            // Can't fully resolve, keep as pattern type for assignability checks
            return Type::TemplateLiteralType {
                texts: texts.to_vec(),
                types: types.to_vec(),
            };
        }

        // Cartesian product: `${A|B}${X|Y}` -> "AX" | "AY" | "BX" | "BY"
        let mut results: Vec<String> = vec![texts[0].clone()];
        for (i, values) in type_values.iter().enumerate() {
            let suffix = texts.get(i + 1).map(|s| s.as_str()).unwrap_or("");
            let mut new_results = Vec::new();
            for prefix in &results {
                for value in values {
                    new_results.push(format!("{prefix}{value}{suffix}"));
                }
            }
            results = new_results;
        }

        if results.len() == 1 {
            Type::StringLiteral(results.into_iter().next().unwrap())
        } else {
            Type::Union(results.into_iter().map(Type::StringLiteral).collect())
        }
    }

    /// Uppercase<T>, Lowercase<T>, Capitalize<T>, Uncapitalize<T>
    pub fn evaluate_intrinsic_string_type(&self, intrinsic: &str, arg: &Type) -> Type {
        match arg {
            Type::StringLiteral(s) => {
                let result = match intrinsic {
                    "Uppercase" => s.to_uppercase(),
                    "Lowercase" => s.to_lowercase(),
                    "Capitalize" => {
                        let mut chars = s.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                        }
                    }
                    "Uncapitalize" => {
                        let mut chars = s.chars();
                        match chars.next() {
                            None => String::new(),
                            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
                        }
                    }
                    _ => return Type::String,
                };
                Type::StringLiteral(result)
            }
            Type::Union(types) => {
                let results: Vec<Type> = types
                    .iter()
                    .map(|t| self.evaluate_intrinsic_string_type(intrinsic, t))
                    .collect();
                self.unify_types(results)
            }
            // For non-literal strings, just return string
            _ => Type::String,
        }
    }
}
