//! Type system definitions for the Zelda TypeScript type checker.
//!
//! This module provides:
//! - [`Type`]: The core type enum representing all TypeScript types
//! - [`TypeArena`] and [`TypeId`]: Type interning for efficient storage and comparison
//! - Supporting structures: [`Property`], [`Param`], [`TypeParam`], [`IndexSignature`]
//! - AST to Type resolution functions in the `resolution` submodule
//!
//! # Type Interning
//!
//! For performance-critical code paths (like assignability checking), use the
//! `TypeArena` to intern types and work with lightweight `TypeId` handles:
//!
//! ```ignore
//! let mut arena = TypeArena::new();
//! let string_id = arena.primitives().string;
//! let obj_id = arena.intern(Type::object(vec![Property::new("x", Type::Number)]));
//!
//! // TypeId comparisons are O(1)
//! assert_ne!(string_id, obj_id);
//! ```

mod arena;
pub mod resolution;

pub use arena::{TypeArena, TypeId};

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    // Primitives
    String,
    Number,
    Boolean,
    Null,
    Undefined,
    Void,
    Any,
    Unknown,
    Never,

    // Literals
    StringLiteral(String),
    NumberLiteral(f64),
    BooleanLiteral(bool),

    // Compound types - use TypeId handles instead of Box<Type>
    Array(TypeId),
    Tuple(Vec<TypeId>),
    Union(Vec<TypeId>),
    Intersection(Vec<TypeId>),

    // Structural types
    Object {
        properties: Vec<Property>,
        index_signature: Option<IndexSignature>,
        /// Base interfaces/types this interface extends (stored as TypeIds).
        /// Empty for plain object types. Resolved during type checking.
        extends: Vec<TypeId>,
        /// Type parameters for generic interfaces: interface Box<T> { ... }
        type_params: Vec<TypeParam>,
    },
    Function {
        params: Vec<Param>,
        return_type: TypeId,
        type_params: Vec<TypeParam>,
        /// Assertion predicate for assertion functions like `asserts val is string`
        type_predicate: Option<TypePredicate>,
    },

    /// Class constructor type - stored in value namespace for classes.
    /// Contains constructor signature AND static members.
    ClassConstructor {
        /// Constructor parameters for `new ClassName(args)`
        params: Vec<Param>,
        /// Type parameters for generic classes
        type_params: Vec<TypeParam>,
        /// Static properties and methods accessible via `ClassName.member`
        static_members: Vec<Property>,
    },

    // Named type reference with `type`: Array<string>, Map<K,V>, User, etc.
    TypeRef {
        name: String,
        type_args: Vec<TypeId>,
    },

    // Type parameter in a generic definition: the T in <T extends Foo>
    TypeParameter {
        name: String,
        constraint: Option<TypeId>,
        default: Option<TypeId>,
    },

    /// keyof T - union of property name literals
    ///
    /// `keyof { x: number; y: string }` = `"x" | "y"`
    KeyOf(TypeId),

    /// Indexed access type: T[K]
    ///
    /// `Person["name"]` gets the type of the "name" property
    /// `T[keyof T]` gets a union of all property value types
    IndexedAccess {
        object_type: TypeId,
        index_type: TypeId,
    },

    /// Mapped type: { [K in keyof T]: T[K] }
    ///
    /// Transforms an object type by mapping over its keys.
    /// Used for utility types like Partial, Required, Readonly, Pick, Record.
    MappedType {
        /// The type parameter variable name (e.g., "K" in [K in keyof T])
        type_param: String,
        /// The constraint on the type parameter (e.g., keyof T)
        constraint: TypeId,
        /// The template for each property value (e.g., T[K])
        template: TypeId,
        /// Modifier for readonly: Some(true) = +readonly, Some(false) = -readonly, None = unchanged
        readonly_modifier: Option<bool>,
        /// Modifier for optional: Some(true) = +?, Some(false) = -?, None = unchanged
        optional_modifier: Option<bool>,
    },

    /// Conditional type: T extends U ? X : Y
    ///
    /// Evaluates to true_type if check_type extends extends_type, otherwise false_type.
    /// When check_type is a naked type parameter with a union, the conditional distributes:
    /// `(A | B) extends U ? X : Y` becomes `(A extends U ? X : Y) | (B extends U ? X : Y)`
    ConditionalType {
        /// The type being checked (e.g., T in `T extends U ? X : Y`)
        check_type: TypeId,
        /// The type to compare against (e.g., U in `T extends U ? X : Y`)
        extends_type: TypeId,
        /// The type if the check succeeds (e.g., X in `T extends U ? X : Y`)
        true_type: TypeId,
        /// The type if the check fails (e.g., Y in `T extends U ? X : Y`)
        false_type: TypeId,
    },

    /// Infer type: `infer R` in conditional types
    ///
    /// Used in conditional type extends clauses to infer a type variable.
    /// `T extends (...args: infer P) => any ? P : never` extracts parameter types.
    InferType {
        /// The name of the type variable to infer (e.g., "R" in `infer R`)
        name: String,
        /// Optional constraint on the inferred type (e.g., `infer R extends SomeType`)
        constraint: Option<TypeId>,
    },

    /// Template literal type: `hello${string}world`
    ///
    /// Represents a string type built from literal strings and type placeholders.
    /// Used for typed event names, CSS-in-JS patterns, etc.
    TemplateLiteralType {
        /// The static string parts (one more than types.len())
        /// For `hello${T}world`, this is ["hello", "world"]
        texts: Vec<String>,
        /// The type placeholders between text parts
        /// For `hello${T}world`, this is [T]
        types: Vec<TypeId>,
    },
}

