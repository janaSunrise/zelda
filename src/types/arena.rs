//! Type interning for efficient type storage and comparison.
//!
//! The `TypeArena` stores types in a deduplicated manner and returns lightweight
//! `TypeId` handles for efficient comparison and storage. This eliminates the need
//! for deep cloning during type checking operations.
//!
//! - Types are stored in a `Vec<Type>` for O(1) lookup by `TypeId`
//! - A `HashMap<Type, TypeId>` provides O(1) interning for deduplication
//! - Primitive types are pre-cached for instant access
//! - `TypeId` is `Copy` + `Eq` + `Hash`, making comparisons trivial
//!
//! The arena can be wrapped in `Arc<TypeArena>` and shared across threads
//! since interned types are immutable. Only the interning operation needs
//! synchronization (or can be done in a single-threaded binding phase).

use rustc_hash::FxHashMap;

use super::{Param, Property, Type, TypeParam};

/// A lightweight, copyable handle to an interned type.
///
/// `TypeId` is the primary way to reference types after interning. It's:
/// - 4 bytes (vs potentially kilobytes for a deep `Type`)
/// - `Copy`, so no cloning overhead
/// - `Eq` + `Hash`, so it can be used in collections
///
/// Use `TypeArena::get(id)` to retrieve the actual `Type` when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub(crate) u32);

impl TypeId {
    // Pre-defined constants for primitive types (indices 0-10 in the arena)
    pub const STRING: TypeId = TypeId(0);
    pub const NUMBER: TypeId = TypeId(1);
    pub const BOOLEAN: TypeId = TypeId(2);
    pub const NULL: TypeId = TypeId(3);
    pub const UNDEFINED: TypeId = TypeId(4);
    pub const VOID: TypeId = TypeId(5);
    pub const ANY: TypeId = TypeId(6);
    pub const UNKNOWN: TypeId = TypeId(7);
    pub const NEVER: TypeId = TypeId(8);
    pub const TRUE: TypeId = TypeId(9);
    pub const FALSE: TypeId = TypeId(10);

    #[inline]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Get the raw u32 value of this TypeId.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// Check if this TypeId refers to a primitive type (one of the pre-cached constants).
    #[inline]
    pub const fn is_primitive(self) -> bool {
        self.0 <= 10
    }
}

/// Pre-cached primitive type IDs for instant access.
///
/// These are always the first types interned, so their IDs are predictable.
/// All fields use the TypeId constants (e.g., `TypeId::STRING`).
#[derive(Debug, Clone, Copy)]
pub struct PrimitiveTypes {
    pub string: TypeId,
    pub number: TypeId,
    pub boolean: TypeId,
    pub null: TypeId,
    pub undefined: TypeId,
    pub void: TypeId,
    pub any: TypeId,
    pub unknown: TypeId,
    pub never: TypeId,
    pub true_literal: TypeId,
    pub false_literal: TypeId,
}

impl Default for PrimitiveTypes {
    fn default() -> Self {
        Self {
            string: TypeId::STRING,
            number: TypeId::NUMBER,
            boolean: TypeId::BOOLEAN,
            null: TypeId::NULL,
            undefined: TypeId::UNDEFINED,
            void: TypeId::VOID,
            any: TypeId::ANY,
            unknown: TypeId::UNKNOWN,
            never: TypeId::NEVER,
            true_literal: TypeId::TRUE,
            false_literal: TypeId::FALSE,
        }
    }
}

/// An arena for interning and storing types.
///
/// The arena provides:
/// - Deduplication: Identical types share the same `TypeId`
/// - Fast lookup: O(1) access by `TypeId`
/// - Fast interning: O(1) amortized for previously seen types
/// - Pre-cached primitives: Common types available instantly
///
/// # Example
///
/// ```ignore
/// let mut arena = TypeArena::new();
///
/// // Get primitive types
/// let string_id = arena.primitives().string;
/// let number_id = arena.primitives().number;
///
/// // Intern complex types
/// let obj_type = Type::object(vec![Property::new("x", Type::Number)]);
/// let obj_id = arena.intern(obj_type);
///
/// // Same type returns same ID
/// let obj_type2 = Type::object(vec![Property::new("x", Type::Number)]);
/// let obj_id2 = arena.intern(obj_type2);
/// assert_eq!(obj_id, obj_id2);
/// ```
#[derive(Debug, Clone)]
pub struct TypeArena {
    /// Storage for all interned types.
    types: Vec<Type>,
    /// Maps types to their IDs for deduplication.
    /// Uses FxHashMap for faster hashing - Type's hash is compiler-internal,
    /// so we don't need SipHash's DoS resistance.
    intern_cache: FxHashMap<Type, TypeId>,
    /// Pre-cached primitive type IDs.
    primitives: PrimitiveTypes,
}

