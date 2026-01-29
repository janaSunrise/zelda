//! Type resolution and manipulation.

use std::borrow::Cow;

use rustc_hash::FxHashMap;

use oxc_ast::ast::*;

use crate::types::resolution;
use crate::types::{IndexSignature, Param, Property, Type, TypeId, TypeParam};

use super::Checker;

/// Filter substitutions to exclude bound names, using Cow to avoid cloning when not needed.
fn filter_substitutions<'a>(
    substitutions: &'a FxHashMap<String, TypeId>,
    bound_names: &rustc_hash::FxHashSet<String>,
) -> Cow<'a, FxHashMap<String, TypeId>> {
    if bound_names.is_empty() {
        Cow::Borrowed(substitutions)
    } else {
        Cow::Owned(
            substitutions
                .iter()
                .filter(|(k, _)| !bound_names.contains(*k))
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        )
    }
}

/// Filter substitutions to exclude a single bound name, using Cow to avoid cloning when not needed.
fn filter_substitution_single<'a>(
    substitutions: &'a FxHashMap<String, TypeId>,
    bound_name: &str,
) -> Cow<'a, FxHashMap<String, TypeId>> {
    if !substitutions.contains_key(bound_name) {
        Cow::Borrowed(substitutions)
    } else {
        Cow::Owned(
            substitutions
                .iter()
                .filter(|(k, _)| k.as_str() != bound_name)
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        )
    }
}

impl<'a> Checker<'a> {
    pub(super) fn resolve_ts_type(&mut self, ts_type: &TSType) -> TypeId {
        // TSTypeQuery needs symbol table access, so handle it here rather than in resolution module
        if let TSType::TSTypeQuery(query) = ts_type {
            return self.resolve_type_query(query);
        }
        resolution::resolve_ts_type(ts_type, &mut self.symbols.arena)
    }

    /// `typeof x` -> look up x's type in symbol table
    fn resolve_type_query(&self, query: &oxc_ast::ast::TSTypeQuery) -> TypeId {
        use oxc_ast::ast::TSTypeQueryExprName;

        match &query.expr_name {
            TSTypeQueryExprName::IdentifierReference(ident) => {
                let name = ident.name.as_str();
                self.symbols.lookup(name).map(|s| s.ty).unwrap_or(TypeId::ANY)
            }
            // TODO: resolve full qualified chain instead of just the last part
            TSTypeQueryExprName::QualifiedName(qual) => {
                let name = qual.right.name.as_str();
                self.symbols.lookup(name).map(|s| s.ty).unwrap_or(TypeId::ANY)
            }
            TSTypeQueryExprName::TSImportType(_) => TypeId::ANY,
            TSTypeQueryExprName::ThisExpression(_) => TypeId::ANY,
        }
    }

    pub fn widen_type(&mut self, ty_id: TypeId) -> TypeId {
        resolution::widen_type(ty_id, &mut self.symbols.arena)
    }

    /// Flatten and deduplicate a union of two types.
    pub(super) fn union_types(&mut self, a: TypeId, b: TypeId) -> TypeId {
        if a == b {
            return a;
        }

        let mut type_ids = Vec::new();
        let a_ty = self.get_type(a).clone();
        match a_ty {
            Type::Union(inner) => type_ids.extend(inner),
            _ => type_ids.push(a),
        }
        let b_ty = self.get_type(b).clone();
        match b_ty {
            Type::Union(inner) => type_ids.extend(inner),
            _ => type_ids.push(b),
        }

        type_ids.sort();
        type_ids.dedup();

        if type_ids.len() == 1 {
            type_ids.pop().unwrap()
        } else {
            self.intern(Type::Union(type_ids))
        }
    }

    /// [] -> never, [T] -> T, [T, U, ...] -> T | U | ...
    pub(super) fn unify_types(&mut self, type_ids: Vec<TypeId>) -> TypeId {
        match type_ids.len() {
            0 => TypeId::NEVER,
            1 => type_ids.into_iter().next().unwrap(),
            _ => {
                let mut iter = type_ids.into_iter();
                let mut result = iter.next().unwrap();
                for ty_id in iter {
                    result = self.union_types(result, ty_id);
                }
                result
            }
        }
    }

