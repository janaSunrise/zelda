//! Type relation checking (assignability and compatibility).

use rustc_hash::FxHashMap;

use crate::types::{Property, Type};

use super::Checker;

impl<'a> Checker<'a> {
    fn resolve_type_ref(&self, name: &str) -> Option<Type> {
        self.symbols.lookup_type(name).map(|s| s.ty.clone())
    }

    /// Resolve a TypeRef with type arguments, instantiating generic types.
    /// `Box<number>` where `Box = { value: T }` becomes `{ value: number }`.
    pub(super) fn resolve_type_ref_with_args(
        &self,
        name: &str,
        type_args: &[Type],
    ) -> Option<Type> {
        // Intrinsic string types are built into the compiler, not defined in lib.d.ts
        if matches!(
            name,
            "Uppercase" | "Lowercase" | "Capitalize" | "Uncapitalize"
        ) {
            if let Some(arg) = type_args.first() {
                let resolved_arg = if let Type::TypeRef {
                    name: ref_name,
                    type_args: ref_args,
                } = arg
                {
                    self.resolve_type_ref_with_args(ref_name, ref_args)
                        .unwrap_or_else(|| arg.clone())
                } else {
                    arg.clone()
                };
                return Some(self.evaluate_intrinsic_string_type(name, &resolved_arg));
            }
            return Some(Type::String);
        }

        let symbol = self.symbols.lookup_type(name)?;
        let base_type = symbol.ty.clone();

        match &base_type {
            // Generic type alias: we abuse Function with empty params to store alias body in return_type
            Type::Function {
                params,
                return_type,
                type_params,
                ..
            } if params.is_empty() && !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    Some((**return_type).clone())
                } else {
                    let substituted = self.substitute_type_params(return_type, &subs);
                    if let Type::ConditionalType {
                        check_type,
                        extends_type,
                        true_type,
                        false_type,
                    } = &substituted
                    {
                        Some(self.evaluate_conditional_type(
                            check_type,
                            extends_type,
                            true_type,
                            false_type,
                        ))
                    } else if let Type::TemplateLiteralType { texts, types } = &substituted {
                        Some(self.evaluate_template_literal_type(texts, types))
                    } else {
                        Some(substituted)
                    }
                }
            }
            Type::Object { type_params, .. } if !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
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

    /// Resolve all properties of an object type, including inherited properties.
    /// Uses HashMap-based deduplication for O(n+m) complexity.
    pub(super) fn resolve_object_properties(
        &self,
        own_props: &[Property],
        extends: &[Type],
    ) -> Vec<Property> {
        let mut props_map: FxHashMap<String, Property> = FxHashMap::default();

        for base_type in extends {
            if let Type::TypeRef { name, .. } = base_type
                && let Some(resolved) = self.resolve_type_ref(name)
                    && let Type::Object {
                        properties,
                        extends: base_extends,
                        ..
                    } = resolved
                    {
                        let base_props = self.resolve_object_properties(&properties, &base_extends);
                        for prop in base_props {
                            props_map.entry(prop.name.clone()).or_insert(prop);
                        }
                    }
        }

        for prop in own_props {
            props_map.insert(prop.name.clone(), prop.clone());
        }

        props_map.into_values().collect()
    }

    pub fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        // FAST PATHS FIRST - check before ANY resolution to short-circuit common cases
        // Pointer equality means identical types (interned or same allocation)
        if std::ptr::eq(source, target) {
            return true;
        }
        // Any is both a top type (accepts anything) and bottom-like (assignable to anything)
        if matches!(source, Type::Any) || matches!(target, Type::Any) {
            return true;
        }
        // Unknown is a top type - anything is assignable to it
        if matches!(target, Type::Unknown) {
            return true;
        }
        // Never is the bottom type - assignable to everything
        if matches!(source, Type::Never) {
            return true;
        }
        // Nothing is assignable TO never (except never itself, caught by ptr_eq above)
        if matches!(target, Type::Never) {
            return false;
        }

