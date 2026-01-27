My notes on building a Typescript type checker from scratch.

# Understanding Type Checkers

A type checker finds bugs in your code without running it.

Type checkers answer the question: "Does the code make sense?"

Here's the flow of a type checker:
Source code → Parser → AST → Binder → Symbol Table → Checker → Errors

Every type checker does three things:

1. Parse - Turn source code into a tree structure (AST)
2. Bind - Figure out what names refer to
3. Check - Make sure types match up between declarations and usage

## Step 1: Parsing

Parsing turns code into a tree structure called an AST (Abstract Syntax Tree).

This code:

```ts
const x: number = 42;
```

Becomes something like:

```
Variable Declaration
├── name: "x"
├── type: "number"
└── value: 42
```

Trees capture the hierarchical structure of code, making it easier to analyze and manipulate. A function
contains statements. Statements contain expressions. Expressions contain operands.

## Step 2: Binding

Binding connects the usage of names (variables, functions, types) to their declarations.

Binder walks the AST and:
1. Creates symbols for every declaration
2. Builds the scope tree
3. Catches duplicate declarations and undefined references

eg. if you have `x + 1`, the binder figures out what `x` refers to by looking it up in the current scope.

```ts
const x = 1; // declaration of x

function foo() {
  const x = 2; // shadows outer x
  return x; // refers to inner x
}

console.log(x); // refers to outer x
```

### Scopes

A scope is a region where names are visible.

```
Global Scope
├── x: number
└── Function Scope (foo)
    └── x: number (shadows global x)
```

Each scope has a parent pointer. Looking up a name walks up the chain until found (or error if not found).

### Symbols

A symbol is a named entity: a variable, function, class, interface, or type alias.

```ts
const x = 1;        // symbol: x (variable)
function foo() {}   // symbol: foo (function)
interface User {}   // symbol: User (interface)
type ID = string;   // symbol: ID (type alias)
```

Every symbol has:
- A name
- A type
- A location (where it was declared)
- The scope it belongs to

### The Symbol Table

The symbol table is a big lookup table that maps names to symbols.

```
Symbol Table
├── x: { type: number, kind: variable, span: 0..10 }
├── foo: { type: () => void, kind: function, span: 20..40 }
└── User: { type: { name: string }, kind: interface, span: 50..80 }
```

When you write `x + 1`, the binder looks up `x` in the symbol table to find its type.

### Two Namespaces

Typescript has two separate namespaces, values and types.

```ts
interface User { name: string }  // User in type namespace
const User = { create: () => {} }  // User in value namespace

const user: User = User.create();  // both exist simultaneously
//          ^^^^   ^^^^
//          type   value
```

This is why you can have an interface and a variable with the same name. Classes are special — they exist in both namespaces (the class itself is a value, and instances have a type).

## Step 3: Type Checking

For every expression, we determine the type it has. Then we verify that types match where they're expected.

For example, in `const x: number = 42;`, we check that the value `42` is compatible with the declared type `number`.

Type checking is bidirectional.

1. Inference (bottom-up): Compute a type from an expression

   ```ts
   const x = 42; // x has type number
   ```

2. Checking (top-down): Verify an expression matches an expected type
   ```ts
   const x: string = 42; // check 42 against string
   ```

### Type Compatibility

Verifying if one type can be used where another is expected.

- Primitive types: number, string, boolean are compatible with themselves.
- `any` is compatible to/from everything.
- `never` is compatible to everything but nothing is compatible to it.
- `unknown` is assignable to `any` and `unknown`, but not to other types without a type assertion.
- Structural types: Objects are compatible if their properties match in name and type.
  eg. `{ x: number }` is compatible with `{ x: number; y: string }` because it has at least the required properties.
