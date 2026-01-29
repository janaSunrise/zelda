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

use crate::types::{Type, TypeArena, TypeId};

/// Tracks narrowed types within a scope.
///
/// When we enter a conditional branch (if/else), we can narrow types
/// based on the condition expression.
#[derive(Clone, Debug, Default)]
pub struct NarrowingContext {
    /// Variable name -> narrowed type ID. Uses FxHashMap for faster lookups.
    narrowed: FxHashMap<String, TypeId>,
}

impl NarrowingContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Narrow a variable to a specific type.
    pub fn narrow(&mut self, name: String, ty: TypeId) {
        self.narrowed.insert(name, ty);
    }

    /// Get the narrowed type ID for a variable, if any.
    pub fn get_narrowed_id(&self, name: &str) -> Option<TypeId> {
        self.narrowed.get(name).copied()
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
                && let Expression::Identifier(right) = &binary.right
            {
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
fn extract_equality_guard(binary: &BinaryExpression, is_inequality: bool) -> Option<ExtractedGuard> {
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
        && let Expression::StringLiteral(lit) = right
    {
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
        && let Expression::NullLiteral(_) = right
    {
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
        && right_ident.name == "undefined"
    {
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

/// Apply a type guard to narrow a type, using the arena to resolve type IDs.
///
/// The `type_resolver` function takes a type name (for TypeRef) and returns
/// the resolved TypeId if available.
pub fn apply_guard_with_arena<F>(
    arena: &mut TypeArena,
    original_id: TypeId,
    guard: &TypeGuard,
    negated: bool,
    type_resolver: F,
) -> TypeId
where
    F: Fn(&str) -> Option<TypeId> + Copy,
{
    if negated {
        apply_negated_guard_with_arena(arena, original_id, guard, type_resolver)
    } else {
        apply_positive_guard_with_arena(arena, original_id, guard, type_resolver)
    }
}

/// Apply a positive type guard with arena support.
fn apply_positive_guard_with_arena<F>(
    arena: &mut TypeArena,
    original_id: TypeId,
    guard: &TypeGuard,
    type_resolver: F,
) -> TypeId
where
    F: Fn(&str) -> Option<TypeId> + Copy,
{
    match guard {
        TypeGuard::Typeof(type_str) => match type_str.as_str() {
            "string" => narrow_to_type_with_arena(arena, original_id, TypeId::STRING),
            "number" => narrow_to_type_with_arena(arena, original_id, TypeId::NUMBER),
            "boolean" => narrow_to_type_with_arena(arena, original_id, TypeId::BOOLEAN),
            "undefined" => narrow_to_type_with_arena(arena, original_id, TypeId::UNDEFINED),
            "object" => {
                // typeof x === "object" keeps objects, arrays, null
                // For now, just return original if it could be object-like
                original_id
            }
            "function" => {
                // Keep function types, fallback to original if no match
                filter_union_with_arena(
                    arena,
                    original_id,
                    |ty| matches!(ty, Type::Function { .. }),
                    original_id,
                )
            }
            _ => original_id,
        },
        TypeGuard::NotNull => remove_from_union_with_arena(arena, original_id, TypeId::NULL),
        TypeGuard::NotUndefined => {
            remove_from_union_with_arena(arena, original_id, TypeId::UNDEFINED)
        }
        TypeGuard::Instanceof(class_name) => arena.intern(Type::TypeRef {
            name: class_name.clone(),
            type_args: vec![],
        }),
        TypeGuard::Truthy => {
            // Remove null, undefined from union
            let without_null = remove_from_union_with_arena(arena, original_id, TypeId::NULL);
            remove_from_union_with_arena(arena, without_null, TypeId::UNDEFINED)
        }
        TypeGuard::Discriminant(prop_name, prop_value) => {
            // Narrow union to members that have the matching discriminant property
            narrow_by_discriminant_with_arena(arena, original_id, prop_name, prop_value, type_resolver)
        }
    }
}

/// Apply a negated type guard with arena support.
fn apply_negated_guard_with_arena<F>(
    arena: &mut TypeArena,
    original_id: TypeId,
    guard: &TypeGuard,
    type_resolver: F,
) -> TypeId
where
    F: Fn(&str) -> Option<TypeId> + Copy,
{
    match guard {
        TypeGuard::Typeof(type_str) => {
            // typeof x !== "string" removes string from union
            let target_id = match type_str.as_str() {
                "string" => TypeId::STRING,
                "number" => TypeId::NUMBER,
                "boolean" => TypeId::BOOLEAN,
                "undefined" => TypeId::UNDEFINED,
                _ => return original_id,
            };
            remove_from_union_with_arena(arena, original_id, target_id)
        }
        TypeGuard::NotNull => {
            // Negated NotNull means it IS null
            narrow_to_type_with_arena(arena, original_id, TypeId::NULL)
        }
        TypeGuard::NotUndefined => {
            // Negated NotUndefined means it IS undefined
            narrow_to_type_with_arena(arena, original_id, TypeId::UNDEFINED)
        }
        TypeGuard::Instanceof(_) => {
            // Negated instanceof - hard to narrow, just return original
            original_id
        }
        TypeGuard::Truthy => {
            // Negated truthy means it's falsy (null, undefined, false, 0, "")
            // For now, just return original
            original_id
        }
        TypeGuard::Discriminant(prop_name, prop_value) => {
            // Negated discriminant: remove members that match the discriminant
            exclude_by_discriminant_with_arena(arena, original_id, prop_name, prop_value, type_resolver)
        }
    }
}

/// Narrow a type to a target type (for positive guards).
fn narrow_to_type_with_arena(arena: &mut TypeArena, original_id: TypeId, target_id: TypeId) -> TypeId {
    let original = arena.get(original_id).clone();

    match original {
        Type::Union(type_ids) => {
            // Find types in the union that match the target
            let matching: Vec<TypeId> = type_ids
                .iter()
                .filter(|&&id| types_compatible_with_arena(arena, id, target_id))
                .copied()
                .collect();

            simplify_union_ids(arena, matching, target_id)
        }
        _ => target_id,
    }
}

/// Check if two types are compatible for narrowing.
fn types_compatible_with_arena(arena: &TypeArena, a_id: TypeId, b_id: TypeId) -> bool {
    if a_id == b_id {
        return true;
    }

    let a = arena.get(a_id);
    let b = arena.get(b_id);

    match (a, b) {
        (Type::String, Type::String) => true,
        (Type::StringLiteral(_), Type::String) => true,
        (Type::Number, Type::Number) => true,
        (Type::NumberLiteral(_), Type::Number) => true,
        (Type::Boolean, Type::Boolean) => true,
        (Type::BooleanLiteral(_), Type::Boolean) => true,
        (Type::Null, Type::Null) => true,
        (Type::Undefined, Type::Undefined) => true,
        _ => false,
    }
}

/// Remove a type from a union.
fn remove_from_union_with_arena(
    arena: &mut TypeArena,
    ty_id: TypeId,
    to_remove_id: TypeId,
) -> TypeId {
    let ty = arena.get(ty_id).clone();

    match ty {
        Type::Union(type_ids) => {
            let filtered: Vec<TypeId> = type_ids
                .iter()
                .filter(|&&id| !types_match_with_arena(arena, id, to_remove_id))
                .copied()
                .collect();

            simplify_filtered_union_ids(arena, filtered)
        }
        _ => {
            if types_match_with_arena(arena, ty_id, to_remove_id) {
                TypeId::NEVER
            } else {
                ty_id
            }
        }
    }
}

/// Filter union to members matching predicate, returning fallback if no matches.
fn filter_union_with_arena<F>(
    arena: &mut TypeArena,
    ty_id: TypeId,
    pred: F,
    fallback_id: TypeId,
) -> TypeId
where
    F: Fn(&Type) -> bool,
{
    let ty = arena.get(ty_id).clone();

    match ty {
        Type::Union(type_ids) => {
            let filtered: Vec<TypeId> = type_ids
                .iter()
                .filter(|&&id| pred(arena.get(id)))
                .copied()
                .collect();

            if filtered.is_empty() {
                fallback_id
            } else {
                simplify_filtered_union_ids(arena, filtered)
            }
        }
        _ => fallback_id,
    }
}

/// Simplify filtered union: empty -> never, single -> unwrap, else union.
fn simplify_filtered_union_ids(arena: &mut TypeArena, type_ids: Vec<TypeId>) -> TypeId {
    match type_ids.len() {
        0 => TypeId::NEVER,
        1 => type_ids[0],
        _ => arena.intern(Type::Union(type_ids)),
    }
}

/// Simplify union with fallback for empty case.
fn simplify_union_ids(arena: &mut TypeArena, type_ids: Vec<TypeId>, fallback_id: TypeId) -> TypeId {
    match type_ids.len() {
        0 => fallback_id,
        1 => type_ids[0],
        _ => arena.intern(Type::Union(type_ids)),
    }
}

/// Check if two types match (for removal).
fn types_match_with_arena(arena: &TypeArena, a_id: TypeId, b_id: TypeId) -> bool {
    if a_id == b_id {
        return true;
    }

    let a = arena.get(a_id);
    let b = arena.get(b_id);

    match (a, b) {
        (Type::Null, Type::Null) => true,
        (Type::Undefined, Type::Undefined) => true,
        (Type::String, Type::String) => true,
        (Type::StringLiteral(_), Type::String) => true,
        (Type::Number, Type::Number) => true,
        (Type::NumberLiteral(_), Type::Number) => true,
        (Type::Boolean, Type::Boolean) => true,
        (Type::BooleanLiteral(_), Type::Boolean) => true,
        _ => false,
    }
}

/// Narrow a union type to members that have a matching discriminant property.
fn narrow_by_discriminant_with_arena<F>(
    arena: &mut TypeArena,
    original_id: TypeId,
    prop_name: &str,
    prop_value: &str,
    type_resolver: F,
) -> TypeId
where
    F: Fn(&str) -> Option<TypeId> + Copy,
{
    let original = arena.get(original_id).clone();

    match original {
        Type::Union(type_ids) => {
            let matching: Vec<TypeId> = type_ids
                .iter()
                .filter(|&&id| {
                    has_discriminant_property_with_arena(arena, id, prop_name, prop_value, type_resolver)
                })
                .copied()
                .collect();

            simplify_filtered_union_ids(arena, matching)
        }
        Type::TypeRef { ref name, .. } => {
            // If original is a TypeRef to a union, resolve and narrow
            if let Some(resolved_id) = type_resolver(name) {
                let resolved = arena.get(resolved_id).clone();
                if matches!(resolved, Type::Union(_)) {
                    return narrow_by_discriminant_with_arena(
                        arena,
                        resolved_id,
                        prop_name,
                        prop_value,
                        type_resolver,
                    );
                }
            }
            // For non-union types, check if it matches
            if has_discriminant_property_with_arena(arena, original_id, prop_name, prop_value, type_resolver) {
                original_id
            } else {
                TypeId::NEVER
            }
        }
        _ => {
            // For non-union types, check if it matches
            if has_discriminant_property_with_arena(arena, original_id, prop_name, prop_value, type_resolver) {
                original_id
            } else {
                TypeId::NEVER
            }
        }
    }
}

/// Exclude union members that have a matching discriminant property.
fn exclude_by_discriminant_with_arena<F>(
    arena: &mut TypeArena,
    original_id: TypeId,
    prop_name: &str,
    prop_value: &str,
    type_resolver: F,
) -> TypeId
where
    F: Fn(&str) -> Option<TypeId> + Copy,
{
    let original = arena.get(original_id).clone();

    match original {
        Type::Union(type_ids) => {
            let remaining: Vec<TypeId> = type_ids
                .iter()
                .filter(|&&id| {
                    !has_discriminant_property_with_arena(arena, id, prop_name, prop_value, type_resolver)
                })
                .copied()
                .collect();

            simplify_filtered_union_ids(arena, remaining)
        }
        Type::TypeRef { ref name, .. } => {
            // If original is a TypeRef to a union, resolve and exclude
            if let Some(resolved_id) = type_resolver(name) {
                let resolved = arena.get(resolved_id).clone();
                if matches!(resolved, Type::Union(_)) {
                    return exclude_by_discriminant_with_arena(
                        arena,
                        resolved_id,
                        prop_name,
                        prop_value,
                        type_resolver,
                    );
                }
            }
            // For non-union types, check if it matches
            if has_discriminant_property_with_arena(arena, original_id, prop_name, prop_value, type_resolver) {
                TypeId::NEVER
            } else {
                original_id
            }
        }
        _ => {
            if has_discriminant_property_with_arena(arena, original_id, prop_name, prop_value, type_resolver) {
                TypeId::NEVER
            } else {
                original_id
            }
        }
    }
}

/// Check if a type has a discriminant property with a specific string literal value.
fn has_discriminant_property_with_arena<F>(
    arena: &TypeArena,
    ty_id: TypeId,
    prop_name: &str,
    expected_value: &str,
    type_resolver: F,
) -> bool
where
    F: Fn(&str) -> Option<TypeId>,
{
    let ty = arena.get(ty_id);

    match ty {
        Type::Object { properties, .. } => properties.iter().any(|p| {
            if p.name != prop_name {
                return false;
            }
            let prop_ty = arena.get(p.ty);
            matches!(prop_ty, Type::StringLiteral(v) if v == expected_value)
        }),
        Type::TypeRef { name, .. } => {
            // Try to resolve the type reference
            if let Some(resolved_id) = type_resolver(name) {
                has_discriminant_property_with_arena(arena, resolved_id, prop_name, expected_value, type_resolver)
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
        ctx.narrow("x".to_string(), TypeId::STRING);

        assert_eq!(ctx.get_narrowed_id("x"), Some(TypeId::STRING));
        assert_eq!(ctx.get_narrowed_id("y"), None);

        ctx.clear();
        assert_eq!(ctx.get_narrowed_id("x"), None);
    }

    #[test]
    fn test_remove_null_from_union() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::STRING, TypeId::NULL]));
        let result = remove_from_union_with_arena(&mut arena, union_id, TypeId::NULL);
        assert_eq!(result, TypeId::STRING);
    }

    #[test]
    fn test_remove_undefined_from_union() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::NUMBER, TypeId::UNDEFINED]));
        let result = remove_from_union_with_arena(&mut arena, union_id, TypeId::UNDEFINED);
        assert_eq!(result, TypeId::NUMBER);
    }

    #[test]
    fn test_remove_multiple_keeps_union() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::STRING, TypeId::NUMBER, TypeId::NULL]));
        let result = remove_from_union_with_arena(&mut arena, union_id, TypeId::NULL);

        let result_ty = arena.get(result).clone();
        match result_ty {
            Type::Union(ids) => {
                assert_eq!(ids.len(), 2);
                assert!(ids.contains(&TypeId::STRING));
                assert!(ids.contains(&TypeId::NUMBER));
            }
            _ => panic!("Expected union type"),
        }
    }

    #[test]
    fn test_apply_typeof_guard() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::STRING, TypeId::NUMBER]));
        let result = apply_guard_with_arena(
            &mut arena,
            union_id,
            &TypeGuard::Typeof("string".to_string()),
            false,
            |_| None,
        );
        assert_eq!(result, TypeId::STRING);
    }

    #[test]
    fn test_apply_not_null_guard() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::STRING, TypeId::NULL]));
        let result = apply_guard_with_arena(&mut arena, union_id, &TypeGuard::NotNull, false, |_| None);
        assert_eq!(result, TypeId::STRING);
    }

    #[test]
    fn test_apply_truthy_guard() {
        let mut arena = TypeArena::new();
        let union_id = arena.intern(Type::Union(vec![TypeId::STRING, TypeId::NULL, TypeId::UNDEFINED]));
        let result = apply_guard_with_arena(&mut arena, union_id, &TypeGuard::Truthy, false, |_| None);
        assert_eq!(result, TypeId::STRING);
    }
}
