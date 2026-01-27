// Duplicate declarations
const x = 1;
const x = 2;

// Undefined references
const y = undefinedVar;

// Block scoping
{
  const blockVar = 1;
}
const z = blockVar;

// Shadowing
const a = 1;
function test() {
  const a = 2;
  return a;
}

// Type mismatch in assignment
const num: number = "hello";
const str: string = 42;
const bool: boolean = "true";

// Function return type mismatch
function getNumber(): number {
  return "not a number";
}

function getString(): string {
  return 123;
}

// Union types
const union1: string | number = "hello";
const union2: string | number = 42;
const union3: string | number = true;

// Array type mismatch
const nums: number[] = [1, 2, "three"];
const strs: string[] = ["a", "b", 3];

// Object type mismatch
const obj: { name: string; age: number } = {
  name: 123,
  age: "thirty",
};

// Literal types
const lit: "hello" = "world";

// Nested type errors
function process(x: number): string {
  if (x > 0) {
    return x;
  }
  return "negative";
}

// Arrow function return type
const fn: (x: number) => string = (x) => x * 2;

// Multiple errors in one expression
const multi: number = "a" + "b";
