My notes on building a Typescript type checker from scratch.

# Understanding Type Checkers

A type checker finds bugs in your code without running it.

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
