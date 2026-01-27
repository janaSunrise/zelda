//! Type resolution and manipulation helpers.

use std::collections::HashMap;

use oxc_ast::ast::*;

use crate::type_resolution;
use crate::types::{IndexSignature, Param, Property, Type, TypeParam};

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

    /// Substitute type parameters with concrete types.
    ///
    /// Given a type and a substitution map (e.g., {T -> number, U -> string}),
    /// replace all occurrences of type parameters with their concrete types.
    ///
    /// Examples:
    /// - `T` with {T -> number} => `number`
    /// - `Array<T>` with {T -> string} => `Array<string>`
    /// - `{ value: T }` with {T -> number} => `{ value: number }`
    pub fn substitute_type_params(&self, ty: &Type, substitutions: &HashMap<String, Type>) -> Type {
        match ty {
            // Type parameter: look up in substitution map
            Type::TypeParameter { name, .. } => {
                substitutions.get(name).cloned().unwrap_or_else(|| ty.clone())
            }

            // TypeRef: might be a type parameter reference or a generic type
            Type::TypeRef { name, type_args } => {
                // First check if this is a type parameter reference (no type args)
                if type_args.is_empty() {
                    if let Some(substituted) = substitutions.get(name) {
                        return substituted.clone();
                    }
                }

                // Substitute in type arguments
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

            Type::Function { params, return_type, type_params } => {
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

                Type::Function {
                    params: new_params,
                    return_type: Box::new(new_return),
                    type_params: new_type_params,
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
}