/// We can't derive `Eq` because `f64` doesn't implement it (NaN != NaN violates reflexivity).
/// This is safe for our use case since TypeScript number literals are always concrete values
/// and we never construct NaN literals in the type system.
impl Eq for Type {}

/// We can't derive `Hash` because `f64` doesn't implement it. Different bit patterns
/// (like +0.0 vs -0.0) would need special handling, and NaN has no meaningful hash.
/// We solve this by converting f64 to its raw bits via `to_bits()`, giving us a
/// consistent u64 that we can hash normally.
///
/// TypeId is Copy + Hash, so hashing compound types is fast.
impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Type::String
            | Type::Number
            | Type::Boolean
            | Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Any
            | Type::Unknown
            | Type::Never => {}
            Type::StringLiteral(s) => s.hash(state),
            Type::NumberLiteral(n) => n.to_bits().hash(state),
            Type::BooleanLiteral(b) => b.hash(state),
            Type::Array(elem) => elem.hash(state),
            Type::Tuple(types) => types.hash(state),
            Type::Union(types) => types.hash(state),
            Type::Intersection(types) => types.hash(state),
            Type::Object {
                properties,
                index_signature,
                extends,
                type_params,
            } => {
                properties.hash(state);
                index_signature.hash(state);
                extends.hash(state);
                type_params.hash(state);
            }
            Type::Function {
                params,
                return_type,
                type_params,
                type_predicate,
            } => {
                params.hash(state);
                return_type.hash(state);
                type_params.hash(state);
                type_predicate.hash(state);
            }
            Type::ClassConstructor {
                params,
                type_params,
                static_members,
            } => {
                params.hash(state);
                type_params.hash(state);
                static_members.hash(state);
            }
            Type::TypeRef { name, type_args } => {
                name.hash(state);
                type_args.hash(state);
            }
            Type::TypeParameter {
                name,
                constraint,
                default,
            } => {
                name.hash(state);
                constraint.hash(state);
                default.hash(state);
            }
            Type::KeyOf(inner) => inner.hash(state),
            Type::IndexedAccess {
                object_type,
                index_type,
            } => {
                object_type.hash(state);
                index_type.hash(state);
            }
            Type::MappedType {
                type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
            } => {
                type_param.hash(state);
                constraint.hash(state);
                template.hash(state);
                readonly_modifier.hash(state);
                optional_modifier.hash(state);
            }
            Type::ConditionalType {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                check_type.hash(state);
                extends_type.hash(state);
                true_type.hash(state);
                false_type.hash(state);
            }
            Type::InferType { name, constraint } => {
                name.hash(state);
                constraint.hash(state);
            }
            Type::TemplateLiteralType { texts, types } => {
                texts.hash(state);
                types.hash(state);
            }
        }
    }
}

impl Type {
    /// Returns a numeric discriminant for ordering purposes.
    ///
    /// This replaces the previous `format!("{:?}", discriminant)` hack with
    /// an explicit ordering that's both faster and more predictable.
    fn discriminant_order(&self) -> u8 {
        match self {
            Type::String => 0,
            Type::Number => 1,
            Type::Boolean => 2,
            Type::Null => 3,
            Type::Undefined => 4,
            Type::Void => 5,
            Type::Any => 6,
            Type::Unknown => 7,
            Type::Never => 8,
            Type::StringLiteral(_) => 9,
            Type::NumberLiteral(_) => 10,
            Type::BooleanLiteral(_) => 11,
            Type::Array(_) => 12,
            Type::Tuple(_) => 13,
            Type::Union(_) => 14,
            Type::Intersection(_) => 15,
            Type::Object { .. } => 16,
            Type::Function { .. } => 17,
            Type::ClassConstructor { .. } => 18,
            Type::TypeRef { .. } => 19,
            Type::TypeParameter { .. } => 20,
            Type::KeyOf(_) => 21,
            Type::IndexedAccess { .. } => 22,
            Type::MappedType { .. } => 23,
            Type::ConditionalType { .. } => 24,
            Type::InferType { .. } => 25,
            Type::TemplateLiteralType { .. } => 26,
        }
    }
}

