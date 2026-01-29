//! Control flow analysis for type narrowing.
//!
//! This module handles control flow-based type refinement:
//! - typeof checks: `typeof x === "string"` narrows to string
//! - null/undefined checks: `x !== null` removes null from union
//! - instanceof checks: `x instanceof Foo` narrows to Foo
//! - truthiness checks: `if (x)` removes null/undefined
//!
//! The `NarrowingContext` tracks narrowed types within a scope. When entering
//! conditional branches (if/else), types are refined based on the condition.

use rustc_hash::FxHashMap;

use oxc_ast::ast::*;

use crate::types::Type;

/// Tracks narrowed types within a scope.
///
/// When we enter a conditional branch (if/else), we can narrow types
/// based on the condition expression.
#[derive(Clone, Debug)]
#[derive(Default)]
pub struct NarrowingContext {
    /// Variable name -> narrowed type. Uses FxHashMap for faster lookups.
    narrowed: FxHashMap<String, Type>,
}


impl NarrowingContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Narrow a variable to a specific type.
    pub fn narrow(&mut self, name: String, ty: Type) {
        self.narrowed.insert(name, ty);
    }

    /// Get the narrowed type for a variable, if any.
    pub fn get_narrowed(&self, name: &str) -> Option<&Type> {
        self.narrowed.get(name)
    }

    /// Clear all narrowing information (e.g., when exiting a branch).
    pub fn clear(&mut self) {
        self.narrowed.clear();
    }
}

/// A type guard extracted from a condition expression.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeGuard {
    /// typeof x === "string" -> Typeof("string")
    Typeof(String),
    /// x !== null
    NotNull,
    /// x !== undefined
    NotUndefined,
    /// x instanceof Foo -> Instanceof("Foo")
    Instanceof(String),
    /// Truthiness check (removes null/undefined)
    Truthy,
    /// Discriminant property check: x.kind === "circle"
    /// Contains (property_name, literal_value)
    Discriminant(String, String),
}

/// Result of extracting a type guard from an expression.
#[derive(Debug, Clone)]
pub struct ExtractedGuard {
    /// The variable being guarded
    pub variable: String,
    /// The type guard to apply
    pub guard: TypeGuard,
    /// Whether this is negated (for else branches)
    pub negated: bool,
}

/// Extract type guard from a condition expression.
///
/// Returns the variable name and guard type if a guard can be extracted.
pub fn extract_type_guard(expr: &Expression) -> Option<ExtractedGuard> {
    match expr {
        // typeof x === "string" or typeof x === "number" etc.
        Expression::BinaryExpression(binary) => extract_from_binary(binary),

        // Unary negation: !x or !(typeof x === "string")
        Expression::UnaryExpression(unary) => {
            if matches!(unary.operator, UnaryOperator::LogicalNot) {
                // Negate the inner guard
                extract_type_guard(&unary.argument).map(|mut g| {
                    g.negated = !g.negated;
                    g
                })
            } else {
                None
            }
        }

        // Bare identifier: if (x) - truthiness check
        Expression::Identifier(ident) => Some(ExtractedGuard {
            variable: ident.name.to_string(),
            guard: TypeGuard::Truthy,
            negated: false,
        }),

        // Parenthesized expression
        Expression::ParenthesizedExpression(paren) => extract_type_guard(&paren.expression),

        _ => None,
    }
}

/// Extract type guard from a binary expression.
fn extract_from_binary(binary: &BinaryExpression) -> Option<ExtractedGuard> {
    match binary.operator {
        // Strict equality: typeof x === "string" or x === null
        BinaryOperator::StrictEquality | BinaryOperator::Equality => {
            extract_equality_guard(binary, false)
        }

        // Strict inequality: typeof x !== "string" or x !== null
        BinaryOperator::StrictInequality | BinaryOperator::Inequality => {
            extract_equality_guard(binary, true)
        }

        // instanceof: x instanceof Foo
        BinaryOperator::Instanceof => {
            if let Expression::Identifier(left) = &binary.left
                && let Expression::Identifier(right) = &binary.right {
                    return Some(ExtractedGuard {
                        variable: left.name.to_string(),
                        guard: TypeGuard::Instanceof(right.name.to_string()),
                        negated: false,
                    });
                }
            None
        }

        _ => None,
    }
}