impl TypeArena {
    /// Create a new arena with pre-cached primitive types.
    pub fn new() -> Self {
        let mut types = Vec::with_capacity(256); // Pre-allocate for common programs
        let mut intern_cache = FxHashMap::with_capacity_and_hasher(256, Default::default());

        // Intern primitives in a fixed order so their IDs are predictable.
        // The order MUST match the TypeId constants (STRING=0, NUMBER=1, etc.)
        let primitives_list = [
            Type::String,            // TypeId::STRING = 0
            Type::Number,            // TypeId::NUMBER = 1
            Type::Boolean,           // TypeId::BOOLEAN = 2
            Type::Null,              // TypeId::NULL = 3
            Type::Undefined,         // TypeId::UNDEFINED = 4
            Type::Void,              // TypeId::VOID = 5
            Type::Any,               // TypeId::ANY = 6
            Type::Unknown,           // TypeId::UNKNOWN = 7
            Type::Never,             // TypeId::NEVER = 8
            Type::BooleanLiteral(true),  // TypeId::TRUE = 9
            Type::BooleanLiteral(false), // TypeId::FALSE = 10
        ];

        for ty in primitives_list {
            let id = TypeId(types.len() as u32);
            intern_cache.insert(ty.clone(), id);
            types.push(ty);
        }

        Self {
            types,
            intern_cache,
            primitives: PrimitiveTypes::default(),
        }
    }

    #[inline]
    pub fn primitives(&self) -> &PrimitiveTypes {
        &self.primitives
    }

    /// Intern a type, returning its unique ID.
    ///
    /// If the type has been interned before, returns the existing ID.
    /// Otherwise, stores the type and returns a new ID.
    ///
    /// This is O(1) amortized for types that have been seen before,
    /// and O(hash + insert) for new types.
    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(&id) = self.intern_cache.get(&ty) {
            return id;
        }