/// We can't derive `Ord` because `f64` only implements `PartialOrd`. NaN is unordered
/// (NaN < x, NaN > x, and NaN == x are all false). We use `total_cmp()` which defines
/// a total ordering: -NaN < -inf < ... < -0 < +0 < ... < +inf < +NaN.
impl PartialOrd for Type {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// See `PartialOrd` above, this uses `total_cmp()` for the `NumberLiteral` variant.
/// TypeId is Ord, so comparison of compound types is straightforward.
impl Ord for Type {
    fn cmp(&self, other: &Self) -> Ordering {
        // First compare by discriminant using explicit ordering
        match self.discriminant_order().cmp(&other.discriminant_order()) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Then compare by content - TypeId is Copy + Ord
        match (self, other) {
            (Type::String, Type::String) => Ordering::Equal,
            (Type::Number, Type::Number) => Ordering::Equal,
            (Type::Boolean, Type::Boolean) => Ordering::Equal,
            (Type::Null, Type::Null) => Ordering::Equal,
            (Type::Undefined, Type::Undefined) => Ordering::Equal,
            (Type::Void, Type::Void) => Ordering::Equal,
            (Type::Any, Type::Any) => Ordering::Equal,
            (Type::Unknown, Type::Unknown) => Ordering::Equal,
            (Type::Never, Type::Never) => Ordering::Equal,
            (Type::StringLiteral(a), Type::StringLiteral(b)) => a.cmp(b),
            (Type::NumberLiteral(a), Type::NumberLiteral(b)) => a.total_cmp(b),
            (Type::BooleanLiteral(a), Type::BooleanLiteral(b)) => a.cmp(b),
            (Type::Array(a), Type::Array(b)) => a.cmp(b),
            (Type::Tuple(a), Type::Tuple(b)) => a.cmp(b),
            (Type::Union(a), Type::Union(b)) => a.cmp(b),
            (Type::Intersection(a), Type::Intersection(b)) => a.cmp(b),
            (
                Type::Object {
                    properties: pa,
                    index_signature: ia,
                    extends: ea,
                    type_params: ta,
                },
                Type::Object {
                    properties: pb,
                    index_signature: ib,
                    extends: eb,
                    type_params: tb,
                },
            ) => pa
                .cmp(pb)
                .then_with(|| ia.cmp(ib))
                .then_with(|| ea.cmp(eb))
                .then_with(|| ta.cmp(tb)),
            (
                Type::Function {
                    params: pa,
                    return_type: ra,
                    type_params: ta,
                    type_predicate: tpa,
                },
                Type::Function {
                    params: pb,
                    return_type: rb,
                    type_params: tb,
                    type_predicate: tpb,
                },
            ) => pa
                .cmp(pb)
                .then_with(|| ra.cmp(rb))
                .then_with(|| ta.cmp(tb))
                .then_with(|| tpa.cmp(tpb)),
            (
                Type::TypeRef {
                    name: na,
                    type_args: aa,
                },
                Type::TypeRef {
                    name: nb,
                    type_args: ab,
                },
            ) => na.cmp(nb).then_with(|| aa.cmp(ab)),
            (
                Type::TypeParameter {
                    name: na,
                    constraint: ca,
                    default: da,
                },
                Type::TypeParameter {
                    name: nb,
                    constraint: cb,
                    default: db,
                },
            ) => na.cmp(nb).then_with(|| ca.cmp(cb)).then_with(|| da.cmp(db)),
            (
                Type::ClassConstructor {
                    params: pa,
                    type_params: ta,
                    static_members: sa,
                },
                Type::ClassConstructor {
                    params: pb,
                    type_params: tb,
                    static_members: sb,
                },
            ) => pa.cmp(pb).then_with(|| ta.cmp(tb)).then_with(|| sa.cmp(sb)),
            (Type::KeyOf(a), Type::KeyOf(b)) => a.cmp(b),
            (
                Type::IndexedAccess {
                    object_type: oa,
                    index_type: ia,
                },
                Type::IndexedAccess {
                    object_type: ob,
                    index_type: ib,
                },
            ) => oa.cmp(ob).then_with(|| ia.cmp(ib)),
            (
                Type::MappedType {
                    type_param: pa,
                    constraint: ca,
                    template: ta,
                    readonly_modifier: ra,
                    optional_modifier: oa,
                },
                Type::MappedType {
                    type_param: pb,
                    constraint: cb,
                    template: tb,
                    readonly_modifier: rb,
                    optional_modifier: ob,
                },
            ) => pa
                .cmp(pb)
                .then_with(|| ca.cmp(cb))
                .then_with(|| ta.cmp(tb))
                .then_with(|| ra.cmp(rb))
                .then_with(|| oa.cmp(ob)),
            (
                Type::ConditionalType {
                    check_type: ca,
                    extends_type: ea,
                    true_type: ta,
                    false_type: fa,
                },
                Type::ConditionalType {
                    check_type: cb,
                    extends_type: eb,
                    true_type: tb,
                    false_type: fb,
                },
            ) => ca
                .cmp(cb)
                .then_with(|| ea.cmp(eb))
                .then_with(|| ta.cmp(tb))
                .then_with(|| fa.cmp(fb)),
            (
                Type::InferType {
                    name: na,
                    constraint: ca,
                },
                Type::InferType {
                    name: nb,
                    constraint: cb,
                },
            ) => na.cmp(nb).then_with(|| ca.cmp(cb)),
            (
                Type::TemplateLiteralType {
                    texts: ta,
                    types: tya,
                },
                Type::TemplateLiteralType {
                    texts: tb,
                    types: tyb,
                },
            ) => ta.cmp(tb).then_with(|| tya.cmp(tyb)),
            _ => Ordering::Equal, // Same discriminant, shouldn't happen
        }
    }
}

impl Type {
    /// Create an object type with the given properties.
    pub fn object(properties: Vec<Property>) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        }
    }

    /// Create an object type with inheritance (for interfaces with extends).
    pub fn object_with_extends(properties: Vec<Property>, extends: Vec<TypeId>) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends,
            type_params: vec![],
        }
    }

    /// Create a generic object type with type parameters.
    pub fn generic_object(
        type_params: Vec<TypeParam>,
        properties: Vec<Property>,
        extends: Vec<TypeId>,
    ) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends,
            type_params,
        }
    }

    /// Create a function type.
    pub fn function(params: Vec<Param>, return_type: TypeId) -> Self {
        Self::Function {
            params,
            return_type,
            type_params: Vec::new(),
            type_predicate: None,
        }
    }

    /// Create a generic function type.
    pub fn generic_function(
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        return_type: TypeId,
    ) -> Self {
        Self::Function {
            params,
            return_type,
            type_params,
            type_predicate: None,
        }
    }

    /// Create an array type.
    pub fn array(element_type: TypeId) -> Self {
        Self::Array(element_type)
    }

    /// Create a union type.
    pub fn union(types: Vec<TypeId>) -> Self {
        Self::Union(types)
    }

    /// Create an intersection type.
    pub fn intersection(types: Vec<TypeId>) -> Self {
        Self::Intersection(types)
    }

    /// Create a type reference.
    pub fn type_ref(name: impl Into<String>, type_args: Vec<TypeId>) -> Self {
        Self::TypeRef {
            name: name.into(),
            type_args,
        }
    }

    /// Check if this is a primitive type.
    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Type::String | Type::Number | Type::Boolean | Type::Null | Type::Undefined | Type::Void
        )
    }

    /// Simplify a type by applying normalization rules.
    ///
    /// Note: This simplified version works at the TypeId level.
    /// For full simplification with nested type resolution, use TypeArena::simplify().
    ///
    /// - Flatten nested unions/intersections
    /// - Deduplicate elements
    /// - Filter out never from unions
    /// - Single-element union/intersection -> unwrap
    /// - Empty union -> `never`
    /// - Empty intersection -> `unknown`
    pub fn simplify(self, arena: &TypeArena) -> Self {
        match self {
            Type::Union(type_ids) => {
                // Flatten nested unions and collect all elements
                let mut flattened: Vec<TypeId> = Vec::new();
                for id in type_ids {
                    match arena.get(id) {
                        Type::Union(inner_ids) => {
                            // Flatten: add inner union's elements
                            flattened.extend(inner_ids.iter().copied());
                        }
                        Type::Never => {
                            // Filter out never types
                        }
                        _ => {
                            flattened.push(id);
                        }
                    }
                }

                // Deduplicate while preserving order
                let mut seen = std::collections::HashSet::new();
                let deduped: Vec<TypeId> = flattened
                    .into_iter()
                    .filter(|id| seen.insert(*id))
                    .collect();

                match deduped.len() {
                    0 => Type::Never,
                    1 => arena.get(deduped[0]).clone(),
                    _ => Type::Union(deduped),
                }
            }
            Type::Intersection(type_ids) => {
                // Flatten nested intersections
                let mut flattened: Vec<TypeId> = Vec::new();
                for id in type_ids {
                    match arena.get(id) {
                        Type::Intersection(inner_ids) => {
                            flattened.extend(inner_ids.iter().copied());
                        }
                        Type::Never => {
                            // If any member is never, the whole intersection is never
                            return Type::Never;
                        }
                        _ => {
                            flattened.push(id);
                        }
                    }
                }

                // Deduplicate while preserving order
                let mut seen = std::collections::HashSet::new();
                let deduped: Vec<TypeId> = flattened
                    .into_iter()
                    .filter(|id| seen.insert(*id))
                    .collect();

                match deduped.len() {
                    0 => Type::Unknown,
                    1 => arena.get(deduped[0]).clone(),
                    _ => Type::Intersection(deduped),
                }
            }
            other => other,
        }
    }
}

