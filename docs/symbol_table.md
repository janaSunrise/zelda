# The Symbol Table

A symbol table is the memory of the checker. It knows:
- Every declared name in your code
- What type each name has
- Where each name is visible (scoping)

```rs
struct SymbolTable {
    symbols: Vec<Symbol>,      // all symbols
    scopes: Vec<Scope>,        // all scopes
    current_scope: ScopeId,    // current location
}
```

Two flat arrays and a pointer to the current scope.

## How scopes form a tree

Each scope has a `parent` field pointing to its parent scope, forming a tree.

```ts
const a = 1; // global scope

function foo() {
  // function scope (parent: global)
  const b = 2;

  if (true) {
    // block scope (parent: function)
    const c = 3;
  }
}
```

The tree structure:

```
Scope 0 (Global)
│   symbols: { "a": SymbolId(0) }
│
└── Scope 1 (Function)
    │   symbols: { "b": SymbolId(1) }
    │
    └── Scope 2 (Block)
            symbols: { "c": SymbolId(2) }
```

## Walking the tree

When you use a name, we search upward until we find it.

```ts
const x = 1;

function foo() {
  const y = 2;
  console.log(x);
}
```

Lookup for `x` inside `foo`:
1. Check Scope 1 (Function): symbols = { "y" }. Not found. Go to parent.
2. Check Scope 0 (Global): symbols = { "x" }. Found, return SymbolId for x.

## Shadowing

Inner scopes can reuse names from outer scopes.

```ts
const x = 1; // global x

function foo() {
  const x = 2; // shadows global x
  console.log(x); // finds inner x first
}

console.log(x); // finds global x
```

```
Scope 0 (Global)
│   symbols: { "x": SymbolId(0) }  // type: number (1)
│
└── Scope 1 (Function)
  symbols: { "x": SymbolId(1) }  // type: number (2)
```

Lookup starts from the current scope, so the inner `x` is found first. This is called shadowing.

## Push and pop

As the binder walks the AST, it pushes and pops scopes.

```ts
// current_scope = 0 (Global)
const a = 1;

function foo() {
  // push_scope(Function): current_scope = 1
  const b = 2;

  if (true) {
    // push_scope(Block): current_scope = 2
    const c = 3;
  } // pop_scope(): current_scope = 1
} // pop_scope(): current_scope = 0
```

## Defining a symbol

When the binder sees a declaration:

```ts
const x: number = 42;
```

It does:

```rust
// 1. Create the symbol
let symbol = Symbol {
    name: "x",
    ty: Type::Number,
    kind: SymbolKind::Variable,
    span: Span { start: 0, end: 20 },
    scope: current_scope,
};

// 2. Add to symbols array, get ID
let id = SymbolId(self.symbols.len());
self.symbols.push(symbol);

// 3. Register in current scope
self.scopes[current_scope].symbols.insert("x", id);
```
