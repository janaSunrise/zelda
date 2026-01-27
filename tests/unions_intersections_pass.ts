let x: string | number = "hello";
x = 42;

const y: string | number | boolean = true;

interface A { x: number; y: string; }
interface B { x: number; z: boolean; }
function getCommon(val: A | B): number {
    return val.x;
}

interface Named { name: string; }
interface Aged { age: number; }
const person: Named & Aged = { name: "Alice", age: 30 };

type Direction = "left" | "right" | "up" | "down";
const dir: Direction = "left";

function narrowString(x: string | number) {
    if (typeof x === "string") {
        const s: string = x;
    }
}

function narrowNumber(x: string | number) {
    if (typeof x === "number") {
        const n: number = x;
    }
}

function narrowNull(x: string | null) {
    if (x !== null) {
        const s: string = x;
    }
}

function narrowUndefined(x: string | undefined) {
    if (x !== undefined) {
        const s: string = x;
    }
}

const nested: (string | number) | boolean = "hello";
