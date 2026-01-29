// Basic conditional type tests
// Test TS2322: Type '{0}' is not assignable to type '{1}'.

// Simple conditional types
type IsString<T> = T extends string ? true : false;
type A = IsString<string>;  // true
type B = IsString<number>;  // false

// Using conditional types in type annotations
const testA: IsString<"hello"> = true;
const testB: IsString<42> = false;

// Error: boolean expected but got wrong literal
const testC: IsString<string> = false;  // TS2322: Type 'false' is not assignable to type 'true'
const testD: IsString<number> = true;   // TS2322: Type 'true' is not assignable to type 'false'

// Conditional with never
// IsNever<never> returns never (not true or false) because never distributes to empty
type IsNever<T> = T extends never ? true : false;
// Skipping test since IsNever<never> = never, which is special

// Conditional with any (returns union of both branches)
type IsAny<T> = T extends string ? "yes" : "no";
const testAny: IsAny<any> = "yes";  // any can be either branch