/// Display wrapper for Type that has access to the arena.
/// Use this for displaying types that contain TypeId references.
pub struct TypeDisplay<'a> {
    pub ty: &'a Type,
    pub arena: &'a TypeArena,
}

impl<'a> fmt::Display for TypeDisplay<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.ty.fmt_with_arena(f, self.arena)
    }
}

impl Type {
    /// Create a display wrapper that can show nested types.
    pub fn display<'a>(&'a self, arena: &'a TypeArena) -> TypeDisplay<'a> {
        TypeDisplay { ty: self, arena }
    }

    /// Format this type with access to the arena for nested types.
    pub fn fmt_with_arena(&self, f: &mut fmt::Formatter<'_>, arena: &TypeArena) -> fmt::Result {
        match self {
            Type::String => write!(f, "string"),
            Type::Number => write!(f, "number"),
            Type::Boolean => write!(f, "boolean"),
            Type::Null => write!(f, "null"),
            Type::Undefined => write!(f, "undefined"),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Unknown => write!(f, "unknown"),
            Type::Never => write!(f, "never"),

            Type::StringLiteral(s) => write!(f, "\"{}\"", s),
            Type::NumberLiteral(n) => write!(f, "{}", n),
            Type::BooleanLiteral(b) => write!(f, "{}", b),

            Type::Array(elem_id) => {
                write!(f, "{}[]", arena.get(*elem_id).display(arena))
            }
            Type::Tuple(type_ids) => {
                write!(f, "[")?;
                for (i, &id) in type_ids.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arena.get(id).display(arena))?;
                }
                write!(f, "]")
            }
            Type::Union(type_ids) => {
                for (i, &id) in type_ids.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    let ty = arena.get(id);
                    // Wrap function types: ((x: number) => void) | string
                    if matches!(ty, Type::Function { .. }) {
                        write!(f, "({})", ty.display(arena))?;
                    } else {
                        write!(f, "{}", ty.display(arena))?;
                    }
                }
                Ok(())
            }
            Type::Intersection(type_ids) => {
                for (i, &id) in type_ids.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    let ty = arena.get(id);
                    // Wrap unions and functions with parentheses
                    if matches!(ty, Type::Union(_) | Type::Function { .. }) {
                        write!(f, "({})", ty.display(arena))?;
                    } else {
                        write!(f, "{}", ty.display(arena))?;
                    }
                }
                Ok(())
            }

            Type::Object {
                properties,
                index_signature,
                extends: _, // Not shown, resolved at type-check time
                type_params,
            } => {
                // Show type parameters if present
                if !type_params.is_empty() {
                    write!(f, "<")?;
                    for (i, tp) in type_params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", tp.name)?;
                        if let Some(constraint_id) = tp.constraint {
                            write!(f, " extends {}", arena.get(constraint_id).display(arena))?;
                        }
                        if let Some(default_id) = tp.default {
                            write!(f, " = {}", arena.get(default_id).display(arena))?;
                        }
                    }
                    write!(f, ">")?;
                }
                write!(f, "{{ ")?;
                let mut first = true;
                for prop in properties {
                    if !first {
                        write!(f, "; ")?;
                    }
                    first = false;
                    if prop.readonly {
                        write!(f, "readonly ")?;
                    }
                    write!(f, "{}", prop.name)?;
                    if prop.optional {
                        write!(f, "?")?;
                    }
                    write!(f, ": {}", arena.get(prop.ty).display(arena))?;
                }
                if let Some(idx) = index_signature {
                    if !first {
                        write!(f, "; ")?;
                    }
                    write!(f, "[key: {}]: {}",
                        arena.get(idx.key_type).display(arena),
                        arena.get(idx.value_type).display(arena))?;
                }
                write!(f, " }}")
            }

            Type::Function {
                params,
                return_type,
                type_params,
                type_predicate: _,
            } => {
                if !type_params.is_empty() {
                    write!(f, "<")?;
                    for (i, tp) in type_params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", tp.name)?;
                        if let Some(constraint_id) = tp.constraint {
                            write!(f, " extends {}", arena.get(constraint_id).display(arena))?;
                        }
                        if let Some(default_id) = tp.default {
                            write!(f, " = {}", arena.get(default_id).display(arena))?;
                        }
                    }
                    write!(f, ">")?;
                }
                write!(f, "(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if param.rest {
                        write!(f, "...")?;
                    }
                    write!(f, "{}", param.name)?;
                    if param.optional {
                        write!(f, "?")?;
                    }
                    write!(f, ": {}", arena.get(param.ty).display(arena))?;
                }
                write!(f, ") => {}", arena.get(*return_type).display(arena))
            }

            Type::ClassConstructor {
                params,
                type_params,
                static_members,
            } => {
                write!(f, "typeof class")?;
                if !type_params.is_empty() {
                    write!(f, "<")?;
                    for (i, tp) in type_params.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", tp.name)?;
                    }
                    write!(f, ">")?;
                }
                if !static_members.is_empty() {
                    write!(f, " {{ ")?;
                    for (i, prop) in static_members.iter().enumerate() {
                        if i > 0 {
                            write!(f, "; ")?;
                        }
                        write!(f, "static {}: {}", prop.name, arena.get(prop.ty).display(arena))?;
                    }
                    write!(f, " }}")?;
                }
                write!(
                    f,
                    " (constructor: ({}) => instance)",
                    params
                        .iter()
                        .map(|p| format!("{}: {}", p.name, arena.get(p.ty).display(arena)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }

            Type::TypeRef { name, type_args } => {
                write!(f, "{}", name)?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    for (i, &arg_id) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arena.get(arg_id).display(arena))?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }

            Type::TypeParameter {
                name,
                constraint,
                default,
            } => {
                write!(f, "{}", name)?;
                if let Some(constraint_id) = constraint {
                    write!(f, " extends {}", arena.get(*constraint_id).display(arena))?;
                }
                if let Some(default_id) = default {
                    write!(f, " = {}", arena.get(*default_id).display(arena))?;
                }
                Ok(())
            }

            Type::KeyOf(inner_id) => write!(f, "keyof {}", arena.get(*inner_id).display(arena)),

            Type::IndexedAccess {
                object_type,
                index_type,
            } => {
                write!(f, "{}[{}]",
                    arena.get(*object_type).display(arena),
                    arena.get(*index_type).display(arena))
            }

            Type::MappedType {
                type_param,
                constraint,
                template,
                readonly_modifier,
                optional_modifier,
            } => {
                write!(f, "{{ ")?;
                // Readonly modifier
                match readonly_modifier {
                    Some(true) => write!(f, "+readonly ")?,
                    Some(false) => write!(f, "-readonly ")?,
                    None => {}
                }
                // Key mapping
                write!(f, "[{} in {}]", type_param, arena.get(*constraint).display(arena))?;
                // Optional modifier
                match optional_modifier {
                    Some(true) => write!(f, "?")?,
                    Some(false) => write!(f, "-?")?,
                    None => {}
                }
                write!(f, ": {}; }}", arena.get(*template).display(arena))
            }

            Type::ConditionalType {
                check_type,
                extends_type,
                true_type,
                false_type,
            } => {
                write!(
                    f,
                    "{} extends {} ? {} : {}",
                    arena.get(*check_type).display(arena),
                    arena.get(*extends_type).display(arena),
                    arena.get(*true_type).display(arena),
                    arena.get(*false_type).display(arena)
                )
            }

            Type::InferType { name, constraint } => {
                write!(f, "infer {}", name)?;
                if let Some(constraint_id) = constraint {
                    write!(f, " extends {}", arena.get(*constraint_id).display(arena))?;
                }
                Ok(())
            }

            Type::TemplateLiteralType { texts, types } => {
                write!(f, "`")?;
                for (i, text) in texts.iter().enumerate() {
                    write!(f, "{}", text)?;
                    if let Some(&type_id) = types.get(i) {
                        write!(f, "${{{}}}", arena.get(type_id).display(arena))?;
                    }
                }
                write!(f, "`")
            }
        }
    }
}

