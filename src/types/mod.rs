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

pub use arena::{PrimitiveTypes, TypeArena, TypeId};

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

    // Compound types
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Union(Vec<Type>),
    Intersection(Vec<Type>),

    // Structural types
    Object {
        properties: Vec<Property>,
        index_signature: Option<IndexSignature>,
        /// Base interfaces/types this interface extends (stored as TypeRefs).
        /// Empty for plain object types. Resolved during type checking.
        extends: Vec<Type>,
        /// Type parameters for generic interfaces: interface Box<T> { ... }
        type_params: Vec<TypeParam>,
    },
    Function {
        params: Vec<Param>,
        return_type: Box<Type>,
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
        type_args: Vec<Type>,
    },

    // Type parameter in a generic definition: the T in <T extends Foo>
    TypeParameter {
        name: String,
        constraint: Option<Box<Type>>,
        default: Option<Box<Type>>,
    },

    /// keyof T - union of property name literals
    ///
    /// `keyof { x: number; y: string }` = `"x" | "y"`
    KeyOf(Box<Type>),

    /// Indexed access type: T[K]
    ///
    /// `Person["name"]` gets the type of the "name" property
    /// `T[keyof T]` gets a union of all property value types
    IndexedAccess {
        object_type: Box<Type>,
        index_type: Box<Type>,
    },

    /// Mapped type: { [K in keyof T]: T[K] }
    ///
    /// Transforms an object type by mapping over its keys.
    /// Used for utility types like Partial, Required, Readonly, Pick, Record.
    MappedType {
        /// The type parameter variable name (e.g., "K" in [K in keyof T])
        type_param: String,
        /// The constraint on the type parameter (e.g., keyof T)
        constraint: Box<Type>,
        /// The template for each property value (e.g., T[K])
        template: Box<Type>,
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
        check_type: Box<Type>,
        /// The type to compare against (e.g., U in `T extends U ? X : Y`)
        extends_type: Box<Type>,
        /// The type if the check succeeds (e.g., X in `T extends U ? X : Y`)
        true_type: Box<Type>,
        /// The type if the check fails (e.g., Y in `T extends U ? X : Y`)
        false_type: Box<Type>,
    },

    /// Infer type: `infer R` in conditional types
    ///
    /// Used in conditional type extends clauses to infer a type variable.
    /// `T extends (...args: infer P) => any ? P : never` extracts parameter types.
    InferType {
        /// The name of the type variable to infer (e.g., "R" in `infer R`)
        name: String,
        /// Optional constraint on the inferred type (e.g., `infer R extends SomeType`)
        constraint: Option<Box<Type>>,
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
        types: Vec<Type>,
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
            Type::IndexedAccess { object_type, index_type } => {
                object_type.hash(state);
                index_type.hash(state);
            }
            Type::MappedType { type_param, constraint, template, readonly_modifier, optional_modifier } => {
                type_param.hash(state);
                constraint.hash(state);
                template.hash(state);
                readonly_modifier.hash(state);
                optional_modifier.hash(state);
            }
            Type::ConditionalType { check_type, extends_type, true_type, false_type } => {
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
impl Ord for Type {
    fn cmp(&self, other: &Self) -> Ordering {
        // First compare by discriminant using explicit ordering
        match self.discriminant_order().cmp(&other.discriminant_order()) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Then compare by content
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
            ) => na
                .cmp(nb)
                .then_with(|| ca.cmp(cb))
                .then_with(|| da.cmp(db)),
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
            ) => pa
                .cmp(pb)
                .then_with(|| ta.cmp(tb))
                .then_with(|| sa.cmp(sb)),
            (Type::KeyOf(a), Type::KeyOf(b)) => a.cmp(b),
            (
                Type::IndexedAccess { object_type: oa, index_type: ia },
                Type::IndexedAccess { object_type: ob, index_type: ib },
            ) => oa.cmp(ob).then_with(|| ia.cmp(ib)),
            (
                Type::MappedType { type_param: pa, constraint: ca, template: ta, readonly_modifier: ra, optional_modifier: oa },
                Type::MappedType { type_param: pb, constraint: cb, template: tb, readonly_modifier: rb, optional_modifier: ob },
            ) => pa.cmp(pb)
                .then_with(|| ca.cmp(cb))
                .then_with(|| ta.cmp(tb))
                .then_with(|| ra.cmp(rb))
                .then_with(|| oa.cmp(ob)),
            (
                Type::ConditionalType { check_type: ca, extends_type: ea, true_type: ta, false_type: fa },
                Type::ConditionalType { check_type: cb, extends_type: eb, true_type: tb, false_type: fb },
            ) => ca.cmp(cb)
                .then_with(|| ea.cmp(eb))
                .then_with(|| ta.cmp(tb))
                .then_with(|| fa.cmp(fb)),
            (
                Type::InferType { name: na, constraint: ca },
                Type::InferType { name: nb, constraint: cb },
            ) => na.cmp(nb).then_with(|| ca.cmp(cb)),
            (
                Type::TemplateLiteralType { texts: ta, types: tya },
                Type::TemplateLiteralType { texts: tb, types: tyb },
            ) => ta.cmp(tb).then_with(|| tya.cmp(tyb)),
            _ => Ordering::Equal, // Same discriminant, shouldn't happen
        }
    }
}

