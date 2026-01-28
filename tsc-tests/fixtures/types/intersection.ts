interface Named {
    name: string;
}

interface Aged {
    age: number;
}

type Person = Named & Aged;

const person1: Person = { name: "Alice", age: 30 };

// TS2741
const person2: Person = { name: "Bob" };
const person3: Person = { age: 25 };

// TS2322
const person4: Person = { name: "Charlie", age: "thirty" };

function greet(person: Named & Aged): string {
    return `Hello ${person.name}, you are ${person.age} years old`;
}

greet({ name: "Dave", age: 40 });

// TS2345
greet({ name: "Eve" });