        // Resolve KeyOf types to their union of string literals
        if let Type::KeyOf(inner) = source {
            let resolved = self.resolve_keyof(inner);
            return self.is_assignable(&resolved, target);
        }
        if let Type::KeyOf(inner) = target {
            let resolved = self.resolve_keyof(inner);
            return self.is_assignable(source, &resolved);
        }

        // Resolve IndexedAccess types to their property types
        if let Type::IndexedAccess {
            object_type,
            index_type,
        } = source
        {
            let resolved = self.resolve_indexed_access(object_type, index_type);
            return self.is_assignable(&resolved, target);
        }
        if let Type::IndexedAccess {
            object_type,
            index_type,
        } = target
        {
            let resolved = self.resolve_indexed_access(object_type, index_type);
            return self.is_assignable(source, &resolved);
        }

        // Resolve MappedType to concrete object type
        if let Type::MappedType {
            type_param,
            constraint,
            template,
            readonly_modifier,
            optional_modifier,
        } = source
        {
            let resolved = self.resolve_mapped_type(
                type_param,
                constraint,
                template,
                *readonly_modifier,
                *optional_modifier,
            );
            return self.is_assignable(&resolved, target);
        }
        if let Type::MappedType {
            type_param,
            constraint,
            template,
            readonly_modifier,
            optional_modifier,
        } = target
        {
            let resolved = self.resolve_mapped_type(
                type_param,
                constraint,
                template,
                *readonly_modifier,
                *optional_modifier,
            );
            return self.is_assignable(source, &resolved);
        }

        // Resolve ConditionalType by evaluating it
        if let Type::ConditionalType {
            check_type,
            extends_type,
            true_type,
            false_type,
        } = source
        {
            let resolved =
                self.evaluate_conditional_type(check_type, extends_type, true_type, false_type);
            return self.is_assignable(&resolved, target);
        }
        if let Type::ConditionalType {
            check_type,
            extends_type,
            true_type,
            false_type,
        } = target
        {
            let resolved =
                self.evaluate_conditional_type(check_type, extends_type, true_type, false_type);
            return self.is_assignable(source, &resolved);
        }

        // Resolve TemplateLiteralType by evaluating it
        if let Type::TemplateLiteralType { texts, types } = source {
            let resolved = self.evaluate_template_literal_type(texts, types);
            // If evaluation returns same template literal (not concrete), handle specially
            if !matches!(&resolved, Type::TemplateLiteralType { .. }) {
                return self.is_assignable(&resolved, target);
            }
            // Template literal with non-concrete types matches string
            return matches!(target, Type::String | Type::Any);
        }
        if let Type::TemplateLiteralType { texts, types } = target {
            let resolved = self.evaluate_template_literal_type(texts, types);
            // If evaluation returns same template literal (not concrete), handle specially
            if !matches!(&resolved, Type::TemplateLiteralType { .. }) {
                return self.is_assignable(source, &resolved);
            }
            // Check if source is a string literal that matches the pattern
            if let Type::StringLiteral(s) = source {
                return self.string_matches_template_pattern(s, texts, types);
            }
            // Any string can potentially match a string pattern
            return matches!(source, Type::String | Type::Any);
        }

        // Handle TypeRef resolution with type argument instantiation
        if let Type::TypeRef { name, type_args } = source
            && let Some(resolved) = self.resolve_type_ref_with_args(name, type_args) {
                return self.is_assignable(&resolved, target);
            }
        if let Type::TypeRef { name, type_args } = target
            && let Some(resolved) = self.resolve_type_ref_with_args(name, type_args) {
                return self.is_assignable(source, &resolved);
            }

        // Same type (structural equality - ptr equality handled in fast path above)
        if source == target {
            return true;
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
            return source_types.iter().all(|s| self.is_assignable(s, target));
        }

        // Union target: source must be assignable to at least one branch
        if let Type::Union(target_types) = target {
            return target_types.iter().any(|t| self.is_assignable(source, t));
        }

        // Intersection source: if any member is assignable, the whole is
        if let Type::Intersection(source_types) = source {
            return source_types.iter().any(|s| self.is_assignable(s, target));
        }

