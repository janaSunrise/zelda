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

Type narrowing reducing the type of a variable from a broader type to a more specific type.

This can be done using type guards, conditional checks, and control flow analysis.

```ts
function process(x: string | number) {
  if (typeof x === "string") {
    // x is narrowed to `string`
    x.toUpperCase();
  } else {
    // x is narrowed to `number`
    x.toFixed();
  }
}
```

Type guards are expressions that perform runtime checks which guarantee the type in a certain scope.

Type guards:

- `typeof x === "string"`: narrows to string
- `x instanceof Date`: narrows to date
- `"prop" in x`: narrows to types with that property

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
