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

use std::collections::HashMap;

use super::{IndexSignature, Param, Property, Type, TypeParam};

/// A lightweight, copyable handle to an interned type.
///
/// `TypeId` is the primary way to reference types after interning. It's:
/// - 4 bytes (vs potentially kilobytes for a deep `Type`)
/// - `Copy`, so no cloning overhead
/// - `Eq` + `Hash`, so it can be used in collections
///
/// Use `TypeArena::get(id)` to retrieve the actual `Type` when needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(u32);

impl TypeId {
    #[inline]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// Get the raw u32 value of this TypeId.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }
}

/// Pre-cached primitive type IDs for instant access.
///
/// These are always the first types interned, so their IDs are predictable.
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
pub struct TypeArena {
    /// Storage for all interned types.
    types: Vec<Type>,
    /// Maps types to their IDs for deduplication.
    intern_cache: HashMap<Type, TypeId>,
    /// Pre-cached primitive type IDs.
    primitives: PrimitiveTypes,
}

impl TypeArena {
    /// Create a new arena with pre-cached primitive types.
    pub fn new() -> Self {
        let mut types = Vec::with_capacity(256); // Pre-allocate for common programs
        let mut intern_cache = HashMap::with_capacity(256);

        // Intern primitives in a fixed order so their IDs are predictable
        let primitives = [
            Type::String,
            Type::Number,
            Type::Boolean,
            Type::Null,
            Type::Undefined,
            Type::Void,
            Type::Any,
            Type::Unknown,
            Type::Never,
            Type::BooleanLiteral(true),
            Type::BooleanLiteral(false),
        ];

        for ty in primitives {
            let id = TypeId(types.len() as u32);
            intern_cache.insert(ty.clone(), id);
            types.push(ty);
        }

        let primitives = PrimitiveTypes {
            string: TypeId(0),
            number: TypeId(1),
            boolean: TypeId(2),
            null: TypeId(3),
            undefined: TypeId(4),
            void: TypeId(5),
            any: TypeId(6),
            unknown: TypeId(7),
            never: TypeId(8),
            true_literal: TypeId(9),
            false_literal: TypeId(10),
        };

        Self {
            types,
            intern_cache,
            primitives,
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

    /// Intern a type, recursively interning nested types first.
    ///
    /// This ensures all nested types (e.g., in unions, arrays, objects)
    /// are also interned, which is important for deep type structures.
    pub fn intern_deep(&mut self, ty: Type) -> TypeId {
        // For simple types, just intern directly
        match ty {
            Type::String
            | Type::Number
            | Type::Boolean
            | Type::Null
            | Type::Undefined
            | Type::Void
            | Type::Any
            | Type::Unknown
            | Type::Never
            | Type::StringLiteral(_)
            | Type::NumberLiteral(_)
            | Type::BooleanLiteral(_) => self.intern(ty),

            Type::Array(elem) => {
                let elem_id = self.intern_deep(*elem);
                let elem_ty = self.get(elem_id).clone();
                self.intern(Type::Array(Box::new(elem_ty)))
            }

            Type::Tuple(types) => {
                let interned: Vec<Type> = types
                    .into_iter()
                    .map(|t| {
                        let id = self.intern_deep(t);
                        self.get(id).clone()
                    })
                    .collect();
                self.intern(Type::Tuple(interned))
            }

            Type::Union(types) => {
                let interned: Vec<Type> = types
                    .into_iter()
                    .map(|t| {
                        let id = self.intern_deep(t);
                        self.get(id).clone()
                    })
                    .collect();
                self.intern(Type::Union(interned))
            }

            Type::Intersection(types) => {
                let interned: Vec<Type> = types
                    .into_iter()
                    .map(|t| {
                        let id = self.intern_deep(t);
                        self.get(id).clone()
                    })
                    .collect();
                self.intern(Type::Intersection(interned))
            }

            Type::Object {
                properties,
                index_signature,
                extends,
                type_params,
            } => {
                let interned_props: Vec<Property> = properties
                    .into_iter()
                    .map(|p| {
                        let ty_id = self.intern_deep(p.ty);
                        Property {
                            name: p.name,
                            ty: self.get(ty_id).clone(),
                            optional: p.optional,
                            readonly: p.readonly,
                        }
                    })
                    .collect();

                let interned_idx = index_signature.map(|idx| {
                    let key_id = self.intern_deep(*idx.key_type);
                    let val_id = self.intern_deep(*idx.value_type);
                    IndexSignature {
                        key_type: Box::new(self.get(key_id).clone()),
                        value_type: Box::new(self.get(val_id).clone()),
                    }
                });

                let interned_extends: Vec<Type> = extends
                    .into_iter()
                    .map(|t| {
                        let id = self.intern_deep(t);
                        self.get(id).clone()
                    })
                    .collect();

                let interned_params: Vec<TypeParam> = type_params
                    .into_iter()
                    .map(|tp| self.intern_type_param(tp))
                    .collect();

                self.intern(Type::Object {
                    properties: interned_props,
                    index_signature: interned_idx,
                    extends: interned_extends,
                    type_params: interned_params,
                })
            }

            Type::Function {
                params,
                return_type,
                type_params,
            } => {
                let interned_params: Vec<Param> = params
                    .into_iter()
                    .map(|p| {
                        let ty_id = self.intern_deep(p.ty);
                        Param {
                            name: p.name,
                            ty: self.get(ty_id).clone(),
                            optional: p.optional,
                            rest: p.rest,
                        }
                    })
                    .collect();

                let ret_id = self.intern_deep(*return_type);
                let interned_ret = self.get(ret_id).clone();

                let interned_type_params: Vec<TypeParam> = type_params
                    .into_iter()
                    .map(|tp| self.intern_type_param(tp))
                    .collect();

                self.intern(Type::Function {
                    params: interned_params,
                    return_type: Box::new(interned_ret),
                    type_params: interned_type_params,
                })
            }

            Type::ClassConstructor {
                params,
                type_params,
                static_members,
            } => {
                let interned_params: Vec<Param> = params
                    .into_iter()
                    .map(|p| {
                        let ty_id = self.intern_deep(p.ty);
                        Param {
                            name: p.name,
                            ty: self.get(ty_id).clone(),
                            optional: p.optional,
                            rest: p.rest,
                        }
                    })
                    .collect();

                let interned_type_params: Vec<TypeParam> = type_params
                    .into_iter()
                    .map(|tp| self.intern_type_param(tp))
                    .collect();

                let interned_statics: Vec<Property> = static_members
                    .into_iter()
                    .map(|p| {
                        let ty_id = self.intern_deep(p.ty);
                        Property {
                            name: p.name,
                            ty: self.get(ty_id).clone(),
                            optional: p.optional,
                            readonly: p.readonly,
                        }
                    })
                    .collect();

                self.intern(Type::ClassConstructor {
                    params: interned_params,
                    type_params: interned_type_params,
                    static_members: interned_statics,
                })
            }

            Type::TypeRef { name, type_args } => {
                let interned_args: Vec<Type> = type_args
                    .into_iter()
                    .map(|t| {
                        let id = self.intern_deep(t);
                        self.get(id).clone()
                    })
                    .collect();
                self.intern(Type::TypeRef {
                    name,
                    type_args: interned_args,
                })
            }

            Type::TypeParameter {
                name,
                constraint,
                default,
            } => {
                let interned_constraint = constraint.map(|c| {
                    let id = self.intern_deep(*c);
                    Box::new(self.get(id).clone())
                });
                let interned_default = default.map(|d| {
                    let id = self.intern_deep(*d);
                    Box::new(self.get(id).clone())
                });
                self.intern(Type::TypeParameter {
                    name,
                    constraint: interned_constraint,
                    default: interned_default,
                })
            }
        }
    }

    /// Helper to intern a type parameter's constraint and default.
    fn intern_type_param(&mut self, tp: TypeParam) -> TypeParam {
        let constraint = tp.constraint.map(|c| {
            let id = self.intern_deep(*c);
            Box::new(self.get(id).clone())
        });
        let default = tp.default.map(|d| {
            let id = self.intern_deep(*d);
            Box::new(self.get(id).clone())
        });
        TypeParam {
            name: tp.name,
            constraint,
            default,
        }
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
    /// This is faster than interning for known primitives.
    pub fn primitive_id(&self, ty: &Type) -> Option<TypeId> {
        match ty {
            Type::String => Some(self.primitives.string),
            Type::Number => Some(self.primitives.number),
            Type::Boolean => Some(self.primitives.boolean),
            Type::Null => Some(self.primitives.null),
            Type::Undefined => Some(self.primitives.undefined),
            Type::Void => Some(self.primitives.void),
            Type::Any => Some(self.primitives.any),
            Type::Unknown => Some(self.primitives.unknown),
            Type::Never => Some(self.primitives.never),
            Type::BooleanLiteral(true) => Some(self.primitives.true_literal),
            Type::BooleanLiteral(false) => Some(self.primitives.false_literal),
            _ => None,
        }
    }

    #[inline]
    pub fn intern_fast(&mut self, ty: Type) -> TypeId {
        self.primitive_id(&ty).unwrap_or_else(|| self.intern(ty))
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
        assert_eq!(arena.primitives().string, TypeId(0));
        assert_eq!(arena.primitives().number, TypeId(1));
        assert_eq!(arena.primitives().boolean, TypeId(2));
        assert_eq!(arena.primitives().null, TypeId(3));
        assert_eq!(arena.primitives().undefined, TypeId(4));
        assert_eq!(arena.primitives().void, TypeId(5));
        assert_eq!(arena.primitives().any, TypeId(6));
        assert_eq!(arena.primitives().unknown, TypeId(7));
        assert_eq!(arena.primitives().never, TypeId(8));
    }

    #[test]
    fn test_intern_deduplicates() {
        let mut arena = TypeArena::new();

        let obj1 = Type::object(vec![Property::new("x", Type::Number)]);
        let obj2 = Type::object(vec![Property::new("x", Type::Number)]);

        let id1 = arena.intern(obj1);
        let id2 = arena.intern(obj2);

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_get_returns_correct_type() {
        let mut arena = TypeArena::new();

        let obj = Type::object(vec![Property::new("name", Type::String)]);
        let id = arena.intern(obj.clone());

        assert_eq!(arena.get(id), &obj);
    }

    #[test]
    fn test_primitive_id_fast_path() {
        let arena = TypeArena::new();

        assert_eq!(arena.primitive_id(&Type::String), Some(arena.primitives().string));
        assert_eq!(arena.primitive_id(&Type::Number), Some(arena.primitives().number));
        assert_eq!(
            arena.primitive_id(&Type::object(vec![])),
            None
        );
    }

    #[test]
    fn test_intern_deep_handles_nested() {
        let mut arena = TypeArena::new();

        let nested = Type::Union(vec![
            Type::Array(Box::new(Type::String)),
            Type::object(vec![Property::new("x", Type::Number)]),
        ]);

        let id = arena.intern_deep(nested.clone());
        assert_eq!(arena.get(id), &nested);
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