impl Type {
    pub fn object(properties: Vec<Property>) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        }
    }

    /// Create an object type with inheritance (for interfaces with extends).
    pub fn object_with_extends(properties: Vec<Property>, extends: Vec<Type>) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends,
            type_params: vec![],
        }
    }

    pub fn generic_object(
        type_params: Vec<TypeParam>,
        properties: Vec<Property>,
        extends: Vec<Type>,
    ) -> Self {
        Self::Object {
            properties,
            index_signature: None,
            extends,
            type_params,
        }
    }

    pub fn function(params: Vec<Param>, return_type: Type) -> Self {
        Self::Function {
            params,
            return_type: Box::new(return_type),
            type_params: Vec::new(),
            type_predicate: None,
        }
    }

    pub fn generic_function(
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        return_type: Type,
    ) -> Self {
        Self::Function {
            params,
            return_type: Box::new(return_type),
            type_params,
            type_predicate: None,
        }
    }

    pub fn array(element_type: Type) -> Self {
        Self::Array(Box::new(element_type))
    }

    pub fn union(types: Vec<Type>) -> Self {
        Self::Union(types)
    }

    pub fn intersection(types: Vec<Type>) -> Self {
        Self::Intersection(types)
    }

    pub fn type_ref(name: impl Into<String>, type_args: Vec<Type>) -> Self {
        Self::TypeRef {
            name: name.into(),
            type_args,
        }
    }

    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            Type::String
                | Type::Number
                | Type::Boolean
                | Type::Null
                | Type::Undefined
                | Type::Void
        )
    }

    /// Simplify a type by applying normalization rules.
    ///
    /// - `T | never` -> `T` (never is identity for union)
    /// - `T & never` -> `never` (never absorbs intersection)
    /// - Single-element union/intersection -> unwrap
    /// - Empty union -> `never`
    /// - Flatten nested unions/intersections
    pub fn simplify(self) -> Self {
        match self {
            Type::Union(types) => {
                // Flatten nested unions and filter out never
                let mut simplified: Vec<Type> = types
                    .into_iter()
                    .flat_map(|t| {
                        let t = t.simplify();
                        if let Type::Union(inner) = t {
                            inner
                        } else {
                            vec![t]
                        }
                    })
                    .filter(|t| !matches!(t, Type::Never))
                    .collect();

                // Deduplicate
                simplified.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                simplified.dedup();

                match simplified.len() {
                    0 => Type::Never,
                    1 => simplified.into_iter().next().unwrap(),
                    _ => Type::Union(simplified),
                }
            }
            Type::Intersection(types) => {
                // If any member is never, the whole intersection is never
                let simplified: Vec<Type> = types.into_iter().map(|t| t.simplify()).collect();

                if simplified.iter().any(|t| matches!(t, Type::Never)) {
                    return Type::Never;
                }

                // Flatten nested intersections
                let mut flattened: Vec<Type> = simplified
                    .into_iter()
                    .flat_map(|t| {
                        if let Type::Intersection(inner) = t {
                            inner
                        } else {
                            vec![t]
                        }
                    })
                    .collect();

                // Deduplicate
                flattened.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                flattened.dedup();

                match flattened.len() {
                    0 => Type::Unknown, // Empty intersection is unknown (top type)
                    1 => flattened.into_iter().next().unwrap(),
                    _ => Type::Intersection(flattened),
                }
            }
            other => other,
        }
    }
}

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

            Type::Array(elem) => write!(f, "{}[]", elem),
            Type::Tuple(types) => {
                write!(f, "[")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, "]")
            }
            Type::Union(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    // Wrap function types: ((x: number) => void) | string
                    if matches!(ty, Type::Function { .. }) {
                        write!(f, "({})", ty)?;
                    } else {
                        write!(f, "{}", ty)?;
                    }
                }
                Ok(())
            }
            Type::Intersection(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " & ")?;
                    }
                    // Wrap unions and functions with parentheses
                    if matches!(ty, Type::Union(_) | Type::Function { .. }) {
                        write!(f, "({})", ty)?;
                    } else {
                        write!(f, "{}", ty)?;
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
                        if let Some(constraint) = &tp.constraint {
                            write!(f, " extends {}", constraint)?;
                        }
                        if let Some(default) = &tp.default {
                            write!(f, " = {}", default)?;
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
                    write!(f, ": {}", prop.ty)?;
                }
                if let Some(idx) = index_signature {
                    if !first {
                        write!(f, "; ")?;
                    }
                    write!(f, "[key: {}]: {}", idx.key_type, idx.value_type)?;
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
                        if let Some(constraint) = &tp.constraint {
                            write!(f, " extends {}", constraint)?;
                        }
                        if let Some(default) = &tp.default {
                            write!(f, " = {}", default)?;
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
                    write!(f, ": {}", param.ty)?;
                }
                write!(f, ") => {}", return_type)
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
                        write!(f, "static {}: {}", prop.name, prop.ty)?;
                    }
                    write!(f, " }}")?;
                }
                write!(
                    f,
                    " (constructor: ({}) => instance)",
                    params
                        .iter()
                        .map(|p| format!("{}: {}", p.name, p.ty))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }

            Type::TypeRef { name, type_args } => {
                write!(f, "{}", name)?;
                if !type_args.is_empty() {
                    write!(f, "<")?;
                    for (i, arg) in type_args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", arg)?;
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
                if let Some(constraint) = constraint {
                    write!(f, " extends {}", constraint)?;
                }
                if let Some(default) = default {
                    write!(f, " = {}", default)?;
                }
                Ok(())
            }

            Type::KeyOf(inner) => write!(f, "keyof {}", inner),

            Type::IndexedAccess { object_type, index_type } => {
                write!(f, "{}[{}]", object_type, index_type)
            }

            Type::MappedType { type_param, constraint, template, readonly_modifier, optional_modifier } => {
                write!(f, "{{ ")?;
                // Readonly modifier
                match readonly_modifier {
                    Some(true) => write!(f, "+readonly ")?,
                    Some(false) => write!(f, "-readonly ")?,
                    None => {}
                }
                // Key mapping
                write!(f, "[{} in {}]", type_param, constraint)?;
                // Optional modifier
                match optional_modifier {
                    Some(true) => write!(f, "?")?,
                    Some(false) => write!(f, "-?")?,
                    None => {}
                }
                write!(f, ": {}; }}", template)
            }

            Type::ConditionalType { check_type, extends_type, true_type, false_type } => {
                write!(f, "{} extends {} ? {} : {}", check_type, extends_type, true_type, false_type)
            }

            Type::InferType { name, constraint } => {
                write!(f, "infer {}", name)?;
                if let Some(c) = constraint {
                    write!(f, " extends {}", c)?;
                }
                Ok(())
            }

            Type::TemplateLiteralType { texts, types } => {
                write!(f, "`")?;
                for (i, text) in texts.iter().enumerate() {
                    write!(f, "{}", text)?;
                    if let Some(ty) = types.get(i) {
                        write!(f, "${{{}}}", ty)?;
                    }
                }
                write!(f, "`")
            }
        }
    }
}

