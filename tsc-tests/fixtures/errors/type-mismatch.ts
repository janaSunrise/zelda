// TS2322 - primitive mismatches
const a: number = "string";
const b: string = 123;
const c: boolean = "true";
const d: number = true;
const e: string = false;

// TS2322 - array mismatches
const arr1: number[] = ["one", "two"];
const arr2: string[] = [1, 2, 3];

// TS2322 - return type mismatches
function returnsNumber(): number {
    return "not a number";
}

function returnsString(): string {
    return 42;
}

const num: number = returnsString();
const str: string = returnsNumber();
