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

use super::{Param, Property, Type};

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

    /// Check if this TypeId refers to a primitive type (one of the pre-cached constants).
    #[inline]
    pub const fn is_primitive(self) -> bool {
        self.0 <= 10
    }
}

/// An arena for interning and storing types.
///
/// The arena provides:
/// - Deduplication: Identical types share the same `TypeId`
/// - Fast lookup: O(1) access by `TypeId`
/// - Fast interning: O(1) amortized for previously seen types
/// - Fixed primitive IDs: `TypeId::STRING`, `TypeId::NUMBER`, ...
///
/// # Example
///
/// ```ignore
/// let mut arena = TypeArena::new();
///
/// // Primitive types have fixed IDs
/// assert_eq!(arena.get(TypeId::STRING), &Type::String);
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
}

impl TypeArena {
    /// Create a new arena with pre-cached primitive types.
    pub fn new() -> Self {
        let mut types = Vec::with_capacity(256); // Pre-allocate for common programs
        let mut intern_cache = FxHashMap::with_capacity_and_hasher(256, Default::default());

        // Intern primitives in a fixed order so their IDs are predictable.
        // The order MUST match the TypeId constants (STRING=0, NUMBER=1, etc.)
        let primitives_list = [
            Type::String,                // TypeId::STRING = 0
            Type::Number,                // TypeId::NUMBER = 1
            Type::Boolean,               // TypeId::BOOLEAN = 2
            Type::Null,                  // TypeId::NULL = 3
            Type::Undefined,             // TypeId::UNDEFINED = 4
            Type::Void,                  // TypeId::VOID = 5
            Type::Any,                   // TypeId::ANY = 6
            Type::Unknown,               // TypeId::UNKNOWN = 7
            Type::Never,                 // TypeId::NEVER = 8
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
        }
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
        if types.contains(&TypeId::NEVER) {
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

    /// Create and intern an object type.
    pub fn object(&mut self, properties: Vec<Property>) -> TypeId {
        self.intern(Type::Object {
            properties,
            index_signature: None,
            extends: vec![],
            type_params: vec![],
        })
    }

    /// Create and intern a type reference.
    pub fn type_ref(&mut self, name: impl Into<String>, type_args: Vec<TypeId>) -> TypeId {
        self.intern(Type::TypeRef {
            name: name.into(),
            type_args,
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
        let expected = [
            (TypeId::STRING, Type::String),
            (TypeId::NUMBER, Type::Number),
            (TypeId::BOOLEAN, Type::Boolean),
            (TypeId::NULL, Type::Null),
            (TypeId::UNDEFINED, Type::Undefined),
            (TypeId::VOID, Type::Void),
            (TypeId::ANY, Type::Any),
            (TypeId::UNKNOWN, Type::Unknown),
            (TypeId::NEVER, Type::Never),
            (TypeId::TRUE, Type::BooleanLiteral(true)),
            (TypeId::FALSE, Type::BooleanLiteral(false)),
        ];
        for (id, ty) in expected {
            assert_eq!(arena.get(id), &ty);
        }
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
    fn test_union_with_type_ids() {
        let mut arena = TypeArena::new();

        // Create a union of primitive types using TypeIds
        let union = Type::Union(vec![TypeId::STRING, TypeId::NUMBER]);
        let id = arena.intern(union.clone());

        assert_eq!(arena.get(id), &union);
    }

    #[test]
    fn test_type_id_is_copy() {
        let mut arena = TypeArena::new();
        let id = arena.intern(Type::String);
        let id2 = id; // Copy
        assert_eq!(id, id2);
    }

    #[test]
    fn test_boolean_literals_cached() {
        let mut arena = TypeArena::new();

        let true_id = arena.intern(Type::BooleanLiteral(true));
        let false_id = arena.intern(Type::BooleanLiteral(false));

        assert_eq!(true_id, TypeId::TRUE);
        assert_eq!(false_id, TypeId::FALSE);
    }
}