        // New type: add to storage and cache
        let id = TypeId(self.types.len() as u32);
        self.intern_cache.insert(ty.clone(), id);
        self.types.push(ty);
        id
    }

    /// Get a type by its ID.
    ///
    /// Panics if the ID is invalid (not returned by this arena).
    #[inline]
    pub fn get(&self, id: TypeId) -> &Type {
        &self.types[id.0 as usize]
    }

    #[inline]
    pub fn try_get(&self, id: TypeId) -> Option<&Type> {
        self.types.get(id.0 as usize)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.types.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        // Arena always has primitives, so "empty" means only primitives
        self.types.len() <= 11
    }

    /// Get the TypeId for a primitive type, if it matches.
    ///
    /// This is faster than interning for known primitives - O(1) constant lookup.
    #[inline]
    pub fn primitive_id(&self, ty: &Type) -> Option<TypeId> {
        match ty {
            Type::String => Some(TypeId::STRING),
            Type::Number => Some(TypeId::NUMBER),
            Type::Boolean => Some(TypeId::BOOLEAN),
            Type::Null => Some(TypeId::NULL),
            Type::Undefined => Some(TypeId::UNDEFINED),
            Type::Void => Some(TypeId::VOID),
            Type::Any => Some(TypeId::ANY),
            Type::Unknown => Some(TypeId::UNKNOWN),
            Type::Never => Some(TypeId::NEVER),
            Type::BooleanLiteral(true) => Some(TypeId::TRUE),
            Type::BooleanLiteral(false) => Some(TypeId::FALSE),
            _ => None,
        }
    }

    #[inline]
    pub fn intern_fast(&mut self, ty: Type) -> TypeId {
        self.primitive_id(&ty).unwrap_or_else(|| self.intern(ty))
    }

    // =====================================================
    // Convenience methods for creating and interning types
    // =====================================================

    /// Create and intern an array type.
    #[inline]
    pub fn array(&mut self, element: TypeId) -> TypeId {
        self.intern(Type::Array(element))
    }

    /// Create and intern a tuple type.
    #[inline]
    pub fn tuple(&mut self, types: Vec<TypeId>) -> TypeId {
        self.intern(Type::Tuple(types))
    }

    /// Create and intern a union type. Filters out duplicates and never types.
    pub fn union(&mut self, types: Vec<TypeId>) -> TypeId {
        // Filter out never types and deduplicate
        let mut filtered: Vec<TypeId> = types
            .into_iter()
            .filter(|&id| id != TypeId::NEVER)
            .collect();

        // Deduplicate by sorting
        filtered.sort_by_key(|id| id.0);
        filtered.dedup();

        match filtered.len() {
            0 => TypeId::NEVER,
            1 => filtered[0],
            _ => self.intern(Type::Union(filtered)),
        }
    }

    /// Create and intern an intersection type. Returns never if any element is never.
    pub fn intersection(&mut self, types: Vec<TypeId>) -> TypeId {
        // If any is never, result is never
        if types.iter().any(|&id| id == TypeId::NEVER) {
            return TypeId::NEVER;
        }

        // Deduplicate
        let mut deduped: Vec<TypeId> = types;
        deduped.sort_by_key(|id| id.0);
        deduped.dedup();

        match deduped.len() {
            0 => TypeId::UNKNOWN,
            1 => deduped[0],
            _ => self.intern(Type::Intersection(deduped)),
        }
    }

    /// Create and intern a function type.
    pub fn function(&mut self, params: Vec<Param>, return_type: TypeId) -> TypeId {
        self.intern(Type::Function {
            params,
            return_type,
            type_params: vec![],
            type_predicate: None,
        })
    }

    /// Create and intern a generic function type.
    pub fn generic_function(
        &mut self,
        type_params: Vec<TypeParam>,
        params: Vec<Param>,
        return_type: TypeId,
    ) -> TypeId {
        self.intern(Type::Function {
            params,
            return_type,
            type_params,
            type_predicate: None,
        })
    }

    /// Create and intern an object type.
    pub fn object(&mut self, properties: Vec<Property>) -> TypeId {
        self.intern(Type::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        })
    }

    /// Create and intern an object type with inheritance.
    pub fn object_with_extends(&mut self, properties: Vec<Property>, extends: Vec<TypeId>) -> TypeId {
        self.intern(Type::Object {
            properties,
            index_signature: None,
            extends,
            type_params: vec![],
        })
    }

    /// Create and intern a generic object type.
    pub fn generic_object(
        &mut self,
        type_params: Vec<TypeParam>,
        properties: Vec<Property>,
        extends: Vec<TypeId>,
    ) -> TypeId {
        self.intern(Type::Object {
            properties,
            index_signature: None,
            extends,
            type_params,
        })
    }

    /// Create and intern a type reference.
    pub fn type_ref(&mut self, name: impl Into<String>, type_args: Vec<TypeId>) -> TypeId {
        self.intern(Type::TypeRef {
            name: name.into(),
            type_args,
        })
    }

    /// Create and intern a string literal type.
    pub fn string_literal(&mut self, value: impl Into<String>) -> TypeId {
        self.intern(Type::StringLiteral(value.into()))
    }

    /// Create and intern a number literal type.
    pub fn number_literal(&mut self, value: f64) -> TypeId {
        self.intern(Type::NumberLiteral(value))
    }

    /// Create and intern a keyof type.
    pub fn keyof(&mut self, inner: TypeId) -> TypeId {
        self.intern(Type::KeyOf(inner))
    }

    /// Create and intern an indexed access type.
    pub fn indexed_access(&mut self, object_type: TypeId, index_type: TypeId) -> TypeId {
        self.intern(Type::IndexedAccess {
            object_type,
            index_type,
        })
    }

    /// Create and intern a mapped type.
    pub fn mapped_type(
        &mut self,
        type_param: String,
        constraint: TypeId,
        template: TypeId,
        readonly_modifier: Option<bool>,
        optional_modifier: Option<bool>,
    ) -> TypeId {
        self.intern(Type::MappedType {
            type_param,
            constraint,
            template,
            readonly_modifier,
            optional_modifier,
        })
    }

    /// Create and intern a conditional type.
    pub fn conditional_type(
        &mut self,
        check_type: TypeId,
        extends_type: TypeId,
        true_type: TypeId,
        false_type: TypeId,
    ) -> TypeId {
        self.intern(Type::ConditionalType {
            check_type,
            extends_type,
            true_type,
            false_type,
        })
    }

    /// Create and intern an infer type.
    pub fn infer_type(&mut self, name: impl Into<String>, constraint: Option<TypeId>) -> TypeId {
        self.intern(Type::InferType {
            name: name.into(),
            constraint,
        })
    }

    /// Create and intern a type parameter.
    pub fn type_parameter(
        &mut self,
        name: impl Into<String>,
        constraint: Option<TypeId>,
        default: Option<TypeId>,
    ) -> TypeId {
        self.intern(Type::TypeParameter {
            name: name.into(),
            constraint,
            default,
        })
    }

    /// Create and intern a template literal type.
    pub fn template_literal_type(&mut self, texts: Vec<String>, types: Vec<TypeId>) -> TypeId {
        self.intern(Type::TemplateLiteralType { texts, types })
    }

    /// Create and intern a class constructor type.
    pub fn class_constructor(
        &mut self,
        params: Vec<Param>,
        type_params: Vec<TypeParam>,
        static_members: Vec<Property>,
    ) -> TypeId {
        self.intern(Type::ClassConstructor {
            params,
            type_params,
            static_members,
        })
    }
}

