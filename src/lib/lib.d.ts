// Zelda built-in type declarations
// Minimal lib.d.ts for essential TypeScript built-ins

// Primitive interfaces
interface String {
    readonly length: number;
    charAt(pos: number): string;
    charCodeAt(index: number): number;
    concat(...strings: string[]): string;
    includes(searchString: string, position?: number): boolean;
    indexOf(searchString: string, position?: number): number;
    lastIndexOf(searchString: string, position?: number): number;
    slice(start?: number, end?: number): string;
    split(separator: string | RegExp, limit?: number): string[];
    substring(start: number, end?: number): string;
    toLowerCase(): string;
    toUpperCase(): string;
    trim(): string;
    trimStart(): string;
    trimEnd(): string;
    replace(searchValue: string | RegExp, replaceValue: string): string;
    startsWith(searchString: string, position?: number): boolean;
    endsWith(searchString: string, endPosition?: number): boolean;
    padStart(maxLength: number, fillString?: string): string;
    padEnd(maxLength: number, fillString?: string): string;
    repeat(count: number): string;
}

interface Number {
    toFixed(fractionDigits?: number): string;
    toExponential(fractionDigits?: number): string;
    toPrecision(precision?: number): string;
    toString(radix?: number): string;
    valueOf(): number;
}

interface Boolean {
    valueOf(): boolean;
}

interface Symbol {
    toString(): string;
    valueOf(): symbol;
}

// Array
interface Array<T> {
    readonly length: number;
    push(...items: T[]): number;
    pop(): T | undefined;
    shift(): T | undefined;
    unshift(...items: T[]): number;
    concat(...items: (T | T[])[]): T[];
    join(separator?: string): string;
    slice(start?: number, end?: number): T[];
    splice(start: number, deleteCount?: number, ...items: T[]): T[];
    indexOf(searchElement: T, fromIndex?: number): number;
    lastIndexOf(searchElement: T, fromIndex?: number): number;
    includes(searchElement: T, fromIndex?: number): boolean;
    find(predicate: (value: T, index: number, array: T[]) => boolean): T | undefined;
    findIndex(predicate: (value: T, index: number, array: T[]) => boolean): number;
    filter(predicate: (value: T, index: number, array: T[]) => boolean): T[];
    map<U>(callbackfn: (value: T, index: number, array: T[]) => U): U[];
    forEach(callbackfn: (value: T, index: number, array: T[]) => void): void;
    reduce<U>(callbackfn: (previousValue: U, currentValue: T, currentIndex: number, array: T[]) => U, initialValue: U): U;
    reduceRight<U>(callbackfn: (previousValue: U, currentValue: T, currentIndex: number, array: T[]) => U, initialValue: U): U;
    every(predicate: (value: T, index: number, array: T[]) => boolean): boolean;
    some(predicate: (value: T, index: number, array: T[]) => boolean): boolean;
    sort(compareFn?: (a: T, b: T) => number): T[];
    reverse(): T[];
    flat<D extends number = 1>(depth?: D): T[];
    flatMap<U>(callback: (value: T, index: number, array: T[]) => U | U[]): U[];
    fill(value: T, start?: number, end?: number): T[];
    copyWithin(target: number, start: number, end?: number): T[];
    at(index: number): T | undefined;
}

interface ReadonlyArray<T> {
    readonly length: number;
    concat(...items: (T | ReadonlyArray<T>)[]): T[];
    join(separator?: string): string;
    slice(start?: number, end?: number): T[];
    indexOf(searchElement: T, fromIndex?: number): number;
    includes(searchElement: T, fromIndex?: number): boolean;
    find(predicate: (value: T, index: number, array: ReadonlyArray<T>) => boolean): T | undefined;
    filter(predicate: (value: T, index: number, array: ReadonlyArray<T>) => boolean): T[];
    map<U>(callbackfn: (value: T, index: number, array: ReadonlyArray<T>) => U): U[];
    forEach(callbackfn: (value: T, index: number, array: ReadonlyArray<T>) => void): void;
    reduce<U>(callbackfn: (previousValue: U, currentValue: T, currentIndex: number, array: ReadonlyArray<T>) => U, initialValue: U): U;
    every(predicate: (value: T, index: number, array: ReadonlyArray<T>) => boolean): boolean;
    some(predicate: (value: T, index: number, array: ReadonlyArray<T>) => boolean): boolean;
    at(index: number): T | undefined;
}