/// Simple Display implementation for primitive types.
/// For compound types that need arena access, use `ty.display(arena)`.
impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::String => write!(f, "string"),
            Type::Number => write!(f, "number"),
            Type::Boolean => write!(f, "boolean"),
            Type::Null => write!(f, "null"),
            Type::Undefined => write!(f, "undefined"),
            Type::Void => write!(f, "void"),
            Type::Any => write!(f, "any"),
            Type::Unknown => write!(f, "unknown"),
            Type::Never => write!(f, "never"),
            Type::StringLiteral(s) => write!(f, "\"{}\"", s),
            Type::NumberLiteral(n) => write!(f, "{}", n),
            Type::BooleanLiteral(b) => write!(f, "{}", b),
            // For compound types, show a simplified representation without nested details
            Type::Array(_) => write!(f, "<array>"),
            Type::Tuple(_) => write!(f, "<tuple>"),
            Type::Union(_) => write!(f, "<union>"),
            Type::Intersection(_) => write!(f, "<intersection>"),
            Type::Object { .. } => write!(f, "<object>"),
            Type::Function { .. } => write!(f, "<function>"),
            Type::ClassConstructor { .. } => write!(f, "<class>"),
            Type::TypeRef { name, .. } => write!(f, "{}", name),
            Type::TypeParameter { name, .. } => write!(f, "{}", name),
            Type::KeyOf(_) => write!(f, "keyof <type>"),
            Type::IndexedAccess { .. } => write!(f, "<indexed-access>"),
            Type::MappedType { .. } => write!(f, "<mapped-type>"),
            Type::ConditionalType { .. } => write!(f, "<conditional>"),
            Type::InferType { name, .. } => write!(f, "infer {}", name),
            Type::TemplateLiteralType { .. } => write!(f, "<template-literal>"),
        }
    }
}

