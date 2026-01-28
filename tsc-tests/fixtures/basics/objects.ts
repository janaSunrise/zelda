interface User {
    name: string;
    age: number;
}

const user1: User = { name: "Alice", age: 30 };

// TS2741
const user2: User = { name: "Bob" };

// TS2322
const user3: User = { name: "Charlie", age: "thirty" };

// TS2353
const user4: User = { name: "Dave", age: 25, email: "dave@example.com" };

const userName = user1.name;
const userAge = user1.age;

// TS2339
const userEmail = user1.email;
