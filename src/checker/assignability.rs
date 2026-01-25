//! Type assignability checking.
//!
//! Determines if a source type can be assigned to a target type.
//! Uses structural typing with special rules for:
//! - Unions and intersections
//! - Literal types and their base types
//! - Object structural compatibility
//! - Function variance (covariant return, contravariant params)

use crate::types::Type;

use super::Checker;

impl<'a> Checker<'a> {
    /// Resolve a TypeRef to its underlying type by looking it up in the type namespace.
    fn resolve_type_ref(&self, name: &str) -> Option<Type> {
        self.symbols.lookup_type(name).map(|s| s.ty.clone())
    }

    /// Check if source type is assignable to target type.
    pub fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        // Handle TypeRef resolution, only clone when actually needed
        if let Type::TypeRef { name, .. } = source {
            if let Some(resolved) = self.resolve_type_ref(name) {
                return self.is_assignable(&resolved, target);
            }
        }
        if let Type::TypeRef { name, .. } = target {
            if let Some(resolved) = self.resolve_type_ref(name) {
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
                ..
            },
            Type::Object {
                properties: target_props,
                ..
            },
        ) = (source, target)
        {
            return self.is_object_assignable(source_props, target_props);
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
    /// Target's required properties must all exist in source with compatible types.
    fn is_object_assignable(
        &self,
        source_props: &[crate::types::Property],
        target_props: &[crate::types::Property],
    ) -> bool {
        for target_prop in target_props {
            if target_prop.optional {
                continue;
            }
            let source_prop = source_props.iter().find(|p| p.name == target_prop.name);
            match source_prop {
                None => return false,
                Some(sp) => {
                    if !self.is_assignable(&sp.ty, &target_prop.ty) {
                        return false;
                    }
                }
            }
        }
        true
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
