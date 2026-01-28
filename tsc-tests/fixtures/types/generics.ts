function identity<T>(x: T): T {
    return x;
}

const num = identity(42);
const str = identity("hello");
const explicitNum: number = identity<number>(42);

// TS2322
const wrongType: string = identity(42);

interface Box<T> {
    value: T;
}

const numBox: Box<number> = { value: 42 };
const strBox: Box<string> = { value: "hello" };

// TS2322
const badBox: Box<number> = { value: "not a number" };

interface Lengthwise {
    length: number;
}

function logLength<T extends Lengthwise>(arg: T): T {
    console.log(arg.length);
    return arg;
}

logLength("hello");
logLength([1, 2, 3]);

// TS2345
logLength(42);
