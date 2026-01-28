//! Type relation checking (assignability and compatibility).
//!
//! It determines if types are assignable and structurally compatible.

use crate::types::{Property, Type};

use super::Checker;

impl<'a> Checker<'a> {
    /// Resolve a TypeRef to its underlying type by looking it up in the type namespace.
    /// If the resolved type is generic and type arguments are provided, instantiate it.
    fn resolve_type_ref(&self, name: &str) -> Option<Type> {
        self.symbols.lookup_type(name).map(|s| s.ty.clone())
    }

    /// Resolve a TypeRef with type arguments, instantiating generic types.
    ///
    /// For `Box<number>` where Box is `{ value: T }`, this returns `{ value: number }`.
    /// For `Container` where Container has default `<T = string>`, this applies the defaults.
    pub(super) fn resolve_type_ref_with_args(&self, name: &str, type_args: &[Type]) -> Option<Type> {
        let symbol = self.symbols.lookup_type(name)?;
        let base_type = symbol.ty.clone();

        // Extract type parameters from the base type and instantiate
        match &base_type {
            Type::Object { type_params, .. } if !type_params.is_empty() => {
                // Build substitution map - this handles both explicit args and defaults
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    // No substitutions possible (no args and no defaults)
                    Some(base_type)
                } else {
                    Some(self.substitute_type_params(&base_type, &subs))
                }
            }
            Type::Function { type_params, .. } if !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    Some(base_type)
                } else {
                    Some(self.substitute_type_params(&base_type, &subs))
                }
            }
            _ => Some(base_type),
        }
    }

    /// Resolve all properties of an object type, including inherited properties from extends.
    /// Properties in derived interfaces override those in base interfaces.
    pub(super) fn resolve_object_properties(
        &self,
        own_props: &[Property],
        extends: &[Type],
    ) -> Vec<Property> {
        let mut all_props: Vec<Property> = Vec::new();

        // Collect properties from base types first
        for base_type in extends {
            if let Type::TypeRef { name, .. } = base_type {
                if let Some(resolved) = self.resolve_type_ref(name) {
                    if let Type::Object {
                        properties,
                        extends: base_extends,
                        ..
                    } = resolved
                    {
                        let base_props = self.resolve_object_properties(&properties, &base_extends);
                        for prop in base_props {
                            if !all_props.iter().any(|p| p.name == prop.name) {
                                all_props.push(prop);
                            }
                        }
                    }
                }
            }
        }

        // Own properties override inherited ones
        for prop in own_props {
            all_props.retain(|p| p.name != prop.name);
            all_props.push(prop.clone());
        }

        all_props
    }

    pub fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        // Handle TypeRef resolution with type argument instantiation
        if let Type::TypeRef { name, type_args } = source {
            if let Some(resolved) = self.resolve_type_ref_with_args(name, type_args) {
                return self.is_assignable(&resolved, target);
            }
        }
        if let Type::TypeRef { name, type_args } = target {
            if let Some(resolved) = self.resolve_type_ref_with_args(name, type_args) {
                return self.is_assignable(source, &resolved);
            }
        }

        // Same type
        if source == target {
            return true;
        }

        // Any accepts and provides anything
        if matches!(source, Type::Any) || matches!(target, Type::Any) {
            return true;
        }

        // Unknown accepts anything
        if matches!(target, Type::Unknown) {
            return true;
        }

        // Never is assignable to everything
        if matches!(source, Type::Never) {
            return true;
        }

        // Nothing is assignable to never (except never itself, handled above)
        if matches!(target, Type::Never) {
            return false;
        }

        // In strict mode, null/undefined are only assignable to void, any, unknown, null, undefined
        if matches!(source, Type::Null | Type::Undefined) {
            // null/undefined are assignable to void
            if matches!(target, Type::Void) {
                return true;
            }
            // In non-strict mode, null/undefined are assignable to any type
            // (except never, already handled above)
            return true;
        }

        // void is only assignable to void, any, unknown (already handled)
        // undefined is assignable to void
        if matches!(source, Type::Undefined) && matches!(target, Type::Void) {
            return true;
        }

        // Literal types are assignable to their base types
        match (source, target) {
            (Type::StringLiteral(_), Type::String) => return true,
            (Type::NumberLiteral(_), Type::Number) => return true,
            (Type::BooleanLiteral(_), Type::Boolean) => return true,
            _ => {}
        }

        // Union source: all branches must be assignable to target
        // Check this BEFORE union target to handle Union to Union correctly
        if let Type::Union(source_types) = source {
            return source_types
                .iter()
                .all(|s| self.is_assignable(s, target));
        }

        // Union target: source must be assignable to at least one branch
        if let Type::Union(target_types) = target {
            return target_types
                .iter()
                .any(|t| self.is_assignable(source, t));
        }

        // Intersection source: if any member is assignable, the whole is
        if let Type::Intersection(source_types) = source {
            return source_types
                .iter()
                .any(|s| self.is_assignable(s, target));
        }

        // Intersection target: must be assignable to all members
        if let Type::Intersection(target_types) = target {
            return target_types
                .iter()
                .all(|t| self.is_assignable(source, t));
        }

        // Array types - covariant
        if let (Type::Array(source_elem), Type::Array(target_elem)) = (source, target) {
            return self.is_assignable(source_elem, target_elem);
        }

        // Tuple types - element-wise compatibility
        if let (Type::Tuple(source_types), Type::Tuple(target_types)) = (source, target) {
            if source_types.len() != target_types.len() {
                return false;
            }
            return source_types
                .iter()
                .zip(target_types.iter())
                .all(|(s, t)| self.is_assignable(s, t));
        }

        // Tuple assignable to array - all elements must match
        if let (Type::Tuple(source_types), Type::Array(target_elem)) = (source, target) {
            return source_types
                .iter()
                .all(|s| self.is_assignable(s, target_elem));
        }

        // Object types - structural compatibility
        if let (
            Type::Object {
                properties: source_props,
                index_signature: source_idx,
                extends: source_extends,
                ..
            },
            Type::Object {
                properties: target_props,
                index_signature: target_idx,
                extends: target_extends,
                ..
            },
        ) = (source, target)
        {
            let resolved_source_props = self.resolve_object_properties(source_props, source_extends);
            let resolved_target_props = self.resolve_object_properties(target_props, target_extends);
            return self.is_object_assignable(
                &resolved_source_props,
                source_idx.as_ref(),
                &resolved_target_props,
                target_idx.as_ref(),
            );
        }

        // Primitive types with built-in properties (for constraint checking)
        // Use apparent type to look up properties from lib.d.ts interfaces
        if let Type::Object { properties: target_props, .. } = target {
            let apparent_source = self.get_apparent_type(source);
            if matches!(apparent_source, Type::TypeRef { .. }) {
                // Check if all required target properties exist on the apparent type
                let source_has_props = target_props.iter().all(|p| {
                    self.has_property(source, &p.name)
                });
                if source_has_props {
                    return true;
                }
            }
        }

        // Function types - contravariant params, covariant return
        if let (
            Type::Function {
                params: source_params,
                return_type: source_return,
                ..
            },
            Type::Function {
                params: target_params,
                return_type: target_return,
                ..
            },
        ) = (source, target)
        {
            return self.is_function_assignable(
                source_params,
                source_return,
                target_params,
                target_return,
            );
        }

        false
    }

    /// Check structural compatibility of object types.
    ///
    /// Rules:
    /// 1. Target's required properties must exist in source with compatible types
    ///    (or be satisfied by source's index signature)
    /// 2. If target has an index signature, all source properties must be compatible
    ///    with the index signature's value type
    /// 3. Index signature compatibility: source index signature value type must be
    ///    assignable to target index signature value type
    fn is_object_assignable(
        &self,
        source_props: &[crate::types::Property],
        source_idx: Option<&crate::types::IndexSignature>,
        target_props: &[crate::types::Property],
        target_idx: Option<&crate::types::IndexSignature>,
    ) -> bool {
        // Check that all required target properties are satisfied
        for target_prop in target_props {
            if target_prop.optional {
                continue;
            }

            // First, look for an explicit source property
            let source_prop = source_props.iter().find(|p| p.name == target_prop.name);
            match source_prop {
                Some(sp) => {
                    if !self.is_assignable(&sp.ty, &target_prop.ty) {
                        return false;
                    }
                }
                None => {
                    // No explicit property - check if source's index signature can satisfy it
                    if let Some(idx_sig) = source_idx {
                        // String index signatures can satisfy any property
                        if matches!(*idx_sig.key_type, Type::String) {
                            if !self.is_assignable(&idx_sig.value_type, &target_prop.ty) {
                                return false;
                            }
                        } else {
                            // Number index can't satisfy string property names
                            return false;
                        }
                    } else {
                        // No source property and no index signature
                        return false;
                    }
                }
            }
        }

        // If target has an index signature, all source properties must be compatible
        if let Some(target_idx_sig) = target_idx {
            // Check all source properties against target's index signature
            for source_prop in source_props {
                // Only check properties that match the index key type
                let key_matches = match &*target_idx_sig.key_type {
                    Type::String => true, // String index applies to all properties
                    Type::Number => source_prop.name.parse::<f64>().is_ok(), // Number index only for numeric keys
                    _ => false,
                };

                if key_matches {
                    if !self.is_assignable(&source_prop.ty, &target_idx_sig.value_type) {
                        return false;
                    }
                }
            }

            // If source has an index signature, its value type must be compatible
            if let Some(source_idx_sig) = source_idx {
                if !self.is_assignable(&source_idx_sig.value_type, &target_idx_sig.value_type) {
                    return false;
                }
            }
        }

        true
    }

    /// Find properties that are required in target but missing in source.
    /// Returns a list of property names that are missing.
    pub(super) fn find_missing_properties(&self, source: &Type, target: &Type) -> Vec<String> {
        // Resolve TypeRefs first
        let resolved_source = if let Type::TypeRef { name, type_args } = source {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| source.clone())
        } else {
            source.clone()
        };
        let resolved_target = if let Type::TypeRef { name, type_args } = target {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| target.clone())
        } else {
            target.clone()
        };

        // Get properties from both types
        let source_props = match &resolved_source {
            Type::Object { properties, extends, .. } => {
                self.resolve_object_properties(properties, extends)
            }
            _ => vec![],
        };

        let target_props = match &resolved_target {
            Type::Object { properties, extends, .. } => {
                self.resolve_object_properties(properties, extends)
            }
            _ => vec![],
        };

        // Find required target properties missing from source
        let source_prop_names: std::collections::HashSet<&str> =
            source_props.iter().map(|p| p.name.as_str()).collect();

        target_props
            .iter()
            .filter(|p| !p.optional && !source_prop_names.contains(p.name.as_str()))
            .map(|p| p.name.clone())
            .collect()
    }

    /// Check function type compatibility.
    ///
    /// - Return type: covariant (source return must be assignable to target return)
    /// - Parameters: contravariant (target param must be assignable to source param)
    /// - Source can have fewer parameters than target (callback compatibility)
    fn is_function_assignable(
        &self,
        source_params: &[crate::types::Param],
        source_return: &Type,
        target_params: &[crate::types::Param],
        target_return: &Type,
    ) -> bool {
        // Return type: covariant
        if !self.is_assignable(source_return, target_return) {
            return false;
        }

        // Parameters: source can have fewer (callback compatibility)
        if source_params.len() > target_params.len() {
            return false;
        }

        // Contravariant: target param must be assignable to source param
        for (sp, tp) in source_params.iter().zip(target_params.iter()) {
            if !self.is_assignable(&tp.ty, &sp.ty) {
                return false;
            }
        }

        true
    }
}