    /// Replace type parameters with concrete types.
    /// `Array<T>` with {T -> string} => `Array<string>`
    pub fn substitute_type_params(
        &mut self,
        ty_id: TypeId,
        substitutions: &FxHashMap<String, TypeId>,
    ) -> TypeId {
        // Fast path for primitives - no substitution needed
        if ty_id.is_primitive() {
            return ty_id;
        }

        let ty = self.get_type(ty_id).clone();
        match ty {
            Type::TypeParameter { ref name, .. } => {
                substitutions.get(name).copied().unwrap_or(ty_id)
            }

            Type::TypeRef { ref name, ref type_args } => {
                // Bare TypeRef with no args might be a type parameter reference
                if type_args.is_empty() {
                    if let Some(&substituted) = substitutions.get(name) {
                        return substituted;
                    }
                }

                let new_args: Vec<TypeId> = type_args
                    .iter()
                    .map(|&arg| self.substitute_type_params(arg, substitutions))
                    .collect();

                self.arena_mut().type_ref(name.clone(), new_args)
            }

            // Compound types: recurse into their components
            Type::Array(elem_id) => {
                let new_elem = self.substitute_type_params(elem_id, substitutions);
                self.arena_mut().array(new_elem)
            }

            Type::Tuple(ref type_ids) => {
                let new_ids: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&id| self.substitute_type_params(id, substitutions))
                    .collect();
                self.arena_mut().tuple(new_ids)
            }

            Type::Union(ref type_ids) => {
                let new_ids: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&id| self.substitute_type_params(id, substitutions))
                    .collect();
                self.arena_mut().union(new_ids)
            }