/// Property in an object type: { name: string, age?: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Property {
    pub name: String,
    pub ty: Type,
    pub optional: bool,
    pub readonly: bool,
}

impl Property {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
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
    pub ty: Type,
    pub optional: bool,
    pub rest: bool,
}

impl Param {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
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
    pub name: String,                   // T
    pub constraint: Option<Box<Type>>,  // extends SomeType
    pub default: Option<Box<Type>>,     // = DefaultType
}

impl TypeParam {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            constraint: None,
            default: None,
        }
    }

    pub fn with_constraint(mut self, constraint: Type) -> Self {
        self.constraint = Some(Box::new(constraint));
        self
    }

    pub fn with_default(mut self, default: Type) -> Self {
        self.default = Some(Box::new(default));
        self
    }
}

/// Index signature: { [key: string]: number }
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IndexSignature {
    pub key_type: Box<Type>,
    pub value_type: Box<Type>,
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
    pub type_annotation: Option<Box<Type>>,
}

impl TypePredicate {
    pub fn new(parameter_name: impl Into<String>, asserts: bool, type_annotation: Option<Type>) -> Self {
        Self {
            parameter_name: parameter_name.into(),
            asserts,
            type_annotation: type_annotation.map(Box::new),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Type::array(Type::String).to_string(), "string[]");
        assert_eq!(Type::array(Type::Number).to_string(), "number[]");
        assert_eq!(
            Type::array(Type::array(Type::String)).to_string(),
            "string[][]"
        );
    }

