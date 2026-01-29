// Distributive conditional type tests
// Test TS2322: Type '{0}' is not assignable to type '{1}'.

// Distributive conditional: distributes over unions
type MyExclude<T, U> = T extends U ? never : T;
type MyExtract<T, U> = T extends U ? T : never;

// Test Exclude
type StringOrNumber = string | number;
type ExcludeString = MyExclude<StringOrNumber, string>;  // Should be number

const exclTest1: ExcludeString = 42;      // OK
const exclTest2: ExcludeString = "hello"; // TS2322: Type 'string' is not assignable to type 'number'

// Test Extract
type ExtractString = MyExtract<StringOrNumber, string>;  // Should be string

const extTest1: ExtractString = "hello";  // OK
const extTest2: ExtractString = 42;       // TS2322: Type 'number' is not assignable to type 'string'

// More complex distribution
type MyUnion = "a" | "b" | "c" | 1 | 2;
type OnlyStrings = MyExtract<MyUnion, string>;  // "a" | "b" | "c"
type OnlyNumbers = MyExtract<MyUnion, number>;  // 1 | 2

const strTest: OnlyStrings = "a";  // OK
const numTest: OnlyNumbers = 1;    // OK
const strErr: OnlyStrings = 1;     // TS2322: Type '1' is not assignable to type '"a" | "b" | "c"'