impl Default for TypeArena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_ids_are_stable() {
        let arena = TypeArena::new();
        // Verify PrimitiveTypes struct matches TypeId constants
        assert_eq!(arena.primitives().string, TypeId::STRING);
        assert_eq!(arena.primitives().number, TypeId::NUMBER);
        assert_eq!(arena.primitives().boolean, TypeId::BOOLEAN);
        assert_eq!(arena.primitives().null, TypeId::NULL);
        assert_eq!(arena.primitives().undefined, TypeId::UNDEFINED);
        assert_eq!(arena.primitives().void, TypeId::VOID);
        assert_eq!(arena.primitives().any, TypeId::ANY);
        assert_eq!(arena.primitives().unknown, TypeId::UNKNOWN);
        assert_eq!(arena.primitives().never, TypeId::NEVER);
        assert_eq!(arena.primitives().true_literal, TypeId::TRUE);
        assert_eq!(arena.primitives().false_literal, TypeId::FALSE);

        // Verify the actual numeric values for stability across rebuilds
        assert_eq!(TypeId::STRING.as_raw(), 0);
        assert_eq!(TypeId::NUMBER.as_raw(), 1);
        assert_eq!(TypeId::BOOLEAN.as_raw(), 2);
        assert_eq!(TypeId::NEVER.as_raw(), 8);
    }

    #[test]
    fn test_intern_deduplicates() {
        let mut arena = TypeArena::new();

        // Create two identical object types with TypeId-based properties
        let obj1 = Type::Object {
            properties: vec![Property::new("x", TypeId::NUMBER)],
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        };
        let obj2 = Type::Object {
            properties: vec![Property::new("x", TypeId::NUMBER)],
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        };

        let id1 = arena.intern(obj1);
        let id2 = arena.intern(obj2);

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_get_returns_correct_type() {
        let mut arena = TypeArena::new();

        let obj = Type::Object {
            properties: vec![Property::new("name", TypeId::STRING)],
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        };
        let id = arena.intern(obj.clone());

        assert_eq!(arena.get(id), &obj);
    }

    #[test]
    fn test_primitive_id_fast_path() {
        let arena = TypeArena::new();

        assert_eq!(
            arena.primitive_id(&Type::String),
            Some(arena.primitives().string)
        );
        assert_eq!(
            arena.primitive_id(&Type::Number),
            Some(arena.primitives().number)
        );
        // Object types don't have a primitive ID
        let obj = Type::Object {
            properties: vec![],
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        };
        assert_eq!(arena.primitive_id(&obj), None);
    }

    #[test]
    fn test_union_with_type_ids() {
        let mut arena = TypeArena::new();

        // Create a union of primitive types using TypeIds
        let union = Type::Union(vec![TypeId::STRING, TypeId::NUMBER]);
        let id = arena.intern(union.clone());

        assert_eq!(arena.get(id), &union);
    }

    #[test]
    fn test_type_id_is_copy() {
        let arena = TypeArena::new();
        let id = arena.primitives().string;
        let id2 = id; // Copy
        assert_eq!(id, id2);
    }

    #[test]
    fn test_boolean_literals_cached() {
        let mut arena = TypeArena::new();

        let true_id = arena.intern(Type::BooleanLiteral(true));
        let false_id = arena.intern(Type::BooleanLiteral(false));

        assert_eq!(true_id, arena.primitives().true_literal);
        assert_eq!(false_id, arena.primitives().false_literal);
    }
}
