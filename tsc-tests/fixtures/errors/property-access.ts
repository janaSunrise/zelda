interface User {
    name: string;
    age: number;
}

const user: User = { name: "Alice", age: 30 };

// TS2339
const email = user.email;
const address = user.address;
user.phone = "123-456-7890";

const obj = { x: 1, y: 2 };

// TS2339
const z = obj.z;

const str = "hello";
const len = str.length;
const char = str.charAt(0);

const num = 42;

// TS2339
const numLen = num.length;
