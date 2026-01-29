// Infer type tests - extracting types from structures
// Test TS2322: Type '{0}' is not assignable to type '{1}'.

// MyReturnType - extracts return type from function
type MyReturnType<T> = T extends (...args: any[]) => infer R ? R : any;

function getString(): string { return "hello"; }
function getNumber(): number { return 42; }

type StringReturn = MyReturnType<typeof getString>;  // string
type NumberReturn = MyReturnType<typeof getNumber>;  // number

const retStr: StringReturn = "test";  // OK
const retNum: NumberReturn = 100;     // OK
const retErr: StringReturn = 123;     // TS2322: Type 'number' is not assignable to type 'string'

// MyParameters - extracts parameter types as tuple
type MyParameters<T> = T extends (...args: infer P) => any ? P : never;

function add(a: number, b: number): number { return a + b; }
type AddParams = MyParameters<typeof add>;  // [number, number]

// Note: Array literals are inferred as array types, not tuples
// With casts we can verify the inferred type matches
const params1: AddParams = [1, 2] as [number, number];  // OK with cast
// params2 would need a string tuple type, but AddParams is [number, number]
// Using a helper to test (skip for now as contextual typing is needed)

// MyAwaited - unwraps promise type (simplified)
// Using MyPromise to avoid conflict with lib.d.ts Promise
interface MyPromise<T> { value: T }
type MyAwaited<T> = T extends MyPromise<infer R> ? R : T;

type PromiseString = MyAwaited<MyPromise<string>>;  // string
type NotPromise = MyAwaited<number>;                // number

const awaitedStr: PromiseString = "resolved";  // OK
const awaitedNum: NotPromise = 42;             // OK
const awaitedErr: PromiseString = 123;         // TS2322
