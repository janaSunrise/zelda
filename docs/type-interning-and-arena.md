# Understanding Type Interning and the Type Arena

This document explains how we manages types internally using two key concepts: type interning and the type arena.

These mechanisms work together to make the type checker fast, memory efficient, and correct.

## The Problem We're Solving

When type checking Typescript code, we encounter the same types over and over again. Consider this simple example:

```ts
let x: string;
let y: string;
let z: string;
```

A naive implementation would create three separate `Type::String` objects in memory.

Now imagine a large codebase with thousands of variables. If we're creating full type objects for every single one, we end up
with heavy memory usage and slow comparisons (since comparing two complex types such as objects requires walking their entire
structure).

The solution is to store each unique type exactly once, and refer to it using a small identifier. This is called interning.

## What is Type Interning?

Type interning is the process of ensuring structurally identical types share the same representation in memory. Instead of
having multiple copies of `{ x: number, y: number }` floating around, we store it once and hand out references to that single
stored copy.

## TypeId: The Lightweight Handle

A `TypeId` is a 32-bit unsigned integer wrapped in a struct:

```rs
pub struct TypeId(pub(crate) u32);
```

It can be copied freely without any allocation overhead. When you pass a `TypeId` around, you're just passing a number.

The `TypeId` acts as an index into the arena's storage. If you have `TypeId(42)`, that means "go look at position 42 in the type
storage to find the actual type".

### Pre-cached Primitive Types

For common primitive types that appear constantly in Typescript code, we assign fixed well-known IDs:

- `TypeId(0)` is always `string`
- `TypeId(1)` is always `number`
- `TypeId(2)` is always `boolean`
- `TypeId(3)` is always `null`
- `TypeId(4)` is always `undefined`
- `TypeId(5)` is always `void`
- `TypeId(6)` is always `any`
- `TypeId(7)` is always `unknown`
- `TypeId(8)` is always `never`
- `TypeId(9)` is always `true` (literal)
- `TypeId(10)` is always `false` (literal)

This means checking if something is a string is as simple as comparing `id == TypeId::STRING`. No table lookup needed.

## The Type Arena

The `TypeArena` is the central storage for all types in the system. It has two main components:

1. A vector of types: This is where the actual `Type` values live. Each position in the vector corresponds to a `TypeId`. Position 0 holds the string type, position 1 holds the number type, and so on.
2. A deduplication cache: This is a hash map that goes from `Type` to `TypeId`. When we want to intern a new type, we first check this cache. If an identical type already exists, we return its existing ID instead of creating a duplicate.

### How Interning Works

When you call `arena.intern(some_type)`, here's what happens:

1. The arena hashes the type and looks it up in the cache
2. If the cache contains this type, return the existing `TypeId` immediately
3. If the cache doesn't contain it, assign a new `TypeId` (the current length of the vector), store the type in the vector, add the mapping to the cache, and return the new ID

This process is called interning because we're internalizing the type into our storage system.

### The Beauty of Deduplication

Consider these two type aliases that happen to be structurally identical:

```ts
type Point = { x: number; y: number };
type Vector = { x: number; y: number };
```

When we resolve `Point`, we create an object type with two properties and intern it. Let's say it gets `TypeId(50)`.

When we resolve `Vector`, we create the exact same object type structure. When we try to intern it, the cache lookup finds the
existing entry and returns `TypeId(50)`.

Both `Point` and `Vector` end up with the same `TypeId`. This is correct behavior.

In Typescript's structural type system, these two types are indeed the same type. A `Point` can be assigned to a `Vector` and vice
versa.

## How Types Reference Other Types

Here's where the design gets elegant. When a type needs to reference another type, it doesn't store a full copy. It stores a `TypeId` instead.

For example, an array type is defined as:

```rs
Type::Array(TypeId) // The TypeId points to the element type
```

An array of strings is `Type::Array(TypeId::STRING)`. An array of numbers is `Type::Array(TypeId::NUMBER)`. The array type itself is
tiny because it just stores a 4-byte reference to its element type.

The same pattern applies everywhere:

```rs
Type::Union(Vec<TypeId>) // Union stores IDs of its member types

Type::Function {
    params: Vec<Param>, // Each param has a TypeId for its type
    return_type: TypeId, // Return type is just a TypeId
    // ...
}
```

This creates a graph structure where types reference other types through IDs. The arena owns all the actual type data, and
everything else just holds lightweight handles.

## The Type Resolution Pipeline

When parsing Typescript code, we go through this pipeline:

1. Parse: The parser (oxc) creates an AST with type annotations as syntax nodes
2. Resolve: We walk the AST and convert type syntax into `Type` values, immediately interning them to get `TypeId` values
3. Store: Symbols in the symbol table store their types as `TypeId` values
4. Check: The type checker works with `TypeId` values, looking up full types from the arena only when needed

At every stage after resolution, we're working with lightweight IDs rather than full type structures.

## Integration with the Type Checker

The `Checker` struct holds a mutable reference to the symbol table, which owns the arena. This gives the checker full access to
intern new types and look up existing ones.

```rust
// Get a type by its ID
let ty = checker.get_type(some_id);

// Create and intern a new type
let new_id = checker.intern(Type::Array(TypeId::STRING));
```

When checking type compatibility, the checker can often short-circuit. If `source_id == target_id`, the types are identical and definitely compatible. Only when the IDs differ does the checker need to look up the actual types and perform structural comparison.

## The Flow of Type Through the System

Let's trace what happens when Typescript code declares a variable:

```ts
let items: string[];
```

1. The parser creates an AST node representing the type annotation `string[]`
2. The resolver sees this is an array type with element type `string`
3. The resolver calls `resolve_ts_type` on the inner `string` type, which returns `TypeId::STRING` (the pre-cached primitive)
4. The resolver calls `arena.array(TypeId::STRING)` to create an array of strings
5. Inside `arena.array`, it constructs `Type::Array(TypeId::STRING)` and calls `intern`
6. If this is the first array of strings we've seen, it gets a new ID. If we've seen one before, we get that existing ID
7. The returned `TypeId` is stored in the symbol table entry for `items`

Later, when type checking an assignment to `items`:
```ts
items = ["hello", "world"];
```

1. The checker infers the type of `["hello", "world"]` and gets a `TypeId`
2. The checker looks up the declared type of `items` and gets its `TypeId`
3. The checker calls `is_assignable(inferred_id, declared_id)`
4. If the IDs match, we're done. If not, look up both types and check structurally.
