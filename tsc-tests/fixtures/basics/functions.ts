function add(a: number, b: number): number {
    return a + b;
}

const multiply = (a: number, b: number): number => a * b;

const result1 = add(1, 2);
const result2 = multiply(3, 4);

// TS2345
const bad1 = add("one", 2);
const bad2 = multiply(3, "four");

// TS2554
const bad3 = add(1);
const bad4 = add(1, 2, 3);