    #[test]
    fn test_tuple_display() {
        assert_eq!(
            Type::Tuple(vec![Type::String, Type::Number]).to_string(),
            "[string, number]"
        );
    }

    #[test]
    fn test_union_display() {
        assert_eq!(
            Type::union(vec![Type::String, Type::Number]).to_string(),
            "string | number"
        );
        assert_eq!(
            Type::union(vec![Type::String, Type::Null, Type::Undefined]).to_string(),
            "string | null | undefined"
        );
    }

    #[test]
    fn test_intersection_display() {
        let a = Type::object(vec![Property::new("a", Type::String)]);
        let b = Type::object(vec![Property::new("b", Type::Number)]);
        assert_eq!(
            Type::intersection(vec![a, b]).to_string(),
            "{ a: string } & { b: number }"
        );
    }

    #[test]
    fn test_object_display() {
        let obj = Type::object(vec![
            Property::new("name", Type::String),
            Property::new("age", Type::Number).optional(),
        ]);
        assert_eq!(obj.to_string(), "{ name: string; age?: number }");

        let readonly_obj = Type::object(vec![Property::new("id", Type::Number).readonly()]);
        assert_eq!(readonly_obj.to_string(), "{ readonly id: number }");
    }

    #[test]
    fn test_function_display() {
        let func = Type::function(
            vec![Param::new("x", Type::Number), Param::new("y", Type::Number)],
            Type::Number,
        );
        assert_eq!(func.to_string(), "(x: number, y: number) => number");

        let func_optional = Type::function(
            vec![
                Param::new("x", Type::Number),
                Param::new("y", Type::Number).optional(),
            ],
            Type::Number,
        );
        assert_eq!(
            func_optional.to_string(),
            "(x: number, y?: number) => number"
        );

        let func_rest = Type::function(
            vec![Param::new("args", Type::array(Type::Number)).rest()],
            Type::Number,
        );
        assert_eq!(func_rest.to_string(), "(...args: number[]) => number");
    }

