//! Literal type checking.
//!
//! # Contextual Typing
//!
//! Literals are checked against an expected type from the context:
//! - Tuple contextual typing: `const x: [number, string] = [1, "hello"]`
//! - Object literal checks: missing properties, excess properties, type compatibility
//!
//! # Excess Property Checking
//!
//! Fresh object literals are checked for excess properties that don't exist
//! in the target type (unless an index signature allows arbitrary keys).

use oxc_ast::ast::*;
use oxc_span::Span;

use crate::types::Type;

use super::{Checker, TypeError};

impl<'a> Checker<'a> {
    /// Check an array literal against an expected type.
    ///
    /// Handles contextual typing for tuples: [1, "hello"] against [number, string]
    pub(super) fn check_array_literal_against_type(
        &mut self,
        arr: &ArrayExpression,
        expected: &Type,
        span: Span,
    ) {
        // If expected type is a tuple, check element-by-element
        if let Type::Tuple(expected_types) = expected {
            // Check length
            if arr.elements.len() != expected_types.len() {
                let inferred = self.infer_array_literal(arr);
                self.errors
                    .push(TypeError::not_assignable(&inferred, expected, span));
                return;
            }

            // Check each element against expected type
            for (elem, expected_type) in arr.elements.iter().zip(expected_types.iter()) {
                if let Some(expr) = elem.as_expression() {
                    let elem_type = self.infer_expression(expr);
                    if !self.is_assignable(&elem_type, expected_type) {
                        self.errors.push(TypeError::not_assignable(
                            &elem_type,
                            expected_type,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // If expected type is an array, check all elements against element type
        if let Type::Array(expected_elem) = expected {
            for elem in &arr.elements {
                if let Some(expr) = elem.as_expression() {
                    let elem_type = self.infer_expression(expr);
                    if !self.is_assignable(&elem_type, expected_elem) {
                        self.errors.push(TypeError::not_assignable(
                            &elem_type,
                            expected_elem,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // Fall back to normal assignability
        let inferred = self.infer_array_literal(arr);
        if !self.is_assignable(&inferred, expected) {
            self.errors
                .push(TypeError::not_assignable(&inferred, expected, span));
        }
    }

    /// Check an object literal against an expected type.
    ///
    /// This performs:
    /// 1. Missing property check - all required properties must be present
    /// 2. Excess property check - no extra properties allowed (fresh literal only)
    /// 3. Property type compatibility check
    pub(super) fn check_object_literal_against_type(
        &mut self,
        obj: &ObjectExpression,
        expected: &Type,
        span: Span,
    ) {
        // Resolve TypeRef to actual type, instantiating generic types
        let resolved = if let Type::TypeRef { name, type_args } = expected {
            self.resolve_type_ref_with_args(name, type_args)
                .unwrap_or_else(|| expected.clone())
        } else {
            expected.clone()
        };

        let Type::Object {
            properties: expected_props,
            index_signature,
            extends,
            ..
        } = &resolved
        else {
            let source = self.infer_object_literal(obj);
            if !self.is_assignable(&source, expected) {
                self.errors
                    .push(TypeError::not_assignable(&source, expected, span));
            }
            return;
        };

        let expected_props = self.resolve_object_properties(expected_props, extends);

        // Collect properties from the object literal
        let mut literal_props: Vec<(String, Span)> = Vec::new();
        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop
                && let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                } {
                    literal_props.push((name.clone(), p.span));

                    // Check property type compatibility
                    if let Some(expected_prop) = expected_props.iter().find(|ep| ep.name == name) {
                        let value_type = self.infer_expression(&p.value);
                        if !self.is_assignable(&value_type, &expected_prop.ty) {
                            // Check if the error is due to missing properties
                            let missing =
                                self.find_missing_properties(&value_type, &expected_prop.ty);
                            if missing.len() > 1 {
                                // TS2739 for multiple missing properties
                                self.errors.push(TypeError::missing_properties(
                                    &missing,
                                    &value_type,
                                    &expected_prop.ty,
                                    p.span,
                                ));
                            } else if missing.len() == 1 {
                                // TS2741 for single missing property
                                self.errors.push(TypeError::missing_property(
                                    &missing[0],
                                    &value_type,
                                    &expected_prop.ty,
                                    p.span,
                                ));
                            } else {
                                self.errors.push(TypeError::not_assignable(
                                    &value_type,
                                    &expected_prop.ty,
                                    p.span,
                                ));
                            }
                        }
                    }
                }
        }

        let literal_prop_names: std::collections::HashSet<&str> =
            literal_props.iter().map(|(n, _)| n.as_str()).collect();

        // Check for missing required properties
        // Collect all missing properties first to decide between TS2739 and TS2741
        let source_type = self.infer_object_literal(obj);
        let missing_props: Vec<String> = expected_props
            .iter()
            .filter(|p| !p.optional && !literal_prop_names.contains(p.name.as_str()))
            .map(|p| p.name.clone())
            .collect();

        if missing_props.len() > 1 {
            // TS2739: Multiple missing properties
            self.errors.push(TypeError::missing_properties(
                &missing_props,
                &source_type,
                expected,
                span,
            ));
        } else if missing_props.len() == 1 {
            // TS2741: Single missing property
            self.errors.push(TypeError::missing_property(
                &missing_props[0],
                &source_type,
                expected,
                span,
            ));
        }

        // Check for excess properties (only if no index signature)
        // If index signature exists, verify value types match
        let expected_prop_names: std::collections::HashSet<&str> =
            expected_props.iter().map(|p| p.name.as_str()).collect();

        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop
                && let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                } {
                    // Property not in expected_props - either excess or needs index sig check
                    if !expected_prop_names.contains(name.as_str()) {
                        if let Some(idx_sig) = index_signature {
                            // Index signature exists - check value type compatibility
                            let value_type = self.infer_expression(&p.value);
                            if !self.is_assignable(&value_type, &idx_sig.value_type) {
                                self.errors.push(TypeError::not_assignable(
                                    &value_type,
                                    &idx_sig.value_type,
                                    p.span,
                                ));
                            }
                        } else {
                            // No index signature - excess property error
                            self.errors
                                .push(TypeError::excess_property(&name, expected, p.span));
                        }
                    }
                }
        }
    }
}