/// Extract guard from equality/inequality expressions.
fn extract_equality_guard(
    binary: &BinaryExpression,
    is_inequality: bool,
) -> Option<ExtractedGuard> {
    // typeof x === "string"
    if let Some(guard) = extract_typeof_guard(&binary.left, &binary.right, is_inequality) {
        return Some(guard);
    }
    // "string" === typeof x (reversed)
    if let Some(guard) = extract_typeof_guard(&binary.right, &binary.left, is_inequality) {
        return Some(guard);
    }

    // x === null or x !== null
    if let Some(guard) = extract_null_guard(&binary.left, &binary.right, is_inequality) {
        return Some(guard);
    }
    // null === x (reversed)
    if let Some(guard) = extract_null_guard(&binary.right, &binary.left, is_inequality) {
        return Some(guard);
    }

    // x === undefined or x !== undefined
    if let Some(guard) = extract_undefined_guard(&binary.left, &binary.right, is_inequality) {
        return Some(guard);
    }
    // undefined === x (reversed)
    if let Some(guard) = extract_undefined_guard(&binary.right, &binary.left, is_inequality) {
        return Some(guard);
    }

    // x.kind === "circle" (discriminant narrowing)
    if let Some(guard) = extract_discriminant_guard(&binary.left, &binary.right, is_inequality) {
        return Some(guard);
    }
    // "circle" === x.kind (reversed)
    if let Some(guard) = extract_discriminant_guard(&binary.right, &binary.left, is_inequality) {
        return Some(guard);
    }

    None
}

/// Extract typeof guard: typeof x === "string"
fn extract_typeof_guard(
    left: &Expression,
    right: &Expression,
    negated: bool,
) -> Option<ExtractedGuard> {
    if let Expression::UnaryExpression(unary) = left
        && matches!(unary.operator, UnaryOperator::Typeof)
            && let Expression::Identifier(ident) = &unary.argument
                && let Expression::StringLiteral(lit) = right {
                    return Some(ExtractedGuard {
                        variable: ident.name.to_string(),
                        guard: TypeGuard::Typeof(lit.value.to_string()),
                        negated,
                    });
                }
    None
}

/// Extract null guard: x === null or x !== null
fn extract_null_guard(
    left: &Expression,
    right: &Expression,
    is_inequality: bool,
) -> Option<ExtractedGuard> {
    if let Expression::Identifier(ident) = left
        && let Expression::NullLiteral(_) = right {
            return Some(ExtractedGuard {
                variable: ident.name.to_string(),
                guard: TypeGuard::NotNull,
                // For x === null, we negate (narrowing to null)
                // For x !== null, we don't negate (removing null)
                negated: !is_inequality,
            });
        }
    None
}

/// Extract undefined guard: x === undefined or x !== undefined
fn extract_undefined_guard(
    left: &Expression,
    right: &Expression,
    is_inequality: bool,
) -> Option<ExtractedGuard> {
    if let Expression::Identifier(ident) = left
        && let Expression::Identifier(right_ident) = right
            && right_ident.name == "undefined" {
                return Some(ExtractedGuard {
                    variable: ident.name.to_string(),
                    guard: TypeGuard::NotUndefined,
                    // For x === undefined, we negate (narrowing to undefined)
                    // For x !== undefined, we don't negate (removing undefined)
                    negated: !is_inequality,
                });
            }
    None
}

/// Extract discriminant guard: x.kind === "circle"
///
/// Used for discriminated union narrowing where a property with a literal type
/// identifies which union member we have.
fn extract_discriminant_guard(
    left: &Expression,
    right: &Expression,
    is_inequality: bool,
) -> Option<ExtractedGuard> {
    // Left must be a member expression: x.kind
    if let Expression::StaticMemberExpression(member) = left {
        // Get the object variable name
        if let Expression::Identifier(obj_ident) = &member.object {
            // Right must be a string literal: "circle"
            if let Expression::StringLiteral(lit) = right {
                let variable = obj_ident.name.to_string();
                let property = member.property.name.to_string();
                let value = lit.value.to_string();

                return Some(ExtractedGuard {
                    variable,
                    guard: TypeGuard::Discriminant(property, value),
                    negated: is_inequality,
                });
            }
        }
    }
    None
}

