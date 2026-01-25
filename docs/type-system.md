## Typescript's Type System

### Structural Typing

Typescript uses structural typing, not nominal typing.

> Nominal typing is a type system where type compatibility is determined by explicit declaration, name,
> or inheritance rather than structure.

> Structural typing is a type system where type compatibility is determined by the actual structure, properties,
> and methods of data, rather than explicit declarations or names.

Structural typing means that if two types have the same shape/structure, they are considered compatible, despite
their declared names.

```ts
interface Point {
  x: number;
  y: number;
}
interface Coordinate {
  x: number;
  y: number;
}

const p: Point = { x: 1, y: 2 };
const c: Coordinate = p; // Compatible due to structure
```

### Primitive Types

- `string`: Text
- `number`: All numeric values (int/float)
- `boolean`: True/false
- `null` and `undefined`: Absence of value
- `void`: No return value (for functions)
- `any`: Opt-out of type checking
- `unknown`: Type-safe counterpart to `any`
- `never`: Represents impossible values (e.g., function that never returns)

### Literal Types

Typescript can narrow primitives to exact values.

```ts
const x = "hello"; // type: "hello" literal
let y = "hello"; // type: string (widened)
```

`const` declarations get literal types. `let` gets widened types.

### Union Types

A value that can be one of multiple types.

```ts
type StringOrNum = string | number;

function process(x: StringOrNum) {
  // can only use operations valid for BOTH string and number
  x.toString(); // works!
  x.toFixed(); // error: string doesn't have toFixed
}
```

### Intersection Types

A value that must satisfy multiple types simultaneously.

```ts
type Named = { name: string };
type Aged = { age: number };

type Person = Named & Aged; // { name: string; age: number }
```

### Type Narrowing

Type narrowing is reducing a variable's type from broad to specific based on runtime checks.

```ts
function process(x: string | number) {
  if (typeof x === "string") {
    x.toUpperCase(); // x is string here
  } else {
    x.toFixed(); // x is number here
  }
}
```

The checker sees the `typeof` check and understands that `x` must be a `string` in the true branch and a `number` in the
false branch.

#### Type Guards

Type guards are expressions that narrow types.

- `typeof x === "string"`: narrows to string
- `x instanceof Date`: narrows to date
- `"prop" in x`: narrows to types with that property

#### How Narrowing Actually Works

Think of it like this: at the start of a function, a parameter `x: string | number` could be either. As you add conditions, you're eliminating possibilities.

```ts
function f(x: string | number | null) {
  // x: string | number | null

  if (x === null) {
    return; // early return
  }
  // x: string | number (null eliminated)

  if (typeof x === "string") {
    // x: string
  } else {
    // x: number
  }
}
```

Each branch remembers what checks came before it.

### Control Flow Analysis

This is one of the trickier parts of a type checker.

#### The Problem

Consider this code:

```ts
function f(x: string | number) {
  if (typeof x === "string") {
    console.log(x.toUpperCase());
  }
  console.log(x); // what's x here?
}
```

After the if block, what type is `x`? Still `string | number` - the if block doesn't change the type for code after it, only code
inside it.

What about this?

```ts
function f(x: string | number) {
  if (typeof x === "string") {
    return;
  }
  console.log(x); // what's x here?
}
```

Now `x` is `number`. Because if `x` were a string, we would have returned. The only way to reach that last line is if `x` is
not a string.

The checker needs to understand control flow to get this right.

#### Control Flow Graph

Internally, we model code as a graph of flow nodes. Each node represents a point in the code, and edges represent possible execution paths.

```ts
function f(x: string | number) {
  if (typeof x === "string") {
    return x.length;
  }
  return x.toFixed();
}
```

At each node, we track what type the variable has. When paths split (if/else), we narrow differently in each branch.

#### Implementing It

The basic algorithm:

1. Walk the AST and build a control flow graph
2. At each node, compute the type of each variable based on:
   - The type from the previous node
   - Any narrowing that happens at this node
3. When paths merge (after if/else), compute the union of types from all incoming paths

Here's the tricky part. When you see a variable reference like `x.toUpperCase()`, you need to:

1. Find where `x` was declared
2. Walk backwards through the control flow to find what type `x` has at this point
3. Apply all the narrowing that happened along the way

#### Exhaustiveness

When you've eliminated all possibilities, you get `never`:

```ts
function f(x: string | number) {
  if (typeof x === "string") {
    return;
  }
  if (typeof x === "number") {
    return;
  }
  // x: never (unreachable)
  x; // this line can never execute
}
```

This is useful for exhaustiveness checking:

```ts
type Shape = Circle | Square | Triangle;

function area(s: Shape) {
  switch (s.kind) {
    case "circle":
      return Math.PI * s.r ** 2;
    case "square":
      return s.side ** 2;
    // forgot triangle.
  }

  // s: Triangle (not never, so we missed a case)
  const _exhaustive: never = s; // error: Triangle is not never
}
```

If we handled all cases, `s` would be `never` at the end, and the assignment would work.

#### Loops

Loops complicate things because the type at the start of a loop depends on:

1. The type before the loop
2. The type at the end of the loop body (which loops back)

```ts
function f(x: string | number | null) {
  while (x !== null) {
    // x: string | number (null removed by condition)
    if (typeof x === "string") {
      x = x.length; // x becomes number
    }
    // x: number (either was number, or we converted it)
  }
  // x: null (loop only exits when x is null)
}
```

The checker needs to iterate until the types stabilize (fixed-point iteration).

### Generics

Types with parameters let you write reusable code.

```ts
function identity<T>(x: T): T {
  return x;
}

identity(42); // T: number
identity("hello"); // T: string
```

Constraints limit what types can be substituted.

```ts
function getLength<T extends { length: number }>(x: T): number {
  return x.length;
}

getLength("hello"); // string has length
getLength([1, 2]); // array has length
getLength(42); // error: number has no length
```

### Functions

Functions have parameter and return types.

```ts
type BinaryOp = (a: number, b: number) => number;

const add: BinaryOp = (a, b) => a + b;
```

Variance matters for function compatibility:

- Parameters are contravariant
- Return types are covariant

> Contravariant: Able to accept broader types. eg. you can pass a function that accepts `Animal` where a function that
> accepts `Dog` is expected.

> Covariant: Able to produce more specific types. eg. you can use a function that returns `Dog` where a function that
> returns `Animal` is expected.

### Objects and Interfaces

Objects are collections of properties.

```ts
interface User {
  name: string;
  age: number;
  email?: string; // optional
  readonly id: number; // immutable
}
```

Fresh object literals are checked strictly:

```ts
interface Point {
  x: number;
  y: number;
}

const p: Point = { x: 1, y: 2, z: 3 }; // error: `z` doesn't exist.

const temp = { x: 1, y: 2, z: 3 };
const p2: Point = temp; // no error, excess property check only for fresh literals
```

### Classes

```ts
class Animal {
  constructor(public name: string) {}
}

const a: Animal = new Animal("Rex"); // Animal is a type
const AnimalClass = Animal; // Animal is also a value
```

Typescript uses structural typing for classes.

```ts
class Dog {
  name: string;
  bark() {}
}
class Wolf {
  name: string;
  bark() {}
}

const d: Dog = new Wolf(); // same structure hence compatible
```