/// Property in an object type: { name: string, age?: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Property {
    pub name: String,
    pub ty: TypeId,
    pub optional: bool,
    pub readonly: bool,
}

impl Property {
    pub fn new(name: impl Into<String>, ty: TypeId) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            readonly: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }
}

/// Parameter in a function type: (x: number, ...rest: string[]) => void
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Param {
    pub name: String,
    pub ty: TypeId,
    pub optional: bool,
    pub rest: bool,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: TypeId) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            rest: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn rest(mut self) -> Self {
        self.rest = true;
        self
    }
}

/// Type parameter in a generic: <T extends Constraint = Default>
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeParam {
    pub name: String,               // T
    pub constraint: Option<TypeId>, // extends SomeType
    pub default: Option<TypeId>,    // = DefaultType
}

impl TypeParam {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            constraint: None,
            default: None,
        }
    }

    pub fn with_constraint(mut self, constraint: TypeId) -> Self {
        self.constraint = Some(constraint);
        self
    }

    pub fn with_default(mut self, default: TypeId) -> Self {
        self.default = Some(default);
        self
    }
}

/// Index signature: { [key: string]: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexSignature {
    pub key_type: TypeId,
    pub value_type: TypeId,
}

/// Type predicate for assertion functions: `asserts val is string`
///
/// Used for type narrowing after calling assertion functions.
/// - `parameter_name`: The parameter being asserted (e.g., "val")
/// - `asserts`: Whether this is an `asserts` predicate (vs just `is`)
/// - `type_annotation`: The type being asserted (e.g., `string`), or None for just `asserts val`
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypePredicate {
    /// The parameter name being asserted (e.g., "val" in `asserts val is string`)
    pub parameter_name: String,
    /// Whether this is an assertion predicate (true) or a regular type guard (false)
    pub asserts: bool,
    /// The type being asserted, if any (None for just `asserts val`)
    pub type_annotation: Option<TypeId>,
}