/// Apply a type guard to narrow a type.
pub fn apply_guard(original: &Type, guard: &TypeGuard, negated: bool) -> Type {
    apply_guard_with_resolver(original, guard, negated, |_| None)
}

/// Apply a type guard to narrow a type, with ability to resolve TypeRefs.
///
/// The resolver function takes a type name and returns the resolved type if available.
pub fn apply_guard_with_resolver<F>(
    original: &Type,
    guard: &TypeGuard,
    negated: bool,
    resolver: F,
) -> Type
where
    F: Fn(&str) -> Option<Type> + Copy,
{
    if negated {
        apply_negated_guard_with_resolver(original, guard, resolver)
    } else {
        apply_positive_guard_with_resolver(original, guard, resolver)
    }
}

/// Apply a positive type guard with resolver support.
fn apply_positive_guard_with_resolver<F>(original: &Type, guard: &TypeGuard, resolver: F) -> Type
where
    F: Fn(&str) -> Option<Type> + Copy,
{
    match guard {
        TypeGuard::Typeof(type_str) => {
            match type_str.as_str() {
                "string" => narrow_to_type(original, &Type::String),
                "number" => narrow_to_type(original, &Type::Number),
                "boolean" => narrow_to_type(original, &Type::Boolean),
                "undefined" => narrow_to_type(original, &Type::Undefined),
                "object" => {
                    // typeof x === "object" keeps objects, arrays, null
                    // For now, just return original if it could be object-like
                    original.clone()
                }
                "function" => {
                    // Keep function types, fallback to original if no match
                    filter_union(original, |t| matches!(t, Type::Function { .. }), original)
                }
                _ => original.clone(),
            }
        }
        TypeGuard::NotNull => remove_from_union(original, &Type::Null),
        TypeGuard::NotUndefined => remove_from_union(original, &Type::Undefined),
        TypeGuard::Instanceof(class_name) => Type::TypeRef {
            name: class_name.clone(),
            type_args: vec![],
        },
        TypeGuard::Truthy => {
            // Remove null, undefined from union
            let without_null = remove_from_union(original, &Type::Null);
            remove_from_union(&without_null, &Type::Undefined)
        }
        TypeGuard::Discriminant(prop_name, prop_value) => {
            // Narrow union to members that have the matching discriminant property
            narrow_by_discriminant_with_resolver(original, prop_name, prop_value, resolver)
        }
    }
}

/// Apply a negated type guard with resolver support.
fn apply_negated_guard_with_resolver<F>(original: &Type, guard: &TypeGuard, resolver: F) -> Type
where
    F: Fn(&str) -> Option<Type> + Copy,
{
    match guard {
        TypeGuard::Typeof(type_str) => {
            // typeof x !== "string" removes string from union
            let target = match type_str.as_str() {
                "string" => Type::String,
                "number" => Type::Number,
                "boolean" => Type::Boolean,
                "undefined" => Type::Undefined,
                _ => return original.clone(),
            };
            remove_from_union(original, &target)
        }
        TypeGuard::NotNull => {
            // Negated NotNull means it IS null
            narrow_to_type(original, &Type::Null)
        }
        TypeGuard::NotUndefined => {
            // Negated NotUndefined means it IS undefined
            narrow_to_type(original, &Type::Undefined)
        }
        TypeGuard::Instanceof(_) => {
            // Negated instanceof - hard to narrow, just return original
            original.clone()
        }
        TypeGuard::Truthy => {
            // Negated truthy means it's falsy (null, undefined, false, 0, "")
            // For now, just return original
            original.clone()
        }
        TypeGuard::Discriminant(prop_name, prop_value) => {
            // Negated discriminant: remove members that match the discriminant
            exclude_by_discriminant_with_resolver(original, prop_name, prop_value, resolver)
        }
    }
}

