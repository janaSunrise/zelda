//! Type relation checking (assignability and compatibility).

use rustc_hash::FxHashMap;

use crate::types::{Property, Type, TypeId};

use super::Checker;

impl<'a> Checker<'a> {
    fn resolve_type_ref(&self, name: &str) -> Option<TypeId> {
        self.symbols.lookup_type(name).map(|s| s.ty)
    }

    /// Resolve a TypeRef with type arguments, instantiating generic types.
    /// `Box<number>` where `Box = { value: T }` becomes `{ value: number }`.
    pub(super) fn resolve_type_ref_with_args(
        &mut self,
        name: &str,
        type_args: &[TypeId],
    ) -> Option<TypeId> {
        // Intrinsic string types are built into the compiler, not defined in lib.d.ts
        if matches!(
            name,
            "Uppercase" | "Lowercase" | "Capitalize" | "Uncapitalize"
        ) {
            if let Some(&arg_id) = type_args.first() {
                let arg = self.get_type(arg_id).clone();
                let resolved_arg_id = if let Type::TypeRef {
                    name: ref ref_name,
                    type_args: ref ref_args,
                } = arg
                {
                    self.resolve_type_ref_with_args(ref_name, ref_args)
                        .unwrap_or(arg_id)
                } else {
                    arg_id
                };
                return Some(self.evaluate_intrinsic_string_type(name, resolved_arg_id));
            }
            return Some(TypeId::STRING);
        }

        let symbol = self.symbols.lookup_type(name)?;
        let base_type_id = symbol.ty;
        let base_type = self.get_type(base_type_id).clone();

        match base_type {
            // Generic type alias: we abuse Function with empty params to store alias body in return_type
            Type::Function {
                ref params,
                return_type,
                ref type_params,
                ..
            } if params.is_empty() && !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    Some(return_type)
                } else {
                    let substituted_id = self.substitute_type_params(return_type, &subs);
                    let substituted = self.get_type(substituted_id).clone();
                    match substituted {
                        Type::ConditionalType {
                            check_type,
                            extends_type,
                            true_type,
                            false_type,
                        } => Some(self.evaluate_conditional_type(
                            check_type,
                            extends_type,
                            true_type,
                            false_type,
                        )),
                        Type::TemplateLiteralType { ref texts, ref types } => {
                            Some(self.evaluate_template_literal_type(texts, types))
                        }
                        _ => Some(substituted_id),
                    }
                }
            }
            Type::Object { ref type_params, .. } if !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    Some(base_type_id)
                } else {
                    Some(self.substitute_type_params(base_type_id, &subs))
                }
            }
            Type::Function { ref type_params, .. } if !type_params.is_empty() => {
                let subs = self.build_substitution_map(type_params, type_args);
                if subs.is_empty() {
                    Some(base_type_id)
                } else {
                    Some(self.substitute_type_params(base_type_id, &subs))
                }
            }
            _ => Some(base_type_id),
        }
    }

    /// Resolve all properties of an object type, including inherited properties.
    /// Uses HashMap-based deduplication for O(n+m) complexity.
    pub(super) fn resolve_object_properties(
        &self,
        own_props: &[Property],
        extends: &[TypeId],
    ) -> Vec<Property> {
        let mut props_map: FxHashMap<String, Property> = FxHashMap::default();

        for &base_type_id in extends {
            let base_type = self.get_type(base_type_id);
            if let Type::TypeRef { name, .. } = base_type {
                if let Some(resolved_id) = self.resolve_type_ref(name) {
                    let resolved = self.get_type(resolved_id);
                    if let Type::Object {
                        properties,
                        extends: base_extends,
                        ..
                    } = resolved
                    {
                        let base_props = self.resolve_object_properties(properties, base_extends);
                        for prop in base_props {
                            props_map.entry(prop.name.clone()).or_insert(prop);
                        }
                    }
                }
            }
        }

        for prop in own_props {
            props_map.insert(prop.name.clone(), prop.clone());
        }

        props_map.into_values().collect()
    }

    pub fn is_assignable(&mut self, source_id: TypeId, target_id: TypeId) -> bool {
        // FAST PATHS FIRST - check before ANY resolution to short-circuit common cases
        // TypeId equality means identical types (interned)
        if source_id == target_id {
            return true;
        }
        // Any is both a top type (accepts anything) and bottom-like (assignable to anything)
        if source_id == TypeId::ANY || target_id == TypeId::ANY {
            return true;
        }
        // Unknown is a top type - anything is assignable to it
        if target_id == TypeId::UNKNOWN {
            return true;
        }
        // Never is the bottom type - assignable to everything
        if source_id == TypeId::NEVER {
            return true;
        }
        // Nothing is assignable TO never (except never itself, caught above)
        if target_id == TypeId::NEVER {
            return false;
        }

        let source = self.get_type(source_id).clone();
        let target = self.get_type(target_id).clone();

        // Resolve KeyOf types to their union of string literals
        if let Type::KeyOf(inner_id) = source {
            // Note: resolve_keyof needs &mut self, so we use a workaround here
            // For now, treat KeyOf as assignable to string | number
            return self.is_keyof_assignable(inner_id, target_id);
        }
        if let Type::KeyOf(inner_id) = target {
            return self.is_source_assignable_to_keyof(source_id, inner_id);
        }

        // Resolve IndexedAccess types to their property types
        if let Type::IndexedAccess {
            object_type,
            index_type,
        } = source
        {
            return self.is_indexed_access_assignable(object_type, index_type, target_id);
        }
        if let Type::IndexedAccess {
            object_type,
            index_type,
        } = target
        {
            return self.is_source_assignable_to_indexed_access(source_id, object_type, index_type);
        }

        // Resolve MappedType to concrete object type
        if let Type::MappedType {
            ref type_param,
            constraint,
            template,
            readonly_modifier,
            optional_modifier,
        } = source
        {
            return self.is_mapped_type_assignable(
                type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
                target_id,
            );
        }
        if let Type::MappedType {
            ref type_param,
            constraint,
            template,
            readonly_modifier,
            optional_modifier,
        } = target
        {
            return self.is_source_assignable_to_mapped_type(
                source_id,
                type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
            );
        }

        // Resolve ConditionalType by evaluating it
        if let Type::ConditionalType {
            check_type,
            extends_type,
            true_type,
            false_type,
        } = source
        {
            return self.is_conditional_assignable(
                check_type,
                extends_type,
                true_type,
                false_type,
                target_id,
            );
        }
        if let Type::ConditionalType {
            check_type,
            extends_type,
            true_type,
            false_type,
        } = target
        {
            return self.is_source_assignable_to_conditional(
                source_id,
                check_type,
                extends_type,
                true_type,
                false_type,
            );
        }

        // Resolve TemplateLiteralType by evaluating it
        if let Type::TemplateLiteralType { ref texts, ref types } = source {
            return self.is_template_literal_assignable(texts, types, target_id);
        }
        if let Type::TemplateLiteralType { ref texts, ref types } = target {
            return self.is_source_assignable_to_template_literal(source_id, texts, types);
        }

        // Handle TypeRef resolution with type argument instantiation
        if let Type::TypeRef { ref name, ref type_args } = source {
            return self.is_type_ref_assignable(name, type_args, target_id);
        }
        if let Type::TypeRef { ref name, ref type_args } = target {
            return self.is_source_assignable_to_type_ref(source_id, name, type_args);
        }

        // In strict mode, null/undefined are only assignable to void, any, unknown, null, undefined
        if matches!(source, Type::Null | Type::Undefined) {
            // null/undefined are assignable to void
            if matches!(target, Type::Void) {
                return true;
            }
            // In non-strict mode, null/undefined are assignable to any type
            return true;
        }

        // undefined is assignable to void
        if matches!(source, Type::Undefined) && matches!(target, Type::Void) {
            return true;
        }

        // Literal types are assignable to their base types
        match (&source, &target) {
            (Type::StringLiteral(_), Type::String) => return true,
            (Type::NumberLiteral(_), Type::Number) => return true,
            (Type::BooleanLiteral(_), Type::Boolean) => return true,
            _ => {}
        }

        // Union source: all branches must be assignable to target
        if let Type::Union(source_type_ids) = source {
            return source_type_ids
                .iter()
                .all(|&s| self.is_assignable(s, target_id));
        }

        // Union target: source must be assignable to at least one branch
        if let Type::Union(target_type_ids) = target {
            return target_type_ids
                .iter()
                .any(|&t| self.is_assignable(source_id, t));
        }

        // Intersection source: if any member is assignable, the whole is
        if let Type::Intersection(source_type_ids) = source {
            return source_type_ids
                .iter()
                .any(|&s| self.is_assignable(s, target_id));
        }

        // Intersection target: must be assignable to all members
        if let Type::Intersection(target_type_ids) = target {
            return target_type_ids
                .iter()
                .all(|&t| self.is_assignable(source_id, t));
        }

        // Array types - covariant
        if let (Type::Array(source_elem_id), Type::Array(target_elem_id)) = (&source, &target) {
            return self.is_assignable(*source_elem_id, *target_elem_id);
        }

        // Tuple types - element-wise compatibility
        if let (Type::Tuple(source_type_ids), Type::Tuple(target_type_ids)) = (&source, &target) {
            if source_type_ids.len() != target_type_ids.len() {
                return false;
            }
            return source_type_ids
                .iter()
                .zip(target_type_ids.iter())
                .all(|(&s, &t)| self.is_assignable(s, t));
        }

        // Tuple assignable to array - all elements must match
        if let (Type::Tuple(source_type_ids), Type::Array(target_elem_id)) = (&source, &target) {
            return source_type_ids
                .iter()
                .all(|&s| self.is_assignable(s, *target_elem_id));
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
        ) = (&source, &target)
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
        // E.g., string is assignable to { length: number } because String interface has length
        if let Type::Object {
            properties: ref target_props,
            ..
        } = target
        {
            // Check if source has all required properties (works for primitives via interface lookup)
            let source_has_props = target_props
                .iter()
                .filter(|p| !p.optional)
                .all(|p| {
                    let prop_ty = self.get_property_type(source_id, &p.name);
                    if prop_ty == TypeId::ANY {
                        return false;
                    }
                    self.is_assignable(prop_ty, p.ty)
                });
            if source_has_props {
                return true;
            }
        }

        // Function types - contravariant params, covariant return
        if let (
            Type::Function {
                params: source_params,
                return_type: source_return_id,
                ..
            },
            Type::Function {
                params: target_params,
                return_type: target_return_id,
                ..
            },
        ) = (&source, &target)
        {
            return self.is_function_assignable(
                source_params,
                *source_return_id,
                target_params,
                *target_return_id,
            );
        }

        false
    }

    // Helper methods to handle complex type resolutions without &mut self in is_assignable
    fn is_keyof_assignable(&self, _inner_id: TypeId, target_id: TypeId) -> bool {
        // keyof T is assignable to string | number | symbol
        let target = self.get_type(target_id);
        matches!(target, Type::String | Type::Number | Type::Any | Type::Unknown)
            || matches!(target, Type::Union(_))
    }

    fn is_source_assignable_to_keyof(&self, source_id: TypeId, _inner_id: TypeId) -> bool {
        // string | number is assignable to keyof T
        let source = self.get_type(source_id);
        matches!(
            source,
            Type::String | Type::Number | Type::StringLiteral(_) | Type::NumberLiteral(_)
        )
    }

    fn is_indexed_access_assignable(
        &self,
        _object_type_id: TypeId,
        _index_type_id: TypeId,
        _target_id: TypeId,
    ) -> bool {
        // Simplified: treat as any for now
        true
    }

    fn is_source_assignable_to_indexed_access(
        &self,
        _source_id: TypeId,
        _object_type_id: TypeId,
        _index_type_id: TypeId,
    ) -> bool {
        true
    }

    fn is_mapped_type_assignable(
        &self,
        _type_param: &str,
        _constraint_id: TypeId,
        _template_id: TypeId,
        _readonly_modifier: Option<bool>,
        _optional_modifier: Option<bool>,
        _target_id: TypeId,
    ) -> bool {
        // Simplified: treat as any for now
        true
    }

    fn is_source_assignable_to_mapped_type(
        &self,
        _source_id: TypeId,
        _type_param: &str,
        _constraint_id: TypeId,
        _template_id: TypeId,
        _readonly_modifier: Option<bool>,
        _optional_modifier: Option<bool>,
    ) -> bool {
        true
    }

    fn is_conditional_assignable(
        &self,
        _check_type_id: TypeId,
        _extends_type_id: TypeId,
        _true_type_id: TypeId,
        _false_type_id: TypeId,
        _target_id: TypeId,
    ) -> bool {
        // Simplified: treat as any for now
        true
    }

    fn is_source_assignable_to_conditional(
        &self,
        _source_id: TypeId,
        _check_type_id: TypeId,
        _extends_type_id: TypeId,
        _true_type_id: TypeId,
        _false_type_id: TypeId,
    ) -> bool {
        true
    }

    fn is_template_literal_assignable(
        &self,
        _texts: &[String],
        _type_ids: &[TypeId],
        target_id: TypeId,
    ) -> bool {
        let target = self.get_type(target_id);
        matches!(target, Type::String | Type::Any)
    }

    fn is_source_assignable_to_template_literal(
        &self,
        source_id: TypeId,
        texts: &[String],
        type_ids: &[TypeId],
    ) -> bool {
        let source = self.get_type(source_id);
        if let Type::StringLiteral(s) = source {
            return self.string_matches_template_pattern(s, texts, type_ids);
        }
        matches!(source, Type::String | Type::Any)
    }

    fn is_type_ref_assignable(
        &mut self,
        name: &str,
        type_args: &[TypeId],
        target_id: TypeId,
    ) -> bool {
        if let Some(resolved_id) = self.resolve_type_ref_with_args(name, type_args) {
            return self.is_assignable(resolved_id, target_id);
        }
        false
    }

    fn is_source_assignable_to_type_ref(
        &mut self,
        source_id: TypeId,
        name: &str,
        type_args: &[TypeId],
    ) -> bool {
        if let Some(resolved_id) = self.resolve_type_ref_with_args(name, type_args) {
            return self.is_assignable(source_id, resolved_id);
        }
        false
    }

    /// Check structural compatibility of object types.
    fn is_object_assignable(
        &mut self,
        source_props: &[Property],
        source_idx: Option<&crate::types::IndexSignature>,
        target_props: &[Property],
        target_idx: Option<&crate::types::IndexSignature>,
    ) -> bool {
        let source_props_map: FxHashMap<&str, &Property> =
            source_props.iter().map(|p| (p.name.as_str(), p)).collect();

        for target_prop in target_props {
            if target_prop.optional {
                continue;
            }

            match source_props_map.get(target_prop.name.as_str()) {
                Some(sp) => {
                    if !self.is_assignable(sp.ty, target_prop.ty) {
                        return false;
                    }
                }
                None => {
                    // No explicit property - check if source's index signature can satisfy it
                    if let Some(idx_sig) = source_idx {
                        // String index signatures can satisfy any property
                        let key_ty = self.get_type(idx_sig.key_type);
                        if matches!(key_ty, Type::String) {
                            if !self.is_assignable(idx_sig.value_type, target_prop.ty) {
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
                let target_key_ty = self.get_type(target_idx_sig.key_type);
                let key_matches = match target_key_ty {
                    Type::String => true,
                    Type::Number => source_prop.name.parse::<f64>().is_ok(),
                    _ => false,
                };

                if key_matches && !self.is_assignable(source_prop.ty, target_idx_sig.value_type) {
                    return false;
                }
            }

            // If source has an index signature, its value type must be compatible
            if let Some(source_idx_sig) = source_idx
                && !self.is_assignable(source_idx_sig.value_type, target_idx_sig.value_type)
            {
                return false;
            }
        }

        true
    }

    /// Find properties that are required in target but missing in source.
    pub(super) fn find_missing_properties(
        &mut self,
        source_id: TypeId,
        target_id: TypeId,
    ) -> Vec<String> {
        // Resolve TypeRefs first
        let source = self.get_type(source_id).clone();
        let resolved_source_id = if let Type::TypeRef { ref name, ref type_args } = source {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or(source_id)
        } else {
            source_id
        };
        let target = self.get_type(target_id).clone();
        let resolved_target_id = if let Type::TypeRef { ref name, ref type_args } = target {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or(target_id)
        } else {
            target_id
        };

        // Get properties from both types
        let resolved_source = self.get_type(resolved_source_id).clone();
        let source_props = match resolved_source {
            Type::Object {
                ref properties,
                ref extends,
                ..
            } => self.resolve_object_properties(properties, extends),
            _ => vec![],
        };

        let resolved_target = self.get_type(resolved_target_id).clone();
        let target_props = match resolved_target {
            Type::Object {
                ref properties,
                ref extends,
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
    fn is_function_assignable(
        &mut self,
        source_params: &[crate::types::Param],
        source_return_id: TypeId,
        target_params: &[crate::types::Param],
        target_return_id: TypeId,
    ) -> bool {
        // Return type: covariant
        if !self.is_assignable(source_return_id, target_return_id) {
            return false;
        }

        // Parameters: source can have fewer (callback compatibility)
        if source_params.len() > target_params.len() {
            return false;
        }

        // Contravariant: target param must be assignable to source param
        for (sp, tp) in source_params.iter().zip(target_params.iter()) {
            if !self.is_assignable(tp.ty, sp.ty) {
                return false;
            }
        }

        true
    }

    /// Check if a string literal matches a template literal pattern.
    fn string_matches_template_pattern(
        &self,
        s: &str,
        texts: &[String],
        type_ids: &[TypeId],
    ) -> bool {
        if texts.is_empty() {
            return false;
        }

        // Must start with first text segment
        if !s.starts_with(&texts[0]) {
            return false;
        }

        // Must end with last text segment (if there's more than one)
        if texts.len() > 1 && !s.ends_with(texts.last().unwrap()) {
            return false;
        }

        // For simple check with one placeholder
        if type_ids.len() == 1 {
            let ty = self.get_type(type_ids[0]);
            if matches!(ty, Type::String) {
                let prefix = &texts[0];
                let suffix = texts.get(1).map(|s| s.as_str()).unwrap_or("");
                if prefix.len() + suffix.len() <= s.len() {
                    let middle = &s[prefix.len()..s.len() - suffix.len()];
                    return !middle.is_empty() || (prefix.len() + suffix.len() == s.len());
                }
                return false;
            }

            // For union placeholders
            if let Type::Union(union_type_ids) = ty {
                let prefix = &texts[0];
                let suffix = texts.get(1).map(|s| s.as_str()).unwrap_or("");
                if s.len() >= prefix.len() + suffix.len() {
                    let middle = &s[prefix.len()..s.len() - suffix.len()];
                    return union_type_ids.iter().any(|&t_id| {
                        let t = self.get_type(t_id);
                        if let Type::StringLiteral(lit) = t {
                            lit == middle
                        } else {
                            false
                        }
                    });
                }
            }
        }

        true
    }
}
