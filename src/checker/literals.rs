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

use crate::types::{Type, TypeId};

use super::{Checker, TypeError};

impl<'a> Checker<'a> {
    /// Check an array literal against an expected type.
    ///
    /// Handles contextual typing for tuples: [1, "hello"] against [number, string]
    pub(super) fn check_array_literal_against_type(
        &mut self,
        arr: &ArrayExpression,
        expected_id: TypeId,
        span: Span,
    ) {
        let expected = self.get_type(expected_id).clone();

        // If expected type is a tuple, check element-by-element
        if let Type::Tuple(expected_type_ids) = expected {
            // Check length
            if arr.elements.len() != expected_type_ids.len() {
                let inferred_id = self.infer_array_literal(arr);
                let inferred_str = self.fmt_type(inferred_id);
                let expected_str = self.fmt_type(expected_id);
                self.errors
                    .push(TypeError::not_assignable(&inferred_str, &expected_str, span));
                return;
            }

            // Check each element against expected type
            for (elem, &expected_type_id) in arr.elements.iter().zip(expected_type_ids.iter()) {
                if let Some(expr) = elem.as_expression() {
                    let elem_type_id = self.infer_expression(expr);
                    if !self.is_assignable(elem_type_id, expected_type_id) {
                        let elem_str = self.fmt_type(elem_type_id);
                        let expected_str = self.fmt_type(expected_type_id);
                        self.errors.push(TypeError::not_assignable(
                            &elem_str,
                            &expected_str,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // If expected type is an array, check all elements against element type
        if let Type::Array(expected_elem_id) = expected {
            for elem in &arr.elements {
                if let Some(expr) = elem.as_expression() {
                    let elem_type_id = self.infer_expression(expr);
                    if !self.is_assignable(elem_type_id, expected_elem_id) {
                        let elem_str = self.fmt_type(elem_type_id);
                        let expected_str = self.fmt_type(expected_elem_id);
                        self.errors.push(TypeError::not_assignable(
                            &elem_str,
                            &expected_str,
                            span,
                        ));
                    }
                }
            }
            return;
        }

        // Fall back to normal assignability
        let inferred_id = self.infer_array_literal(arr);
        if !self.is_assignable(inferred_id, expected_id) {
            let inferred_str = self.fmt_type(inferred_id);
            let expected_str = self.fmt_type(expected_id);
            self.errors
                .push(TypeError::not_assignable(&inferred_str, &expected_str, span));
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
        expected_id: TypeId,
        span: Span,
    ) {
        // Resolve TypeRef and computed types (keyof, mapped types, indexed access)
        let resolved_id = self.resolve_computed_type(expected_id);

        let resolved = self.get_type(resolved_id).clone();
        let (expected_props_owned, index_signature, extends) = match resolved {
            Type::Object {
                properties,
                index_signature,
                extends,
                ..
            } => (properties, index_signature, extends),
            _ => {
                let source_id = self.infer_object_literal(obj);
                if !self.is_assignable(source_id, expected_id) {
                    let source_str = self.fmt_type(source_id);
                    let expected_str = self.fmt_type(expected_id);
                    self.errors
                        .push(TypeError::not_assignable(&source_str, &expected_str, span));
                }
                return;
            }
        };

        let expected_props = self.resolve_object_properties(&expected_props_owned, &extends);

        // Collect properties from the object literal
        let mut literal_props: Vec<(String, Span)> = Vec::new();
        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop
                && let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                }
            {
                literal_props.push((name.clone(), p.span));

                // Check property type compatibility
                if let Some(expected_prop) = expected_props.iter().find(|ep| ep.name == name) {
                    let value_type_id = self.infer_expression(&p.value);
                    if !self.is_assignable(value_type_id, expected_prop.ty) {
                        // Check if the error is due to missing properties
                        let missing =
                            self.find_missing_properties(value_type_id, expected_prop.ty);
                        let value_str = self.fmt_type(value_type_id);
                        let expected_str = self.fmt_type(expected_prop.ty);
                        if missing.len() > 1 {
                            // TS2739 for multiple missing properties
                            self.errors.push(TypeError::missing_properties(
                                &missing,
                                &value_str,
                                &expected_str,
                                p.span,
                            ));
                        } else if missing.len() == 1 {
                            // TS2741 for single missing property
                            self.errors.push(TypeError::missing_property(
                                &missing[0],
                                &value_str,
                                &expected_str,
                                p.span,
                            ));
                        } else {
                            self.errors.push(TypeError::not_assignable(
                                &value_str,
                                &expected_str,
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
        let source_type_id = self.infer_object_literal(obj);
        let missing_props: Vec<String> = expected_props
            .iter()
            .filter(|p| !p.optional && !literal_prop_names.contains(p.name.as_str()))
            .map(|p| p.name.clone())
            .collect();

        if missing_props.len() > 1 {
            // TS2739: Multiple missing properties
            let source_str = self.fmt_type(source_type_id);
            let expected_str = self.fmt_type(expected_id);
            self.errors.push(TypeError::missing_properties(
                &missing_props,
                &source_str,
                &expected_str,
                span,
            ));
        } else if missing_props.len() == 1 {
            // TS2741: Single missing property
            let source_str = self.fmt_type(source_type_id);
            let expected_str = self.fmt_type(expected_id);
            self.errors.push(TypeError::missing_property(
                &missing_props[0],
                &source_str,
                &expected_str,
                span,
            ));
        }

        // Check for excess properties (only if no index signature)
        let expected_prop_names: std::collections::HashSet<&str> =
            expected_props.iter().map(|p| p.name.as_str()).collect();

        for prop in &obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop
                && let Some(name) = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => Some(ident.name.to_string()),
                    PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                    PropertyKey::NumericLiteral(n) => Some(n.value.to_string()),
                    _ => None,
                }
            {
                // Property not in expected_props - either excess or needs index sig check
                if !expected_prop_names.contains(name.as_str()) {
                    if let Some(idx_sig) = &index_signature {
                        // Index signature exists - check value type compatibility
                        let value_type_id = self.infer_expression(&p.value);
                        if !self.is_assignable(value_type_id, idx_sig.value_type) {
                            let value_str = self.fmt_type(value_type_id);
                            let idx_value_str = self.fmt_type(idx_sig.value_type);
                            self.errors.push(TypeError::not_assignable(
                                &value_str,
                                &idx_value_str,
                                p.span,
                            ));
                        }
                    } else {
                        // No index signature - excess property error
                        let expected_str = self.fmt_type(expected_id);
                        self.errors
                            .push(TypeError::excess_property(&name, &expected_str, p.span));
                    }
                }
            }
        }
    }
}