/// Narrow a type to a target type (for positive guards).
fn narrow_to_type(original: &Type, target: &Type) -> Type {
    match original {
        Type::Union(types) => {
            // Find types in the union that match the target
            let matching: Vec<Type> = types
                .iter()
                .filter(|t| types_compatible(t, target))
                .cloned()
                .collect();

            match matching.len() {
                0 => target.clone(), // Fall back to target if nothing matches
                1 => matching.into_iter().next().expect("checked len == 1"),
                _ => Type::Union(matching),
            }
        }
        _ => target.clone(),
    }
}

/// Check if two types are compatible for narrowing.
fn types_compatible(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::String, Type::String) => true,
        (Type::StringLiteral(_), Type::String) => true,
        (Type::Number, Type::Number) => true,
        (Type::NumberLiteral(_), Type::Number) => true,
        (Type::Boolean, Type::Boolean) => true,
        (Type::BooleanLiteral(_), Type::Boolean) => true,
        (Type::Null, Type::Null) => true,
        (Type::Undefined, Type::Undefined) => true,
        _ => a == b,
    }
}

/// Remove a type from a union.
pub fn remove_from_union(ty: &Type, to_remove: &Type) -> Type {
    match ty {
        Type::Union(types) => {
            let filtered: Vec<Type> = types
                .iter()
                .filter(|t| !types_match(t, to_remove))
                .cloned()
                .collect();

            simplify_filtered_union(filtered)
        }
        _ => {
            if types_match(ty, to_remove) {
                Type::Never
            } else {
                ty.clone()
            }
        }
    }
}

/// Filter union to members matching predicate, returning fallback if no matches.
fn filter_union<F>(ty: &Type, pred: F, fallback: &Type) -> Type
where
    F: Fn(&Type) -> bool,
{
    match ty {
        Type::Union(types) => {
            let filtered: Vec<Type> = types.iter().filter(|t| pred(t)).cloned().collect();
            if filtered.is_empty() {
                fallback.clone()
            } else {
                simplify_filtered_union(filtered)
            }
        }
        _ => fallback.clone(),
    }
}

/// Simplify filtered union: empty -> never, single -> unwrap, else union.
fn simplify_filtered_union(types: Vec<Type>) -> Type {
    match types.len() {
        0 => Type::Never,
        1 => types.into_iter().next().expect("checked len == 1"),
        _ => Type::Union(types),
    }
}

/// Check if two types match (for removal).
fn types_match(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Null, Type::Null) => true,
        (Type::Undefined, Type::Undefined) => true,
        (Type::String, Type::String) => true,
        (Type::StringLiteral(_), Type::String) => true,
        (Type::Number, Type::Number) => true,
        (Type::NumberLiteral(_), Type::Number) => true,
        (Type::Boolean, Type::Boolean) => true,
        (Type::BooleanLiteral(_), Type::Boolean) => true,
        _ => a == b,
    }
}

/// Narrow a union type to members that have a matching discriminant property.
pub fn narrow_by_discriminant_with_resolver<F>(
    original: &Type,
    prop_name: &str,
    prop_value: &str,
    resolver: F,
) -> Type
where
    F: Fn(&str) -> Option<Type> + Copy,
{
    match original {
        Type::Union(types) => {
            let matching: Vec<Type> = types
                .iter()
                .filter(|t| {
                    has_discriminant_property_with_resolver(t, prop_name, prop_value, resolver)
                })
                .cloned()
                .collect();

            simplify_filtered_union(matching)
        }
        Type::TypeRef { name, .. } => {
            // If original is a TypeRef to a union, resolve and narrow
            if let Some(resolved) = resolver(name)
                && matches!(resolved, Type::Union(_)) {
                    return narrow_by_discriminant_with_resolver(
                        &resolved, prop_name, prop_value, resolver,
                    );
                }
            // For non-union types, check if it matches
            if has_discriminant_property_with_resolver(original, prop_name, prop_value, resolver) {
                original.clone()
            } else {
                Type::Never
            }
        }
        _ => {
            // For non-union types, check if it matches
            if has_discriminant_property_with_resolver(original, prop_name, prop_value, resolver) {
                original.clone()
            } else {
                Type::Never
            }
        }
    }
}