impl TypePredicate {
    pub fn new(
        parameter_name: impl Into<String>,
        asserts: bool,
        type_annotation: Option<TypeId>,
    ) -> Self {
        Self {
            parameter_name: parameter_name.into(),
            asserts,
            type_annotation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::arena::TypeArena;

    #[test]
    fn test_primitive_display() {
        assert_eq!(Type::String.to_string(), "string");
        assert_eq!(Type::Number.to_string(), "number");
        assert_eq!(Type::Boolean.to_string(), "boolean");
        assert_eq!(Type::Null.to_string(), "null");
        assert_eq!(Type::Undefined.to_string(), "undefined");
        assert_eq!(Type::Void.to_string(), "void");
        assert_eq!(Type::Any.to_string(), "any");
        assert_eq!(Type::Unknown.to_string(), "unknown");
        assert_eq!(Type::Never.to_string(), "never");
    }

    #[test]
    fn test_literal_display() {
        assert_eq!(Type::StringLiteral("hello".into()).to_string(), "\"hello\"");
        assert_eq!(Type::NumberLiteral(42.0).to_string(), "42");
        assert_eq!(Type::BooleanLiteral(true).to_string(), "true");
    }

    #[test]
    fn test_array_display() {
        let mut arena = TypeArena::new();
        let arr = Type::array(TypeId::STRING);
        let arr_id = arena.intern(arr.clone());
        assert_eq!(arena.get(arr_id).display(&arena).to_string(), "string[]");

        let arr2 = Type::array(TypeId::NUMBER);
        let arr2_id = arena.intern(arr2.clone());
        assert_eq!(arena.get(arr2_id).display(&arena).to_string(), "number[]");
    }

    #[test]
    fn test_tuple_display() {
        let mut arena = TypeArena::new();
        let tuple = Type::Tuple(vec![TypeId::STRING, TypeId::NUMBER]);
        let tuple_id = arena.intern(tuple.clone());
        assert_eq!(arena.get(tuple_id).display(&arena).to_string(), "[string, number]");
    }

    #[test]
    fn test_union_display() {
        let mut arena = TypeArena::new();
        let union = Type::union(vec![TypeId::STRING, TypeId::NUMBER]);
        let union_id = arena.intern(union.clone());
        assert_eq!(arena.get(union_id).display(&arena).to_string(), "string | number");
    }

    #[test]
    fn test_intersection_display() {
        let mut arena = TypeArena::new();
        let obj_a = Type::object(vec![Property::new("a", TypeId::STRING)]);
        let obj_b = Type::object(vec![Property::new("b", TypeId::NUMBER)]);
        let a_id = arena.intern(obj_a);
        let b_id = arena.intern(obj_b);
        let intersection = Type::intersection(vec![a_id, b_id]);
        let int_id = arena.intern(intersection);
        assert_eq!(
            arena.get(int_id).display(&arena).to_string(),
            "{ a: string } & { b: number }"
        );
    }

    #[test]
    fn test_object_display() {
        let mut arena = TypeArena::new();
        let obj = Type::object(vec![
            Property::new("name", TypeId::STRING),
            Property::new("age", TypeId::NUMBER).optional(),
        ]);
        let obj_id = arena.intern(obj);
        assert_eq!(arena.get(obj_id).display(&arena).to_string(), "{ name: string; age?: number }");

        let readonly_obj = Type::object(vec![Property::new("id", TypeId::NUMBER).readonly()]);
        let readonly_id = arena.intern(readonly_obj);
        assert_eq!(arena.get(readonly_id).display(&arena).to_string(), "{ readonly id: number }");
    }

    #[test]
    fn test_function_display() {
        let mut arena = TypeArena::new();
        let func = Type::function(
            vec![Param::new("x", TypeId::NUMBER), Param::new("y", TypeId::NUMBER)],
            TypeId::NUMBER,
        );
        let func_id = arena.intern(func);
        assert_eq!(arena.get(func_id).display(&arena).to_string(), "(x: number, y: number) => number");

        let func_optional = Type::function(
            vec![
                Param::new("x", TypeId::NUMBER),
                Param::new("y", TypeId::NUMBER).optional(),
            ],
            TypeId::NUMBER,
        );
        let func_opt_id = arena.intern(func_optional);
        assert_eq!(
            arena.get(func_opt_id).display(&arena).to_string(),
            "(x: number, y?: number) => number"
        );

        let arr_num = arena.intern(Type::array(TypeId::NUMBER));
        let func_rest = Type::function(
            vec![Param::new("args", arr_num).rest()],
            TypeId::NUMBER,
        );
        let func_rest_id = arena.intern(func_rest);
        assert_eq!(arena.get(func_rest_id).display(&arena).to_string(), "(...args: number[]) => number");
    }

    #[test]
    fn test_generic_function_display() {
        let mut arena = TypeArena::new();
        let t_ref = arena.intern(Type::type_ref("T", vec![]));
        let func = Type::generic_function(
            vec![TypeParam::new("T")],
            vec![Param::new("x", t_ref)],
            t_ref,
        );
        let func_id = arena.intern(func);
        assert_eq!(arena.get(func_id).display(&arena).to_string(), "<T>(x: T) => T");

        let func_constrained = Type::generic_function(
            vec![TypeParam::new("T").with_constraint(TypeId::STRING)],
            vec![Param::new("x", t_ref)],
            t_ref,
        );
        let func_constr_id = arena.intern(func_constrained);
        assert_eq!(
            arena.get(func_constr_id).display(&arena).to_string(),
            "<T extends string>(x: T) => T"
        );
    }

    #[test]
    fn test_type_ref_display() {
        let mut arena = TypeArena::new();
        let arr_str = Type::type_ref("Array", vec![TypeId::STRING]);
        let arr_id = arena.intern(arr_str);
        assert_eq!(arena.get(arr_id).display(&arena).to_string(), "Array<string>");

        let map_type = Type::type_ref("Map", vec![TypeId::STRING, TypeId::NUMBER]);
        let map_id = arena.intern(map_type);
        assert_eq!(arena.get(map_id).display(&arena).to_string(), "Map<string, number>");

        assert_eq!(Type::type_ref("User", vec![]).to_string(), "User");
    }

    #[test]
    fn test_is_primitive() {
        assert!(Type::String.is_primitive());
        assert!(Type::Number.is_primitive());
        assert!(Type::Boolean.is_primitive());
        assert!(!Type::Any.is_primitive());
        assert!(!Type::array(TypeId::STRING).is_primitive());
    }

    #[test]
    fn test_object_with_index_signature() {
        let mut arena = TypeArena::new();
        let obj = Type::Object {
            properties: vec![],
            index_signature: Some(IndexSignature {
                key_type: TypeId::STRING,
                value_type: TypeId::NUMBER,
            }),
            extends: vec![],
            type_params: vec![],
        };
        let obj_id = arena.intern(obj);
        assert_eq!(arena.get(obj_id).display(&arena).to_string(), "{ [key: string]: number }");
    }

    #[test]
    fn test_simplify_union_with_never() {
        let arena = TypeArena::new();
        let union = Type::Union(vec![TypeId::STRING, TypeId::NEVER]);
        assert_eq!(union.simplify(&arena), Type::String);
    }

    #[test]
    fn test_simplify_union_all_never() {
        let arena = TypeArena::new();
        let union = Type::Union(vec![TypeId::NEVER, TypeId::NEVER]);
        assert_eq!(union.simplify(&arena), Type::Never);
    }

    #[test]
    fn test_simplify_union_single_element() {
        let arena = TypeArena::new();
        let union = Type::Union(vec![TypeId::STRING]);
        assert_eq!(union.simplify(&arena), Type::String);
    }

    #[test]
    fn test_simplify_intersection_with_never() {
        let arena = TypeArena::new();
        let intersection = Type::Intersection(vec![TypeId::STRING, TypeId::NEVER]);
        assert_eq!(intersection.simplify(&arena), Type::Never);
    }

    #[test]
    fn test_simplify_intersection_single_element() {
        let arena = TypeArena::new();
        let intersection = Type::Intersection(vec![TypeId::STRING]);
        assert_eq!(intersection.simplify(&arena), Type::String);
    }

    #[test]
    fn test_simplify_nested_union() {
        let mut arena = TypeArena::new();
        // Create inner union first
        let inner_union = arena.intern(Type::Union(vec![TypeId::NUMBER, TypeId::BOOLEAN]));
        // Create outer union with the inner union id
        let nested = Type::Union(vec![TypeId::STRING, inner_union]);
        let simplified = nested.simplify(&arena);
        if let Type::Union(type_ids) = simplified {
            assert_eq!(type_ids.len(), 3);
        } else {
            panic!("Expected Union");
        }
    }

    #[test]
    fn test_simplify_deduplicates() {
        let arena = TypeArena::new();
        let union = Type::Union(vec![TypeId::STRING, TypeId::STRING, TypeId::NUMBER]);
        let simplified = union.simplify(&arena);
        if let Type::Union(type_ids) = simplified {
            assert_eq!(type_ids.len(), 2);
        } else {
            panic!("Expected Union");
        }
    }

    #[test]
    fn test_discriminant_order_is_consistent() {
        // Verify discriminant ordering matches the enum order
        assert!(Type::String.discriminant_order() < Type::Number.discriminant_order());
        assert!(Type::Number.discriminant_order() < Type::Boolean.discriminant_order());
        assert!(
            Type::Never.discriminant_order() < Type::StringLiteral("".into()).discriminant_order()
        );
        assert!(
            Type::BooleanLiteral(true).discriminant_order()
                < Type::Array(TypeId::STRING).discriminant_order()
        );
    }

    #[test]
    fn test_type_ordering_works() {
        let mut types = vec![Type::Number, Type::String, Type::Boolean];
        types.sort();
        assert_eq!(types, vec![Type::String, Type::Number, Type::Boolean]);
    }
}