        // Intersection target: must be assignable to all members
        if let Type::Intersection(target_types) = target {
            return target_types.iter().all(|t| self.is_assignable(source, t));
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
            let resolved_source_props =
                self.resolve_object_properties(source_props, source_extends);
            let resolved_target_props =
                self.resolve_object_properties(target_props, target_extends);
            return self.is_object_assignable(
                &resolved_source_props,
                source_idx.as_ref(),
                &resolved_target_props,
                target_idx.as_ref(),
            );
        }

        // Primitive types with built-in properties (for constraint checking)
        // Use apparent type to look up properties from lib.d.ts interfaces
        if let Type::Object {
            properties: target_props,
            ..
        } = target
        {
            let apparent_source = self.get_apparent_type(source);
            if matches!(apparent_source, Type::TypeRef { .. }) {
                // Check if all required target properties exist on the apparent type
                let source_has_props = target_props
                    .iter()
                    .all(|p| self.has_property(source, &p.name));
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
    fn is_object_assignable(
        &self,
        source_props: &[crate::types::Property],
        source_idx: Option<&crate::types::IndexSignature>,
        target_props: &[crate::types::Property],
        target_idx: Option<&crate::types::IndexSignature>,
    ) -> bool {
        let source_props_map: FxHashMap<&str, &crate::types::Property> =
            source_props.iter().map(|p| (p.name.as_str(), p)).collect();

        for target_prop in target_props {
            if target_prop.optional {
                continue;
            }

            match source_props_map.get(target_prop.name.as_str()) {
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

                if key_matches
                    && !self.is_assignable(&source_prop.ty, &target_idx_sig.value_type) {
                        return false;
                    }
            }

            // If source has an index signature, its value type must be compatible
            if let Some(source_idx_sig) = source_idx
                && !self.is_assignable(&source_idx_sig.value_type, &target_idx_sig.value_type) {
                    return false;
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
            Type::Object {
                properties,
                extends,
                ..
            } => self.resolve_object_properties(properties, extends),
            _ => vec![],
        };

        let target_props = match &resolved_target {
            Type::Object {
                properties,
                extends,
                ..
            } => self.resolve_object_properties(properties, extends),
            _ => vec![],
        };

        // Find required target properties missing from source
        let source_prop_names: rustc_hash::FxHashSet<&str> =
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

    /// Check if a string literal matches a template literal pattern.
    ///
    /// For example, "Hello, World!" matches `Hello, ${string}!`
    fn string_matches_template_pattern(&self, s: &str, texts: &[String], types: &[Type]) -> bool {
        // Simple pattern matching: check prefix and suffix
        if texts.is_empty() {
            return false;
        }

        // Must start with first text segment
        if !s.starts_with(&texts[0]) {
            return false;
        }

        // Must end with last text segment (if there's more than one)
        if texts.len() > 1
            && !s.ends_with(texts.last().unwrap()) {
                return false;
            }

        // For complex patterns with multiple placeholders, use regex-like matching
        // For now, simple check: if there's one placeholder with string type, allow any string in between
        if types.len() == 1 && matches!(&types[0], Type::String) {
            // Check that prefix and suffix don't overlap
            let prefix = &texts[0];
            let suffix = texts.get(1).map(|s| s.as_str()).unwrap_or("");
            if prefix.len() + suffix.len() <= s.len() {
                let middle = &s[prefix.len()..s.len() - suffix.len()];
                // Any middle string is valid for ${string}
                return !middle.is_empty() || (prefix.len() + suffix.len() == s.len());
            }
            return false;
        }

        // For union placeholders, check if the middle part is in the union
        if types.len() == 1
            && let Type::Union(union_types) = &types[0] {
                let prefix = &texts[0];
                let suffix = texts.get(1).map(|s| s.as_str()).unwrap_or("");
                if s.len() >= prefix.len() + suffix.len() {
                    let middle = &s[prefix.len()..s.len() - suffix.len()];
                    // Check if middle is one of the union options
                    return union_types.iter().any(|t| {
                        if let Type::StringLiteral(lit) = t {
                            lit == middle
                        } else {
                            false
                        }
                    });
                }
            }

        // For other patterns, do a simple structural match
        true
    }
}