// Map and Set
interface Map<K, V> {
    readonly size: number;
    clear(): void;
    delete(key: K): boolean;
    get(key: K): V | undefined;
    has(key: K): boolean;
    set(key: K, value: V): Map<K, V>;
    forEach(callbackfn: (value: V, key: K, map: Map<K, V>) => void): void;
}

interface Set<T> {
    readonly size: number;
    add(value: T): Set<T>;
    clear(): void;
    delete(value: T): boolean;
    has(value: T): boolean;
    forEach(callbackfn: (value: T, value2: T, set: Set<T>) => void): void;
}

interface WeakMap<K extends object, V> {
    delete(key: K): boolean;
    get(key: K): V | undefined;
    has(key: K): boolean;
    set(key: K, value: V): WeakMap<K, V>;
}

interface WeakSet<T extends object> {
    add(value: T): WeakSet<T>;
    delete(value: T): boolean;
    has(value: T): boolean;
}

// Promise
interface Promise<T> {
    then<TResult1 = T, TResult2 = never>(
        onfulfilled?: ((value: T) => TResult1 | Promise<TResult1>) | null,
        onrejected?: ((reason: any) => TResult2 | Promise<TResult2>) | null
    ): Promise<TResult1 | TResult2>;
    catch<TResult = never>(
        onrejected?: ((reason: any) => TResult | Promise<TResult>) | null
    ): Promise<T | TResult>;
    finally(onfinally?: (() => void) | null): Promise<T>;
}

// Error types
interface Error {
    name: string;
    message: string;
    stack?: string;
}

interface ErrorConstructor {
    new (message?: string): Error;
    (message?: string): Error;
}

declare var Error: ErrorConstructor;

interface TypeError extends Error {}
interface RangeError extends Error {}
interface ReferenceError extends Error {}
interface SyntaxError extends Error {}

// Console
interface Console {
    log(...data: any[]): void;
    error(...data: any[]): void;
    warn(...data: any[]): void;
    info(...data: any[]): void;
    debug(...data: any[]): void;
    trace(...data: any[]): void;
    dir(item?: any): void;
    table(tabularData?: any): void;
    time(label?: string): void;
    timeEnd(label?: string): void;
    timeLog(label?: string, ...data: any[]): void;
    clear(): void;
    count(label?: string): void;
    countReset(label?: string): void;
    group(...data: any[]): void;
    groupCollapsed(...data: any[]): void;
    groupEnd(): void;
    assert(condition?: boolean, ...data: any[]): void;
}

declare var console: Console;

// JSON
interface JSON {
    parse(text: string, reviver?: (key: string, value: any) => any): any;
    stringify(value: any, replacer?: (key: string, value: any) => any, space?: string | number): string;
    stringify(value: any, replacer?: (string | number)[] | null, space?: string | number): string;
}

declare var JSON: JSON;

// Math
interface Math {
    readonly E: number;
    readonly LN10: number;
    readonly LN2: number;
    readonly LOG10E: number;
    readonly LOG2E: number;
    readonly PI: number;
    readonly SQRT1_2: number;
    readonly SQRT2: number;
    abs(x: number): number;
    acos(x: number): number;
    asin(x: number): number;
    atan(x: number): number;
    atan2(y: number, x: number): number;
    ceil(x: number): number;
    cos(x: number): number;
    exp(x: number): number;
    floor(x: number): number;
    log(x: number): number;
    max(...values: number[]): number;
    min(...values: number[]): number;
    pow(x: number, y: number): number;
    random(): number;
    round(x: number): number;
    sin(x: number): number;
    sqrt(x: number): number;
    tan(x: number): number;
    trunc(x: number): number;
    sign(x: number): number;
    cbrt(x: number): number;
    log10(x: number): number;
    log2(x: number): number;
    hypot(...values: number[]): number;
}

declare var Math: Math;

// Date
interface Date {
    toString(): string;
    toDateString(): string;
    toTimeString(): string;
    toLocaleString(): string;
    toLocaleDateString(): string;
    toLocaleTimeString(): string;
    valueOf(): number;
    getTime(): number;
    getFullYear(): number;
    getMonth(): number;
    getDate(): number;
    getDay(): number;
    getHours(): number;
    getMinutes(): number;
    getSeconds(): number;
    getMilliseconds(): number;
    getTimezoneOffset(): number;
    setTime(time: number): number;
    setFullYear(year: number, month?: number, date?: number): number;
    setMonth(month: number, date?: number): number;
    setDate(date: number): number;
    setHours(hours: number, min?: number, sec?: number, ms?: number): number;
    setMinutes(min: number, sec?: number, ms?: number): number;
    setSeconds(sec: number, ms?: number): number;
    setMilliseconds(ms: number): number;
    toISOString(): string;
    toJSON(): string;
}