    #[test]
    fn test_generic_function_display() {
        let func = Type::generic_function(
            vec![TypeParam::new("T")],
            vec![Param::new("x", Type::type_ref("T", vec![]))],
            Type::type_ref("T", vec![]),
        );
        assert_eq!(func.to_string(), "<T>(x: T) => T");

        let func_constrained = Type::generic_function(
            vec![TypeParam::new("T").with_constraint(Type::String)],
            vec![Param::new("x", Type::type_ref("T", vec![]))],
            Type::type_ref("T", vec![]),
        );
        assert_eq!(
            func_constrained.to_string(),
            "<T extends string>(x: T) => T"
        );
    }

    #[test]
    fn test_type_ref_display() {
        assert_eq!(
            Type::type_ref("Array", vec![Type::String]).to_string(),
            "Array<string>"
        );
        assert_eq!(
            Type::type_ref("Map", vec![Type::String, Type::Number]).to_string(),
            "Map<string, number>"
        );
        assert_eq!(Type::type_ref("User", vec![]).to_string(), "User");
    }

    #[test]
    fn test_is_primitive() {
        assert!(Type::String.is_primitive());
        assert!(Type::Number.is_primitive());
        assert!(Type::Boolean.is_primitive());
        assert!(!Type::Any.is_primitive());
        assert!(!Type::array(Type::String).is_primitive());
    }

    #[test]
    fn test_object_with_index_signature() {
        let obj = Type::Object {
            properties: vec![],
            index_signature: Some(IndexSignature {
                key_type: Box::new(Type::String),
                value_type: Box::new(Type::Number),
            }),
            extends: vec![],
            type_params: vec![],
        };
        assert_eq!(obj.to_string(), "{ [key: string]: number }");
    }

    #[test]
    fn test_simplify_union_with_never() {
        let union = Type::Union(vec![Type::String, Type::Never]);
        assert_eq!(union.simplify(), Type::String);
    }

    #[test]
    fn test_simplify_union_all_never() {
        let union = Type::Union(vec![Type::Never, Type::Never]);
        assert_eq!(union.simplify(), Type::Never);
    }

    #[test]
    fn test_simplify_union_single_element() {
        let union = Type::Union(vec![Type::String]);
        assert_eq!(union.simplify(), Type::String);
    }

    #[test]
    fn test_simplify_intersection_with_never() {
        let intersection = Type::Intersection(vec![Type::String, Type::Never]);
        assert_eq!(intersection.simplify(), Type::Never);
    }

    #[test]
    fn test_simplify_intersection_single_element() {
        let intersection = Type::Intersection(vec![Type::String]);
        assert_eq!(intersection.simplify(), Type::String);
    }

    #[test]
    fn test_simplify_nested_union() {
        let nested = Type::Union(vec![
            Type::String,
            Type::Union(vec![Type::Number, Type::Boolean]),
        ]);
        let simplified = nested.simplify();
        if let Type::Union(types) = simplified {
            assert_eq!(types.len(), 3);
        } else {
            panic!("Expected Union");
        }
    }

    #[test]
    fn test_simplify_deduplicates() {
        let union = Type::Union(vec![Type::String, Type::String, Type::Number]);
        let simplified = union.simplify();
        if let Type::Union(types) = simplified {
            assert_eq!(types.len(), 2);
        } else {
            panic!("Expected Union");
        }
    }

    #[test]
    fn test_discriminant_order_is_consistent() {
        // Verify discriminant ordering matches the enum order
        assert!(Type::String.discriminant_order() < Type::Number.discriminant_order());
        assert!(Type::Number.discriminant_order() < Type::Boolean.discriminant_order());
        assert!(Type::Never.discriminant_order() < Type::StringLiteral("".into()).discriminant_order());
        assert!(Type::BooleanLiteral(true).discriminant_order() < Type::Array(Box::new(Type::String)).discriminant_order());
    }

    #[test]
    fn test_type_ordering_works() {
        let mut types = vec![
            Type::Number,
            Type::String,
            Type::Boolean,
        ];
        types.sort();
        assert_eq!(types, vec![Type::String, Type::Number, Type::Boolean]);
    }
}
