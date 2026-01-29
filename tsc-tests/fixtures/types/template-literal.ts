// Template literal type tests
// Test TS2322: Type '{0}' is not assignable to type '{1}'.

// Basic template literal type
type Greeting = `Hello, ${string}!`;

const greet1: Greeting = "Hello, World!";   // OK
const greet2: Greeting = "Hello, TypeScript!";  // OK
const greet3: Greeting = "Hi, World!";      // TS2322: Does not match pattern

// Template with union - produces all combinations
type EventName = `on${"Click" | "Hover" | "Focus"}`;

const event1: EventName = "onClick";   // OK
const event2: EventName = "onHover";   // OK
const event3: EventName = "onBlur";    // TS2322: Not in union

// Template with literal types
type Color = "red" | "blue" | "green";
type ColorClass = `color-${Color}`;

const class1: ColorClass = "color-red";    // OK
const class2: ColorClass = "color-blue";   // OK
const class3: ColorClass = "color-yellow"; // TS2322: Not valid color

// Uppercase intrinsic (if implemented)
type UpperGreeting = Uppercase<"hello">;  // "HELLO"
const upper: UpperGreeting = "HELLO";     // OK
const upperErr: UpperGreeting = "hello";  // TS2322

// Capitalize intrinsic
type CapName = Capitalize<"world">;  // "World"
const capName: CapName = "World";    // OK
const capErr: CapName = "world";     // TS2322