/// Exclude union members that have a matching discriminant property.
pub fn exclude_by_discriminant_with_resolver<F>(
    original: &Type,
    prop_name: &str,
    prop_value: &str,
    resolver: F,
) -> Type
where
    F: Fn(&str) -> Option<Type> + Copy,
{
    match original {
        Type::Union(types) => {
            let remaining: Vec<Type> = types
                .iter()
                .filter(|t| {
                    !has_discriminant_property_with_resolver(t, prop_name, prop_value, resolver)
                })
                .cloned()
                .collect();

            simplify_filtered_union(remaining)
        }
        Type::TypeRef { name, .. } => {
            // If original is a TypeRef to a union, resolve and exclude
            if let Some(resolved) = resolver(name)
                && matches!(resolved, Type::Union(_)) {
                    return exclude_by_discriminant_with_resolver(
                        &resolved, prop_name, prop_value, resolver,
                    );
                }
            // For non-union types, check if it matches
            if has_discriminant_property_with_resolver(original, prop_name, prop_value, resolver) {
                Type::Never
            } else {
                original.clone()
            }
        }
        _ => {
            if has_discriminant_property_with_resolver(original, prop_name, prop_value, resolver) {
                Type::Never
            } else {
                original.clone()
            }
        }
    }
}

/// Check if a type has a discriminant property with a specific string literal value.
pub fn has_discriminant_property_with_resolver<F>(
    ty: &Type,
    prop_name: &str,
    expected_value: &str,
    resolver: F,
) -> bool
where
    F: Fn(&str) -> Option<Type>,
{
    match ty {
        Type::Object { properties, .. } => properties.iter().any(|p| {
            p.name == prop_name && matches!(&p.ty, Type::StringLiteral(v) if v == expected_value)
        }),
        Type::TypeRef { name, .. } => {
            // Try to resolve the type reference
            if let Some(resolved) = resolver(name) {
                has_discriminant_property_with_resolver(
                    &resolved,
                    prop_name,
                    expected_value,
                    resolver,
                )
            } else {
                // If we can't resolve, be conservative and return false
                false
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_narrowing_context() {
        let mut ctx = NarrowingContext::new();
        ctx.narrow("x".to_string(), Type::String);

        assert_eq!(ctx.get_narrowed("x"), Some(&Type::String));
        assert_eq!(ctx.get_narrowed("y"), None);

        ctx.clear();
        assert_eq!(ctx.get_narrowed("x"), None);
    }

    #[test]
    fn test_remove_null_from_union() {
        let union = Type::Union(vec![Type::String, Type::Null]);
        let result = remove_from_union(&union, &Type::Null);
        assert_eq!(result, Type::String);
    }

    #[test]
    fn test_remove_undefined_from_union() {
        let union = Type::Union(vec![Type::Number, Type::Undefined]);
        let result = remove_from_union(&union, &Type::Undefined);
        assert_eq!(result, Type::Number);
    }

    #[test]
    fn test_remove_multiple_keeps_union() {
        let union = Type::Union(vec![Type::String, Type::Number, Type::Null]);
        let result = remove_from_union(&union, &Type::Null);
        assert_eq!(result, Type::Union(vec![Type::String, Type::Number]));
    }

    #[test]
    fn test_apply_typeof_guard() {
        let union = Type::Union(vec![Type::String, Type::Number]);
        let result = apply_guard(&union, &TypeGuard::Typeof("string".to_string()), false);
        assert_eq!(result, Type::String);
    }

    #[test]
    fn test_apply_not_null_guard() {
        let union = Type::Union(vec![Type::String, Type::Null]);
        let result = apply_guard(&union, &TypeGuard::NotNull, false);
        assert_eq!(result, Type::String);
    }

    #[test]
    fn test_apply_truthy_guard() {
        let union = Type::Union(vec![Type::String, Type::Null, Type::Undefined]);
        let result = apply_guard(&union, &TypeGuard::Truthy, false);
        assert_eq!(result, Type::String);
    }
}
