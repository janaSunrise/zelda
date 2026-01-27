// @errors: 2322, 2339, 2322, 2322

const x: string | number = true;

interface A { x: number; y: string; }
interface B { x: number; z: boolean; }
function getNotCommon(val: A | B): string {
    return val.y;
}

const partial: { a: number } & { b: string } = { a: 1 };

type Direction = "left" | "right" | "up" | "down";
const invalid: Direction = "diagonal";