            Type::Intersection(ref type_ids) => {
                let new_ids: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&id| self.substitute_type_params(id, substitutions))
                    .collect();
                self.arena_mut().intersection(new_ids)
            }

            Type::Object {
                ref properties,
                ref index_signature,
                ref extends,
                ref type_params,
            } => {
                let new_props: Vec<Property> = properties
                    .iter()
                    .map(|p| Property {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(p.ty, substitutions),
                        optional: p.optional,
                        readonly: p.readonly,
                    })
                    .collect();

                let new_idx = index_signature.as_ref().map(|idx| IndexSignature {
                    key_type: self.substitute_type_params(idx.key_type, substitutions),
                    value_type: self.substitute_type_params(idx.value_type, substitutions),
                });

                let new_extends: Vec<TypeId> = extends
                    .iter()
                    .map(|&id| self.substitute_type_params(id, substitutions))
                    .collect();

                let remaining_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .filter(|tp| !substitutions.contains_key(&tp.name))
                    .cloned()
                    .collect();

                self.intern(Type::Object {
                    properties: new_props,
                    index_signature: new_idx,
                    extends: new_extends,
                    type_params: remaining_type_params,
                })
            }

            Type::Function {
                ref params,
                return_type,
                ref type_params,
                ref type_predicate,
            } => {
                let bound_names: rustc_hash::FxHashSet<_> =
                    type_params.iter().map(|tp| tp.name.clone()).collect();
                let filtered_subs = filter_substitutions(substitutions, &bound_names);

                let new_params: Vec<Param> = params
                    .iter()
                    .map(|p| Param {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(p.ty, &filtered_subs),
                        optional: p.optional,
                        rest: p.rest,
                    })
                    .collect();

                let new_return = self.substitute_type_params(return_type, &filtered_subs);

                let new_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .map(|tp| TypeParam {
                        name: tp.name.clone(),
                        constraint: tp
                            .constraint
                            .map(|c| self.substitute_type_params(c, &filtered_subs)),
                        default: tp
                            .default
                            .map(|d| self.substitute_type_params(d, &filtered_subs)),
                    })
                    .collect();

                let new_predicate = type_predicate.as_ref().map(|tp| {
                    crate::types::TypePredicate {
                        parameter_name: tp.parameter_name.clone(),
                        asserts: tp.asserts,
                        type_annotation: tp
                            .type_annotation
                            .map(|id| self.substitute_type_params(id, &filtered_subs)),
                    }
                });

                self.intern(Type::Function {
                    params: new_params,
                    return_type: new_return,
                    type_params: new_type_params,
                    type_predicate: new_predicate,
                })
            }

            Type::ClassConstructor {
                ref params,
                ref type_params,
                ref static_members,
            } => {
                let bound_names: rustc_hash::FxHashSet<_> =
                    type_params.iter().map(|tp| tp.name.clone()).collect();
                let filtered_subs = filter_substitutions(substitutions, &bound_names);

                let new_params: Vec<Param> = params
                    .iter()
                    .map(|p| Param {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(p.ty, &filtered_subs),
                        optional: p.optional,
                        rest: p.rest,
                    })
                    .collect();

                let new_type_params: Vec<TypeParam> = type_params
                    .iter()
                    .map(|tp| TypeParam {
                        name: tp.name.clone(),
                        constraint: tp
                            .constraint
                            .map(|c| self.substitute_type_params(c, &filtered_subs)),
                        default: tp
                            .default
                            .map(|d| self.substitute_type_params(d, &filtered_subs)),
                    })
                    .collect();

                let new_static_members: Vec<Property> = static_members
                    .iter()
                    .map(|p| Property {
                        name: p.name.clone(),
                        ty: self.substitute_type_params(p.ty, &filtered_subs),
                        optional: p.optional,
                        readonly: p.readonly,
                    })
                    .collect();

                self.intern(Type::ClassConstructor {
                    params: new_params,
                    type_params: new_type_params,
                    static_members: new_static_members,
                })
            }

            Type::KeyOf(inner_id) => {
                let new_inner = self.substitute_type_params(inner_id, substitutions);
                self.intern(Type::KeyOf(new_inner))
            }

            Type::IndexedAccess {
                object_type,
                index_type,
            } => {
                let new_obj = self.substitute_type_params(object_type, substitutions);
                let new_idx = self.substitute_type_params(index_type, substitutions);
                self.intern(Type::IndexedAccess {
                    object_type: new_obj,
                    index_type: new_idx,
                })
            }

            Type::MappedType {
                ref type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
            } => {
                let filtered_subs = filter_substitution_single(substitutions, type_param);
                let new_constraint = self.substitute_type_params(constraint, &filtered_subs);
                let new_template = self.substitute_type_params(template, &filtered_subs);

                self.intern(Type::MappedType {
                    type_param: type_param.clone(),
                    constraint: new_constraint,
                    template: new_template,
                    readonly_modifier,
                    optional_modifier,
                })
            }

            Type::ConditionalType {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                let new_check = self.substitute_type_params(check_type, substitutions);
                let new_extends = self.substitute_type_params(extends_type, substitutions);
                let new_true = self.substitute_type_params(true_type, substitutions);
                let new_false = self.substitute_type_params(false_type, substitutions);

                self.intern(Type::ConditionalType {
                    check_type: new_check,
                    extends_type: new_extends,
                    true_type: new_true,
                    false_type: new_false,
                })
            }

            Type::InferType {
                ref name,
                constraint,
            } => {
                let filtered_subs = filter_substitution_single(substitutions, name);
                let new_constraint = constraint.map(|c| self.substitute_type_params(c, &filtered_subs));

                self.intern(Type::InferType {
                    name: name.clone(),
                    constraint: new_constraint,
                })
            }

            Type::TemplateLiteralType {
                ref texts,
                ref types,
            } => {
                let new_types: Vec<TypeId> = types
                    .iter()
                    .map(|&id| self.substitute_type_params(id, substitutions))
                    .collect();

                self.intern(Type::TemplateLiteralType {
                    texts: texts.clone(),
                    types: new_types,
                })
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
            | Type::BooleanLiteral(_) => ty_id,
        }
    }

    /// Resolve a keyof type to a union of string literal types.
    ///
    /// `keyof { x: number; y: string }` resolves to `"x" | "y"`
    pub fn resolve_keyof(&mut self, ty_id: TypeId) -> TypeId {
        // First resolve the type if it's a TypeRef
        let ty = self.get_type(ty_id).clone();
        let resolved_id = match &ty {
            Type::TypeRef { name, type_args } => self
                .resolve_type_ref_with_args(name, type_args)
                .unwrap_or(ty_id),
            Type::TypeParameter {
                constraint: Some(constraint_id),
                ..
            } => {
                // For a type parameter with constraint, keyof T extends C gives keyof C
                let cid = *constraint_id;
                return self.resolve_keyof(cid);
            }
            _ => ty_id,
        };

        let resolved = self.get_type(resolved_id).clone();
        match resolved {
            Type::Object {
                properties,
                extends,
                ..
            } => {
                // Collect all property names including from extended interfaces
                let all_props = self.resolve_object_properties(&properties, &extends);
                let keys: Vec<TypeId> = all_props
                    .iter()
                    .map(|p| self.intern(Type::StringLiteral(p.name.clone())))
                    .collect();

                if keys.is_empty() {
                    TypeId::NEVER
                } else if keys.len() == 1 {
                    keys.into_iter().next().unwrap()
                } else {
                    self.intern(Type::Union(keys))
                }
            }
            Type::Union(type_ids) => {
                // keyof (A | B) = (keyof A) & (keyof B)
                let resolved_keys: Vec<TypeId> =
                    type_ids.iter().map(|&id| self.resolve_keyof(id)).collect();
                if resolved_keys.is_empty() {
                    TypeId::NEVER
                } else if resolved_keys.len() == 1 {
                    resolved_keys.into_iter().next().unwrap()
                } else {
                    self.intern(Type::Intersection(resolved_keys))
                }
            }
            Type::Intersection(type_ids) => {
                // keyof (A & B) = (keyof A) | (keyof B)
                let resolved_keys: Vec<TypeId> =
                    type_ids.iter().map(|&id| self.resolve_keyof(id)).collect();
                self.unify_types(resolved_keys)
            }
            Type::Any => {
                let string_or_number = vec![TypeId::STRING, TypeId::NUMBER];
                self.intern(Type::Union(string_or_number))
            }
            Type::Unknown => TypeId::NEVER,
            _ => TypeId::NEVER, // Primitives have no keys
        }
    }

    /// Resolve a mapped type to a concrete object type.
    ///
    /// `{ [K in keyof Person]: Person[K] }` resolves to `{ name: string; age: number }`
    pub fn resolve_mapped_type(
        &mut self,
        type_param: &str,
        constraint_id: TypeId,
        template_id: TypeId,
        readonly_modifier: Option<bool>,
        optional_modifier: Option<bool>,
    ) -> TypeId {
        // First resolve the constraint to get the keys
        let constraint = self.get_type(constraint_id).clone();
        let resolved_constraint_id = if let Type::KeyOf(inner_id) = constraint {
            self.resolve_keyof(inner_id)
        } else {
            constraint_id
        };

        // Get the list of keys to iterate over
        let resolved_constraint = self.get_type(resolved_constraint_id).clone();
        let key_ids: Vec<TypeId> = match resolved_constraint {
            Type::Union(type_ids) => type_ids,
            Type::StringLiteral(_) => vec![resolved_constraint_id],
            Type::Never => {
                return self.arena_mut().object(vec![]);
            }
            _ => return TypeId::ANY, // Can't resolve mapped type with this constraint
        };

        // Build properties by substituting each key
        let mut properties: Vec<Property> = Vec::new();

        for key_id in key_ids {
            let key_ty = self.get_type(key_id).clone();
            if let Type::StringLiteral(key_name) = key_ty {
                // Create substitution map: type_param -> key
                let mut subs = FxHashMap::default();
                subs.insert(type_param.to_string(), key_id);

                // Substitute in the template to get the property type
                let prop_type_id = self.substitute_type_params(template_id, &subs);

                // If the template is an indexed access like T[K], resolve it
                let prop_ty = self.get_type(prop_type_id).clone();
                let resolved_prop_type_id = if let Type::IndexedAccess {
                    object_type,
                    index_type,
                } = prop_ty
                {
                    self.resolve_indexed_access(object_type, index_type)
                } else {
                    prop_type_id
                };

                let mut prop = Property::new(key_name.clone(), resolved_prop_type_id);

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

        self.arena_mut().object(properties)
    }

    /// Resolve an indexed access type T[K] to the property type.
    ///
    /// `Person["name"]` resolves to `string`
    /// `Person[keyof Person]` resolves to union of all property types
    pub fn resolve_indexed_access(&mut self, object_type_id: TypeId, index_type_id: TypeId) -> TypeId {
        // First, try to resolve KeyOf if that's what index_type is
        let index_ty = self.get_type(index_type_id).clone();
        let resolved_index_id = if let Type::KeyOf(inner_id) = index_ty {
            self.resolve_keyof(inner_id)
        } else {
            index_type_id
        };

        // Resolve the object type if it's a TypeRef
        let object_ty = self.get_type(object_type_id).clone();
        let resolved_object_id = match &object_ty {
            Type::TypeRef { name, type_args } => self
                .resolve_type_ref_with_args(name, type_args)
                .unwrap_or(object_type_id),
            _ => object_type_id,
        };

        let resolved_index = self.get_type(resolved_index_id).clone();
        let resolved_object = self.get_type(resolved_object_id).clone();

        match resolved_index {
            // String literal key: T["prop"]
            Type::StringLiteral(ref key) => self.get_property_type(resolved_object_id, key),
            // Union of keys: T["a" | "b"] = T["a"] | T["b"]
            Type::Union(key_ids) => {
                let type_ids: Vec<TypeId> = key_ids
                    .iter()
                    .map(|&k| self.resolve_indexed_access(resolved_object_id, k))
                    .collect();
                self.unify_types(type_ids)
            }
            // Number literal: mainly for tuples
            Type::NumberLiteral(idx) => {
                match resolved_object {
                    Type::Tuple(type_ids) => {
                        let i = idx as usize;
                        type_ids.get(i).copied().unwrap_or(TypeId::ANY)
                    }
                    Type::Array(elem_id) => elem_id,
                    _ => TypeId::ANY,
                }
            }
            // String index: get index signature value type
            Type::String => {
                if let Type::Object {
                    index_signature: Some(ref idx),
                    ..
                } = resolved_object
                {
                    let key_ty = self.get_type(idx.key_type);
                    if matches!(key_ty, Type::String) {
                        return idx.value_type;
                    }
                }
                TypeId::ANY
            }
            // Number index: for arrays or number index signatures
            Type::Number => {
                if let Type::Array(elem_id) = resolved_object {
                    return elem_id;
                }
                if let Type::Object {
                    index_signature: Some(ref idx),
                    ..
                } = resolved_object
                {
                    let key_ty = self.get_type(idx.key_type);
                    if matches!(key_ty, Type::Number) {
                        return idx.value_type;
                    }
                }
                TypeId::ANY
            }
            _ => TypeId::ANY,
        }
    }

    /// Build a substitution map from type parameters and type arguments.
    ///
    /// Given type params [T, U] and type args [number, string],
    /// builds {T -> number, U -> string}.
    pub fn build_substitution_map(
        &self,
        type_params: &[TypeParam],
        type_args: &[TypeId],
    ) -> FxHashMap<String, TypeId> {
        let mut map = FxHashMap::default();

        for (i, param) in type_params.iter().enumerate() {
            if let Some(&arg_id) = type_args.get(i) {
                map.insert(param.name.clone(), arg_id);
            } else if let Some(default_id) = param.default {
                // Use default if no argument provided
                map.insert(param.name.clone(), default_id);
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
        &mut self,
        type_params: &[TypeParam],
        params: &[Param],
        arg_type_ids: &[TypeId],
    ) -> FxHashMap<String, TypeId> {
        let mut inferred = FxHashMap::default();

        // For each parameter, try to infer type arguments from the corresponding argument
        for (i, param) in params.iter().enumerate() {
            if let Some(&arg_type_id) = arg_type_ids.get(i) {
                self.infer_from_types(param.ty, arg_type_id, type_params, &mut inferred);
            }
        }

        // Fill in defaults for type parameters that weren't inferred
        for tp in type_params {
            if !inferred.contains_key(&tp.name)
                && let Some(default_id) = tp.default {
                    inferred.insert(tp.name.clone(), default_id);
                }
        }

        inferred
    }

    /// Check if a type argument satisfies its constraint.
    ///
    /// Returns true if the constraint is satisfied (or there is no constraint).
    pub fn satisfies_constraint(&mut self, type_arg_id: TypeId, constraint_id: TypeId) -> bool {
        self.is_assignable(type_arg_id, constraint_id)
    }

    /// Infer type arguments by matching a parameter type against an argument type.
    ///
    /// param_type: The declared type from the function signature (may contain type params)
    /// arg_type: The actual type from the call site
    fn infer_from_types(
        &mut self,
        param_type_id: TypeId,
        arg_type_id: TypeId,
        type_params: &[TypeParam],
        inferred: &mut FxHashMap<String, TypeId>,
    ) {
        let param_type = self.get_type(param_type_id).clone();
        let arg_type = self.get_type(arg_type_id).clone();

        match param_type {
            // If param is a TypeRef to one of our type params, infer it
            Type::TypeRef { ref name, ref type_args } if type_args.is_empty() => {
                // Check if this is a type parameter
                if type_params.iter().any(|tp| &tp.name == name) {
                    // Widen the argument type for inference
                    let widened = self.widen_type(arg_type_id);
                    if let Some(&existing) = inferred.get(name) {
                        // If already inferred, unify the types
                        if existing != widened {
                            // Create a union of the two inferences
                            let unified = self.union_types(existing, widened);
                            inferred.insert(name.clone(), unified);
                        }
                    } else {
                        inferred.insert(name.clone(), widened);
                    }
                }
            }

            // If param is an array, recurse into element type
            Type::Array(param_elem_id) => {
                if let Type::Array(arg_elem_id) = arg_type {
                    self.infer_from_types(param_elem_id, arg_elem_id, type_params, inferred);
                }
            }

            // If param is a tuple, recurse into each element
            Type::Tuple(param_type_ids) => {
                if let Type::Tuple(arg_type_ids) = arg_type {
                    for (&pt, &at) in param_type_ids.iter().zip(arg_type_ids.iter()) {
                        self.infer_from_types(pt, at, type_params, inferred);
                    }
                }
            }

            // If param is a function, recurse into params and return
            Type::Function {
                params: ref param_params,
                return_type: param_ret_id,
                ..
            } => {
                if let Type::Function {
                    params: ref arg_params,
                    return_type: arg_ret_id,
                    ..
                } = arg_type
                {
                    for (pp, ap) in param_params.iter().zip(arg_params.iter()) {
                        self.infer_from_types(pp.ty, ap.ty, type_params, inferred);
                    }
                    self.infer_from_types(param_ret_id, arg_ret_id, type_params, inferred);
                }
            }

            // For objects, try to match properties
            Type::Object {
                properties: ref param_props,
                ..
            } => {
                if let Type::Object {
                    properties: ref arg_props,
                    ..
                } = arg_type
                {
                    for pp in param_props {
                        if let Some(ap) = arg_props.iter().find(|p| p.name == pp.name) {
                            self.infer_from_types(pp.ty, ap.ty, type_params, inferred);
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
        &mut self,
        check_type_id: TypeId,
        extends_type_id: TypeId,
        true_type_id: TypeId,
        false_type_id: TypeId,
    ) -> TypeId {
        // First, resolve TypeRefs in check_type to handle cases like MyExtract<MyUnion, string>
        // where MyUnion is a TypeRef to a union type
        let check_ty = self.get_type(check_type_id).clone();
        let resolved_check_id = if let Type::TypeRef { ref name, ref type_args } = check_ty {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or(check_type_id)
        } else {
            check_type_id
        };

        // Also resolve true_type and false_type if they're TypeRefs
        let true_ty = self.get_type(true_type_id).clone();
        let resolved_true_id = if let Type::TypeRef { ref name, ref type_args } = true_ty {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or(true_type_id)
        } else {
            true_type_id
        };

        let false_ty = self.get_type(false_type_id).clone();
        let resolved_false_id = if let Type::TypeRef { ref name, ref type_args } = false_ty {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or(false_type_id)
        } else {
            false_type_id
        };

        // Handle special cases with resolved types
        let resolved_check = self.get_type(resolved_check_id).clone();
        match resolved_check {
            // never extends U ? X : Y = never
            Type::Never => return TypeId::NEVER,

            // any extends U ? X : Y = X | Y (both branches are possible)
            Type::Any => {
                return self.union_types(resolved_true_id, resolved_false_id);
            }

            // Distributive conditional types: Union distributes
            // (A | B) extends U ? X : Y = (A extends U ? X : Y) | (B extends U ? X : Y)
            Type::Union(type_ids) => {
                // Check if true_type or false_type is the same as check_type (common pattern)
                let true_is_check = resolved_true_id == resolved_check_id;
                let false_is_check = resolved_false_id == resolved_check_id;

                let results: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&t| {
                        // If true_type was the union, substitute with current member
                        let subst_true = if true_is_check { t } else { resolved_true_id };
                        // If false_type was the union, substitute with current member
                        let subst_false = if false_is_check { t } else { resolved_false_id };
                        self.evaluate_conditional_type(t, extends_type_id, subst_true, subst_false)
                    })
                    .collect();
                let unified = self.unify_types(results);
                return self.simplify_type(unified);
            }

            _ => {}
        }

        // Check for infer types in extends_type and extract inferred variables
        let mut inferred = FxHashMap::default();
        let has_infer = self.collect_infer_types(extends_type_id);

        if !has_infer.is_empty() {
            // Pattern match and infer types
            // For function types (like from typeof), use resolved_check
            // For TypeRefs like Box<string>, use original check_type to preserve structure
            let check_ty_2 = self.get_type(check_type_id).clone();
            let check_for_infer = if matches!(check_ty_2, Type::TypeRef { .. }) {
                check_type_id
            } else {
                resolved_check_id
            };
            if self.infer_from_conditional(check_for_infer, extends_type_id, &mut inferred) {
                // Substitute inferred types into true_type
                let result = self.substitute_type_params(true_type_id, &inferred);
                return result;
            } else {
                // Pattern didn't match, return false_type
                return false_type_id;
            }
        }

        // Normal assignability check
        if self.is_assignable(check_type_id, extends_type_id) {
            true_type_id
        } else {
            false_type_id
        }
    }

    /// Collect all infer type variable names from a type.
    fn collect_infer_types(&self, ty_id: TypeId) -> Vec<String> {
        let mut result = Vec::new();
        self.collect_infer_types_inner(ty_id, &mut result);
        result
    }

    fn collect_infer_types_inner(&self, ty_id: TypeId, result: &mut Vec<String>) {
        let ty = self.get_type(ty_id);
        match ty {
            Type::InferType { name, constraint } => {
                result.push(name.clone());
                if let Some(c_id) = *constraint {
                    self.collect_infer_types_inner(c_id, result);
                }
            }
            Type::Array(elem_id) => self.collect_infer_types_inner(*elem_id, result),
            Type::Tuple(type_ids) => {
                for &t_id in type_ids {
                    self.collect_infer_types_inner(t_id, result);
                }
            }
            Type::Union(type_ids) | Type::Intersection(type_ids) => {
                for &t_id in type_ids {
                    self.collect_infer_types_inner(t_id, result);
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for p in params {
                    self.collect_infer_types_inner(p.ty, result);
                }
                self.collect_infer_types_inner(*return_type, result);
            }
            Type::Object { properties, .. } => {
                for p in properties {
                    self.collect_infer_types_inner(p.ty, result);
                }
            }
            Type::ConditionalType {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                self.collect_infer_types_inner(*check_type, result);
                self.collect_infer_types_inner(*extends_type, result);
                self.collect_infer_types_inner(*true_type, result);
                self.collect_infer_types_inner(*false_type, result);
            }
            Type::TypeRef { type_args, .. } => {
                for &t_id in type_args {
                    self.collect_infer_types_inner(t_id, result);
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
        &mut self,
        check_type_id: TypeId,
        extends_type_id: TypeId,
        inferred: &mut FxHashMap<String, TypeId>,
    ) -> bool {
        let extends_type = self.get_type(extends_type_id).clone();
        let check_type = self.get_type(check_type_id).clone();

        match extends_type {
            // Infer type: capture the corresponding part of check_type
            Type::InferType { ref name, constraint } => {
                // Check constraint if present
                if let Some(c_id) = constraint
                    && !self.is_assignable(check_type_id, c_id) {
                        return false;
                    }
                inferred.insert(name.clone(), check_type_id);
                true
            }

            // Function type: match params and return type
            Type::Function {
                params: ref ext_params,
                return_type: ext_return_id,
                ..
            } => {
                if let Type::Function {
                    params: ref check_params,
                    return_type: check_return_id,
                    ..
                } = check_type
                {
                    // Check if extends_type has a rest parameter with infer type
                    let ext_has_rest_infer = ext_params.len() == 1
                        && ext_params[0].rest
                        && {
                            let param_ty = self.get_type(ext_params[0].ty);
                            if let Type::Array(elem_id) = param_ty {
                                let elem_ty = self.get_type(*elem_id);
                                matches!(elem_ty, Type::InferType { .. })
                            } else {
                                false
                            }
                        };

                    // Check if extends_type has a rest parameter with 'any[]' type
                    let ext_has_any_rest = ext_params.len() == 1
                        && ext_params[0].rest
                        && {
                            let param_ty = self.get_type(ext_params[0].ty);
                            if let Type::Array(elem_id) = param_ty {
                                let elem_ty = self.get_type(*elem_id);
                                matches!(elem_ty, Type::Any)
                            } else {
                                false
                            }
                        };

                    if ext_has_rest_infer {
                        // Capture all params as a tuple type for infer P
                        let param_ty = self.get_type(ext_params[0].ty).clone();
                        if let Type::Array(elem_id) = param_ty {
                            let elem_ty = self.get_type(elem_id).clone();
                            if let Type::InferType { ref name, .. } = elem_ty {
                                // Create tuple from check_params
                                let tuple_type_ids: Vec<TypeId> =
                                    check_params.iter().map(|p| p.ty).collect();
                                let tuple_id = self.intern(Type::Tuple(tuple_type_ids));
                                inferred.insert(name.clone(), tuple_id);
                            }
                        }
                    } else if !ext_has_any_rest {
                        // Strict parameter matching when not using catch-all rest param
                        for (ep, cp) in ext_params.iter().zip(check_params.iter()) {
                            if !self.infer_from_conditional(cp.ty, ep.ty, inferred) {
                                return false;
                            }
                        }
                    }
                    // Otherwise, ext_params can be ignored (any function matches)

                    // Match return type (covariant position)
                    self.infer_from_conditional(check_return_id, ext_return_id, inferred)
                } else {
                    false
                }
            }

            // Array type
            Type::Array(ext_elem_id) => {
                if let Type::Array(check_elem_id) = check_type {
                    self.infer_from_conditional(check_elem_id, ext_elem_id, inferred)
                } else {
                    false
                }
            }

            // Tuple type
            Type::Tuple(ref ext_type_ids) => {
                if let Type::Tuple(ref check_type_ids) = check_type {
                    if ext_type_ids.len() != check_type_ids.len() {
                        return false;
                    }
                    for (&et, &ct) in ext_type_ids.iter().zip(check_type_ids.iter()) {
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
            Type::Object {
                properties: ref ext_props,
                ..
            } => {
                if let Type::Object {
                    properties: ref check_props,
                    ..
                } = check_type
                {
                    for ep in ext_props {
                        if let Some(cp) = check_props.iter().find(|p| p.name == ep.name) {
                            if !self.infer_from_conditional(cp.ty, ep.ty, inferred) {
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
            Type::TypeRef {
                ref name,
                ref type_args,
            } => {
                // Special case: Array<infer R> matches Type::Array
                if name == "Array" && type_args.len() == 1 {
                    if let Type::Array(check_elem_id) = check_type {
                        return self.infer_from_conditional(check_elem_id, type_args[0], inferred);
                    }
                }

                if let Type::TypeRef {
                    name: ref check_name,
                    type_args: ref check_args,
                } = check_type
                {
                    // Names must match (e.g., both Promise)
                    if name != check_name {
                        return false;
                    }
                    // Type arguments must match or be inferable
                    if type_args.len() != check_args.len() {
                        return false;
                    }
                    for (&ea, &ca) in type_args.iter().zip(check_args.iter()) {
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
            _ => self.is_assignable(check_type_id, extends_type_id),
        }
    }

    /// Evaluate `\`prefix${T}suffix\`` - unions produce cartesian product of all combinations.
    pub fn evaluate_template_literal_type(&mut self, texts: &[String], type_ids: &[TypeId]) -> TypeId {
        if type_ids.is_empty() {
            return self.intern(Type::StringLiteral(texts.join("")));
        }

        // Resolve type aliases like `color-${Color}` where Color = "red" | "blue"
        let resolved_type_ids: Vec<TypeId> = type_ids
            .iter()
            .map(|&ty_id| {
                let ty = self.get_type(ty_id).clone();
                if let Type::TypeRef { ref name, ref type_args } = ty {
                    self.resolve_type_ref_with_args(name, type_args)
                        .unwrap_or(ty_id)
                } else {
                    ty_id
                }
            })
            .collect();

        let mut all_concrete = true;
        let mut type_values: Vec<Vec<String>> = Vec::new();

        for &ty_id in &resolved_type_ids {
            let ty = self.get_type(ty_id).clone();
            match ty {
                Type::StringLiteral(s) => type_values.push(vec![s]),
                Type::NumberLiteral(n) => type_values.push(vec![n.to_string()]),
                Type::BooleanLiteral(b) => type_values.push(vec![b.to_string()]),
                Type::Union(union_type_ids) => {
                    let mut values = Vec::new();
                    for &ut_id in &union_type_ids {
                        let ut = self.get_type(ut_id);
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
            return self.intern(Type::TemplateLiteralType {
                texts: texts.to_vec(),
                types: type_ids.to_vec(),
            });
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
            self.intern(Type::StringLiteral(results.into_iter().next().unwrap()))
        } else {
            let string_lits: Vec<TypeId> = results
                .into_iter()
                .map(|s| self.intern(Type::StringLiteral(s)))
                .collect();
            self.intern(Type::Union(string_lits))
        }
    }

    /// Uppercase<T>, Lowercase<T>, Capitalize<T>, Uncapitalize<T>
    pub fn evaluate_intrinsic_string_type(&mut self, intrinsic: &str, arg_id: TypeId) -> TypeId {
        let arg = self.get_type(arg_id).clone();
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
                    _ => return TypeId::STRING,
                };
                self.intern(Type::StringLiteral(result))
            }
            Type::Union(type_ids) => {
                let results: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&t| self.evaluate_intrinsic_string_type(intrinsic, t))
                    .collect();
                self.unify_types(results)
            }
            // For non-literal strings, just return string
            _ => TypeId::STRING,
        }
    }

    /// Simplify a type (e.g., flatten nested unions, remove duplicates).
    fn simplify_type(&mut self, ty_id: TypeId) -> TypeId {
        let ty = self.get_type(ty_id).clone();
        match ty {
            Type::Union(type_ids) if type_ids.len() == 1 => type_ids[0],
            Type::Intersection(type_ids) if type_ids.len() == 1 => type_ids[0],
            _ => ty_id,
        }
    }

    /// Resolve computed types (KeyOf, IndexedAccess, MappedType) to their concrete forms.
    ///
    /// This is necessary for type-level operations that need to be evaluated before
    /// assignability checking. For example:
    /// - `keyof Person` → `"name" | "age"`
    /// - `Person["name"]` → `string`
    /// - `{ [K in keyof Person]: Person[K] }` → `{ name: string; age: number }`
    pub fn resolve_computed_type(&mut self, ty_id: TypeId) -> TypeId {
        // First resolve TypeRef to its underlying type
        let ty = self.get_type(ty_id).clone();
        let (resolved_id, resolved_ty) = match &ty {
            Type::TypeRef { name, type_args } => {
                if let Some(underlying_id) = self.resolve_type_ref_with_args(name, type_args) {
                    let underlying = self.get_type(underlying_id).clone();
                    (underlying_id, underlying)
                } else {
                    (ty_id, ty.clone())
                }
            }
            _ => (ty_id, ty.clone()),
        };

        match resolved_ty {
            Type::KeyOf(inner_id) => {
                // Recursively resolve the inner type first
                let resolved_inner = self.resolve_computed_type(inner_id);
                self.resolve_keyof(resolved_inner)
            }
            Type::IndexedAccess {
                object_type,
                index_type,
            } => {
                let resolved_obj = self.resolve_computed_type(object_type);
                let resolved_idx = self.resolve_computed_type(index_type);
                self.resolve_indexed_access(resolved_obj, resolved_idx)
            }
            Type::MappedType {
                ref type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
            } => {
                // Only resolve the constraint - the template contains bound variable K
                // which will be substituted by resolve_mapped_type for each key
                let resolved_constraint = self.resolve_computed_type(constraint);
                self.resolve_mapped_type(
                    type_param,
                    resolved_constraint,
                    template, // Pass template as-is, don't pre-resolve it
                    readonly_modifier,
                    optional_modifier,
                )
            }
            Type::Union(type_ids) => {
                // Resolve each member of the union
                let resolved: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&id| self.resolve_computed_type(id))
                    .collect();
                self.arena_mut().union(resolved)
            }
            Type::Intersection(type_ids) => {
                // Resolve each member of the intersection
                let resolved: Vec<TypeId> = type_ids
                    .iter()
                    .map(|&id| self.resolve_computed_type(id))
                    .collect();
                self.arena_mut().intersection(resolved)
            }
            _ => resolved_id,
        }
    }
}
