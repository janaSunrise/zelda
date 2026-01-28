type Callback<T> = (value: T) => void;
type Predicate<T> = (value: T) => boolean;
type Mapper<T, U> = (value: T) => U;
type Reducer<T, U> = (acc: U, value: T) => U;

function map<T, U>(arr: T[], fn: Mapper<T, U>): U[] {
    return arr.map(fn);
}

function filter<T>(arr: T[], fn: Predicate<T>): T[] {
    return arr.filter(fn);
}

function reduce<T, U>(arr: T[], fn: Reducer<T, U>, initial: U): U {
    return arr.reduce(fn, initial);
}

function forEach<T>(arr: T[], fn: Callback<T>): void {
    arr.forEach(fn);
}

function find<T>(arr: T[], fn: Predicate<T>): T | undefined {
    return arr.find(fn);
}

function every<T>(arr: T[], fn: Predicate<T>): boolean {
    return arr.every(fn);
}

function some<T>(arr: T[], fn: Predicate<T>): boolean {
    return arr.some(fn);
}

function includes<T>(arr: T[], value: T): boolean {
    return arr.includes(value);
}

function indexOf<T>(arr: T[], value: T): number {
    return arr.indexOf(value);
}

function concat<T>(arr1: T[], arr2: T[]): T[] {
    return arr1.concat(arr2);
}

function flatten<T>(arr: T[][]): T[] {
    return arr.flat();
}

function unique<T>(arr: T[]): T[] {
    return [...new Set(arr)];
}

function reverse<T>(arr: T[]): T[] {
    return [...arr].reverse();
}

function sort<T>(arr: T[], fn?: (a: T, b: T) => number): T[] {
    return [...arr].sort(fn);
}

function take<T>(arr: T[], n: number): T[] {
    return arr.slice(0, n);
}

function drop<T>(arr: T[], n: number): T[] {
    return arr.slice(n);
}

function head<T>(arr: T[]): T | undefined {
    return arr[0];
}

function tail<T>(arr: T[]): T[] {
    return arr.slice(1);
}

function last<T>(arr: T[]): T | undefined {
    return arr[arr.length - 1];
}

function init<T>(arr: T[]): T[] {
    return arr.slice(0, -1);
}

function zip<T, U>(arr1: T[], arr2: U[]): [T, U][] {
    return arr1.map((v, i) => [v, arr2[i]]);
}

function unzip<T, U>(arr: [T, U][]): [T[], U[]] {
    return [arr.map(v => v[0]), arr.map(v => v[1])];
}

function partition<T>(arr: T[], fn: Predicate<T>): [T[], T[]] {
    const pass: T[] = [];
    const fail: T[] = [];
    arr.forEach(v => (fn(v) ? pass : fail).push(v));
    return [pass, fail];
}

function groupBy<T, K extends string | number>(arr: T[], fn: (v: T) => K): Record<K, T[]> {
    return arr.reduce((acc, v) => {
        const key = fn(v);
        (acc[key] = acc[key] || []).push(v);
        return acc;
    }, {} as Record<K, T[]>);
}

function countBy<T, K extends string | number>(arr: T[], fn: (v: T) => K): Record<K, number> {
    return arr.reduce((acc, v) => {
        const key = fn(v);
        acc[key] = (acc[key] || 0) + 1;
        return acc;
    }, {} as Record<K, number>);
}

const nums = [1, 2, 3, 4, 5];
const strs = ["a", "b", "c"];

const doubled = map(nums, n => n * 2);
const evens = filter(nums, n => n % 2 === 0);
const sum = reduce(nums, (a, b) => a + b, 0);
const found = find(nums, n => n > 3);
const allPositive = every(nums, n => n > 0);
const hasEven = some(nums, n => n % 2 === 0);
const hasThree = includes(nums, 3);
const pairs = zip(nums, strs);

// TS2345
const bad1 = map(nums, (n: string) => n.toUpperCase());
const bad2 = filter(strs, (s: number) => s > 0);
const bad3 = reduce(nums, (a: string, b: number) => a + b, "");
const bad4 = find(nums, (n: boolean) => n);
const bad5 = every(strs, (s: number) => s > 0);

// TS2322
const bad6: string[] = map(nums, n => n * 2);
const bad7: number[] = filter(strs, s => s.length > 0);
const bad8: string = reduce(nums, (a, b) => a + b, 0);

// TS2554
const bad9 = map(nums);
const bad10 = filter(nums, n => n > 0, "extra");
const bad11 = reduce(nums, (a, b) => a + b);