interface DateConstructor {
    new (): Date;
    new (value: number | string): Date;
    new (year: number, month: number, date?: number, hours?: number, minutes?: number, seconds?: number, ms?: number): Date;
    (): string;
    now(): number;
    parse(s: string): number;
}

declare var Date: DateConstructor;

// RegExp
interface RegExp {
    test(string: string): boolean;
    readonly source: string;
    readonly global: boolean;
    readonly ignoreCase: boolean;
    readonly multiline: boolean;
    lastIndex: number;
    readonly flags: string;
}

interface RegExpConstructor {
    new (pattern: string | RegExp, flags?: string): RegExp;
    (pattern: string | RegExp, flags?: string): RegExp;
}

declare var RegExp: RegExpConstructor;

// Object
interface Object {
    constructor: Function;
    toString(): string;
    toLocaleString(): string;
    valueOf(): Object;
    hasOwnProperty(v: string | number | symbol): boolean;
    isPrototypeOf(v: Object): boolean;
    propertyIsEnumerable(v: string | number | symbol): boolean;
}

interface ObjectConstructor {
    new (value?: any): Object;
    (value?: any): any;
    keys(o: object): string[];
    values<T>(o: { [s: string]: T } | ArrayLike<T>): T[];
    entries<T>(o: { [s: string]: T } | ArrayLike<T>): [string, T][];
    assign<T extends object, U>(target: T, source: U): T & U;
    freeze<T>(o: T): Readonly<T>;
    seal<T>(o: T): T;
    fromEntries<T = any>(entries: Iterable<readonly [string | number | symbol, T]>): { [k: string]: T };
    is(value1: any, value2: any): boolean;
    getOwnPropertyNames(o: any): string[];
    getPrototypeOf(o: any): any;
    setPrototypeOf(o: any, proto: object | null): any;
    create(o: object | null): any;
}

declare var Object: ObjectConstructor;

// Function
interface Function {
    apply(this: Function, thisArg: any, argArray?: any): any;
    call(this: Function, thisArg: any, ...argArray: any[]): any;
    bind(this: Function, thisArg: any, ...argArray: any[]): any;
    toString(): string;
    readonly length: number;
    readonly name: string;
}

interface FunctionConstructor {
    new (...args: string[]): Function;
    (...args: string[]): Function;
}

declare var Function: FunctionConstructor;

// Utility types
type Partial<T> = { [P in keyof T]?: T[P] };
type Required<T> = { [P in keyof T]-?: T[P] };
type Readonly<T> = { readonly [P in keyof T]: T[P] };
type Pick<T, K extends keyof T> = { [P in K]: T[P] };
type Omit<T, K extends keyof any> = Pick<T, Exclude<keyof T, K>>;
type Record<K extends keyof any, T> = { [P in K]: T };
type Exclude<T, U> = T extends U ? never : T;
type Extract<T, U> = T extends U ? T : never;
type NonNullable<T> = T & {};
type Parameters<T extends (...args: any) => any> = T extends (...args: infer P) => any ? P : never;
type ReturnType<T extends (...args: any) => any> = T extends (...args: any) => infer R ? R : any;
type Awaited<T> = T extends Promise<infer U> ? Awaited<U> : T;

// ArrayLike
interface ArrayLike<T> {
    readonly length: number;
    readonly [n: number]: T;
}

// Iterable
interface Iterator<T> {
    next(): { done: boolean; value: T };
}

interface Iterable<T> {
    [Symbol.iterator](): Iterator<T>;
}

// Global values
declare var NaN: number;
declare var Infinity: number;
declare var undefined: undefined;
declare function parseInt(string: string, radix?: number): number;
declare function parseFloat(string: string): number;
declare function isNaN(number: number): boolean;
declare function isFinite(number: number): boolean;
declare function encodeURI(uri: string): string;
declare function encodeURIComponent(uriComponent: string | number | boolean): string;
declare function decodeURI(encodedURI: string): string;
declare function decodeURIComponent(encodedURIComponent: string): string;

// ArrayBuffer
interface ArrayBuffer {
    readonly byteLength: number;
    slice(begin: number, end?: number): ArrayBuffer;
}

interface ArrayBufferConstructor {
    new (byteLength: number): ArrayBuffer;
    isView(arg: any): boolean;
}

declare var ArrayBuffer: ArrayBufferConstructor;
